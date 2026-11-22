use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::Result;
use glide_log_ipc::{
    DEFAULT_IO_TIMEOUT, LogEntry, LogLevel, LogQuery, LogRequest, LogResponse, RetentionConfig, exchange_at,
    parse_timestamp_ms,
};
use serde_json::{Value, json};

use crate::{bundle, ingest, store::LogStore};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone)]
pub struct McpConfig {
    pub socket_path: Option<PathBuf>,
    pub state_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub bundle_dirs: Vec<PathBuf>,
}

pub fn run_stdio(cfg: &McpConfig) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(reply) = handle_jsonrpc_line(cfg, trimmed) {
            writeln!(out, "{}", serde_json::to_string(&reply)?)?;
            out.flush()?;
        }
    }
    Ok(())
}

#[must_use]
pub fn handle_jsonrpc_line(cfg: &McpConfig, line: &str) -> Option<Value> {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            return Some(jsonrpc_error(&Value::Null, -32700, &format!("Parse error: {err}")));
        }
    };

    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    if method == "notifications/initialized" || method.starts_with("notifications/") {
        return None;
    }

    let id = id.unwrap_or(Value::Null);
    match method {
        "initialize" => Some(jsonrpc_result(
            &id,
            &json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "glide-logd",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )),
        "ping" => Some(jsonrpc_result(&id, &json!({}))),
        "tools/list" => Some(jsonrpc_result(&id, &json!({ "tools": tool_definitions() }))),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let (text, is_error) = match call_tool(cfg, name, &args) {
                Ok(val) => (serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string()), false),
                Err(err) => (json!({ "error": err.to_string() }).to_string(), true),
            };
            Some(jsonrpc_result(
                &id,
                &json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": is_error,
                }),
            ))
        }
        other => Some(jsonrpc_error(&id, -32601, &format!("Method not found: {other}"))),
    }
}

fn jsonrpc_result(id: &Value, result: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn jsonrpc_error(id: &Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "query_logs",
            "description": "Query indexed Glide compositor, greeter, systemd user, and kernel GPU/DRM/PCIe/OOM logs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "min_level": { "type": "string", "enum": ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"] },
                    "levels": { "type": "array", "items": { "type": "string" } },
                    "target": { "type": "string", "description": "Substring filter on tracing target (e.g. glide::gpu)" },
                    "source": { "type": "string", "description": "Source filter (compositor, greeter, journal-user, journal-kernel)" },
                    "session_id": { "type": "string" },
                    "since": { "type": "string", "description": "ISO-8601 UTC timestamp or Unix epoch milliseconds" },
                    "until": { "type": "string", "description": "ISO-8601 UTC timestamp or Unix epoch milliseconds" },
                    "grep": { "type": "string", "description": "Case-insensitive substring search across message, target, and fields" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 10000 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "newest_first": { "type": "boolean" }
                }
            }
        },
        {
            "name": "list_sessions",
            "description": "List recent Glide compositor and greeter sessions with error/warning counts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500 }
                }
            }
        },
        {
            "name": "list_crash_bundles",
            "description": "List persisted self-contained redacted crash bundles from /var/log/glide/crash-bundles and ~/.local/state/glide/crash-bundles.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "read_crash_bundle",
            "description": "Read manifest and redacted log tails from a crash bundle by ID.",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "max_file_bytes": { "type": "integer", "minimum": 1024, "maximum": 4_194_304 }
                }
            }
        },
        {
            "name": "get_gpu_summary",
            "description": "Summarize GPU device initialization lines, DRM/compositor GPU errors, and kernel GPU/PCIe AER/OOM events.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500 }
                }
            }
        },
        {
            "name": "create_crash_bundle",
            "description": "Immediately persist a self-contained redacted crash bundle (directory + .tar.gz) for USB export or diagnosis.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reason": { "type": "string", "description": "Short reason label (e.g. manual-freeze-capture)" }
                }
            }
        },
        {
            "name": "get_retention",
            "description": "Get current Glide log retention policy and storage usage statistics.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "set_retention",
            "description": "Update Glide log retention policy (retain_all full retention vs bounded days/size budget).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "retain_all": { "type": "boolean" },
                    "max_days": { "type": "integer", "minimum": 1 },
                    "max_size_mb": { "type": "integer", "minimum": 16 },
                    "crash_bundle_keep": { "type": "integer", "minimum": 1 }
                }
            }
        }
    ])
}

fn call_tool(cfg: &McpConfig, name: &str, args: &Value) -> Result<Value> {
    let request = match name {
        "query_logs" => LogRequest::QueryLogs { query: parse_query_args(args) },
        "list_sessions" => {
            let limit = args.get("limit").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
            LogRequest::ListSessions { limit }
        }
        "list_crash_bundles" => LogRequest::ListCrashBundles,
        "read_crash_bundle" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing required argument 'id'"))?
                .to_owned();
            let max_file_bytes =
                args.get("max_file_bytes").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
            LogRequest::GetCrashBundle { id, max_file_bytes }
        }
        "get_gpu_summary" => {
            let session_id = args.get("session_id").and_then(Value::as_str).map(ToOwned::to_owned);
            let limit = args.get("limit").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
            LogRequest::GetGpuSummary { session_id, limit }
        }
        "create_crash_bundle" => {
            let reason = args.get("reason").and_then(Value::as_str).unwrap_or("mcp-manual").to_owned();
            LogRequest::CreateCrashBundle { reason }
        }
        "get_retention" => LogRequest::GetRetention,
        "set_retention" => {
            let mut current = match execute_request(cfg, LogRequest::GetRetention)? {
                LogResponse::Retention { config, .. } => config,
                _ => RetentionConfig::default(),
            };
            if let Some(v) = args.get("retain_all").and_then(Value::as_bool) {
                current.retain_all = v;
            }
            if let Some(v) = args.get("max_days").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()) {
                current.max_days = v;
            }
            if let Some(v) = args.get("max_size_mb").and_then(Value::as_u64) {
                current.max_size_mb = v;
            }
            if let Some(v) =
                args.get("crash_bundle_keep").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
            {
                current.crash_bundle_keep = v;
            }
            LogRequest::SetRetention { config: current }
        }
        other => anyhow::bail!("unknown tool: {other}"),
    };

    let response = execute_request(cfg, request)?;
    if let LogResponse::Error { message } = response {
        anyhow::bail!("{message}");
    }
    Ok(serde_json::to_value(response)?)
}

fn parse_query_args(args: &Value) -> LogQuery {
    let min_level = args
        .get("min_level")
        .or_else(|| args.get("level"))
        .and_then(Value::as_str)
        .and_then(LogLevel::parse_lossy);
    let levels = args
        .get("levels")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|v| v.as_str().and_then(LogLevel::parse_lossy)).collect())
        .unwrap_or_default();
    let parse_ts = |key: &str, ms_key: &str| -> Option<i64> {
        args.get(ms_key)
            .and_then(Value::as_i64)
            .or_else(|| args.get(key).and_then(Value::as_str).and_then(parse_timestamp_ms))
            .or_else(|| args.get(key).and_then(Value::as_i64))
    };

    LogQuery {
        min_level,
        levels,
        target: args.get("target").and_then(Value::as_str).map(ToOwned::to_owned),
        source: args.get("source").and_then(Value::as_str).map(ToOwned::to_owned),
        session_id: args.get("session_id").and_then(Value::as_str).map(ToOwned::to_owned),
        since_ms: parse_ts("since", "since_ms"),
        until_ms: parse_ts("until", "until_ms"),
        grep: args.get("grep").and_then(Value::as_str).map(ToOwned::to_owned),
        limit: args.get("limit").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()),
        offset: args.get("offset").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()),
        newest_first: args.get("newest_first").and_then(Value::as_bool).unwrap_or(true),
    }
}

pub fn execute_request(cfg: &McpConfig, request: LogRequest) -> Result<LogResponse> {
    if let Some(ref sock) = cfg.socket_path
        && sock.exists()
        && let Ok((_, resp)) = exchange_at(sock, &request, DEFAULT_IO_TIMEOUT)
    {
        return Ok(resp);
    }
    execute_offline(cfg, request)
}

fn execute_offline(cfg: &McpConfig, request: LogRequest) -> Result<LogResponse> {
    match request {
        LogRequest::Ping => Ok(LogResponse::Pong),
        LogRequest::ListCrashBundles => {
            Ok(LogResponse::CrashBundles { bundles: bundle::list_bundles(&cfg.bundle_dirs) })
        }
        LogRequest::GetCrashBundle { id, max_file_bytes } => {
            let detail = bundle::read_bundle(&cfg.bundle_dirs, &id, max_file_bytes)?;
            Ok(LogResponse::CrashBundle { detail })
        }
        LogRequest::CreateCrashBundle { reason } => {
            let recent = read_fallback_events(&cfg.state_dir);
            let summary = bundle::create_crash_bundle(
                &cfg.bundle_dirs,
                &cfg.state_dir,
                &reason,
                recent.last().map(|e| e.session_id.clone()).filter(|s| !s.is_empty()),
                None,
                None,
                &recent,
            )?;
            Ok(LogResponse::CrashBundleCreated { summary })
        }
        LogRequest::QueryLogs { query } => {
            if let Ok(store) = LogStore::open(&cfg.logs_dir) {
                let (entries, total) = store.query_logs(&query)?;
                if total > 0 {
                    return Ok(LogResponse::Logs { entries, total_matched: total });
                }
            }
            let mut entries: Vec<LogEntry> =
                read_fallback_events(&cfg.state_dir).into_iter().filter(|e| query.matches(e)).collect();
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
            let store = LogStore::open(&cfg.logs_dir)?;
            Ok(LogResponse::Sessions { sessions: store.list_sessions(limit.unwrap_or(50))? })
        }
        LogRequest::GetGpuSummary { session_id, limit } => {
            let store = LogStore::open(&cfg.logs_dir)?;
            Ok(LogResponse::GpuSummary { summary: store.gpu_summary(session_id.as_deref(), limit.unwrap_or(50))? })
        }
        LogRequest::GetRetention => {
            let store = LogStore::open(&cfg.logs_dir)?;
            Ok(LogResponse::Retention {
                config: store.get_retention()?,
                stats: store.stats(&cfg.bundle_dirs)?,
            })
        }
        LogRequest::SetRetention { config } => {
            let store = LogStore::open(&cfg.logs_dir)?;
            store.set_retention(&config)?;
            Ok(LogResponse::Retention { config, stats: store.stats(&cfg.bundle_dirs)? })
        }
        LogRequest::PruneNow { max_days, max_size_mb } => {
            let store = LogStore::open(&cfg.logs_dir)?;
            let (removed_entries, removed_archives, freed_bytes) =
                store.apply_retention(max_days, max_size_mb, true)?;
            Ok(LogResponse::Pruned {
                removed_entries,
                removed_archives,
                freed_bytes,
                stats: store.stats(&cfg.bundle_dirs)?,
            })
        }
        LogRequest::GetStats => {
            let store = LogStore::open(&cfg.logs_dir)?;
            Ok(LogResponse::Stats { stats: store.stats(&cfg.bundle_dirs)? })
        }
        LogRequest::TailLogs { .. } => {
            Ok(LogResponse::Error { message: "TailLogs is not supported in MCP tool calls".to_owned() })
        }
    }
}

fn read_fallback_events(state_dir: &Path) -> Vec<LogEntry> {
    let mut entries = Vec::new();
    let jsonl = state_dir.join("glide.events.jsonl");
    if let Ok(file) = File::open(&jsonl) {
        let mut session = String::new();
        for line in BufReader::new(file).lines().map_while(std::result::Result::ok) {
            if let Some(entry) = ingest::parse_jsonl_line(&line, "compositor", &mut session) {
                entries.push(entry);
            }
        }
        if !entries.is_empty() {
            return entries;
        }
    }
    let plain = state_dir.join("glide.log");
    if let Ok(file) = File::open(&plain) {
        for line in BufReader::new(file).lines().map_while(std::result::Result::ok) {
            if let Some(entry) = ingest::parse_plain_log_line(&line, "compositor-text", "offline-file") {
                entries.push(entry);
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use glide_log_ipc::now_ms;

    #[test]
    fn mcp_initialize_list_and_offline_tool_calls_work() {
        let root = std::env::temp_dir().join(format!("glide-logd-mcp-{}-{}", std::process::id(), now_ms()));
        let state_dir = root.join("state");
        let logs_dir = state_dir.join("logs");
        let bdir = root.join("bundles");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(
            state_dir.join("glide.events.jsonl"),
            r#"{"ts_ms":1700000000000,"ts_iso":"2023-11-14T22:13:20.000Z","level":"ERROR","target":"glide::gpu","thread":"main","pid":99,"message":"pageflip failed","fields":{}}"#,
        )
        .unwrap();

        let cfg = McpConfig {
            socket_path: Some(root.join("missing.sock")),
            state_dir,
            logs_dir,
            bundle_dirs: vec![bdir],
        };

        let init = handle_jsonrpc_line(&cfg, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "glide-logd");

        assert!(handle_jsonrpc_line(&cfg, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());

        let tools = handle_jsonrpc_line(&cfg, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 8);

        let query = handle_jsonrpc_line(
            &cfg,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"query_logs","arguments":{"min_level":"WARN"}}}"#,
        )
        .unwrap();
        assert_eq!(query["result"]["isError"], false);
        let text = query["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("pageflip failed"), "{text}");

        let created = handle_jsonrpc_line(
            &cfg,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_crash_bundle","arguments":{"reason":"test-mcp"}}}"#,
        )
        .unwrap();
        assert_eq!(created["result"]["isError"], false);

        let listed = handle_jsonrpc_line(
            &cfg,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_crash_bundles","arguments":{}}}"#,
        )
        .unwrap();
        assert!(listed["result"]["content"][0]["text"].as_str().unwrap().contains("test-mcp"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
