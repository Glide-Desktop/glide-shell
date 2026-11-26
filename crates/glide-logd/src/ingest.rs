use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use glide_log_ipc::{
    CompositorEventLine, LogEntry, LogLevel, format_compact_utc, format_iso_utc, now_ms, parse_timestamp_ms,
    redact_sensitive,
};
use tokio::sync::mpsc;

use crate::store::LogStore;

#[derive(Debug, Clone)]
struct WatchedFile {
    path: PathBuf,
    source: &'static str,
    is_jsonl: bool,
    inode: u64,
    offset: u64,
    session_id: String,
    archived_rotated_inodes: HashMap<PathBuf, u64>,
}

impl WatchedFile {
    fn new(path: PathBuf, source: &'static str, is_jsonl: bool) -> Self {
        Self {
            path,
            source,
            is_jsonl,
            inode: 0,
            offset: 0,
            session_id: String::new(),
            archived_rotated_inodes: HashMap::new(),
        }
    }
}

pub fn spawn_file_ingester(state_dir: PathBuf, store: Arc<Mutex<LogStore>>, tx: mpsc::Sender<LogEntry>) {
    tokio::spawn(async move {
        let mut watched = vec![
            WatchedFile::new(state_dir.join("glide.events.jsonl"), "compositor", true),
            WatchedFile::new(state_dir.join("glide-greeter.events.jsonl"), "greeter", true),
            WatchedFile::new(state_dir.join("glide.log"), "compositor-text", false),
            WatchedFile::new(state_dir.join("glide-greeter.log"), "greeter-text", false),
        ];

        if let Ok(guard) = store.lock() {
            for wf in &mut watched {
                let key = format!("offset:{}", wf.path.display());
                if let Ok(Some(saved)) = guard.get_meta(&key)
                    && let Some((ino_s, off_s)) = saved.split_once(':')
                    && let (Ok(ino), Ok(off)) = (ino_s.parse::<u64>(), off_s.parse::<u64>())
                {
                    wf.inode = ino;
                    wf.offset = off;
                }
            }
        }

        loop {
            let mut batch: Vec<LogEntry> = Vec::new();
            let jsonl_active = state_dir.join("glide.events.jsonl").is_file();
            let greeter_jsonl_active = state_dir.join("glide-greeter.events.jsonl").is_file();

            for wf in &mut watched {
                archive_rotated_if_new(&state_dir, wf, &store);

                if !wf.is_jsonl
                    && ((wf.source == "compositor-text" && jsonl_active)
                        || (wf.source == "greeter-text" && greeter_jsonl_active))
                {
                    continue;
                }

                if let Ok(entries) = poll_watched_file(wf) {
                    if !entries.is_empty()
                        && let Ok(guard) = store.lock()
                    {
                        let key = format!("offset:{}", wf.path.display());
                        let _ = guard.set_meta(&key, &format!("{}:{}", wf.inode, wf.offset));
                    }
                    batch.extend(entries);
                }
            }

            for entry in batch {
                if tx.send(entry).await.is_err() {
                    return;
                }
            }

            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
}

fn archive_rotated_if_new(state_dir: &Path, wf: &mut WatchedFile, store: &Arc<Mutex<LogStore>>) {
    let Some(base_name) = wf.path.file_name().and_then(|s| s.to_str()) else {
        return;
    };
    let rotated = state_dir.join(format!("{base_name}.1"));
    let Ok(meta) = fs::metadata(&rotated) else {
        return;
    };
    if !meta.is_file() || meta.len() == 0 {
        return;
    }
    let ino = meta.ino();
    if wf.archived_rotated_inodes.get(&rotated).copied() == Some(ino) {
        return;
    }
    let meta_key = format!("archived:{base_name}:{ino}");
    let already = store.lock().ok().and_then(|g| g.get_meta(&meta_key).ok()).flatten().is_some();
    if already {
        wf.archived_rotated_inodes.insert(rotated, ino);
        return;
    }

    let archive_name = format!("{base_name}-{}-ino{ino}", format_compact_utc(now_ms()));
    if let Ok(guard) = store.lock()
        && guard.archive_file(&rotated, &archive_name).is_ok()
    {
        let _ = guard.set_meta(&meta_key, "1");
        wf.archived_rotated_inodes.insert(rotated, ino);
    }
}

fn poll_watched_file(wf: &mut WatchedFile) -> std::io::Result<Vec<LogEntry>> {
    let meta = fs::metadata(&wf.path)?;
    if !meta.is_file() {
        return Ok(Vec::new());
    }
    let ino = meta.ino();
    let len = meta.len();

    if ino != wf.inode || len < wf.offset {
        wf.inode = ino;
        wf.offset = 0;
        wf.session_id = format!("{}-{}-ino{ino}", wf.source, format_compact_utc(now_ms()));
    }
    if len == wf.offset {
        return Ok(Vec::new());
    }
    if wf.session_id.is_empty() {
        wf.session_id = format!("{}-{}-ino{ino}", wf.source, format_compact_utc(now_ms()));
    }

    let mut file = File::open(&wf.path)?;
    file.seek(SeekFrom::Start(wf.offset))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut entries = Vec::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        if !line.ends_with('\n') {
            break;
        }
        wf.offset += u64::try_from(bytes).unwrap_or(0);
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }

        let parsed = if wf.is_jsonl {
            parse_jsonl_line(trimmed, wf.source, &mut wf.session_id)
        } else {
            parse_plain_log_line(trimmed, wf.source, &wf.session_id)
        };
        if let Some(entry) = parsed {
            entries.push(entry);
        }
    }

    Ok(entries)
}

#[must_use]
pub fn parse_jsonl_line(line: &str, source: &str, current_session_id: &mut String) -> Option<LogEntry> {
    let raw: CompositorEventLine = serde_json::from_str(line).ok()?;
    if raw.message == "starting glide" {
        let prefix = if source == "greeter" { "greeter" } else { "session" };
        *current_session_id = format!("{prefix}-{}-pid{}", format_compact_utc(raw.ts_ms), raw.pid);
    } else if current_session_id.is_empty() {
        *current_session_id = format!("{source}-{}-pid{}", format_compact_utc(raw.ts_ms), raw.pid);
    }

    let mut fields = serde_json::Map::new();
    for (k, v) in raw.fields {
        let cleaned = match v {
            serde_json::Value::String(s) => serde_json::Value::String(redact_sensitive(&s)),
            other => other,
        };
        fields.insert(k, cleaned);
    }

    Some(LogEntry {
        id: 0,
        ts_ms: raw.ts_ms,
        ts_iso: if raw.ts_iso.is_empty() { format_iso_utc(raw.ts_ms) } else { raw.ts_iso },
        session_id: current_session_id.clone(),
        source: source.to_owned(),
        level: raw.level,
        target: raw.target,
        thread: raw.thread,
        pid: Some(raw.pid),
        message: redact_sensitive(&raw.message),
        fields,
    })
}

#[must_use]
pub fn parse_plain_log_line(line: &str, source: &str, session_id: &str) -> Option<LogEntry> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    let first = parts.next()?;
    let (ts_ms, ts_iso, rest) = if let Some(ms) = parse_timestamp_ms(first) {
        let rem = trimmed[first.len()..].trim_start();
        (ms, format_iso_utc(ms), rem)
    } else {
        let ms = now_ms();
        (ms, format_iso_utc(ms), trimmed)
    };

    let mut rest_parts = rest.split_whitespace();
    let maybe_level = rest_parts.next().and_then(LogLevel::parse_lossy);
    let (level, after_level) = if let Some(lvl) = maybe_level {
        let token = rest.split_whitespace().next().unwrap_or("");
        (lvl, rest[token.len()..].trim_start())
    } else {
        (LogLevel::Info, rest)
    };

    let (target, message) = if let Some((head, msg)) = after_level.split_once(": ") {
        let tgt = head.split_whitespace().last().unwrap_or(source);
        (tgt.to_owned(), msg.to_owned())
    } else {
        (source.to_owned(), after_level.to_owned())
    };

    Some(LogEntry {
        id: 0,
        ts_ms,
        ts_iso,
        session_id: session_id.to_owned(),
        source: source.to_owned(),
        level,
        target,
        thread: None,
        pid: None,
        message: redact_sensitive(&message),
        fields: serde_json::Map::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jsonl_and_updates_session_id_on_header() {
        let line = r#"{"ts_ms":1700000000000,"ts_iso":"2023-11-14T22:13:20.000Z","level":"INFO","target":"glide::logging","thread":"main","pid":777,"message":"starting glide","fields":{"version":"0.1.4"}}"#;
        let mut session = String::new();
        let entry = parse_jsonl_line(line, "compositor", &mut session).unwrap();
        assert_eq!(entry.level, LogLevel::Info);
        assert!(session.starts_with("session-"));
        assert!(session.ends_with("-pid777"));
        assert_eq!(entry.session_id, session);
    }

    #[test]
    fn parses_plain_tracing_line() {
        let line = "2026-11-22T08:00:00.123456Z  WARN main glide::gpu: pageflip slow";
        let entry = parse_plain_log_line(line, "compositor-text", "sess-1").unwrap();
        assert_eq!(entry.level, LogLevel::Warn);
        assert_eq!(entry.target, "glide::gpu");
        assert_eq!(entry.message, "pageflip slow");
    }
}
