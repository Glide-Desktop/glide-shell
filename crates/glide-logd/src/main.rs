mod bundle;
mod ingest;
mod journal;
mod mcp;
mod server;
mod store;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use glide_log_ipc::{LogEntry, crash_bundle_dirs, logs_dir, socket_path, state_dir};
use tokio::sync::{broadcast, mpsc};

use crate::{mcp::McpConfig, server::DaemonContext, store::LogStore};

const USAGE: &str = r"glide-logd — Glide log collector, crash bundler, and MCP server

USAGE:
    glide-logd [OPTIONS]

OPTIONS:
    --mcp               Run stdio JSON-RPC 2.0 MCP server (with offline fallback)
    --socket <PATH>     Override Unix socket path (default: $XDG_RUNTIME_DIR/glide-logd.sock)
    --state-dir <DIR>   Override Glide state directory (default: $XDG_STATE_HOME/glide)
    -h, --help          Show this help
    -V, --version       Show version
";

struct CliOptions {
    mcp_mode: bool,
    socket_override: Option<PathBuf>,
    state_dir_override: Option<PathBuf>,
}

fn parse_cli() -> Result<Option<CliOptions>> {
    let mut args = std::env::args().skip(1);
    let mut opts = CliOptions { mcp_mode: false, socket_override: None, state_dir_override: None };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mcp" => opts.mcp_mode = true,
            "--socket" => {
                let val = args.next().ok_or_else(|| anyhow::anyhow!("--socket requires a path"))?;
                opts.socket_override = Some(PathBuf::from(val));
            }
            "--state-dir" => {
                let val = args.next().ok_or_else(|| anyhow::anyhow!("--state-dir requires a path"))?;
                opts.state_dir_override = Some(PathBuf::from(val));
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("glide-logd {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => anyhow::bail!("unknown argument: {other}\n\n{USAGE}"),
        }
    }
    Ok(Some(opts))
}

fn resolve_paths(opts: &CliOptions) -> Result<(PathBuf, PathBuf, Option<PathBuf>, Vec<PathBuf>)> {
    let state = opts
        .state_dir_override
        .clone()
        .or_else(state_dir)
        .or_else(|| glide_config::state_home().map(|d| d.join("glide")))
        .ok_or_else(|| anyhow::anyhow!("cannot determine Glide state directory (set XDG_STATE_HOME or HOME)"))?;
    let logs = if opts.state_dir_override.is_some() {
        state.join("logs")
    } else {
        logs_dir().unwrap_or_else(|| state.join("logs"))
    };
    let sock = opts.socket_override.clone().or_else(socket_path);
    let mut bdirs = crash_bundle_dirs();
    let state_bundles = state.join("crash-bundles");
    if !bdirs.contains(&state_bundles) {
        bdirs.push(state_bundles);
    }
    Ok((state, logs, sock, bdirs))
}

fn main() -> Result<()> {
    let Some(opts) = parse_cli()? else {
        return Ok(());
    };
    let (state_dir, logs_dir, sock_opt, bundle_dirs) = resolve_paths(&opts)?;

    if opts.mcp_mode {
        let cfg = McpConfig { socket_path: sock_opt, state_dir, logs_dir, bundle_dirs };
        return mcp::run_stdio(&cfg);
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let socket = sock_opt.ok_or_else(|| {
        anyhow::anyhow!("cannot determine socket path (set XDG_RUNTIME_DIR, GLIDE_LOGD_SOCK, or --socket)")
    })?;

    let store = Arc::new(Mutex::new(LogStore::open(&logs_dir)?));
    let (live_tx, _) = broadcast::channel::<LogEntry>(2048);
    let ctx = Arc::new(DaemonContext {
        store: Arc::clone(&store),
        state_dir: state_dir.clone(),
        bundle_dirs,
        live_tx,
    });

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let (ingest_tx, mut ingest_rx) = mpsc::channel::<LogEntry>(4096);

        ingest::spawn_file_ingester(state_dir, Arc::clone(&store), ingest_tx.clone());
        journal::spawn_journal_streams(ingest_tx);

        let store_for_retention = Arc::clone(&store);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_hours(1));
            interval.tick().await;
            loop {
                interval.tick().await;
                let s = Arc::clone(&store_for_retention);
                let _ = tokio::task::spawn_blocking(move || {
                    if let Ok(guard) = s.lock() {
                        let _ = guard.apply_retention(None, None, false);
                    }
                })
                .await;
            }
        });

        let ctx_for_consumer = Arc::clone(&ctx);
        tokio::spawn(async move {
            let mut cooldowns: HashMap<String, Instant> = HashMap::new();
            let mut buffer: Vec<LogEntry> = Vec::with_capacity(64);

            loop {
                match tokio::time::timeout(Duration::from_millis(200), ingest_rx.recv()).await {
                    Ok(Some(entry)) => {
                        buffer.push(entry);
                        if buffer.len() >= 64 {
                            flush_ingested_batch(&ctx_for_consumer, &mut buffer, &mut cooldowns).await;
                        }
                    }
                    Ok(None) => {
                        if !buffer.is_empty() {
                            flush_ingested_batch(&ctx_for_consumer, &mut buffer, &mut cooldowns).await;
                        }
                        break;
                    }
                    Err(_) => {
                        if !buffer.is_empty() {
                            flush_ingested_batch(&ctx_for_consumer, &mut buffer, &mut cooldowns).await;
                        }
                    }
                }
            }
        });

        server::serve(&socket, ctx).await
    })
}

async fn flush_ingested_batch(
    ctx: &Arc<DaemonContext>,
    buffer: &mut Vec<LogEntry>,
    cooldowns: &mut HashMap<String, Instant>,
) {
    let batch = std::mem::take(buffer);
    let store = Arc::clone(&ctx.store);
    let stored = tokio::task::spawn_blocking(move || {
        store.lock().ok().and_then(|mut guard| guard.insert_batch(&batch).ok()).unwrap_or(batch)
    })
    .await
    .unwrap_or_default();

    let mut trigger_reason: Option<String> = None;
    for entry in stored {
        if trigger_reason.is_none()
            && let Some(reason) = bundle::detect_trigger(&entry)
        {
            let cooldown = if reason == "panic" { Duration::from_secs(2) } else { Duration::from_secs(15) };
            let allow = cooldowns.get(&reason).is_none_or(|last| last.elapsed() >= cooldown);
            if allow {
                cooldowns.insert(reason.clone(), Instant::now());
                trigger_reason = Some(reason);
            }
        }
        let _ = ctx.live_tx.send(entry);
    }

    if let Some(reason) = trigger_reason {
        let ctx_clone = Arc::clone(ctx);
        tokio::task::spawn_blocking(move || match ctx_clone.create_bundle_now(&reason) {
            Ok(summary) => {
                tracing::warn!(
                    bundle_id = %summary.id,
                    reason = %summary.reason,
                    dir = %summary.dir_path,
                    "persisted crash bundle"
                );
            }
            Err(err) => {
                tracing::error!(reason = %reason, error = %err, "failed to persist crash bundle");
            }
        });
    }
}
