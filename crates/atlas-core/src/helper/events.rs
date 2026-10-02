//! `/var/lib/atlas-core/events.jsonl`: update and rollback events the helper
//! records, one JSON object per line, world-readable. Apps turn new lines into
//! crash reports (see `crash::collect_events`) when the user opted in.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};

const MAX_ERROR_CHARS: usize = 300;
pub const DEFAULT_PATH: &str = "/var/lib/atlas-core/events.jsonl";

/// Events that `record-event` accepts from greenboot scripts.
pub const CLI_EVENTS: &[&str] = &["health-check-failed", "health-check-passed"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// `update-staged`, `update-failed`, `rollback-requested`,
    /// `rollback-failed`, `channel-switched`, `channel-switch-failed`,
    /// `update-applied`, `rollback-applied`, `automatic-rollback`,
    /// `health-check-failed`, `health-check-passed`.
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Scrubbed error text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// RFC 3339 UTC.
    pub time: String,
}

impl Event {
    pub fn new(event: &str, version: Option<String>, error: Option<&str>) -> Event {
        let scrubber = crate::crash::Scrubber::new(None, hostname().as_deref());
        Event {
            event: event.to_string(),
            version,
            error: error.map(|e| scrubber.scrub(e).chars().take(MAX_ERROR_CHARS).collect()),
            time: crate::history::now_rfc3339(),
        }
    }
}

fn hostname() -> Option<String> {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|h| h.trim().to_string())
}

const MAX_BYTES: u64 = 512 * 1024;
const KEEP_LINES: usize = 1000;

/// Append one event (file created 0644). Past 512 KiB the file is cut to its
/// last 1000 lines.
pub fn append(path: &Path, event: &Event) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let line = serde_json::to_string(event).map_err(io::Error::other)?;
    crate::fsutil::append_line(path, &line, 0o644)?;
    if fs::symlink_metadata(path)?.len() > MAX_BYTES {
        truncate(path, KEEP_LINES)?;
    }
    Ok(())
}

fn truncate(path: &Path, keep: usize) -> io::Result<()> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(keep)..].join("\n");
    let tmp = path.with_extension("jsonl.tmp");
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)?;
    f.write_all(tail.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    fs::rename(tmp, path)
}

/// All events, oldest first. Missing file or bad lines give fewer events.
pub fn read(path: &Path) -> Vec<Event> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_errors_are_capped_and_file_is_truncated() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.jsonl");
        let e = Event::new("update-failed", None, Some(&"x".repeat(5000)));
        assert_eq!(e.error.as_ref().unwrap().chars().count(), 300);
        for _ in 0..2000 {
            append(&p, &e).unwrap();
        }
        assert!(fs::metadata(&p).unwrap().len() <= MAX_BYTES + 1000);
        assert!(read(&p).len() <= 2000 && read(&p).len() >= KEEP_LINES);
    }

    #[test]
    fn append_and_read_with_scrubbed_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("v/events.jsonl");
        append(
            &p,
            &Event::new(
                "update-failed",
                Some("44.1".into()),
                Some("cannot read /home/zach/x from 10.0.0.2"),
            ),
        )
        .unwrap();
        append(&p, &Event::new("update-staged", None, None)).unwrap();
        let ev = read(&p);
        assert_eq!(ev.len(), 2);
        assert_eq!(
            ev[0].error.as_deref(),
            Some("cannot read /home/USER/x from <ip>")
        );
        assert!(!fs::read_to_string(&p).unwrap().contains("\"error\":null"));
        assert!(read(&d.path().join("none")).is_empty());
    }
}
