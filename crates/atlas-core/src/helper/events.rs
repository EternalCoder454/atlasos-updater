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
        // paths and addresses included: the file is world-readable
        let scrubber = crate::crash::Scrubber::for_system();
        Event {
            event: event.to_string(),
            version,
            error: error.map(|e| {
                scrubber
                    .scrub_message(e)
                    .chars()
                    .take(MAX_ERROR_CHARS)
                    .collect()
            }),
            time: crate::history::now_rfc3339(),
        }
    }
}

/// The file is cut when it grows past `MAX_BYTES`, down to at most
/// `KEEP_BYTES` and `KEEP_LINES` (a line is at most about 2.5 KB even with a
/// 300 char error full of escapes, so one cut always lands far below the
/// limit).
const MAX_BYTES: u64 = 512 * 1024;
const KEEP_BYTES: usize = 256 * 1024;
const KEEP_LINES: usize = 1000;

/// Append one event (file created 0644). A lock file serializes the helper
/// and `record-event` (greenboot), so no line is lost to a concurrent cut.
/// A failed cut is logged and does not fail the append.
pub fn append(path: &Path, event: &Event) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let line = serde_json::to_string(event).map_err(io::Error::other)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path.with_extension("jsonl.lock"))?;
    lock.lock()?; // released when `lock` is dropped
    crate::fsutil::append_line(path, &line, 0o644)?;
    if fs::symlink_metadata(path)?.len() > MAX_BYTES
        && let Err(e) = truncate(path)
    {
        eprintln!("atlas-system-helper: cannot shrink {}: {e}", path.display());
    }
    Ok(())
}

fn truncate(path: &Path) -> io::Result<()> {
    let bytes = fs::read(path)?;
    let lines: Vec<&[u8]> = bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    let (mut total, mut first) = (0usize, lines.len());
    while first > 0
        && lines.len() - first < KEEP_LINES
        && total + lines[first - 1].len() < KEEP_BYTES
    {
        first -= 1;
        total += lines[first].len() + 1;
    }
    let tmp = path.with_extension(format!("jsonl.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)?;
    let res = (|| {
        for l in &lines[first..] {
            f.write_all(l)?;
            f.write_all(b"\n")?;
        }
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// All events, oldest first. A missing file or bad lines give fewer events.
pub fn read(path: &Path) -> Vec<Event> {
    crate::fsutil::lossy_lines(&fs::read(path).unwrap_or_default())
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_appenders_with_cuts_lose_no_structure() {
        let d = tempfile::tempdir().unwrap();
        let p = std::sync::Arc::new(d.path().join("e.jsonl"));
        // escapes make every error 6x bigger in the file: forces many cuts
        let err = "\u{1}\"\\".repeat(100);
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let (p, err) = (p.clone(), err.clone());
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        append(&p, &Event::new("update-failed", None, Some(&err))).unwrap();
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let text = fs::read_to_string(&*p).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert!(!lines.is_empty() && lines.len() <= 1600);
        assert!(
            lines
                .iter()
                .all(|l| serde_json::from_str::<Event>(l).is_ok()),
            "torn line"
        );
        assert!(fs::metadata(&*p).unwrap().len() <= MAX_BYTES + 4096);
        let stray: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(stray.is_empty());
    }

    #[test]
    fn one_cut_goes_well_below_the_limit() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.jsonl");
        let e = Event::new("update-failed", None, Some(&"\u{1}".repeat(300)));
        let line = serde_json::to_string(&e).unwrap().len() as u64 + 1;
        assert!(line > 1500, "escaped error line is {line} bytes");
        let (mut prev, mut cut) = (0, false);
        for _ in 0..2000 {
            append(&p, &e).unwrap();
            let now = fs::metadata(&p).unwrap().len();
            if now < prev {
                cut = true;
                break;
            }
            prev = now;
        }
        assert!(cut, "no cut seen");
        assert!(fs::metadata(&p).unwrap().len() <= KEEP_BYTES as u64 + 4096);
    }

    #[test]
    fn torn_utf8_line_spoils_only_itself() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.jsonl");
        append(&p, &Event::new("update-staged", None, None)).unwrap();
        let mut data = fs::read(&p).unwrap();
        data.extend_from_slice(b"{\"event\":\"x\",\"time\":\"\xe2\x82");
        fs::write(&p, data).unwrap();
        append(&p, &Event::new("rollback-requested", None, None)).unwrap();
        let names: Vec<_> = read(&p).into_iter().map(|e| e.event).collect();
        assert_eq!(names, ["update-staged", "rollback-requested"]);
    }

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
        assert!(read(&p).len() <= 2000 && !read(&p).is_empty());
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
        assert_eq!(ev[0].error.as_deref(), Some("cannot read <path>"));
        assert!(!fs::read_to_string(&p).unwrap().contains("\"error\":null"));
        assert!(read(&d.path().join("none")).is_empty());
    }
}
