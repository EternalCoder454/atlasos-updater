//! `/var/lib/atlas-core/events.jsonl`: update and rollback events the helper
//! records, one JSON object per line, world-readable. Apps turn new lines into
//! crash reports (see `crash::collect_events`) when the user opted in.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};

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
            error: error.map(|e| scrubber.scrub(e)),
            time: crate::history::now_rfc3339(),
        }
    }
}

fn hostname() -> Option<String> {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|h| h.trim().to_string())
}

/// Append one event (file created 0644).
pub fn append(path: &Path, event: &Event) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(event).map_err(io::Error::other)?;
    line.push('\n');
    OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o644)
        .open(path)?
        .write_all(line.as_bytes())
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
