//! Live progress of an update, as the helper reports it in its `Progress`
//! property (see DESIGN.md), and the parsers that turn what bootc and
//! rpm-ostree print into it. The parsers are pure: they take bytes and return
//! [`Progress`] values, so unit tests run them over captured output.

use serde::{Deserialize, Serialize};

/// What the helper is doing right now. `total == 0` means unknown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// `"upgrade"` or `"switch"`.
    pub op: String,
    /// `"downloading"` or `"installing"`.
    pub stage: String,
    /// Bytes when downloading, steps when installing.
    pub done: u64,
    /// 0 = unknown (show an indeterminate bar).
    pub total: u64,
    /// What is happening, e.g. "Deploying Image"; may be empty.
    pub detail: String,
}

/// `Progress::op` of the operations that change the OS image: an update being
/// downloaded and staged, and a switch of channel (or driver image). They are
/// the only values the helper reports; the screen-edge glow follows exactly
/// these ([`is_image_operation`]).
pub const OP_UPGRADE: &str = "upgrade";
pub const OP_SWITCH: &str = "switch";

/// Whether the helper's `Progress` property value (`""` when nothing runs)
/// says that the OS image is being changed: JSON whose `op` is
/// [`OP_UPGRADE`] or [`OP_SWITCH`]. Anything else (empty, not JSON, another
/// `op`) is not.
pub fn is_image_operation(json: &str) -> bool {
    // Only `op` is looked at, so a newer helper that adds fields still counts.
    serde_json::from_str::<serde_json::Value>(json)
        .is_ok_and(|v| matches!(v["op"].as_str(), Some(OP_UPGRADE | OP_SWITCH)))
}

pub const DOWNLOADING: &str = "downloading";
pub const INSTALLING: &str = "installing";

/// Longest line kept; a longer one is dropped (a runaway child must not make
/// the helper buffer without limit).
const MAX_LINE: usize = 64 * 1024;

/// Splits a byte stream into lines, however the chunks fall.
#[derive(Default)]
struct Lines {
    buf: Vec<u8>,
    skipping: bool,
}

impl Lines {
    /// The complete lines in `chunk` (without the newline).
    fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for part in chunk.split_inclusive(|b| *b == b'\n') {
            let ends = part.ends_with(b"\n");
            let body = if ends { &part[..part.len() - 1] } else { part };
            if !self.skipping {
                if self.buf.len() + body.len() > MAX_LINE {
                    self.skipping = true;
                    self.buf.clear();
                } else {
                    self.buf.extend_from_slice(body);
                }
            }
            if ends {
                if !self.skipping {
                    out.push(String::from_utf8_lossy(&self.buf).into_owned());
                }
                self.buf.clear();
                self.skipping = false;
            }
        }
        out
    }

    /// The incomplete last line.
    fn partial(&self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }
}

/// Parser for `bootc upgrade|switch --progress-fd`: one JSON object per line.
/// Importing and staging are both step counts; they are joined into one range
/// so the bar does not go backwards between them.
#[derive(Default)]
pub struct BootcParser {
    lines: Lines,
    importing_total: u64,
}

impl BootcParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes read from the progress pipe; returns the updates in them.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Progress> {
        self.lines
            .feed(chunk)
            .iter()
            .filter_map(|l| self.line(l))
            .collect()
    }

    /// One JSON line; `None` for anything that is not a progress update.
    pub fn line(&mut self, line: &str) -> Option<Progress> {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        let num = |k: &str| v.get(k).and_then(serde_json::Value::as_u64);
        let task = v.get("task").and_then(|t| t.as_str()).unwrap_or("");
        match v.get("type")?.as_str()? {
            "ProgressBytes" => Some(Progress {
                stage: DOWNLOADING.into(),
                done: num("bytes")?,
                total: num("bytesTotal").unwrap_or(0),
                ..Progress::default()
            }),
            "ProgressSteps" => {
                let (steps, total) = (num("steps")?, num("stepsTotal")?);
                let current = v
                    .get("subtasks")
                    .and_then(|s| s.as_array())
                    .and_then(|s| {
                        s.iter().find(|t| {
                            t.get("completed").and_then(serde_json::Value::as_bool) == Some(false)
                        })
                    })
                    .and_then(|t| t.get("description")?.as_str())
                    .or_else(|| v.get("description")?.as_str())
                    .unwrap_or("")
                    .to_string();
                let (done, total) = if task == "importing" {
                    self.importing_total = total;
                    (steps, total)
                } else {
                    (
                        self.importing_total.saturating_add(steps),
                        self.importing_total.saturating_add(total),
                    )
                };
                Some(Progress {
                    stage: INSTALLING.into(),
                    done,
                    total,
                    detail: current,
                    ..Progress::default()
                })
            }
            _ => None,
        }
    }
}

/// The steps rpm-ostree prints after downloading, in order. A line starting
/// with the text (`Running ...` for the script steps) is that step.
const RPM_OSTREE_STEPS: [(&str, &str); 8] = [
    ("Checking out tree", "Checking out tree"),
    ("Importing rpm-md", "Importing rpm-md"),
    ("Resolving dependencies", "Resolving dependencies"),
    ("Checking out packages", "Checking out packages"),
    ("Running ", "Running scripts"),
    ("Writing rpmdb", "Writing rpmdb"),
    ("Writing OSTree commit", "Writing OSTree commit"),
    ("Staging deployment", "Staging deployment"),
];

/// Stateful parser for the stdout of `rpm-ostree upgrade|rebase` (not a
/// terminal). Feed it chunks as they arrive; they may split lines.
#[derive(Default)]
pub struct RpmOstreeParser {
    lines: Lines,
    needed: u64,
    finished: u64,
    /// Index of the current step once installing started.
    step: Option<usize>,
    /// The incomplete line last looked at, so it is not reported twice.
    seen_partial: String,
}

impl RpmOstreeParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed stdout bytes; returns the updates in them.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Progress> {
        let mut out = Vec::new();
        let complete = self.lines.feed(chunk);
        if !complete.is_empty() {
            self.seen_partial.clear();
        }
        for line in complete {
            out.extend(self.line(&line, true));
        }
        // "[1/2] Fetching layer x (300 MB)..." has no newline until it is done
        let partial = self.lines.partial();
        if partial.ends_with("...") && partial != self.seen_partial {
            out.extend(self.line(&partial, false));
            self.seen_partial = partial;
        }
        out
    }

    /// A line; `complete` is false for a started step whose end has not come.
    fn line(&mut self, line: &str, complete: bool) -> Option<Progress> {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ostree chunk layers needed:") {
            return self.need(rest);
        }
        if let Some(rest) = line.strip_prefix("custom layers needed:") {
            return self.need(rest);
        }
        if line.starts_with('[')
            && (line.contains("Fetching layer ") || line.contains("Fetching ostree chunk "))
        {
            if complete && line.ends_with("...done") {
                self.finished += parse_size(paren(line)?)?;
            }
            return Some(self.downloading());
        }
        let name = line
            .strip_suffix("...done")
            .or_else(|| line.strip_suffix("..."))?;
        let idx = RPM_OSTREE_STEPS
            .iter()
            .position(|(prefix, _)| name.starts_with(prefix))?;
        // never backwards (the script steps repeat)
        if self.step.is_some_and(|s| idx < s) {
            return None;
        }
        let first = self.step != Some(idx);
        self.step = Some(idx);
        // the repeated lines of one step change nothing
        first.then(|| Progress {
            stage: INSTALLING.into(),
            done: idx as u64,
            total: RPM_OSTREE_STEPS.len() as u64,
            detail: RPM_OSTREE_STEPS[idx].1.into(),
            ..Progress::default()
        })
    }

    fn need(&mut self, rest: &str) -> Option<Progress> {
        self.needed += parse_size(paren(rest)?)?;
        Some(self.downloading())
    }

    fn downloading(&self) -> Progress {
        Progress {
            stage: DOWNLOADING.into(),
            // never more done than needed
            done: if self.needed > 0 {
                self.finished.min(self.needed)
            } else {
                self.finished
            },
            total: self.needed,
            ..Progress::default()
        }
    }
}

/// The text of the last `( ... )` in `s`.
fn paren(s: &str) -> Option<&str> {
    let end = s.rfind(')')?;
    let start = s[..end].rfind('(')?;
    Some(&s[start + 1..end])
}

/// A GLib `g_format_size` text ("300.0 MB", "12 bytes", "1.5 GiB") in bytes.
/// The space is a no-break space in some locales' output; both count.
fn parse_size(s: &str) -> Option<u64> {
    let mut parts = s.split_whitespace();
    let n: f64 = parts.next()?.replace(',', ".").parse().ok()?;
    let mult = match parts.next()? {
        "byte" | "bytes" | "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0_f64.powi(4),
        _ => return None,
    };
    (n.is_finite() && n >= 0.0).then(|| (n * mult).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOTC: &str = include_str!("../tests/fixtures/bootc-progress.jsonl");
    const RPM: &str = include_str!("../tests/fixtures/rpm-ostree-upgrade.txt");

    #[test]
    fn json_round_trip_uses_camel_case_names() {
        let p = Progress {
            op: "upgrade".into(),
            stage: "downloading".into(),
            done: 1,
            total: 2,
            detail: "x".into(),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(
            j,
            r#"{"op":"upgrade","stage":"downloading","done":1,"total":2,"detail":"x"}"#
        );
        assert_eq!(serde_json::from_str::<Progress>(&j).unwrap(), p);
    }

    #[test]
    fn only_an_upgrade_or_a_switch_is_an_image_operation() {
        let j = |op: &str| {
            format!(r#"{{"op":"{op}","stage":"downloading","done":1,"total":2,"detail":""}}"#)
        };
        assert!(is_image_operation(&j("upgrade")));
        assert!(is_image_operation(&j("switch")));
        // not image operations
        for op in ["apps", "firmware", "check", "flatpak", ""] {
            assert!(!is_image_operation(&j(op)), "{op}");
        }
        assert!(!is_image_operation(""));
        assert!(!is_image_operation("not json"));
        assert!(!is_image_operation(r#"{"stage":"downloading"}"#));
        // what the helper serves for both operations
        let p = Progress {
            op: OP_UPGRADE.into(),
            ..Progress::default()
        };
        assert!(is_image_operation(&serde_json::to_string(&p).unwrap()));
    }

    #[test]
    fn bootc_fixture_goes_from_download_to_staging() {
        let mut p = BootcParser::new();
        let all = p.feed(BOOTC.as_bytes());
        // Start is not progress; every other line is
        assert_eq!(all.len(), BOOTC.lines().count() - 1);
        let first = &all[0];
        assert_eq!(first.stage, "downloading");
        assert_eq!((first.done, first.total), (8193, 300_028_591));
        let last_dl = all.iter().rfind(|p| p.stage == "downloading").unwrap();
        assert_eq!(last_dl.done, last_dl.total);
        let last = all.last().unwrap();
        assert_eq!(last.stage, "installing");
        // importing (1 step) + staging (3 steps), all done
        assert_eq!((last.done, last.total), (4, 4));
        assert_eq!(
            last.detail, "Deploying Image",
            "all done: the step's own name"
        );
        let staging = all.iter().find(|p| p.detail == "Merging Image").unwrap();
        assert_eq!((staging.done, staging.total), (1, 4));
        // never backwards while installing
        let steps: Vec<u64> = all
            .iter()
            .filter(|p| p.stage == "installing")
            .map(|p| p.done)
            .collect();
        assert!(steps.windows(2).all(|w| w[0] <= w[1]), "{steps:?}");
    }

    #[test]
    fn bootc_ignores_junk_and_splits_chunks() {
        let mut p = BootcParser::new();
        assert!(p.feed(b"not json\n{\"type\":\"Start\"}\n{}\n").is_empty());
        let line = r#"{"type":"ProgressBytes","task":"pulling","bytes":5,"bytesTotal":10}"#;
        let (a, b) = line.split_at(20);
        assert!(p.feed(a.as_bytes()).is_empty());
        let got = p.feed(format!("{b}\n").as_bytes());
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].done, got[0].total), (5, 10));
    }

    fn rpm(chunk: usize) -> Vec<Progress> {
        let mut p = RpmOstreeParser::new();
        RPM.as_bytes()
            .chunks(chunk)
            .flat_map(|c| p.feed(c))
            .collect()
    }

    #[test]
    fn rpm_ostree_fixture_downloads_then_installs() {
        let all = rpm(RPM.len());
        let dl: Vec<_> = all.iter().filter(|p| p.stage == "downloading").collect();
        assert_eq!((dl[0].done, dl[0].total), (0, 300_000_000));
        let last_dl = dl.last().unwrap();
        assert_eq!((last_dl.done, last_dl.total), (300_000_000, 300_000_000));
        let inst: Vec<_> = all.iter().filter(|p| p.stage == "installing").collect();
        let details: Vec<&str> = inst.iter().map(|p| p.detail.as_str()).collect();
        assert_eq!(
            details,
            [
                "Checking out tree",
                "Importing rpm-md",
                "Resolving dependencies",
                "Checking out packages",
                "Running scripts",
                "Writing rpmdb",
                "Writing OSTree commit",
                "Staging deployment"
            ]
        );
        assert_eq!(inst[0].done, 0);
        assert_eq!(inst.last().unwrap().done, 7);
        assert!(inst.iter().all(|p| p.total == 8));
        // downloading comes strictly before installing
        let first_inst = all.iter().position(|p| p.stage == "installing").unwrap();
        assert!(all[first_inst..].iter().all(|p| p.stage == "installing"));
    }

    #[test]
    fn rpm_ostree_gives_the_same_result_for_any_chunking() {
        let whole = rpm(RPM.len());
        for n in [1, 2, 7, 64] {
            let mut got = rpm(n);
            got.dedup();
            let mut want = whole.clone();
            want.dedup();
            assert_eq!(got, want, "chunk size {n}");
        }
    }

    #[test]
    fn rpm_ostree_a_started_layer_is_seen_before_it_finishes() {
        let mut p = RpmOstreeParser::new();
        p.feed(b"custom layers needed: 2 (300.0\xc2\xa0MB)\n");
        let got = p.feed(b"[1/2] Fetching layer abc (100.0 MB)...");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].done, 0, "a started layer is not finished");
        let got = p.feed(b"done\n");
        assert_eq!((got[0].done, got[0].total), (100_000_000, 300_000_000));
    }

    #[test]
    fn rpm_ostree_counts_ostree_chunks_and_layers_and_never_exceeds_the_total() {
        let mut p = RpmOstreeParser::new();
        p.feed(b"ostree chunk layers needed: 2 (200.0 MB)\ncustom layers needed: 1 (100.0 MB)\n");
        let got = p.feed(b"[1/3] Fetching ostree chunk sha256:abc (100.0 MB)...done\n");
        assert_eq!((got[0].done, got[0].total), (100_000_000, 300_000_000));
        let got = p.feed(b"[2/3] Fetching ostree chunk sha256:def (100.0 MB)...done\n");
        assert_eq!(got[0].done, 200_000_000);
        p.feed(b"[3/3] Fetching layer 2578b (100.0 MB)...done\n");
        // a size that rounds up past the total is clamped
        let got = p.feed(b"[4/3] Fetching layer zzz (100.0 MB)...done\n");
        assert_eq!((got[0].done, got[0].total), (300_000_000, 300_000_000));
    }

    #[test]
    fn rpm_ostree_does_not_report_the_same_partial_line_twice() {
        let mut p = RpmOstreeParser::new();
        p.feed(b"custom layers needed: 1 (100.0 MB)\n");
        assert_eq!(p.feed(b"[1/1] Fetching layer abc (100.0 MB)...").len(), 1);
        assert!(p.feed(b"").is_empty());
        assert_eq!(p.feed(b"done\n").len(), 1);
    }

    #[test]
    fn rpm_ostree_sums_both_kinds_of_needed_layers() {
        let mut p = RpmOstreeParser::new();
        p.feed(b"ostree chunk layers needed: 3 (1.5 GB)\n");
        let got = p.feed(b"custom layers needed: 1 (500.0 kB)\n");
        assert_eq!(got[0].total, 1_500_500_000);
    }

    #[test]
    fn rpm_ostree_with_nothing_needed_goes_straight_to_installing() {
        let mut p = RpmOstreeParser::new();
        let got = p.feed(
            b"Pulling manifest: x\nostree chunk layers already present: 65\nChecking out tree 4013e8a...done\n",
        );
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].stage.as_str(), got[0].done), ("installing", 0));
    }

    #[test]
    fn rpm_ostree_unknown_lines_are_ignored_and_steps_never_go_back() {
        let mut p = RpmOstreeParser::new();
        assert!(
            p.feed(b"Enabled rpm-md repositories: a b\nFreed: 48.1 kB\n")
                .is_empty()
        );
        p.feed(b"Writing rpmdb...done\n");
        assert!(p.feed(b"Running pre scripts...done\n").is_empty());
        assert!(p.feed(b"Writing rpmdb...done\n").is_empty());
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("300.0\u{a0}MB"), Some(300_000_000));
        assert_eq!(parse_size("12 bytes"), Some(12));
        assert_eq!(parse_size("1.5 GiB"), Some(1_610_612_736));
        assert_eq!(parse_size("1 parsec"), None);
    }

    #[test]
    fn overlong_lines_are_dropped_not_buffered() {
        let mut l = Lines::default();
        assert!(l.feed(&vec![b'a'; MAX_LINE + 10]).is_empty());
        assert!(l.feed(b"more\nnext\n") == ["next"]);
    }
}
