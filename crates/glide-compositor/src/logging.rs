use std::{
    fmt,
    fs::File,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

use tracing::field::{Field, Visit};
use tracing_subscriber::{
    EnvFilter, Layer, filter::LevelFilter, layer::Context, layer::SubscriberExt, util::SubscriberInitExt,
};

const SMITHAY_FILTER: &str = "smithay=info,\
     smithay::wayland::selection=debug,\
     smithay::backend::egl=warn,\
     smithay::backend::renderer::gles=warn,\
     smithay::backend::drm=info,\
     smithay::backend::session::libseat=info";

const FILTER_VARIABLE: &str = "GLIDE_LOG";
pub const MAX_LOG_BYTES: u64 = 64 * 1024 * 1024;
const KEPT_LOGS: u32 = 5;

static SYNC_WRITERS: OnceLock<Mutex<Vec<CappedWriter>>> = OnceLock::new();

fn register_sync_writer(writer: CappedWriter) {
    let list = SYNC_WRITERS.get_or_init(|| Mutex::new(Vec::new()));
    list.lock().unwrap_or_else(PoisonError::into_inner).push(writer);
}

pub fn flush_and_sync_all() {
    if let Some(list) = SYNC_WRITERS.get() {
        let writers = list.lock().unwrap_or_else(PoisonError::into_inner).clone();
        for writer in writers {
            writer.flush_and_sync();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Session,
    Greeter,
}

impl Mode {
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Session => "glide.log",
            Self::Greeter => "glide-greeter.log",
        }
    }

    #[must_use]
    pub const fn events_file_name(self) -> &'static str {
        match self {
            Self::Session => "glide.events.jsonl",
            Self::Greeter => "glide-greeter.events.jsonl",
        }
    }

    #[must_use]
    pub fn from_args(mut args: impl Iterator<Item = String>) -> Self {
        if args.any(|arg| arg == "--greeter") { Self::Greeter } else { Self::Session }
    }
}

fn default_filter() -> String {
    let glide = if cfg!(debug_assertions) { "debug" } else { "info" };
    format!("glide={glide},{SMITHAY_FILTER}")
}

fn filter_spec() -> (String, &'static str) {
    for variable in [FILTER_VARIABLE, "RUST_LOG"] {
        if let Ok(spec) = std::env::var(variable)
            && !spec.trim().is_empty()
        {
            return (spec, variable);
        }
    }
    (default_filter(), "default")
}

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_owned());
        let location = info
            .location()
            .map_or_else(|| "<unknown location>".to_owned(), |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("<unnamed>");
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic in thread '{thread}' at {location}: {message}\n{backtrace}");
        flush_and_sync_all();
        previous(info);
    }));
}

#[derive(Debug)]
struct Capped {
    file: File,
    written: u64,
    limit: u64,
    stopped: bool,
}

impl Capped {
    const fn new(file: File, limit: u64) -> Self {
        Self { file, written: 0, limit, stopped: false }
    }

    fn write_line(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.stopped {
            return Ok(buf.len());
        }
        let len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if self.written.saturating_add(len) > self.limit {
            self.stopped = true;
            let notice =
                format!("log reached {} MiB; further lines are dropped until the next start\n", self.limit >> 20);
            self.file.write_all(notice.as_bytes())?;
            let _ = self.file.flush();
            let _ = self.file.sync_data();
            return Ok(buf.len());
        }
        self.written += len;
        self.file.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush_and_sync(&mut self) {
        let _ = self.file.flush();
        let _ = self.file.sync_data();
    }
}

#[derive(Debug, Clone)]
struct CappedWriter(Arc<Mutex<Capped>>);

impl CappedWriter {
    fn new(file: File, limit: u64) -> Self {
        Self(Arc::new(Mutex::new(Capped::new(file, limit))))
    }

    fn flush_and_sync(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).flush_and_sync();
    }
}

impl Write for CappedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).write_line(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).file.flush()
    }
}

#[derive(Default)]
struct JsonFieldVisitor {
    message: Option<String>,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl Visit for JsonFieldVisitor {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_owned(), serde_json::Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), serde_json::Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_owned(), serde_json::Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if let Some(num) = serde_json::Number::from_f64(value) {
            self.fields.insert(field.name().to_owned(), serde_json::Value::Number(num));
        } else {
            self.fields.insert(field.name().to_owned(), serde_json::Value::String(value.to_string()));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.fields.insert(field.name().to_owned(), serde_json::Value::String(value.to_owned()));
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.fields.insert(field.name().to_owned(), serde_json::Value::String(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(rendered);
        } else {
            self.fields.insert(field.name().to_owned(), serde_json::Value::String(rendered));
        }
    }
}

struct JsonlLayer {
    jsonl_writer: CappedWriter,
    text_writer: CappedWriter,
}

impl JsonlLayer {
    const fn new(jsonl_writer: CappedWriter, text_writer: CappedWriter) -> Self {
        Self { jsonl_writer, text_writer }
    }

    fn write_event_line(&self, event: &tracing::Event<'_>) {
        let meta = event.metadata();
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);

        let ts_ms = now_ms();
        let ts_iso = format_iso_utc(ts_ms);
        let thread = std::thread::current().name().map(ToOwned::to_owned);
        let level = match *meta.level() {
            tracing::Level::ERROR => "ERROR",
            tracing::Level::WARN => "WARN",
            tracing::Level::INFO => "INFO",
            tracing::Level::DEBUG => "DEBUG",
            tracing::Level::TRACE => "TRACE",
        };
        let message = visitor.message.unwrap_or_default();
        let record = serde_json::json!({
            "ts_ms": ts_ms,
            "ts_iso": ts_iso,
            "level": level,
            "target": meta.target(),
            "thread": thread,
            "pid": std::process::id(),
            "message": message,
            "fields": visitor.fields,
        });

        if let Ok(mut encoded) = serde_json::to_string(&record) {
            encoded.push('\n');
            let mut writer = self.jsonl_writer.clone();
            let _ = writer.write_all(encoded.as_bytes());
        }

        if *meta.level() <= tracing::Level::WARN {
            self.jsonl_writer.flush_and_sync();
            self.text_writer.flush_and_sync();
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for JsonlLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        self.write_event_line(event);
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn format_iso_utc(ts_ms: i64) -> String {
    let secs = ts_ms.div_euclid(1_000);
    let millis = ts_ms.rem_euclid(1_000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    let hour = sod / 3_600;
    let minute = (sod % 3_600) / 60;
    let second = sod % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

#[must_use]
pub fn init(mode: Mode) -> Option<PathBuf> {
    let (spec, source) = filter_spec();
    let file_filter = EnvFilter::try_new(&spec).unwrap_or_else(|_| EnvFilter::new(default_filter()));
    let jsonl_filter = EnvFilter::try_new(&spec).unwrap_or_else(|_| EnvFilter::new(default_filter()));
    let stderr =
        tracing_subscriber::fmt::layer().with_ansi(false).with_writer(std::io::stderr).with_filter(LevelFilter::WARN);
    let Some((path, file, jsonl_file)) = open_log_files(mode) else {
        let everything = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_thread_names(true)
            .with_writer(std::io::stderr)
            .with_filter(file_filter);
        tracing_subscriber::registry().with(everything).init();
        log_header(mode, None, &spec, source);
        return None;
    };
    let writer = CappedWriter::new(file, MAX_LOG_BYTES);
    register_sync_writer(writer.clone());
    let jsonl_layer = jsonl_file.map(|jf| {
        let jsonl_writer = CappedWriter::new(jf, MAX_LOG_BYTES);
        register_sync_writer(jsonl_writer.clone());
        JsonlLayer::new(jsonl_writer, writer.clone()).with_filter(jsonl_filter)
    });
    let to_file = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_thread_names(true)
        .with_writer(move || writer.clone())
        .with_filter(file_filter);
    tracing_subscriber::registry().with(to_file).with(jsonl_layer).with(stderr).init();
    log_header(mode, Some(&path), &spec, source);
    Some(path)
}

fn kernel() -> String {
    // SAFETY: `uname` only writes into the zeroed struct it is given.
    let mut name: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `name` is a valid, writable `utsname`.
    if unsafe { libc::uname(&raw mut name) } != 0 {
        return "unknown".into();
    }
    let field = |chars: &[libc::c_char]| {
        let bytes: Vec<u8> = chars.iter().take_while(|c| **c != 0).map(|c| c.to_ne_bytes()[0]).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    format!("{} {} {}", field(&name.sysname), field(&name.release), field(&name.machine))
}

fn log_header(mode: Mode, path: Option<&Path>, filter: &str, source: &str) {
    let env = |name: &str| std::env::var(name).unwrap_or_default();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        ?mode,
        pid = std::process::id(),
        kernel = kernel(),
        log = ?path,
        filter,
        filter_from = source,
        session_type = env("XDG_SESSION_TYPE"),
        desktop = env("XDG_CURRENT_DESKTOP"),
        vt = env("XDG_VTNR"),
        seat = env("XDG_SEAT"),
        "starting glide"
    );
}

fn rotate(dir: &Path, name: &str, kept: u32) {
    for index in (1..kept).rev() {
        let _ = std::fs::rename(dir.join(format!("{name}.{index}")), dir.join(format!("{name}.{}", index + 1)));
    }
    let _ = std::fs::rename(dir.join(name), dir.join(format!("{name}.1")));
}

fn open_log_files(mode: Mode) -> Option<(PathBuf, File, Option<File>)> {
    let dir = log_dirs().into_iter().find(|dir| usable_dir(dir))?;
    let name = mode.file_name();
    rotate(&dir, name, KEPT_LOGS);
    let path = dir.join(name);
    let file = File::create(&path).ok()?;

    let events_name = mode.events_file_name();
    rotate(&dir, events_name, KEPT_LOGS);
    let events_file = File::create(dir.join(events_name)).ok();

    Some((path, file, events_file))
}

fn usable_dir(dir: &Path) -> bool {
    let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
    // SAFETY: `getuid` has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir() && meta.uid() == uid) && tempfile_probe(dir)
}

fn tempfile_probe(dir: &Path) -> bool {
    let probe = dir.join(format!(".write-probe-{}", std::process::id()));
    let writable = File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    writable
}

fn log_dirs() -> Vec<PathBuf> {
    // SAFETY: `getuid` has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    glide_config::state_home()
        .into_iter()
        .chain(glide_config::runtime_dir())
        .map(|base| base.join("glide"))
        .chain(std::iter::once(std::env::temp_dir().join(format!("glide-{uid}"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_five_logs_survive_repeated_restarts() {
        let dir = std::env::temp_dir().join(format!("glide-log-rotate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let name = Mode::Session.file_name();
        for run in 1..=8 {
            rotate(&dir, name, KEPT_LOGS);
            std::fs::write(dir.join(name), format!("run {run}")).expect("log is writable");
        }
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
        assert_eq!(read(name).as_deref(), Some("run 8"));
        assert_eq!(read("glide.log.1").as_deref(), Some("run 7"), "the crashed run is one restart back");
        assert_eq!(read("glide.log.5").as_deref(), Some("run 3"));
        assert_eq!(read("glide.log.6"), None, "older runs are dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_greeter_keeps_a_log_of_its_own() {
        assert_eq!(Mode::from_args(["--greeter".to_string()].into_iter()), Mode::Greeter);
        assert_eq!(Mode::from_args(std::iter::empty()), Mode::Session);
        assert_ne!(Mode::Greeter.file_name(), Mode::Session.file_name());
    }

    #[test]
    fn a_runaway_log_stops_at_its_cap_with_one_notice() {
        let path = std::env::temp_dir().join(format!("glide-log-cap-{}", std::process::id()));
        let file = File::create(&path).expect("scratch file");
        let mut capped = Capped::new(file, 10);
        assert_eq!(capped.write_line(b"12345\n").expect("write"), 6);
        assert_eq!(capped.write_line(b"67890\n").expect("write"), 6, "reported as written so tracing carries on");
        capped.write_line(b"more\n").expect("write");
        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(text.starts_with("12345\n"));
        assert_eq!(text.matches("further lines are dropped").count(), 1);
        assert!(!text.contains("67890"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_fallback_directory_is_per_user_not_per_process() {
        let dirs = log_dirs();
        let last = dirs.last().expect("there is always a fallback");
        assert!(last.starts_with(std::env::temp_dir()));
        assert!(!last.to_string_lossy().contains(&std::process::id().to_string()));
    }

    #[test]
    fn jsonl_layer_records_structured_events_and_flushes_on_warn_error() {
        let dir = std::env::temp_dir().join(format!("glide-log-jsonl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let text_path = dir.join(Mode::Session.file_name());
        let jsonl_path = dir.join(Mode::Session.events_file_name());
        let text_writer = CappedWriter::new(File::create(&text_path).expect("text file"), MAX_LOG_BYTES);
        let jsonl_writer = CappedWriter::new(File::create(&jsonl_path).expect("jsonl file"), MAX_LOG_BYTES);
        let subscriber = tracing_subscriber::registry().with(JsonlLayer::new(jsonl_writer, text_writer));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: "glide::gpu", code = 42_u64, "pageflip stalled");
            tracing::error!(reason = "drm", "fatal render error");
        });
        let content = std::fs::read_to_string(&jsonl_path).expect("read jsonl");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("valid json");
        assert_eq!(first["level"], "WARN");
        assert_eq!(first["target"], "glide::gpu");
        assert_eq!(first["message"], "pageflip stalled");
        assert_eq!(first["fields"]["code"], 42);
        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("valid json");
        assert_eq!(second["level"], "ERROR");
        assert_eq!(second["fields"]["reason"], "drm");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
