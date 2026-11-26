use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixStream as StdUnixStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use glide_log_ipc::{LogEntry, LogQuery, LogRequest, LogResponse, MAX_LINE_BYTES};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::broadcast,
};

use crate::{bundle, store::LogStore};

pub struct DaemonContext {
    pub store: Arc<Mutex<LogStore>>,
    pub state_dir: PathBuf,
    pub bundle_dirs: Vec<PathBuf>,
    pub live_tx: broadcast::Sender<LogEntry>,
}

impl DaemonContext {
    pub fn dispatch(&self, request: LogRequest) -> LogResponse {
        match request {
            LogRequest::Ping => LogResponse::Pong,
            LogRequest::TailLogs { .. } => {
                LogResponse::Error { message: "TailLogs must be handled on the streaming connection".to_owned() }
            }
            LogRequest::QueryLogs { query } => {
                let res = self
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("store lock poisoned"))
                    .and_then(|s| s.query_logs(&query));
                match res {
                    Ok((entries, total_matched)) => LogResponse::Logs { entries, total_matched },
                    Err(err) => LogResponse::Error { message: err.to_string() },
                }
            }
            LogRequest::ListSessions { limit } => {
                let res = self
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("store lock poisoned"))
                    .and_then(|s| s.list_sessions(limit.unwrap_or(50)));
                match res {
                    Ok(sessions) => LogResponse::Sessions { sessions },
                    Err(err) => LogResponse::Error { message: err.to_string() },
                }
            }
            LogRequest::ListCrashBundles => {
                LogResponse::CrashBundles { bundles: bundle::list_bundles(&self.bundle_dirs) }
            }
            LogRequest::GetCrashBundle { id, max_file_bytes } => {
                match bundle::read_bundle(&self.bundle_dirs, &id, max_file_bytes) {
                    Ok(detail) => LogResponse::CrashBundle { detail },
                    Err(err) => LogResponse::Error { message: err.to_string() },
                }
            }
            LogRequest::CreateCrashBundle { reason } => match self.create_bundle_now(&reason) {
                Ok(summary) => LogResponse::CrashBundleCreated { summary },
                Err(err) => LogResponse::Error { message: err.to_string() },
            },
            LogRequest::GetGpuSummary { session_id, limit } => {
                let res = self
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("store lock poisoned"))
                    .and_then(|s| s.gpu_summary(session_id.as_deref(), limit.unwrap_or(50)));
                match res {
                    Ok(summary) => LogResponse::GpuSummary { summary },
                    Err(err) => LogResponse::Error { message: err.to_string() },
                }
            }
            LogRequest::GetRetention => {
                let res = self.store.lock().map_err(|_| anyhow::anyhow!("store lock poisoned")).and_then(|s| {
                    let config = s.get_retention()?;
                    let stats = s.stats(&self.bundle_dirs)?;
                    Ok((config, stats))
                });
                match res {
                    Ok((config, stats)) => LogResponse::Retention { config, stats },
                    Err(err) => LogResponse::Error { message: err.to_string() },
                }
            }
            LogRequest::SetRetention { config } => {
                let res = self.store.lock().map_err(|_| anyhow::anyhow!("store lock poisoned")).and_then(|s| {
                    s.set_retention(&config)?;
                    let stats = s.stats(&self.bundle_dirs)?;
                    Ok((config, stats))
                });
                match res {
                    Ok((config, stats)) => LogResponse::Retention { config, stats },
                    Err(err) => LogResponse::Error { message: err.to_string() },
                }
            }
            LogRequest::PruneNow { max_days, max_size_mb } => {
                let res = self.store.lock().map_err(|_| anyhow::anyhow!("store lock poisoned")).and_then(|s| {
                    let (removed_entries, removed_archives, freed_bytes) =
                        s.apply_retention(max_days, max_size_mb, true)?;
                    let stats = s.stats(&self.bundle_dirs)?;
                    Ok((removed_entries, removed_archives, freed_bytes, stats))
                });
                match res {
                    Ok((removed_entries, removed_archives, freed_bytes, stats)) => {
                        LogResponse::Pruned { removed_entries, removed_archives, freed_bytes, stats }
                    }
                    Err(err) => LogResponse::Error { message: err.to_string() },
                }
            }
            LogRequest::GetStats => {
                let res = self
                    .store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("store lock poisoned"))
                    .and_then(|s| s.stats(&self.bundle_dirs));
                match res {
                    Ok(stats) => LogResponse::Stats { stats },
                    Err(err) => LogResponse::Error { message: err.to_string() },
                }
            }
        }
    }

    pub fn create_bundle_now(&self, reason: &str) -> Result<glide_log_ipc::CrashBundleSummary> {
        let (session_id, version, gpu_summary, mut recent) = {
            let guard = self.store.lock().map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
            let sessions = guard.list_sessions(1).unwrap_or_default();
            let latest = sessions.first();
            let session_id = latest.map(|s| s.session_id.clone());
            let version = latest.and_then(|s| s.version.clone());
            let gpu = guard
                .gpu_summary(session_id.as_deref(), 5)
                .ok()
                .and_then(|g| g.device_lines.first().map(|e| e.message.clone()));
            let (entries, _) = guard
                .query_logs(&LogQuery { limit: Some(1_000), newest_first: true, ..LogQuery::default() })
                .unwrap_or_default();
            (session_id, version, gpu, entries)
        };
        recent.reverse();

        bundle::create_crash_bundle(
            &self.bundle_dirs,
            &self.state_dir,
            reason,
            session_id,
            version,
            gpu_summary,
            &recent,
        )
    }
}

pub async fn serve(socket_path: &Path, ctx: Arc<DaemonContext>) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create socket parent {}", parent.display()))?;
    }
    if socket_path.exists() {
        if StdUnixStream::connect(socket_path).is_ok() {
            anyhow::bail!("another glide-logd instance is already listening on {}", socket_path.display());
        }
        let _ = fs::remove_file(socket_path);
    }

    let listener =
        UnixListener::bind(socket_path).with_context(|| format!("bind unix socket {}", socket_path.display()))?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0600 {}", socket_path.display()))?;

    tracing::info!(socket = %socket_path.display(), "glide-logd listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let ctx_clone = Arc::clone(&ctx);
        tokio::spawn(async move {
            if let Err(err) = handle_client(stream, ctx_clone).await {
                tracing::debug!(error = %err, "client connection ended");
            }
        });
    }
}

async fn handle_client(stream: UnixStream, ctx: Arc<DaemonContext>) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        line.clear();
        let n = (&mut reader).take(MAX_LINE_BYTES + 1).read_line(&mut line).await?;
        if n == 0 {
            return Ok(());
        }
        if u64::try_from(n).unwrap_or(u64::MAX) > MAX_LINE_BYTES {
            write_response(
                &mut write_half,
                &LogResponse::Error { message: "request exceeded maximum line length".to_owned() },
            )
            .await?;
            return Ok(());
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: LogRequest = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(err) => {
                write_response(
                    &mut write_half,
                    &LogResponse::Error { message: format!("invalid request JSON: {err}") },
                )
                .await?;
                continue;
            }
        };

        if let LogRequest::TailLogs { query } = request {
            let mut rx = ctx.live_tx.subscribe();
            write_response(&mut write_half, &LogResponse::Subscribed).await?;
            loop {
                match rx.recv().await {
                    Ok(entry) if query.matches(&entry) => {
                        if write_response(&mut write_half, &LogResponse::Entry { entry }).await.is_err() {
                            return Ok(());
                        }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }

        let ctx_for_blocking = Arc::clone(&ctx);
        let response = tokio::task::spawn_blocking(move || ctx_for_blocking.dispatch(request))
            .await
            .unwrap_or_else(|err| LogResponse::Error { message: format!("worker panicked: {err}") });
        write_response(&mut write_half, &response).await?;
    }
}

async fn write_response(writer: &mut (impl AsyncWriteExt + Unpin), response: &LogResponse) -> Result<()> {
    let mut encoded = serde_json::to_string(response)?;
    encoded.push('\n');
    writer.write_all(encoded.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}
