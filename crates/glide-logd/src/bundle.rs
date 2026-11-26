use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use flate2::{Compression, write::GzEncoder};
use glide_log_ipc::{
    CrashBundleDetail, CrashBundleSummary, LogEntry, LogLevel, format_compact_utc, format_iso_utc, is_valid_bundle_id,
    now_ms, redact_sensitive,
};
use serde::{Deserialize, Serialize};

use crate::{journal, store::dir_recursive_bytes};

const DEFAULT_MAX_FILE_BYTES: usize = 256 * 1024;
const BUNDLE_TAIL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashBundleManifest {
    pub id: String,
    pub created_at_ms: i64,
    pub created_at_iso: String,
    pub reason: String,
    pub session_id: Option<String>,
    pub version: Option<String>,
    pub kernel: String,
    pub hostname: String,
    pub gpu_summary: Option<String>,
    pub files: Vec<String>,
}

#[must_use]
pub fn detect_trigger(entry: &LogEntry) -> Option<String> {
    let msg_lower = entry.message.to_ascii_lowercase();
    let target_lower = entry.target.to_ascii_lowercase();

    if msg_lower.contains("panic in thread") || msg_lower.contains("panicked at") {
        return Some("panic".to_owned());
    }

    if entry.source == "journal-kernel" {
        if msg_lower.contains("pcie bus error") || msg_lower.contains("aer:") || msg_lower.contains("aer ") {
            return Some("kernel-pcie-aer".to_owned());
        }
        if msg_lower.contains("out of memory")
            || msg_lower.contains("oom-killer")
            || msg_lower.contains("killed process")
        {
            return Some("kernel-oom".to_owned());
        }
        if msg_lower.contains("xid")
            || msg_lower.contains("nvrm")
            || msg_lower.contains("gpu hang")
            || msg_lower.contains("gpu reset")
            || msg_lower.contains("drm:.*error")
            || (msg_lower.contains("drm") && entry.level >= LogLevel::Error)
            || msg_lower.contains("hung_task")
        {
            return Some("kernel-gpu-error".to_owned());
        }
        return None;
    }

    if entry.source == "journal-user"
        && (msg_lower.contains("failed with result")
            || msg_lower.contains("main process exited")
            || msg_lower.contains("core-dump"))
        && (msg_lower.contains("glide") || target_lower.contains("glide"))
    {
        return Some("service-crash".to_owned());
    }

    if (target_lower.contains("gpu") || target_lower.contains("drm"))
        && (entry.level >= LogLevel::Error
            || msg_lower.contains("drmevent::error")
            || msg_lower.contains("pageflip failed")
            || msg_lower.contains("atomic commit failed"))
    {
        return Some("drm-gpu-error".to_owned());
    }

    if (entry.source == "compositor" || entry.source == "greeter") && entry.level >= LogLevel::Error {
        return Some("compositor-error".to_owned());
    }

    None
}

pub fn create_crash_bundle(
    bundle_dirs: &[PathBuf], state_dir: &Path, reason: &str, session_id: Option<String>, version: Option<String>,
    gpu_summary: Option<String>, recent_events: &[LogEntry],
) -> Result<CrashBundleSummary> {
    let ts_ms = now_ms();
    let ts_iso = format_iso_utc(ts_ms);
    let slug = sanitize_reason(reason);
    let id = format!("glide-bundle-{}-{slug}", format_compact_utc(ts_ms));

    let mut events_buf = String::new();
    for ev in recent_events {
        if let Ok(line) = serde_json::to_string(ev) {
            events_buf.push_str(&redact_sensitive(&line));
            events_buf.push('\n');
        }
    }
    if events_buf.is_empty() {
        events_buf = read_redacted_tail(&state_dir.join("glide.events.jsonl"), BUNDLE_TAIL_BYTES);
    }

    let compositor_log = read_redacted_tail(&state_dir.join("glide.log"), BUNDLE_TAIL_BYTES);
    let journal_user = redact_sensitive(&journal::snapshot_user_journal(500));
    let journal_kernel = redact_sensitive(&journal::snapshot_kernel_journal(500));

    let file_names = vec![
        "manifest.json".to_owned(),
        "compositor.events.jsonl".to_owned(),
        "compositor.log".to_owned(),
        "journal-user.json".to_owned(),
        "journal-kernel.txt".to_owned(),
    ];

    let manifest = CrashBundleManifest {
        id: id.clone(),
        created_at_ms: ts_ms,
        created_at_iso: ts_iso.clone(),
        reason: reason.to_owned(),
        session_id: session_id.clone(),
        version: version.clone(),
        kernel: kernel_uname(),
        hostname: hostname(),
        gpu_summary: gpu_summary.clone(),
        files: file_names.clone(),
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)?;

    let payload: Vec<(&str, &[u8])> = vec![
        ("manifest.json", manifest_json.as_bytes()),
        ("compositor.events.jsonl", events_buf.as_bytes()),
        ("compositor.log", compositor_log.as_bytes()),
        ("journal-user.json", journal_user.as_bytes()),
        ("journal-kernel.txt", journal_kernel.as_bytes()),
    ];

    let mut primary_dir: Option<PathBuf> = None;
    let mut primary_archive: Option<PathBuf> = None;
    let mut primary_bytes = 0_u64;

    for base in bundle_dirs {
        if let Ok((bdir, archive, bytes)) = write_bundle_to_dir(base, &id, &payload)
            && primary_dir.is_none()
        {
            primary_dir = Some(bdir);
            primary_archive = Some(archive);
            primary_bytes = bytes;
        }
    }

    let Some(dir_path) = primary_dir else {
        bail!("no writable crash bundle directory available in {bundle_dirs:?}");
    };

    Ok(CrashBundleSummary {
        id,
        created_at_ms: ts_ms,
        created_at_iso: ts_iso,
        reason: reason.to_owned(),
        session_id,
        version,
        kernel: Some(manifest.kernel),
        gpu_summary,
        dir_path: dir_path.to_string_lossy().into_owned(),
        archive_path: primary_archive.map(|p| p.to_string_lossy().into_owned()),
        total_bytes: primary_bytes,
        files: file_names,
    })
}

fn write_bundle_to_dir(base: &Path, id: &str, payload: &[(&str, &[u8])]) -> Result<(PathBuf, PathBuf, u64)> {
    fs::create_dir_all(base)?;
    let bundle_dir = base.join(id);
    fs::create_dir_all(&bundle_dir)?;
    let _ = fs::set_permissions(&bundle_dir, fs::Permissions::from_mode(0o755));

    for (name, bytes) in payload {
        let file_path = bundle_dir.join(name);
        let mut f = File::create(&file_path)?;
        f.write_all(bytes)?;
        f.flush()?;
        let _ = f.sync_all();
        let _ = fs::set_permissions(&file_path, fs::Permissions::from_mode(0o644));
    }

    let archive_path = base.join(format!("{id}.tar.gz"));
    write_tar_gz(&archive_path, id, payload)?;
    let _ = fs::set_permissions(&archive_path, fs::Permissions::from_mode(0o644));

    let total_bytes = dir_recursive_bytes(&bundle_dir) + fs::metadata(&archive_path).map_or(0, |m| m.len());
    Ok((bundle_dir, archive_path, total_bytes))
}

pub fn write_tar_gz(dest: &Path, root_dir: &str, files: &[(&str, &[u8])]) -> Result<()> {
    let tmp = dest.with_extension("tar.gz.tmp");
    let file = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    let mut gz = GzEncoder::new(file, Compression::default());
    let mtime = u64::try_from(now_ms().div_euclid(1_000)).unwrap_or(0);

    for (name, data) in files {
        let entry_name = format!("{root_dir}/{name}");
        let header = build_ustar_header(&entry_name, data.len() as u64, mtime)?;
        gz.write_all(&header)?;
        gz.write_all(data)?;
        let rem = data.len() % 512;
        if rem != 0 {
            let pad = [0_u8; 512];
            gz.write_all(&pad[..512 - rem])?;
        }
    }
    gz.write_all(&[0_u8; 1024])?;
    let file = gz.finish()?;
    let _ = file.sync_all();
    fs::rename(&tmp, dest)?;
    Ok(())
}

fn build_ustar_header(path: &str, size: u64, mtime: u64) -> Result<[u8; 512]> {
    let mut header = [0_u8; 512];
    let path_bytes = path.as_bytes();
    if path_bytes.len() <= 100 {
        header[..path_bytes.len()].copy_from_slice(path_bytes);
    } else if let Some(split) = path.rfind('/') {
        let (prefix, name) = path.split_at(split);
        let name = &name[1..];
        if prefix.len() > 155 || name.len() > 100 {
            bail!("tar entry path too long: {path}");
        }
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    } else {
        bail!("tar entry path too long: {path}");
    }

    write_octal(&mut header[100..108], 0o644);
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    write_octal(&mut header[124..136], size);
    write_octal(&mut header[136..148], mtime);
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");

    let checksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    let cksum_str = format!("{checksum:06o}\0 ");
    header[148..156].copy_from_slice(cksum_str.as_bytes());
    Ok(header)
}

fn write_octal(slot: &mut [u8], value: u64) {
    let width = slot.len() - 1;
    let formatted = format!("{value:0width$o}");
    let bytes = formatted.as_bytes();
    let start = bytes.len().saturating_sub(width);
    slot[..width].copy_from_slice(&bytes[start..]);
    slot[width] = 0;
}

#[must_use]
pub fn list_bundles(bundle_dirs: &[PathBuf]) -> Vec<CrashBundleSummary> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for base in bundle_dirs {
        let Ok(entries) = fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !is_valid_bundle_id(name) || !seen.insert(name.to_owned()) {
                continue;
            }
            if let Some(summary) = read_bundle_summary(base, &path, name) {
                out.push(summary);
            }
        }
    }

    out.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then_with(|| b.id.cmp(&a.id)));
    out
}

fn read_bundle_summary(base: &Path, bundle_dir: &Path, id: &str) -> Option<CrashBundleSummary> {
    let manifest_path = bundle_dir.join("manifest.json");
    let raw = fs::read_to_string(&manifest_path).ok()?;
    let manifest: CrashBundleManifest = serde_json::from_str(&raw).ok()?;
    let archive = base.join(format!("{id}.tar.gz"));
    let archive_path = archive.is_file().then(|| archive.to_string_lossy().into_owned());
    let total_bytes = dir_recursive_bytes(bundle_dir) + fs::metadata(&archive).map_or(0, |m| m.len());
    Some(CrashBundleSummary {
        id: manifest.id,
        created_at_ms: manifest.created_at_ms,
        created_at_iso: manifest.created_at_iso,
        reason: manifest.reason,
        session_id: manifest.session_id,
        version: manifest.version,
        kernel: Some(manifest.kernel),
        gpu_summary: manifest.gpu_summary,
        dir_path: bundle_dir.to_string_lossy().into_owned(),
        archive_path,
        total_bytes,
        files: manifest.files,
    })
}

pub fn read_bundle(bundle_dirs: &[PathBuf], id: &str, max_file_bytes: Option<usize>) -> Result<CrashBundleDetail> {
    if !is_valid_bundle_id(id) {
        bail!("invalid crash bundle id: {id:?}");
    }
    let cap = max_file_bytes.unwrap_or(DEFAULT_MAX_FILE_BYTES).clamp(1024, 4 * 1024 * 1024);

    for base in bundle_dirs {
        let dir = base.join(id);
        if dir.is_dir()
            && let Some(summary) = read_bundle_summary(base, &dir, id)
        {
            let manifest_json = fs::read_to_string(dir.join("manifest.json")).unwrap_or_default();
            let compositor_events_tail = read_redacted_tail(&dir.join("compositor.events.jsonl"), cap);
            let compositor_log_tail = read_redacted_tail(&dir.join("compositor.log"), cap);
            let journal_user_tail = read_redacted_tail(&dir.join("journal-user.json"), cap);
            let journal_kernel_tail = read_redacted_tail(&dir.join("journal-kernel.txt"), cap);
            return Ok(CrashBundleDetail {
                summary,
                manifest_json,
                compositor_events_tail,
                compositor_log_tail,
                journal_user_tail,
                journal_kernel_tail,
            });
        }
    }
    bail!("crash bundle not found: {id}");
}

fn read_redacted_tail(path: &Path, max_bytes: usize) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let Ok(meta) = file.metadata() else {
        return String::new();
    };
    let len = meta.len();
    let cap = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if len > cap {
        let _ = file.seek(SeekFrom::Start(len - cap));
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let trimmed = if len > cap { text.find('\n').map_or(text.as_ref(), |idx| &text[idx + 1..]) } else { text.as_ref() };
    redact_sensitive(trimmed)
}

fn sanitize_reason(reason: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in reason.chars().take(48) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    let trimmed = slug.trim_end_matches('-');
    if trimmed.is_empty() { "manual".to_owned() } else { trimmed.to_owned() }
}

fn kernel_uname() -> String {
    // SAFETY: `utsname` is a plain C struct safe to zero-initialize.
    let mut name: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `name` points to valid writable storage.
    if unsafe { libc::uname(&raw mut name) } != 0 {
        return "unknown".to_owned();
    }
    let field = |chars: &[libc::c_char]| {
        let bytes: Vec<u8> = chars.iter().take_while(|c| **c != 0).map(|c| c.to_ne_bytes()[0]).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    format!("{} {} {}", field(&name.sysname), field(&name.release), field(&name.machine))
}

fn hostname() -> String {
    fs::read_to_string("/etc/hostname").map_or_else(|_| "unknown".to_owned(), |s| s.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;

    #[test]
    fn creates_reads_and_validates_tar_gz_bundle() {
        let root = std::env::temp_dir().join(format!("glide-logd-bundle-{}-{}", std::process::id(), now_ms()));
        let state_dir = root.join("state");
        let bdir = root.join("bundles");
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(state_dir.join("glide.log"), "password=topsecret\nnormal line\n").unwrap();

        let ev = LogEntry {
            id: 1,
            ts_ms: now_ms(),
            ts_iso: format_iso_utc(now_ms()),
            session_id: "sess-1".into(),
            source: "compositor".into(),
            level: LogLevel::Error,
            target: "glide::gpu".into(),
            thread: Some("main".into()),
            pid: Some(42),
            message: "DRM pageflip failed token=abc123".into(),
            fields: serde_json::Map::new(),
        };

        assert_eq!(detect_trigger(&ev).as_deref(), Some("drm-gpu-error"));

        let summary = create_crash_bundle(
            std::slice::from_ref(&bdir),
            &state_dir,
            "DRM pageflip failed",
            Some("sess-1".into()),
            Some("0.1.4".into()),
            Some("NVIDIA".into()),
            &[ev],
        )
        .unwrap();

        assert!(Path::new(&summary.dir_path).is_dir());
        let archive = PathBuf::from(summary.archive_path.as_ref().unwrap());
        assert!(archive.is_file());

        let mut decoder = GzDecoder::new(File::open(&archive).unwrap());
        let mut tar_bytes = Vec::new();
        decoder.read_to_end(&mut tar_bytes).unwrap();
        let tar_text = String::from_utf8_lossy(&tar_bytes);
        assert!(tar_text.contains("manifest.json"));
        assert!(!tar_text.contains("topsecret"));
        assert!(!tar_text.contains("abc123"));

        let listed = list_bundles(std::slice::from_ref(&bdir));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, summary.id);

        let detail = read_bundle(std::slice::from_ref(&bdir), &summary.id, None).unwrap();
        assert!(detail.compositor_log_tail.contains("[REDACTED]"));
        assert!(!detail.compositor_log_tail.contains("topsecret"));

        let _ = fs::remove_dir_all(&root);
    }
}
