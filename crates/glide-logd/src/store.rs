use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use flate2::{Compression, write::GzEncoder};
use glide_log_ipc::{
    GpuSummary, LogEntry, LogLevel, LogQuery, RetentionConfig, SessionInfo, StorageStats, format_iso_utc, now_ms,
};
use rusqlite::{Connection, OptionalExtension, params, types::Value};

const DB_FILE: &str = "glide-logs.db";
const ARCHIVES_DIR: &str = "archives";
const RETENTION_JSON_FILE: &str = "retention.json";
const MAX_QUERY_LIMIT: usize = 10_000;
const DEFAULT_QUERY_LIMIT: usize = 200;

pub struct LogStore {
    conn: Connection,
    logs_dir: PathBuf,
}

impl LogStore {
    pub fn open(logs_dir: &Path) -> Result<Self> {
        fs::create_dir_all(logs_dir).with_context(|| format!("create logs dir {}", logs_dir.display()))?;
        fs::create_dir_all(logs_dir.join(ARCHIVES_DIR))
            .with_context(|| format!("create archives dir under {}", logs_dir.display()))?;

        let db_path = logs_dir.join(DB_FILE);
        let conn = Connection::open(&db_path).with_context(|| format!("open sqlite {}", db_path.display()))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             CREATE TABLE IF NOT EXISTS sessions (
                 session_id    TEXT PRIMARY KEY,
                 started_at_ms INTEGER NOT NULL,
                 last_seen_ms  INTEGER NOT NULL,
                 mode          TEXT NOT NULL,
                 version       TEXT,
                 kernel        TEXT,
                 pid           INTEGER,
                 entry_count   INTEGER NOT NULL DEFAULT 0,
                 error_count   INTEGER NOT NULL DEFAULT 0,
                 warn_count    INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS logs (
                 id          INTEGER PRIMARY KEY AUTOINCREMENT,
                 ts_ms       INTEGER NOT NULL,
                 ts_iso      TEXT NOT NULL,
                 session_id  TEXT NOT NULL,
                 source      TEXT NOT NULL,
                 level       INTEGER NOT NULL,
                 target      TEXT NOT NULL,
                 thread      TEXT,
                 pid         INTEGER,
                 message     TEXT NOT NULL,
                 fields_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_logs_ts ON logs(ts_ms);
             CREATE INDEX IF NOT EXISTS idx_logs_level_ts ON logs(level, ts_ms);
             CREATE INDEX IF NOT EXISTS idx_logs_session_ts ON logs(session_id, ts_ms);
             CREATE INDEX IF NOT EXISTS idx_logs_target ON logs(target);
             CREATE INDEX IF NOT EXISTS idx_logs_source ON logs(source);
             CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;

        let store = Self { conn, logs_dir: logs_dir.to_path_buf() };
        store.sync_retention_file_on_open()?;
        Ok(store)
    }

    #[must_use]
    pub fn archives_dir(&self) -> PathBuf {
        self.logs_dir.join(ARCHIVES_DIR)
    }

    fn sync_retention_file_on_open(&self) -> Result<()> {
        let json_path = self.logs_dir.join(RETENTION_JSON_FILE);
        if let Ok(raw) = fs::read_to_string(&json_path)
            && let Ok(cfg) = serde_json::from_str::<RetentionConfig>(&raw)
        {
            self.set_retention_db(&cfg)?;
            return Ok(());
        }
        let cfg = self.get_retention()?;
        let _ = write_retention_json(&json_path, &cfg);
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |row| row.get(0))
            .optional()
            .map_err(Into::into)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn insert_batch(&mut self, entries: &[LogEntry]) -> Result<Vec<LogEntry>> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let tx = self.conn.transaction()?;
        let mut inserted = Vec::with_capacity(entries.len());

        {
            let mut insert_log = tx.prepare_cached(
                "INSERT INTO logs (ts_ms, ts_iso, session_id, source, level, target, thread, pid, message, fields_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            let mut upsert_session = tx.prepare_cached(
                "INSERT INTO sessions (session_id, started_at_ms, last_seen_ms, mode, version, kernel, pid, entry_count, error_count, warn_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9)
                 ON CONFLICT(session_id) DO UPDATE SET
                     started_at_ms = MIN(sessions.started_at_ms, excluded.started_at_ms),
                     last_seen_ms  = MAX(sessions.last_seen_ms, excluded.last_seen_ms),
                     version       = COALESCE(sessions.version, excluded.version),
                     kernel        = COALESCE(sessions.kernel, excluded.kernel),
                     pid           = COALESCE(sessions.pid, excluded.pid),
                     entry_count   = sessions.entry_count + 1,
                     error_count   = sessions.error_count + excluded.error_count,
                     warn_count    = sessions.warn_count + excluded.warn_count",
            )?;

            for entry in entries {
                let fields_json = serde_json::to_string(&entry.fields).unwrap_or_else(|_| "{}".to_owned());
                insert_log.execute(params![
                    entry.ts_ms,
                    entry.ts_iso,
                    entry.session_id,
                    entry.source,
                    i64::from(entry.level.severity()),
                    entry.target,
                    entry.thread,
                    entry.pid.map(i64::from),
                    entry.message,
                    fields_json,
                ])?;
                let id = tx.last_insert_rowid();

                if !entry.session_id.is_empty() {
                    let mode = session_mode(entry);
                    let version = entry.fields.get("version").and_then(|v| v.as_str()).map(ToOwned::to_owned);
                    let kernel = entry.fields.get("kernel").and_then(|v| v.as_str()).map(ToOwned::to_owned);
                    let err_inc = i64::from(entry.level == LogLevel::Error);
                    let warn_inc = i64::from(entry.level == LogLevel::Warn);
                    upsert_session.execute(params![
                        entry.session_id,
                        entry.ts_ms,
                        entry.ts_ms,
                        mode,
                        version,
                        kernel,
                        entry.pid.map(i64::from),
                        err_inc,
                        warn_inc,
                    ])?;
                }

                let mut stored = entry.clone();
                stored.id = id;
                inserted.push(stored);
            }
        }

        tx.commit()?;
        Ok(inserted)
    }

    pub fn query_logs(&self, query: &LogQuery) -> Result<(Vec<LogEntry>, u64)> {
        let mut clauses: Vec<String> = Vec::new();
        let mut bind: Vec<Value> = Vec::new();

        if let Some(min_level) = query.min_level {
            clauses.push(format!("level >= ?{}", bind.len() + 1));
            bind.push(Value::Integer(i64::from(min_level.severity())));
        }
        if !query.levels.is_empty() {
            let placeholders: Vec<String> = query
                .levels
                .iter()
                .map(|lvl| {
                    bind.push(Value::Integer(i64::from(lvl.severity())));
                    format!("?{}", bind.len())
                })
                .collect();
            clauses.push(format!("level IN ({})", placeholders.join(", ")));
        }
        if let Some(ref target) = query.target
            && !target.is_empty()
        {
            clauses.push(format!("LOWER(target) LIKE ?{}", bind.len() + 1));
            bind.push(Value::Text(format!("%{}%", target.to_ascii_lowercase())));
        }
        if let Some(ref source) = query.source
            && !source.is_empty()
        {
            clauses.push(format!("LOWER(source) = ?{}", bind.len() + 1));
            bind.push(Value::Text(source.to_ascii_lowercase()));
        }
        if let Some(ref session_id) = query.session_id
            && !session_id.is_empty()
        {
            clauses.push(format!("session_id = ?{}", bind.len() + 1));
            bind.push(Value::Text(session_id.clone()));
        }
        if let Some(since_ms) = query.since_ms {
            clauses.push(format!("ts_ms >= ?{}", bind.len() + 1));
            bind.push(Value::Integer(since_ms));
        }
        if let Some(until_ms) = query.until_ms {
            clauses.push(format!("ts_ms <= ?{}", bind.len() + 1));
            bind.push(Value::Integer(until_ms));
        }
        if let Some(ref grep) = query.grep
            && !grep.is_empty()
        {
            let idx = bind.len() + 1;
            clauses.push(format!(
                "(LOWER(message) LIKE ?{idx} OR LOWER(target) LIKE ?{idx} OR LOWER(fields_json) LIKE ?{idx})"
            ));
            bind.push(Value::Text(format!("%{}%", grep.to_ascii_lowercase())));
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) FROM logs {where_sql}");
        let total_matched: u64 = self
            .conn
            .query_row(&count_sql, rusqlite::params_from_iter(bind.iter()), |row| row.get::<_, i64>(0))
            .map(|v| u64::try_from(v).unwrap_or(0))?;

        let limit = query.limit.unwrap_or(DEFAULT_QUERY_LIMIT).clamp(1, MAX_QUERY_LIMIT);
        let offset = query.offset.unwrap_or(0);
        let order = if query.newest_first { "DESC" } else { "ASC" };

        let limit_idx = bind.len() + 1;
        let offset_idx = bind.len() + 2;
        bind.push(Value::Integer(i64::try_from(limit).unwrap_or(DEFAULT_QUERY_LIMIT as i64)));
        bind.push(Value::Integer(i64::try_from(offset).unwrap_or(0)));

        let select_sql = format!(
            "SELECT id, ts_ms, ts_iso, session_id, source, level, target, thread, pid, message, fields_json
             FROM logs {where_sql}
             ORDER BY ts_ms {order}, id {order}
             LIMIT ?{limit_idx} OFFSET ?{offset_idx}"
        );

        let mut stmt = self.conn.prepare(&select_sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(bind.iter()), row_to_entry)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok((entries, total_matched))
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionInfo>> {
        let limit = limit.clamp(1, 500);
        let mut stmt = self.conn.prepare(
            "SELECT session_id, started_at_ms, last_seen_ms, mode, version, kernel, pid, entry_count, error_count, warn_count
             FROM sessions
             ORDER BY last_seen_ms DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(50)], |row| {
            let started_at_ms: i64 = row.get(1)?;
            let last_seen_ms: i64 = row.get(2)?;
            let pid: Option<i64> = row.get(6)?;
            Ok(SessionInfo {
                session_id: row.get(0)?,
                started_at_ms,
                started_at_iso: format_iso_utc(started_at_ms),
                last_seen_ms,
                last_seen_iso: format_iso_utc(last_seen_ms),
                mode: row.get(3)?,
                version: row.get(4)?,
                kernel: row.get(5)?,
                pid: pid.and_then(|p| u32::try_from(p).ok()),
                entry_count: u64::try_from(row.get::<_, i64>(7)?).unwrap_or(0),
                error_count: u64::try_from(row.get::<_, i64>(8)?).unwrap_or(0),
                warn_count: u64::try_from(row.get::<_, i64>(9)?).unwrap_or(0),
            })
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    pub fn gpu_summary(&self, session_id: Option<&str>, limit: usize) -> Result<GpuSummary> {
        let limit = limit.clamp(1, 500);
        let (device_lines, _) = self.query_logs(&LogQuery {
            target: Some("glide::gpu".into()),
            session_id: session_id.map(ToOwned::to_owned),
            limit: Some(limit),
            newest_first: true,
            ..LogQuery::default()
        })?;

        let (gpu_errors, total_gpu_errors) = self.query_logs(&LogQuery {
            min_level: Some(LogLevel::Warn),
            target: Some("gpu".into()),
            session_id: session_id.map(ToOwned::to_owned),
            limit: Some(limit),
            newest_first: true,
            ..LogQuery::default()
        })?;

        let (kernel_gpu_events, total_kernel_events) = self.query_logs(&LogQuery {
            source: Some("journal-kernel".into()),
            limit: Some(limit),
            newest_first: true,
            ..LogQuery::default()
        })?;

        Ok(GpuSummary {
            device_lines,
            gpu_errors,
            kernel_gpu_events,
            total_gpu_errors,
            total_kernel_events,
        })
    }

    pub fn get_retention(&self) -> Result<RetentionConfig> {
        if let Some(raw) = self.get_meta("retention_config")?
            && let Ok(cfg) = serde_json::from_str::<RetentionConfig>(&raw)
        {
            return Ok(cfg);
        }
        Ok(RetentionConfig::default())
    }

    pub fn set_retention(&self, config: &RetentionConfig) -> Result<()> {
        self.set_retention_db(config)?;
        let _ = write_retention_json(&self.logs_dir.join(RETENTION_JSON_FILE), config);
        Ok(())
    }

    fn set_retention_db(&self, config: &RetentionConfig) -> Result<()> {
        let encoded = serde_json::to_string(config)?;
        self.set_meta("retention_config", &encoded)
    }

    pub fn archive_file(&self, source_path: &Path, archive_name: &str) -> Result<PathBuf> {
        let archives = self.archives_dir();
        fs::create_dir_all(&archives)?;
        let dest_name = if Path::new(archive_name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("gz")) {
            archive_name.to_owned()
        } else {
            format!("{archive_name}.gz")
        };
        let dest = archives.join(dest_name);
        let tmp = archives.join(format!(".tmp-{}.gz", now_ms()));

        let mut reader = File::open(source_path)
            .with_context(|| format!("open log for archiving {}", source_path.display()))?;
        let out_file = File::create(&tmp).with_context(|| format!("create archive temp {}", tmp.display()))?;
        let mut encoder = GzEncoder::new(out_file, Compression::default());
        io::copy(&mut reader, &mut encoder)?;
        let out_file = encoder.finish()?;
        let _ = out_file.sync_data();
        fs::rename(&tmp, &dest)?;
        Ok(dest)
    }

    pub fn stats(&self, bundle_dirs: &[PathBuf]) -> Result<StorageStats> {
        let db_bytes = file_len_or_zero(&self.logs_dir.join(DB_FILE))
            + file_len_or_zero(&self.logs_dir.join(format!("{DB_FILE}-wal")))
            + file_len_or_zero(&self.logs_dir.join(format!("{DB_FILE}-shm")));

        let (archive_count, archive_bytes) = dir_files_stats(&self.archives_dir());
        let mut crash_bundle_count = 0_u64;
        let mut crash_bundle_bytes = 0_u64;
        for dir in bundle_dirs {
            let (cnt, bytes) = dir_entries_stats(dir);
            crash_bundle_count += cnt;
            crash_bundle_bytes += bytes;
        }

        let (total_entries, error_entries, warn_entries): (u64, u64, u64) = self.conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN level >= 4 THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN level = 3 THEN 1 ELSE 0 END), 0)
             FROM logs",
            [],
            |row| {
                Ok((
                    u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                    u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                    u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                ))
            },
        )?;

        let total_sessions: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get::<_, i64>(0))
            .map(|v| u64::try_from(v).unwrap_or(0))?;

        Ok(StorageStats {
            db_bytes,
            archive_bytes,
            archive_count,
            crash_bundle_bytes,
            crash_bundle_count,
            total_bytes: db_bytes + archive_bytes + crash_bundle_bytes,
            total_entries,
            total_sessions,
            error_entries,
            warn_entries,
        })
    }

    pub fn apply_retention(
        &self,
        override_days: Option<u32>,
        override_max_mb: Option<u64>,
        force: bool,
    ) -> Result<(u64, u64, u64)> {
        let cfg = self.get_retention()?;
        if !force && cfg.retain_all {
            return Ok((0, 0, 0));
        }

        let max_days = override_days.unwrap_or(cfg.max_days);
        let max_bytes = override_max_mb.unwrap_or(cfg.max_size_mb).saturating_mul(1024 * 1024);

        let mut removed_entries = 0_u64;
        let mut removed_archives = 0_u64;
        let mut freed_bytes = 0_u64;

        if max_days > 0 {
            let cutoff_ms = now_ms().saturating_sub(i64::from(max_days).saturating_mul(86_400_000));
            let deleted = self.conn.execute("DELETE FROM logs WHERE ts_ms < ?1", params![cutoff_ms])?;
            removed_entries += u64::try_from(deleted).unwrap_or(0);
            let _ = self.conn.execute("DELETE FROM sessions WHERE last_seen_ms < ?1", params![cutoff_ms]);

            let cutoff_age = Duration::from_secs(u64::from(max_days).saturating_mul(86_400));
            if let Ok(read_dir) = fs::read_dir(self.archives_dir()) {
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    if let Ok(meta) = entry.metadata()
                        && meta.is_file()
                        && let Ok(modified) = meta.modified()
                        && let Ok(elapsed) = modified.elapsed()
                        && elapsed > cutoff_age
                        && fs::remove_file(&path).is_ok()
                    {
                        removed_archives += 1;
                        freed_bytes += meta.len();
                    }
                }
            }
        }

        if max_bytes > 0 {
            let mut archive_files: Vec<(std::time::SystemTime, PathBuf, u64)> = Vec::new();
            let mut current_archive_bytes = 0_u64;
            if let Ok(read_dir) = fs::read_dir(self.archives_dir()) {
                for entry in read_dir.flatten() {
                    if let Ok(meta) = entry.metadata()
                        && meta.is_file()
                    {
                        let len = meta.len();
                        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                        current_archive_bytes += len;
                        archive_files.push((mtime, entry.path(), len));
                    }
                }
            }
            archive_files.sort_by_key(|(mtime, _, _)| *mtime);

            let db_bytes = file_len_or_zero(&self.logs_dir.join(DB_FILE));
            let mut total = db_bytes + current_archive_bytes;
            for (_, path, len) in archive_files {
                if total <= max_bytes {
                    break;
                }
                if fs::remove_file(&path).is_ok() {
                    removed_archives += 1;
                    freed_bytes += len;
                    total = total.saturating_sub(len);
                }
            }

            if total > max_bytes {
                let total_rows: i64 =
                    self.conn.query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0)).unwrap_or(0);
                let to_drop = (total_rows / 4).max(1_000);
                let deleted = self.conn.execute(
                    "DELETE FROM logs WHERE id IN (SELECT id FROM logs ORDER BY ts_ms ASC, id ASC LIMIT ?1)",
                    params![to_drop],
                )?;
                removed_entries += u64::try_from(deleted).unwrap_or(0);
            }
        }

        if removed_entries > 0 {
            let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }

        Ok((removed_entries, removed_archives, freed_bytes))
    }
}

fn write_retention_json(path: &Path, cfg: &RetentionConfig) -> io::Result<()> {
    let encoded = serde_json::to_string_pretty(cfg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, encoded)?;
    fs::rename(&tmp, path)
}

fn session_mode(entry: &LogEntry) -> String {
    if let Some(mode) = entry.fields.get("mode").and_then(|v| v.as_str()) {
        return mode.to_ascii_lowercase();
    }
    if entry.source.contains("greeter") {
        "greeter".to_owned()
    } else if entry.source.contains("compositor") {
        "session".to_owned()
    } else {
        entry.source.clone()
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<LogEntry> {
    let level_num: i64 = row.get(5)?;
    let pid: Option<i64> = row.get(8)?;
    let fields_raw: String = row.get(10)?;
    let fields = serde_json::from_str(&fields_raw).unwrap_or_default();
    Ok(LogEntry {
        id: row.get(0)?,
        ts_ms: row.get(1)?,
        ts_iso: row.get(2)?,
        session_id: row.get(3)?,
        source: row.get(4)?,
        level: LogLevel::from_severity(u8::try_from(level_num).unwrap_or(2)),
        target: row.get(6)?,
        thread: row.get(7)?,
        pid: pid.and_then(|p| u32::try_from(p).ok()),
        message: row.get(9)?,
        fields,
    })
}

fn file_len_or_zero(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
}

fn dir_files_stats(dir: &Path) -> (u64, u64) {
    let mut count = 0_u64;
    let mut bytes = 0_u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata()
                && meta.is_file()
            {
                count += 1;
                bytes += meta.len();
            }
        }
    }
    (count, bytes)
}

fn dir_entries_stats(dir: &Path) -> (u64, u64) {
    let mut count = 0_u64;
    let mut bytes = 0_u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    count += 1;
                    bytes += dir_recursive_bytes(&path);
                } else if meta.is_file() {
                    bytes += meta.len();
                }
            }
        }
    }
    (count, bytes)
}

pub fn dir_recursive_bytes(dir: &Path) -> u64 {
    let mut total = 0_u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                } else if meta.is_dir() {
                    total += dir_recursive_bytes(&entry.path());
                }
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glide-logd-store-{name}-{}-{}", std::process::id(), now_ms()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn insert_query_and_retention_modes() {
        let dir = temp_dir("basic");
        let mut store = LogStore::open(&dir).unwrap();
        let now = now_ms();
        let entries = vec![
            LogEntry {
                id: 0,
                ts_ms: now - 10_000,
                ts_iso: format_iso_utc(now - 10_000),
                session_id: "sess-1".into(),
                source: "compositor".into(),
                level: LogLevel::Info,
                target: "glide::logging".into(),
                thread: Some("main".into()),
                pid: Some(100),
                message: "starting glide".into(),
                fields: serde_json::json!({"version": "0.1.4", "mode": "Session"}).as_object().unwrap().clone(),
            },
            LogEntry {
                id: 0,
                ts_ms: now,
                ts_iso: format_iso_utc(now),
                session_id: "sess-1".into(),
                source: "compositor".into(),
                level: LogLevel::Error,
                target: "glide::gpu".into(),
                thread: Some("main".into()),
                pid: Some(100),
                message: "DRM pageflip failed".into(),
                fields: serde_json::Map::new(),
            },
        ];

        let stored = store.insert_batch(&entries).unwrap();
        assert_eq!(stored.len(), 2);
        assert!(stored[0].id > 0);

        let (matched, total) = store
            .query_logs(&LogQuery { min_level: Some(LogLevel::Warn), ..LogQuery::default() })
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(matched[0].message, "DRM pageflip failed");

        let sessions = store.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].error_count, 1);
        assert_eq!(sessions[0].version.as_deref(), Some("0.1.4"));

        let (rem_e, rem_a, _) = store.apply_retention(Some(0), None, false).unwrap();
        assert_eq!((rem_e, rem_a), (0, 0));

        let sample = dir.join("glide.log.1");
        fs::write(&sample, "hello rotated log\n").unwrap();
        let gz = store.archive_file(&sample, "glide-test.log").unwrap();
        assert!(gz.exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
