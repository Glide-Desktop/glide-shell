use std::{
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const SOCKET_NAME: &str = "glide-logd.sock";

pub const SOCKET_ENV: &str = "GLIDE_LOGD_SOCK";

pub const SYSTEM_CRASH_BUNDLE_DIR: &str = "/var/log/glide/crash-bundles";

pub const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;

pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(10);

fn abs_env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from).filter(|p| p.is_absolute())
}

#[must_use]
pub fn socket_path() -> Option<PathBuf> {
    if let Some(explicit) = abs_env_path(SOCKET_ENV) {
        return Some(explicit);
    }
    abs_env_path("XDG_RUNTIME_DIR").map(|dir| dir.join(SOCKET_NAME))
}

#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    let base = abs_env_path("XDG_STATE_HOME").or_else(|| abs_env_path("HOME").map(|h| h.join(".local/state")))?;
    Some(base.join("glide"))
}

#[must_use]
pub fn logs_dir() -> Option<PathBuf> {
    state_dir().map(|d| d.join("logs"))
}

#[must_use]
pub fn user_crash_bundle_dir() -> Option<PathBuf> {
    state_dir().map(|d| d.join("crash-bundles"))
}

#[must_use]
pub fn crash_bundle_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from(SYSTEM_CRASH_BUNDLE_DIR)];
    if let Some(user_dir) = user_crash_bundle_dir()
        && !dirs.contains(&user_dir)
    {
        dirs.push(user_dir);
    }
    dirs
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl LogLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    #[must_use]
    pub const fn severity(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn from_severity(value: u8) -> Self {
        match value {
            0 => Self::Trace,
            1 => Self::Debug,
            2 => Self::Info,
            3 => Self::Warn,
            _ => Self::Error,
        }
    }

    #[must_use]
    pub fn parse_lossy(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "TRACE" => Some(Self::Trace),
            "DEBUG" => Some(Self::Debug),
            "INFO" => Some(Self::Info),
            "WARN" | "WARNING" => Some(Self::Warn),
            "ERROR" | "ERR" | "CRIT" | "CRITICAL" | "FATAL" | "EMERG" | "ALERT" => Some(Self::Error),
            _ => None,
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_lossy(s).ok_or_else(|| format!("unknown log level: {s:?}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositorEventLine {
    pub ts_ms: i64,
    pub ts_iso: String,
    pub level: LogLevel,
    pub target: String,
    #[serde(default)]
    pub thread: Option<String>,
    pub pid: u32,
    pub message: String,
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    #[serde(default)]
    pub id: i64,
    pub ts_ms: i64,
    pub ts_iso: String,
    #[serde(default)]
    pub session_id: String,
    pub source: String,
    pub level: LogLevel,
    pub target: String,
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    pub message: String,
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogQuery {
    #[serde(default)]
    pub min_level: Option<LogLevel>,
    #[serde(default)]
    pub levels: Vec<LogLevel>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub until_ms: Option<i64>,
    #[serde(default)]
    pub grep: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub newest_first: bool,
}

impl LogQuery {
    #[must_use]
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if let Some(min) = self.min_level
            && entry.level < min
        {
            return false;
        }
        if !self.levels.is_empty() && !self.levels.contains(&entry.level) {
            return false;
        }
        if let Some(ref target) = self.target
            && !target.is_empty()
            && !entry.target.to_ascii_lowercase().contains(&target.to_ascii_lowercase())
        {
            return false;
        }
        if let Some(ref source) = self.source
            && !source.is_empty()
            && !entry.source.eq_ignore_ascii_case(source)
        {
            return false;
        }
        if let Some(ref session_id) = self.session_id
            && !session_id.is_empty()
            && entry.session_id != *session_id
        {
            return false;
        }
        if let Some(since) = self.since_ms
            && entry.ts_ms < since
        {
            return false;
        }
        if let Some(until) = self.until_ms
            && entry.ts_ms > until
        {
            return false;
        }
        if let Some(ref grep) = self.grep
            && !grep.is_empty()
        {
            let needle = grep.to_ascii_lowercase();
            let in_msg = entry.message.to_ascii_lowercase().contains(&needle);
            let in_target = entry.target.to_ascii_lowercase().contains(&needle);
            let in_fields = !entry.fields.is_empty()
                && serde_json::to_string(&entry.fields)
                    .is_ok_and(|s| s.to_ascii_lowercase().contains(&needle));
            if !in_msg && !in_target && !in_fields {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub started_at_ms: i64,
    pub started_at_iso: String,
    pub last_seen_ms: i64,
    pub last_seen_iso: String,
    pub mode: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub kernel: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    pub entry_count: u64,
    pub error_count: u64,
    pub warn_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashBundleSummary {
    pub id: String,
    pub created_at_ms: i64,
    pub created_at_iso: String,
    pub reason: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub kernel: Option<String>,
    #[serde(default)]
    pub gpu_summary: Option<String>,
    pub dir_path: String,
    #[serde(default)]
    pub archive_path: Option<String>,
    pub total_bytes: u64,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashBundleDetail {
    pub summary: CrashBundleSummary,
    pub manifest_json: String,
    #[serde(default)]
    pub compositor_events_tail: String,
    #[serde(default)]
    pub compositor_log_tail: String,
    #[serde(default)]
    pub journal_user_tail: String,
    #[serde(default)]
    pub journal_kernel_tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionConfig {
    pub retain_all: bool,
    pub max_days: u32,
    pub max_size_mb: u64,
    pub crash_bundle_keep: u32,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self { retain_all: true, max_days: 30, max_size_mb: 1024, crash_bundle_keep: 50 }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStats {
    pub db_bytes: u64,
    pub archive_bytes: u64,
    pub archive_count: u64,
    pub crash_bundle_bytes: u64,
    pub crash_bundle_count: u64,
    pub total_bytes: u64,
    pub total_entries: u64,
    pub total_sessions: u64,
    pub error_entries: u64,
    pub warn_entries: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuSummary {
    pub device_lines: Vec<LogEntry>,
    pub gpu_errors: Vec<LogEntry>,
    pub kernel_gpu_events: Vec<LogEntry>,
    pub total_gpu_errors: u64,
    pub total_kernel_events: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LogRequest {
    Ping,
    QueryLogs {
        #[serde(default)]
        query: LogQuery,
    },
    TailLogs {
        #[serde(default)]
        query: LogQuery,
    },
    ListSessions {
        #[serde(default)]
        limit: Option<usize>,
    },
    ListCrashBundles,
    GetCrashBundle {
        id: String,
        #[serde(default)]
        max_file_bytes: Option<usize>,
    },
    CreateCrashBundle {
        reason: String,
    },
    GetGpuSummary {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    GetRetention,
    SetRetention {
        config: RetentionConfig,
    },
    PruneNow {
        #[serde(default)]
        max_days: Option<u32>,
        #[serde(default)]
        max_size_mb: Option<u64>,
    },
    GetStats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LogResponse {
    Pong,
    Ok,
    Subscribed,
    Entry {
        entry: LogEntry,
    },
    Logs {
        entries: Vec<LogEntry>,
        total_matched: u64,
    },
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    CrashBundles {
        bundles: Vec<CrashBundleSummary>,
    },
    CrashBundle {
        detail: CrashBundleDetail,
    },
    CrashBundleCreated {
        summary: CrashBundleSummary,
    },
    GpuSummary {
        summary: GpuSummary,
    },
    Retention {
        config: RetentionConfig,
        stats: StorageStats,
    },
    Pruned {
        removed_entries: u64,
        removed_archives: u64,
        freed_bytes: u64,
        stats: StorageStats,
    },
    Stats {
        stats: StorageStats,
    },
    Error {
        message: String,
    },
    #[serde(other)]
    Unknown,
}

pub fn read_line_capped(reader: &mut impl BufRead, line: &mut String) -> io::Result<usize> {
    let mut limited = reader.take(MAX_LINE_BYTES + 1);
    let n = limited.read_line(line)?;
    if u64::try_from(n).unwrap_or(u64::MAX) > MAX_LINE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "IPC line exceeded maximum length"));
    }
    Ok(n)
}

pub fn connect(path: &Path, timeout: Duration) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(stream)
}

pub fn exchange_at(
    path: &Path,
    request: &LogRequest,
    timeout: Duration,
) -> io::Result<(BufReader<UnixStream>, LogResponse)> {
    let mut payload =
        serde_json::to_string(request).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    payload.push('\n');

    let mut reader = BufReader::new(connect(path, timeout)?);
    reader.get_mut().write_all(payload.as_bytes())?;
    reader.get_mut().flush()?;

    let mut line = String::new();
    let n = read_line_capped(&mut reader, &mut line)?;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "glide-logd closed the connection"));
    }
    let response: LogResponse =
        serde_json::from_str(line.trim()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((reader, response))
}

pub fn exchange(request: &LogRequest) -> io::Result<LogResponse> {
    let path = socket_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR not set; cannot locate glide-logd.sock"))?;
    let (_, response) = exchange_at(&path, request, DEFAULT_IO_TIMEOUT)?;
    Ok(response)
}

#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[must_use]
pub fn format_iso_utc(ts_ms: i64) -> String {
    let secs = ts_ms.div_euclid(1_000);
    let millis = ts_ms.rem_euclid(1_000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);

    let (year, month, day) = civil_from_days(days);
    let hour = sod / 3_600;
    let minute = (sod % 3_600) / 60;
    let second = sod % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

#[must_use]
pub fn format_compact_utc(ts_ms: i64) -> String {
    let secs = ts_ms.div_euclid(1_000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);

    let (year, month, day) = civil_from_days(days);
    let hour = sod / 3_600;
    let minute = (sod % 3_600) / 60;
    let second = sod % 60;
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

#[must_use]
pub fn parse_timestamp_ms(input: &str) -> Option<i64> {
    let trimmed = input.trim();
    if let Ok(ms) = trimmed.parse::<i64>() {
        return Some(ms);
    }
    let trimmed = trimmed.strip_suffix('Z').or_else(|| trimmed.strip_suffix('z')).unwrap_or(trimmed);
    let (date_part, time_part) = trimmed.split_once('T').or_else(|| trimmed.split_once(' '))?;
    let mut date_it = date_part.split('-');
    let year: i64 = date_it.next()?.parse().ok()?;
    let month: u32 = date_it.next()?.parse().ok()?;
    let day: u32 = date_it.next()?.parse().ok()?;
    if date_it.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let time_clean = time_part.split(['+', '-']).next()?;
    let (hms, frac) = match time_clean.split_once('.') {
        Some((h, f)) => (h, Some(f)),
        None => (time_clean, None),
    };
    let mut time_it = hms.split(':');
    let hour: i64 = time_it.next()?.parse().ok()?;
    let minute: i64 = time_it.next()?.parse().ok()?;
    let second: i64 = time_it.next().unwrap_or("0").parse().ok()?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..=60).contains(&second) {
        return None;
    }

    let millis: i64 = match frac {
        Some(f) if !f.is_empty() => {
            let digits: String = f.chars().take_while(char::is_ascii_digit).take(3).collect();
            let padded = format!("{digits:0<3}");
            padded.parse().ok()?
        }
        _ => 0,
    };

    let days = days_from_civil(year, month, day);
    let secs = days.checked_mul(86_400)? + hour * 3_600 + minute * 60 + second;
    secs.checked_mul(1_000)?.checked_add(millis)
}

const fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = u64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + u64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + (doe as i64) - 719_468
}

#[must_use]
pub fn redact_sensitive(input: &str) -> String {
    const SENSITIVE_KEYS: &[&str] = &[
        "password",
        "passwd",
        "passphrase",
        "secret",
        "token",
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "private_key",
        "keychain",
        "cookie",
    ];

    let mut out = String::with_capacity(input.len());
    let mut lines = input.split_inclusive('\n').peekable();
    if lines.peek().is_none() {
        return out;
    }

    for line in lines {
        let lower = line.to_ascii_lowercase();
        let has_sensitive = SENSITIVE_KEYS.iter().any(|k| lower.contains(k));
        if !has_sensitive {
            out.push_str(line);
            continue;
        }
        out.push_str(&redact_line(line, SENSITIVE_KEYS));
    }
    out
}

fn redact_line(line: &str, keys: &[&str]) -> String {
    let bytes = line.as_bytes();
    let lower = line.to_ascii_lowercase();
    let mut result = String::with_capacity(line.len());
    let mut cursor = 0usize;

    while cursor < line.len() {
        let mut next_hit: Option<(usize, usize)> = None;
        for &key in keys {
            if let Some(rel) = lower[cursor..].find(key) {
                let pos = cursor + rel;
                let end = pos + key.len();
                let boundary_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
                if boundary_ok && next_hit.is_none_or(|(best_pos, _)| pos < best_pos) {
                    next_hit = Some((pos, end));
                }
            }
        }

        let Some((pos, mut end)) = next_hit else {
            result.push_str(&line[cursor..]);
            break;
        };

        let key_slice = &lower[pos..end];
        let mut scan = end;
        if scan < bytes.len() && (bytes[scan] == b'"' || bytes[scan] == b'\'') {
            scan += 1;
        }
        while scan < bytes.len() && bytes[scan] == b' ' {
            scan += 1;
        }
        let is_bearer = key_slice == "bearer";
        let has_sep = if is_bearer {
            end < bytes.len() && bytes[end].is_ascii_whitespace()
        } else {
            scan < bytes.len() && (bytes[scan] == b'=' || bytes[scan] == b':')
        };

        if !has_sep {
            result.push_str(&line[cursor..end]);
            cursor = end;
            continue;
        }

        if is_bearer {
            while end < bytes.len() && bytes[end] == b' ' {
                end += 1;
            }
        } else {
            end = scan + 1;
            while end < bytes.len() && bytes[end] == b' ' {
                end += 1;
            }
            if key_slice == "authorization" {
                for scheme in ["bearer ", "basic ", "token "] {
                    if lower[end..].starts_with(scheme) {
                        end += scheme.len();
                        while end < bytes.len() && bytes[end] == b' ' {
                            end += 1;
                        }
                        break;
                    }
                }
            }
        }

        result.push_str(&line[cursor..end]);

        if end >= bytes.len() {
            break;
        }

        if bytes[end] == b'"' || bytes[end] == b'\'' {
            let quote = bytes[end];
            result.push(char::from(quote));
            end += 1;
            let mut val_end = end;
            while val_end < bytes.len() {
                if bytes[val_end] == b'\\' && val_end + 1 < bytes.len() {
                    val_end += 2;
                    continue;
                }
                if bytes[val_end] == quote {
                    break;
                }
                val_end += 1;
            }
            result.push_str("[REDACTED]");
            if val_end < bytes.len() && bytes[val_end] == quote {
                result.push(char::from(quote));
                cursor = val_end + 1;
            } else {
                cursor = val_end;
            }
        } else {
            let mut val_end = end;
            while val_end < bytes.len()
                && !matches!(bytes[val_end], b' ' | b'\t' | b'\r' | b'\n' | b',' | b';' | b'}' | b']' | b'"' | b'\'')
            {
                val_end += 1;
            }
            if val_end > end {
                result.push_str("[REDACTED]");
            }
            cursor = val_end;
        }
    }

    result
}

#[must_use]
pub fn is_valid_bundle_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
        && !id.contains('\0')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_protocol_requests_and_responses() {
        let req = LogRequest::QueryLogs {
            query: LogQuery {
                min_level: Some(LogLevel::Warn),
                levels: vec![LogLevel::Warn, LogLevel::Error],
                target: Some("glide::gpu".into()),
                source: Some("compositor".into()),
                session_id: Some("sess-1".into()),
                since_ms: Some(1_700_000_000_000),
                until_ms: None,
                grep: Some("pageflip".into()),
                limit: Some(50),
                offset: Some(0),
                newest_first: true,
            },
        };
        let wire = serde_json::to_string(&req).unwrap();
        assert!(wire.contains(r#""type":"query-logs""#));
        let back: LogRequest = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, req);

        let unknown: LogResponse = serde_json::from_str(r#"{"type":"future-variant","x":1}"#).unwrap();
        assert_eq!(unknown, LogResponse::Unknown);
    }

    #[test]
    fn utc_timestamps_roundtrip() {
        let samples = [0_i64, 1_700_000_000_123, 1_795_248_158_456, 946_684_800_000];
        for ms in samples {
            let iso = format_iso_utc(ms);
            assert_eq!(parse_timestamp_ms(&iso), Some(ms), "iso={iso}");
        }
        assert_eq!(format_iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_compact_utc(0), "19700101T000000Z");
    }

    #[test]
    fn redacts_secrets_in_json_and_kv() {
        let text = r#"auth failed password="super-secret" token=abc12345 Authorization: Bearer eyJhbGciOi.xyz normal=ok"#;
        let redacted = redact_sensitive(text);
        assert!(!redacted.contains("super-secret"), "{redacted}");
        assert!(!redacted.contains("abc12345"), "{redacted}");
        assert!(!redacted.contains("eyJhbGciOi"), "{redacted}");
        assert!(redacted.contains("normal=ok"), "{redacted}");
        assert!(redacted.contains("[REDACTED]"), "{redacted}");
    }

    #[test]
    fn bundle_id_rejects_traversal() {
        assert!(is_valid_bundle_id("glide-bundle-20261122T080000Z-panic"));
        assert!(!is_valid_bundle_id("../etc/passwd"));
        assert!(!is_valid_bundle_id("a/b"));
        assert!(!is_valid_bundle_id(""));
    }

    #[test]
    fn query_matching_filters_levels_and_grep() {
        let entry = LogEntry {
            id: 1,
            ts_ms: 1000,
            ts_iso: format_iso_utc(1000),
            session_id: "s1".into(),
            source: "compositor".into(),
            level: LogLevel::Error,
            target: "glide::gpu".into(),
            thread: Some("main".into()),
            pid: Some(42),
            message: "DRM pageflip failed".into(),
            fields: serde_json::Map::new(),
        };
        let q = LogQuery {
            min_level: Some(LogLevel::Warn),
            target: Some("gpu".into()),
            grep: Some("pageflip".into()),
            ..LogQuery::default()
        };
        assert!(q.matches(&entry));
        let q_miss = LogQuery { levels: vec![LogLevel::Info], ..LogQuery::default() };
        assert!(!q_miss.matches(&entry));
    }
}
