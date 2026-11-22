use std::{
    process::{Command, Stdio},
    time::Duration,
};

use glide_log_ipc::{LogEntry, LogLevel, format_iso_utc, now_ms, redact_sensitive};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command as AsyncCommand,
    sync::mpsc,
};

const KERNEL_KEYWORDS: &[&str] = &[
    "drm",
    "nvidia",
    "nvrm",
    "xid",
    "amdgpu",
    "i915",
    "nouveau",
    "pcie",
    "aer",
    "gpu",
    "oom",
    "out of memory",
    "killed process",
    "hung_task",
    "segfault",
    "wayland",
    "glide",
];

pub fn spawn_journal_streams(tx: mpsc::Sender<LogEntry>) {
    let tx_user = tx.clone();
    tokio::spawn(async move {
        run_journal_loop(
            &[
                "--user",
                "-f",
                "-o",
                "json",
                "-n",
                "50",
                "-u",
                "glide.service",
                "-u",
                "glide-configd.service",
                "-u",
                "glide-portal.service",
            ],
            "journal-user",
            false,
            tx_user,
        )
        .await;
    });

    tokio::spawn(async move {
        run_journal_loop(&["-k", "-f", "-o", "json", "-n", "100"], "journal-kernel", true, tx).await;
    });
}

async fn run_journal_loop(args: &[&str], source: &str, kernel_filter: bool, tx: mpsc::Sender<LogEntry>) {
    loop {
        let child = AsyncCommand::new("journalctl")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn();

        let Ok(mut child) = child else {
            tracing::debug!(source, "journalctl unavailable; skipping stream");
            return;
        };

        if let Some(stdout) = child.stdout.take() {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some(entry) = parse_journal_json_line(&line, source, kernel_filter)
                    && tx.send(entry).await.is_err()
                {
                    return;
                }
            }
        }

        let _ = child.wait().await;
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[must_use]
pub fn parse_journal_json_line(line: &str, source: &str, kernel_filter: bool) -> Option<LogEntry> {
    let val: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = val.as_object()?;

    let raw_msg = match obj.get("MESSAGE") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(bytes)) => {
            let raw: Vec<u8> = bytes.iter().filter_map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok())).collect();
            String::from_utf8_lossy(&raw).into_owned()
        }
        _ => return None,
    };

    if kernel_filter {
        let lower = raw_msg.to_ascii_lowercase();
        if !KERNEL_KEYWORDS.iter().any(|k| lower.contains(k)) {
            return None;
        }
    }

    let ts_ms = obj
        .get("__REALTIME_TIMESTAMP")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .map_or_else(now_ms, |us| us / 1_000);

    let priority = obj
        .get("PRIORITY")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(6);
    let level = match priority {
        0..=3 => LogLevel::Error,
        4 => LogLevel::Warn,
        5..=6 => LogLevel::Info,
        _ => LogLevel::Debug,
    };

    let target = obj
        .get("_SYSTEMD_USER_UNIT")
        .or_else(|| obj.get("SYSLOG_IDENTIFIER"))
        .or_else(|| obj.get("_COMM"))
        .and_then(|v| v.as_str())
        .unwrap_or(source)
        .to_owned();

    let pid = obj.get("_PID").and_then(|v| v.as_str()).and_then(|s| s.parse::<u32>().ok());
    let mut fields = serde_json::Map::new();
    for key in ["_SYSTEMD_USER_UNIT", "_COMM", "SYSLOG_IDENTIFIER", "PRIORITY", "_BOOT_ID"] {
        if let Some(v) = obj.get(key) {
            fields.insert(key.to_owned(), v.clone());
        }
    }

    Some(LogEntry {
        id: 0,
        ts_ms,
        ts_iso: format_iso_utc(ts_ms),
        session_id: String::new(),
        source: source.to_owned(),
        level,
        target,
        thread: None,
        pid,
        message: redact_sensitive(&raw_msg),
        fields,
    })
}

#[must_use]
pub fn snapshot_user_journal(lines: usize) -> String {
    run_snapshot(&["--user", "-o", "json", "-n", &lines.to_string(), "--no-pager"])
}

#[must_use]
pub fn snapshot_kernel_journal(lines: usize) -> String {
    run_snapshot(&["-k", "-n", &lines.to_string(), "--no-pager"])
}

fn run_snapshot(args: &[&str]) -> String {
    match Command::new("journalctl")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kernel_pcie_aer_and_skips_unrelated_lines() {
        let aer = r#"{"__REALTIME_TIMESTAMP":"1700000000123000","PRIORITY":"3","SYSLOG_IDENTIFIER":"kernel","MESSAGE":"pcieport 0000:00:01.0: AER: Uncorrected (Fatal) error received"}"#;
        let parsed = parse_journal_json_line(aer, "journal-kernel", true).expect("should match");
        assert_eq!(parsed.level, LogLevel::Error);
        assert_eq!(parsed.ts_ms, 1_700_000_000_123);
        assert!(parsed.message.contains("AER"));

        let unrelated = r#"{"__REALTIME_TIMESTAMP":"1700000000123000","PRIORITY":"6","SYSLOG_IDENTIFIER":"kernel","MESSAGE":"usb 1-2: new full-speed USB device"}"#;
        assert!(parse_journal_json_line(unrelated, "journal-kernel", true).is_none());
    }
}
