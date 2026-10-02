//! Boot history: `/var/lib/atlas-core/history.jsonl`, one JSON object per line,
//! appended by `atlas-system-helper record-boot` when the booted digest changes.

use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::bootc::Status;

pub const DEFAULT_PATH: &str = "/var/lib/atlas-core/history.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub version: Option<String>,
    pub digest: String,
    #[serde(default)]
    pub image: String,
    /// Image build time, RFC 3339.
    #[serde(default)]
    pub timestamp: Option<String>,
    /// When this machine first booted it, RFC 3339 UTC.
    pub first_booted: String,
}

/// Read the history at the default path, newest first.
pub fn read_default() -> io::Result<Vec<Entry>> {
    read(Path::new(DEFAULT_PATH))
}

/// Read a history file, newest first. A missing file is an empty history;
/// lines that do not parse are skipped.
pub fn read(path: &Path) -> io::Result<Vec<Entry>> {
    let bytes = match fs::read(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut v: Vec<Entry> = crate::fsutil::lossy_lines(&bytes)
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    v.reverse();
    Ok(v)
}

/// Append `entry` unless the last entry has the same digest. Returns whether
/// a line was written. The file is created 0644.
pub fn append_if_new(path: &Path, entry: &Entry) -> io::Result<bool> {
    if let Some(last) = read(path)?.first()
        && last.digest == entry.digest
    {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let line = serde_json::to_string(entry).map_err(io::Error::other)?;
    crate::fsutil::append_line(path, &line, 0o644)?;
    Ok(true)
}

/// Record the booted deployment of `status`. `now` is RFC 3339 (see [`now_rfc3339`]).
pub fn record_boot(path: &Path, status: &Status, now: &str) -> io::Result<bool> {
    let booted = status
        .status
        .booted
        .as_ref()
        .and_then(|b| b.image.as_ref())
        .ok_or_else(|| io::Error::other("bootc reports no booted image"))?;
    let entry = Entry {
        version: booted.version.clone(),
        digest: booted.image_digest.clone(),
        image: booted.image.image.clone(),
        timestamp: booted.timestamp.clone(),
        first_booted: now.to_string(),
    };
    append_if_new(path, &entry)
}

/// The current time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    rfc3339_from_unix(secs)
}

/// Format Unix seconds as UTC RFC 3339 (civil-from-days, no dependencies).
pub fn rfc3339_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootc::fixtures::BOOTED_WITH_UPDATE;
    use std::os::unix::fs::PermissionsExt;

    fn entry(digest: &str) -> Entry {
        Entry {
            version: Some("44.1".into()),
            digest: digest.into(),
            image: "x:stable".into(),
            timestamp: None,
            first_booted: "2026-10-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn missing_file_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert!(read(&d.path().join("nope.jsonl")).unwrap().is_empty());
    }

    #[test]
    fn append_dedups_on_last_digest_and_reads_newest_first() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/history.jsonl");
        assert!(append_if_new(&p, &entry("sha256:a")).unwrap());
        assert!(!append_if_new(&p, &entry("sha256:a")).unwrap());
        assert!(append_if_new(&p, &entry("sha256:b")).unwrap());
        // going back to an older digest is a new boot of it
        assert!(append_if_new(&p, &entry("sha256:a")).unwrap());
        let got: Vec<_> = read(&p).unwrap().into_iter().map(|e| e.digest).collect();
        assert_eq!(got, ["sha256:a", "sha256:b", "sha256:a"]);
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn torn_utf8_line_spoils_only_itself() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.jsonl");
        let mut data = b"{\"digest\":\"sha256:a\",\"first_booted\":\"t\"}\n".to_vec();
        data.extend_from_slice(b"{\"digest\":\"sha256:b\",\"image\":\"\xe2\x82");
        fs::write(&p, data).unwrap();
        assert_eq!(read(&p).unwrap().len(), 1);
        assert!(append_if_new(&p, &entry("sha256:c")).unwrap());
        assert_eq!(read(&p).unwrap().len(), 2);
    }

    #[test]
    fn bad_lines_are_skipped() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.jsonl");
        fs::write(
            &p,
            "garbage\n{\"digest\":\"sha256:a\",\"first_booted\":\"t\"}\n\n",
        )
        .unwrap();
        assert_eq!(read(&p).unwrap().len(), 1);
    }

    #[test]
    fn record_boot_writes_booted_entry_once() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.jsonl");
        let s = Status::from_json(BOOTED_WITH_UPDATE).unwrap();
        assert!(record_boot(&p, &s, "2026-10-02T10:00:00Z").unwrap());
        assert!(!record_boot(&p, &s, "2026-10-03T10:00:00Z").unwrap());
        let e = &read(&p).unwrap()[0];
        assert_eq!(e.version.as_deref(), Some("44.20261001"));
        assert_eq!(e.image, "ghcr.io/eternalcoder454/atlasos:stable");
        assert_eq!(e.timestamp.as_deref(), Some("2026-10-01T04:12:09Z"));
        assert_eq!(e.first_booted, "2026-10-02T10:00:00Z");
    }

    #[test]
    fn record_boot_without_booted_image_fails() {
        let d = tempfile::tempdir().unwrap();
        let s = Status::default();
        assert!(record_boot(&d.path().join("h"), &s, "t").is_err());
    }

    #[test]
    fn rfc3339_formatting() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1_790_000_000), "2026-09-21T14:13:20Z");
    }
}
