use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use glide_log_ipc::{
    CompositorEventLine, CrashBundleDetail, CrashBundleSummary, DEFAULT_IO_TIMEOUT, LogEntry, LogLevel, LogQuery,
    LogRequest, LogResponse, RetentionConfig, SessionInfo, StorageStats, crash_bundle_dirs, exchange_at,
    format_compact_utc, format_iso_utc, is_valid_bundle_id, logs_dir, now_ms, parse_timestamp_ms, read_line_capped,
    redact_sensitive, socket_path, state_dir,
};

use crate::{is_help, printable};

pub const LOGS_USAGE: &str = r"glidectl logs — Inspect Glide logs, sessions, crash bundles, retention, and MCP

USAGE:
    glidectl [--json] logs <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    query [OPTIONS]                 Query logs (daemon or on-disk fallback)
        --level <TRACE|DEBUG|INFO|WARN|ERROR>
        --target <SUBSTR>
        --source <compositor|greeter|journal-user|journal-kernel>
        --session <SESSION_ID>
        --since <ISO_OR_MS>
        --until <ISO_OR_MS>
        --grep <TEXT>
        --limit <N>
    sessions [--limit <N>]          List recorded Glide sessions
    bundles [list]                  List crash bundles
    bundles show <ID>               Show manifest and log tails of a crash bundle
    bundles create [REASON]         Create a new crash bundle immediately
    bundles export <ID> <DIR>       Copy a crash bundle (.tar.gz + dir) to <DIR> (e.g. USB mount)
    tail [OPTIONS]                  Stream live log entries (Ctrl+C to stop)
    retention [show]                Show retention policy and storage usage
    retention set [OPTIONS]         Update retention policy
        --retain-all <true|false>
        --max-days <N>
        --max-size-mb <N>
        --crash-bundle-keep <N>
    retention prune [--max-days <N>] [--max-size-mb <N>]
                                    Prune old logs/archives now
    gpu [--session <ID>] [--limit <N>]
                                    Show GPU/DRM/kernel error summary
    stats                           Show log storage statistics
    mcp                             Start the stdio JSON-RPC 2.0 MCP server (glide-logd --mcp)
";

pub fn run_logs(out: &mut impl Write, args: &[String], json_output: bool) -> Result<i32> {
    if args.is_empty() || is_help(&args[0]) {
        write!(out, "{LOGS_USAGE}")?;
        return Ok(0);
    }

    match args[0].as_str() {
        "mcp" => run_mcp(),
        "tail" => {
            let query = parse_log_query(&args[1..], false)?;
            run_tail(out, &query, json_output)?;
            Ok(0)
        }
        "query" => {
            let query = parse_log_query(&args[1..], true)?;
            let resp = request_with_fallback(LogRequest::QueryLogs { query })?;
            emit_response(out, &resp, json_output)
        }
        "sessions" => {
            let limit = parse_opt_usize(&args[1..], "--limit")?;
            let resp = request_with_fallback(LogRequest::ListSessions { limit })?;
            emit_response(out, &resp, json_output)
        }
        "gpu" => {
            let session_id = parse_opt_string(&args[1..], "--session")?;
            let limit = parse_opt_usize(&args[1..], "--limit")?;
            let resp = request_with_fallback(LogRequest::GetGpuSummary { session_id, limit })?;
            emit_response(out, &resp, json_output)
        }
        "stats" => {
            let resp = request_with_fallback(LogRequest::GetStats)?;
            emit_response(out, &resp, json_output)
        }
        "bundles" | "bundle" => run_bundles(out, &args[1..], json_output),
        "retention" => run_retention(out, &args[1..], json_output),
        other => bail!("unknown logs subcommand: {other}\n\n{LOGS_USAGE}"),
    }
}

fn run_mcp() -> Result<i32> {
    let binary = find_logd_binary();
    let status = Command::new(&binary)
        .arg("--mcp")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch {} --mcp", binary.display()))?;
    Ok(status.code().unwrap_or(1))
}

fn find_logd_binary() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("glide-logd");
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from("glide-logd")
}

fn run_bundles(out: &mut impl Write, args: &[String], json_output: bool) -> Result<i32> {
    match args.first().map(String::as_str) {
        None | Some("list" | "ls") => {
            let resp = request_with_fallback(LogRequest::ListCrashBundles)?;
            emit_response(out, &resp, json_output)
        }
        Some("show" | "read") => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("Usage: glidectl logs bundles show <ID>"))?;
            let max_file_bytes = parse_opt_usize(&args[2..], "--max-bytes")?;
            let resp = request_with_fallback(LogRequest::GetCrashBundle { id: id.clone(), max_file_bytes })?;
            emit_response(out, &resp, json_output)
        }
        Some("create") => {
            let reason = if args.len() > 1 { args[1..].join(" ") } else { "manual".to_owned() };
            let resp = request_with_fallback(LogRequest::CreateCrashBundle { reason })?;
            emit_response(out, &resp, json_output)
        }
        Some("export") => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("Usage: glidectl logs bundles export <ID> <DIR>"))?;
            let dest = args.get(2).ok_or_else(|| anyhow::anyhow!("Usage: glidectl logs bundles export <ID> <DIR>"))?;
            let exported = export_bundle_to_dir(id, Path::new(dest))?;
            if json_output {
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "type": "exported",
                        "id": id,
                        "destination": exported,
                    }))?
                )?;
            } else {
                writeln!(out, "Exported {id} to {}", exported.display())?;
            }
            out.flush()?;
            Ok(0)
        }
        Some(other) if is_help(other) => {
            write!(out, "{LOGS_USAGE}")?;
            Ok(0)
        }
        Some(other) => bail!("unknown bundles subcommand: {other}"),
    }
}

fn run_retention(out: &mut impl Write, args: &[String], json_output: bool) -> Result<i32> {
    match args.first().map(String::as_str) {
        None | Some("show" | "get") => {
            let resp = request_with_fallback(LogRequest::GetRetention)?;
            emit_response(out, &resp, json_output)
        }
        Some("set") => {
            let mut cfg = match request_with_fallback(LogRequest::GetRetention)? {
                LogResponse::Retention { config, .. } => config,
                _ => RetentionConfig::default(),
            };
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--retain-all" => {
                        let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--retain-all requires true|false"))?;
                        cfg.retain_all = parse_bool(val)?;
                        idx += 2;
                    }
                    "--max-days" => {
                        let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--max-days requires a number"))?;
                        cfg.max_days = val.parse()?;
                        idx += 2;
                    }
                    "--max-size-mb" => {
                        let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--max-size-mb requires a number"))?;
                        cfg.max_size_mb = val.parse()?;
                        idx += 2;
                    }
                    "--crash-bundle-keep" => {
                        let val = args
                            .get(idx + 1)
                            .ok_or_else(|| anyhow::anyhow!("--crash-bundle-keep requires a number"))?;
                        cfg.crash_bundle_keep = val.parse()?;
                        idx += 2;
                    }
                    other => bail!("unknown option for retention set: {other}"),
                }
            }
            let resp = request_with_fallback(LogRequest::SetRetention { config: cfg })?;
            emit_response(out, &resp, json_output)
        }
        Some("prune") => {
            let max_days = parse_opt_u32(&args[1..], "--max-days")?;
            let max_size_mb = parse_opt_u64(&args[1..], "--max-size-mb")?;
            let resp = request_with_fallback(LogRequest::PruneNow { max_days, max_size_mb })?;
            emit_response(out, &resp, json_output)
        }
        Some(other) => bail!("unknown retention subcommand: {other}"),
    }
}

fn run_tail(out: &mut impl Write, query: &LogQuery, json_output: bool) -> Result<()> {
    if let Some(sock) = socket_path()
        && sock.exists()
        && let Ok((mut reader, resp)) = exchange_at(&sock, &LogRequest::TailLogs { query: query.clone() }, DEFAULT_IO_TIMEOUT)
        && matches!(resp, LogResponse::Subscribed)
    {
        reader.get_ref().set_read_timeout(None)?;
        let mut line = String::new();
        loop {
            line.clear();
            if read_line_capped(&mut reader, &mut line)? == 0 {
                return Ok(());
            }
            if let Ok(LogResponse::Entry { entry }) = serde_json::from_str::<LogResponse>(line.trim()) {
                print_entry(out, &entry, json_output)?;
                out.flush()?;
            }
        }
    }

    let Some(sdir) = state_dir() else {
        bail!("glide-logd is not running and XDG_STATE_HOME/HOME is unset");
    };
    let path = sdir.join("glide.events.jsonl");
    if !json_output {
        eprintln!("glide-logd offline; tailing {} directly (Ctrl+C to stop)...", path.display());
    }
    let mut offset = fs::metadata(&path).map_or(0, |m| m.len());
    let mut session = String::new();
    loop {
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() < offset {
                offset = 0;
            }
            if meta.len() > offset
                && let Ok(mut file) = File::open(&path)
                && file.seek(SeekFrom::Start(offset)).is_ok()
            {
                let mut reader = BufReader::new(file);
                let mut line = String::new();
                while let Ok(n) = reader.read_line(&mut line) {
                    if n == 0 || !line.ends_with('\n') {
                        break;
                    }
                    offset += u64::try_from(n).unwrap_or(0);
                    if let Some(entry) = parse_jsonl_line(line.trim(), "compositor", &mut session)
                        && query.matches(&entry)
                    {
                        print_entry(out, &entry, json_output)?;
                        out.flush()?;
                    }
                    line.clear();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub fn parse_log_query(args: &[String], newest_first_default: bool) -> Result<LogQuery> {
    let mut query = LogQuery { newest_first: newest_first_default, ..LogQuery::default() };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--level" | "--min-level" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--level requires a value"))?;
                query.min_level =
                    Some(LogLevel::parse_lossy(val).ok_or_else(|| anyhow::anyhow!("invalid level: {val}"))?);
                idx += 2;
            }
            "--target" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--target requires a value"))?;
                query.target = Some(val.clone());
                idx += 2;
            }
            "--source" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--source requires a value"))?;
                query.source = Some(val.clone());
                idx += 2;
            }
            "--session" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--session requires a value"))?;
                query.session_id = Some(val.clone());
                idx += 2;
            }
            "--since" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--since requires a timestamp"))?;
                query.since_ms = Some(
                    parse_timestamp_ms(val)
                        .ok_or_else(|| anyhow::anyhow!("invalid --since timestamp (use ISO-8601 UTC or epoch ms): {val}"))?,
                );
                idx += 2;
            }
            "--until" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--until requires a timestamp"))?;
                query.until_ms = Some(
                    parse_timestamp_ms(val)
                        .ok_or_else(|| anyhow::anyhow!("invalid --until timestamp (use ISO-8601 UTC or epoch ms): {val}"))?,
                );
                idx += 2;
            }
            "--grep" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--grep requires text"))?;
                query.grep = Some(val.clone());
                idx += 2;
            }
            "--limit" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--limit requires a number"))?;
                query.limit = Some(val.parse()?);
                idx += 2;
            }
            "--offset" => {
                let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("--offset requires a number"))?;
                query.offset = Some(val.parse()?);
                idx += 2;
            }
            "--oldest-first" => {
                query.newest_first = false;
                idx += 1;
            }
            "--newest-first" => {
                query.newest_first = true;
                idx += 1;
            }
            other => bail!("unknown logs option: {other}"),
        }
    }
    Ok(query)
}

fn parse_opt_usize(args: &[String], flag: &str) -> Result<Option<usize>> {
    let mut idx = 0;
    while idx < args.len() {
        if args[idx] == flag {
            let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))?;
            return Ok(Some(val.parse()?));
        }
        idx += 1;
    }
    Ok(None)
}

fn parse_opt_u32(args: &[String], flag: &str) -> Result<Option<u32>> {
    Ok(parse_opt_usize(args, flag)?.map(|v| u32::try_from(v).unwrap_or(u32::MAX)))
}

fn parse_opt_u64(args: &[String], flag: &str) -> Result<Option<u64>> {
    let mut idx = 0;
    while idx < args.len() {
        if args[idx] == flag {
            let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))?;
            return Ok(Some(val.parse()?));
        }
        idx += 1;
    }
    Ok(None)
}

fn parse_opt_string(args: &[String], flag: &str) -> Result<Option<String>> {
    let mut idx = 0;
    while idx < args.len() {
        if args[idx] == flag {
            let val = args.get(idx + 1).ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))?;
            return Ok(Some(val.clone()));
        }
        idx += 1;
    }
    Ok(None)
}

fn parse_bool(val: &str) -> Result<bool> {
    match val.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => bail!("expected true or false, got {val:?}"),
    }
}

fn request_with_fallback(request: LogRequest) -> Result<LogResponse> {
    if let Some(sock) = socket_path()
        && sock.exists()
        && let Ok((_, resp)) = exchange_at(&sock, &request, DEFAULT_IO_TIMEOUT)
    {
        return Ok(resp);
    }
    offline_fallback(request)
}

fn offline_fallback(request: LogRequest) -> Result<LogResponse> {
    match request {
        LogRequest::Ping => Ok(LogResponse::Pong),
        LogRequest::QueryLogs { query } => {
            let mut entries: Vec<LogEntry> =
                read_disk_entries().into_iter().filter(|e| query.matches(e)).collect();
            if query.newest_first {
                entries.reverse();
            }
            let total_matched = entries.len() as u64;
            let offset = query.offset.unwrap_or(0);
            let limit = query.limit.unwrap_or(200);
            let paged = entries.into_iter().skip(offset).take(limit).collect();
            Ok(LogResponse::Logs { entries: paged, total_matched })
        }
        LogRequest::ListSessions { limit } => {
            let entries = read_disk_entries();
            let mut map: BTreeMap<String, SessionInfo> = BTreeMap::new();
            for e in entries {
                let sid = if e.session_id.is_empty() { "offline".to_owned() } else { e.session_id.clone() };
                let info = map.entry(sid.clone()).or_insert_with(|| SessionInfo {
                    session_id: sid,
                    started_at_ms: e.ts_ms,
                    started_at_iso: e.ts_iso.clone(),
                    last_seen_ms: e.ts_ms,
                    last_seen_iso: e.ts_iso.clone(),
                    mode: e.source.clone(),
                    version: e.fields.get("version").and_then(|v| v.as_str()).map(ToOwned::to_owned),
                    kernel: e.fields.get("kernel").and_then(|v| v.as_str()).map(ToOwned::to_owned),
                    pid: e.pid,
                    entry_count: 0,
                    error_count: 0,
                    warn_count: 0,
                });
                info.started_at_ms = info.started_at_ms.min(e.ts_ms);
                info.started_at_iso = format_iso_utc(info.started_at_ms);
                info.last_seen_ms = info.last_seen_ms.max(e.ts_ms);
                info.last_seen_iso = format_iso_utc(info.last_seen_ms);
                info.entry_count += 1;
                if e.level == LogLevel::Error {
                    info.error_count += 1;
                } else if e.level == LogLevel::Warn {
                    info.warn_count += 1;
                }
            }
            let mut sessions: Vec<SessionInfo> = map.into_values().collect();
            sessions.sort_by_key(|s| std::cmp::Reverse(s.last_seen_ms));
            sessions.truncate(limit.unwrap_or(50));
            Ok(LogResponse::Sessions { sessions })
        }
        LogRequest::ListCrashBundles => Ok(LogResponse::CrashBundles { bundles: list_disk_bundles() }),
        LogRequest::GetCrashBundle { id, max_file_bytes } => {
            Ok(LogResponse::CrashBundle { detail: read_disk_bundle(&id, max_file_bytes.unwrap_or(256 * 1024))? })
        }
        LogRequest::CreateCrashBundle { reason } => {
            Ok(LogResponse::CrashBundleCreated { summary: create_disk_bundle(&reason)? })
        }
        LogRequest::GetGpuSummary { session_id, limit } => {
            let cap = limit.unwrap_or(50);
            let entries = read_disk_entries();
            let matches_sess = |e: &LogEntry| session_id.as_deref().is_none_or(|s| e.session_id == s);
            let device_lines: Vec<LogEntry> = entries
                .iter()
                .rev()
                .filter(|e| matches_sess(e) && e.target.to_ascii_lowercase().contains("glide::gpu"))
                .take(cap)
                .cloned()
                .collect();
            let gpu_errors: Vec<LogEntry> = entries
                .iter()
                .rev()
                .filter(|e| matches_sess(e) && e.level >= LogLevel::Warn && e.target.to_ascii_lowercase().contains("gpu"))
                .take(cap)
                .cloned()
                .collect();
            let total_gpu_errors = gpu_errors.len() as u64;
            Ok(LogResponse::GpuSummary {
                summary: glide_log_ipc::GpuSummary {
                    device_lines,
                    gpu_errors,
                    kernel_gpu_events: Vec::new(),
                    total_gpu_errors,
                    total_kernel_events: 0,
                },
            })
        }
        LogRequest::GetRetention => {
            let cfg = read_retention_json();
            Ok(LogResponse::Retention { config: cfg, stats: disk_stats() })
        }
        LogRequest::SetRetention { config } => {
            write_retention_json(&config)?;
            Ok(LogResponse::Retention { config, stats: disk_stats() })
        }
        LogRequest::GetStats => Ok(LogResponse::Stats { stats: disk_stats() }),
        LogRequest::PruneNow { .. } => Ok(LogResponse::Error {
            message: "glide-logd is offline; start glide-logd to prune the SQLite database".to_owned(),
        }),
        LogRequest::TailLogs { .. } => Ok(LogResponse::Error { message: "use glidectl logs tail".to_owned() }),
    }
}

fn read_retention_json() -> RetentionConfig {
    let Some(dir) = logs_dir() else {
        return RetentionConfig::default();
    };
    fs::read_to_string(dir.join("retention.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_retention_json(cfg: &RetentionConfig) -> Result<()> {
    let dir = logs_dir().ok_or_else(|| anyhow::anyhow!("cannot locate logs dir (XDG_STATE_HOME/HOME unset)"))?;
    fs::create_dir_all(&dir)?;
    let encoded = serde_json::to_string_pretty(cfg)?;
    fs::write(dir.join("retention.json"), encoded)?;
    Ok(())
}

fn disk_stats() -> StorageStats {
    let mut stats = StorageStats::default();
    if let Some(dir) = logs_dir() {
        stats.db_bytes = fs::metadata(dir.join("glide-logs.db")).map_or(0, |m| m.len());
        if let Ok(entries) = fs::read_dir(dir.join("archives")) {
            for e in entries.flatten() {
                if let Ok(m) = e.metadata()
                    && m.is_file()
                {
                    stats.archive_count += 1;
                    stats.archive_bytes += m.len();
                }
            }
        }
    }
    let bundles = list_disk_bundles();
    stats.crash_bundle_count = bundles.len() as u64;
    stats.crash_bundle_bytes = bundles.iter().map(|b| b.total_bytes).sum();
    stats.total_bytes = stats.db_bytes + stats.archive_bytes + stats.crash_bundle_bytes;
    stats
}

fn read_disk_entries() -> Vec<LogEntry> {
    let Some(dir) = state_dir() else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let mut session = String::new();
    let jsonl = dir.join("glide.events.jsonl");
    if let Ok(file) = File::open(&jsonl) {
        for line in BufReader::new(file).lines().map_while(std::result::Result::ok) {
            if let Some(entry) = parse_jsonl_line(&line, "compositor", &mut session) {
                entries.push(entry);
            }
        }
    }
    entries
}

fn parse_jsonl_line(line: &str, source: &str, session_id: &mut String) -> Option<LogEntry> {
    let raw: CompositorEventLine = serde_json::from_str(line).ok()?;
    if raw.message == "starting glide" || session_id.is_empty() {
        *session_id = format!("session-{}-pid{}", format_compact_utc(raw.ts_ms), raw.pid);
    }
    Some(LogEntry {
        id: 0,
        ts_ms: raw.ts_ms,
        ts_iso: if raw.ts_iso.is_empty() { format_iso_utc(raw.ts_ms) } else { raw.ts_iso },
        session_id: session_id.clone(),
        source: source.to_owned(),
        level: raw.level,
        target: raw.target,
        thread: raw.thread,
        pid: Some(raw.pid),
        message: redact_sensitive(&raw.message),
        fields: raw.fields,
    })
}

fn list_disk_bundles() -> Vec<CrashBundleSummary> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for base in crash_bundle_dirs() {
        let Ok(read_dir) = fs::read_dir(&base) else {
            continue;
        };
        for entry in read_dir.flatten() {
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
            if let Some(summary) = read_disk_bundle_summary(&base, &path, name) {
                out.push(summary);
            }
        }
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at_ms));
    out
}

fn read_disk_bundle_summary(base: &Path, dir: &Path, id: &str) -> Option<CrashBundleSummary> {
    let raw = fs::read_to_string(dir.join("manifest.json")).ok()?;
    let val: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let archive = base.join(format!("{id}.tar.gz"));
    let total_bytes = dir_bytes(dir) + fs::metadata(&archive).map_or(0, |m| m.len());
    Some(CrashBundleSummary {
        id: val.get("id")?.as_str()?.to_owned(),
        created_at_ms: val.get("created_at_ms")?.as_i64()?,
        created_at_iso: val.get("created_at_iso")?.as_str()?.to_owned(),
        reason: val.get("reason")?.as_str()?.to_owned(),
        session_id: val.get("session_id").and_then(|v| v.as_str()).map(ToOwned::to_owned),
        version: val.get("version").and_then(|v| v.as_str()).map(ToOwned::to_owned),
        kernel: val.get("kernel").and_then(|v| v.as_str()).map(ToOwned::to_owned),
        gpu_summary: val.get("gpu_summary").and_then(|v| v.as_str()).map(ToOwned::to_owned),
        dir_path: dir.to_string_lossy().into_owned(),
        archive_path: archive.is_file().then(|| archive.to_string_lossy().into_owned()),
        total_bytes,
        files: val
            .get("files")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(ToOwned::to_owned)).collect())
            .unwrap_or_default(),
    })
}

fn read_disk_bundle(id: &str, cap: usize) -> Result<CrashBundleDetail> {
    if !is_valid_bundle_id(id) {
        bail!("invalid crash bundle id: {id:?}");
    }
    for base in crash_bundle_dirs() {
        let dir = base.join(id);
        if dir.is_dir()
            && let Some(summary) = read_disk_bundle_summary(&base, &dir, id)
        {
            return Ok(CrashBundleDetail {
                summary,
                manifest_json: fs::read_to_string(dir.join("manifest.json")).unwrap_or_default(),
                compositor_events_tail: read_tail(&dir.join("compositor.events.jsonl"), cap),
                compositor_log_tail: read_tail(&dir.join("compositor.log"), cap),
                journal_user_tail: read_tail(&dir.join("journal-user.json"), cap),
                journal_kernel_tail: read_tail(&dir.join("journal-kernel.txt"), cap),
            });
        }
    }
    bail!("crash bundle not found: {id}");
}

fn create_disk_bundle(reason: &str) -> Result<CrashBundleSummary> {
    let ts_ms = now_ms();
    let ts_iso = format_iso_utc(ts_ms);
    let slug: String = reason
        .chars()
        .take(40)
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    let id = format!("glide-bundle-{}-{}", format_compact_utc(ts_ms), if slug.is_empty() { "manual" } else { slug });

    let sdir = state_dir().unwrap_or_else(|| PathBuf::from("."));
    let events = read_tail(&sdir.join("glide.events.jsonl"), 2 * 1024 * 1024);
    let log = read_tail(&sdir.join("glide.log"), 2 * 1024 * 1024);
    let manifest = serde_json::json!({
        "id": id,
        "created_at_ms": ts_ms,
        "created_at_iso": ts_iso,
        "reason": reason,
        "session_id": null,
        "version": env!("CARGO_PKG_VERSION"),
        "kernel": "offline",
        "hostname": fs::read_to_string("/etc/hostname").map_or_else(|_| "unknown".into(), |s| s.trim().to_owned()),
        "gpu_summary": null,
        "files": ["manifest.json", "compositor.events.jsonl", "compositor.log"],
    });
    let manifest_json = serde_json::to_string_pretty(&manifest)?;

    for base in crash_bundle_dirs() {
        if fs::create_dir_all(&base).is_err() {
            continue;
        }
        let dir = base.join(&id);
        if fs::create_dir_all(&dir).is_err() {
            continue;
        }
        fs::write(dir.join("manifest.json"), &manifest_json)?;
        fs::write(dir.join("compositor.events.jsonl"), &events)?;
        fs::write(dir.join("compositor.log"), &log)?;
        return Ok(CrashBundleSummary {
            id,
            created_at_ms: ts_ms,
            created_at_iso: ts_iso,
            reason: reason.to_owned(),
            session_id: None,
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            kernel: None,
            gpu_summary: None,
            dir_path: dir.to_string_lossy().into_owned(),
            archive_path: None,
            total_bytes: dir_bytes(&dir),
            files: vec!["manifest.json".into(), "compositor.events.jsonl".into(), "compositor.log".into()],
        });
    }
    bail!("no writable crash bundle directory found");
}

pub fn export_bundle_to_dir(id: &str, dest_dir: &Path) -> Result<PathBuf> {
    if !is_valid_bundle_id(id) {
        bail!("invalid crash bundle id: {id:?}");
    }
    fs::create_dir_all(dest_dir).with_context(|| format!("create destination {}", dest_dir.display()))?;

    for base in crash_bundle_dirs() {
        let archive = base.join(format!("{id}.tar.gz"));
        if archive.is_file() {
            let target = dest_dir.join(format!("{id}.tar.gz"));
            fs::copy(&archive, &target)
                .with_context(|| format!("copy {} to {}", archive.display(), target.display()))?;
            return Ok(target);
        }
        let dir = base.join(id);
        if dir.is_dir() {
            let target = dest_dir.join(id);
            copy_dir_recursive(&dir, &target)?;
            return Ok(target);
        }
    }
    bail!("crash bundle not found: {id}");
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.metadata()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn read_tail(path: &Path, max_bytes: usize) -> String {
    let Ok(mut f) = File::open(path) else {
        return String::new();
    };
    let len = f.metadata().map_or(0, |m| m.len());
    let cap = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if len > cap {
        let _ = f.seek(SeekFrom::Start(len - cap));
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    redact_sensitive(&String::from_utf8_lossy(&buf))
}

fn dir_bytes(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(fs::Metadata::is_file)
        .map(|m| m.len())
        .sum()
}

#[allow(clippy::too_many_lines)]
fn emit_response(out: &mut impl Write, resp: &LogResponse, json_output: bool) -> Result<i32> {
    if json_output {
        writeln!(out, "{}", serde_json::to_string_pretty(resp)?)?;
        out.flush()?;
        return Ok(i32::from(matches!(resp, LogResponse::Error { .. } | LogResponse::Unknown)));
    }
    match resp {
        LogResponse::Pong => writeln!(out, "PONG")?,
        LogResponse::Ok => writeln!(out, "OK")?,
        LogResponse::Subscribed => writeln!(out, "Subscribed")?,
        LogResponse::Entry { entry } => print_entry(out, entry, false)?,
        LogResponse::Logs { entries, total_matched } => {
            for entry in entries {
                print_entry(out, entry, false)?;
            }
            writeln!(out, "--- {} shown ({total_matched} matched) ---", entries.len())?;
        }
        LogResponse::Sessions { sessions } => {
            writeln!(
                out,
                "{:<34} {:<10} {:<10} {:<8} {:<8} {:<24}",
                "SESSION_ID", "MODE", "VERSION", "ERRORS", "WARNS", "LAST_SEEN"
            )?;
            writeln!(out, "{}", "-".repeat(98))?;
            for s in sessions {
                writeln!(
                    out,
                    "{:<34} {:<10} {:<10} {:<8} {:<8} {:<24}",
                    printable(&s.session_id),
                    printable(&s.mode),
                    s.version.as_deref().map_or("-", |v| v),
                    s.error_count,
                    s.warn_count,
                    printable(&s.last_seen_iso)
                )?;
            }
        }
        LogResponse::CrashBundles { bundles } => {
            if bundles.is_empty() {
                writeln!(out, "no crash bundles found")?;
            } else {
                writeln!(out, "{:<42} {:<24} {:<18} {:<10}", "ID", "CREATED", "REASON", "BYTES")?;
                writeln!(out, "{}", "-".repeat(98))?;
                for b in bundles {
                    writeln!(
                        out,
                        "{:<42} {:<24} {:<18} {:<10}",
                        printable(&b.id),
                        printable(&b.created_at_iso),
                        printable(&b.reason),
                        b.total_bytes
                    )?;
                }
            }
        }
        LogResponse::CrashBundle { detail } => {
            writeln!(out, "=== {} ({}) ===", printable(&detail.summary.id), printable(&detail.summary.reason))?;
            writeln!(out, "Dir:     {}", printable(&detail.summary.dir_path))?;
            if let Some(ref arc) = detail.summary.archive_path {
                writeln!(out, "Archive: {}", printable(arc))?;
            }
            writeln!(out, "\n--- manifest.json ---\n{}", detail.manifest_json)?;
            if !detail.compositor_log_tail.is_empty() {
                writeln!(out, "\n--- compositor.log ---\n{}", detail.compositor_log_tail)?;
            }
            if !detail.journal_kernel_tail.is_empty() {
                writeln!(out, "\n--- journal-kernel.txt ---\n{}", detail.journal_kernel_tail)?;
            }
        }
        LogResponse::CrashBundleCreated { summary } => {
            writeln!(out, "Created crash bundle: {}", printable(&summary.id))?;
            writeln!(out, "  dir:     {}", printable(&summary.dir_path))?;
            if let Some(ref arc) = summary.archive_path {
                writeln!(out, "  archive: {}", printable(arc))?;
            }
        }
        LogResponse::GpuSummary { summary } => {
            writeln!(
                out,
                "GPU summary: {} device lines, {} GPU/DRM errors, {} kernel events",
                summary.device_lines.len(),
                summary.total_gpu_errors,
                summary.total_kernel_events
            )?;
            for e in &summary.gpu_errors {
                print_entry(out, e, false)?;
            }
            for e in &summary.kernel_gpu_events {
                print_entry(out, e, false)?;
            }
        }
        LogResponse::Retention { config, stats } => {
            writeln!(out, "retain_all:        {}", config.retain_all)?;
            writeln!(out, "max_days:          {}", config.max_days)?;
            writeln!(out, "max_size_mb:       {}", config.max_size_mb)?;
            writeln!(out, "crash_bundle_keep: {}", config.crash_bundle_keep)?;
            print_stats(out, stats)?;
        }
        LogResponse::Pruned { removed_entries, removed_archives, freed_bytes, stats } => {
            writeln!(
                out,
                "Pruned {removed_entries} log entries, {removed_archives} archives ({freed_bytes} bytes freed)"
            )?;
            print_stats(out, stats)?;
        }
        LogResponse::Stats { stats } => print_stats(out, stats)?,
        LogResponse::Error { message } => {
            eprintln!("Error: {}", printable(message));
            return Ok(1);
        }
        LogResponse::Unknown => {
            eprintln!("Error: glide-logd sent an unknown response");
            return Ok(1);
        }
    }
    out.flush()?;
    Ok(0)
}

fn print_stats(out: &mut impl Write, stats: &StorageStats) -> std::io::Result<()> {
    writeln!(
        out,
        "storage: total={} bytes (db={}, archives={} [{} files], bundles={} [{} bundles])",
        stats.total_bytes,
        stats.db_bytes,
        stats.archive_bytes,
        stats.archive_count,
        stats.crash_bundle_bytes,
        stats.crash_bundle_count
    )?;
    writeln!(
        out,
        "indexed: entries={} (errors={}, warns={}), sessions={}",
        stats.total_entries, stats.error_entries, stats.warn_entries, stats.total_sessions
    )
}

fn print_entry(out: &mut impl Write, entry: &LogEntry, json_output: bool) -> Result<()> {
    if json_output {
        writeln!(out, "{}", serde_json::to_string(entry)?)?;
    } else {
        writeln!(
            out,
            "{} {:<5} [{}] {}: {}",
            printable(&entry.ts_iso),
            entry.level.as_str(),
            printable(&entry.source),
            printable(&entry.target),
            printable(&entry.message)
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parses_logs_query_flags() {
        let q = parse_log_query(
            &words(&["--level", "warn", "--target", "glide::gpu", "--grep", "pageflip", "--limit", "25"]),
            true,
        )
        .unwrap();
        assert_eq!(q.min_level, Some(LogLevel::Warn));
        assert_eq!(q.target.as_deref(), Some("glide::gpu"));
        assert_eq!(q.grep.as_deref(), Some("pageflip"));
        assert_eq!(q.limit, Some(25));
        assert!(q.newest_first);
    }

    #[test]
    fn logs_help_prints_usage() {
        let mut out = Vec::new();
        let code = run_logs(&mut out, &words(&["--help"]), false).unwrap();
        assert_eq!(code, 0);
        assert!(String::from_utf8(out).unwrap().contains("glidectl logs"));
    }
}
