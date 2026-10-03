//! The Changelog page: the release notes of every AtlasOS version this
//! computer went through, from GitHub's release list and the machine's
//! history. Versions it skipped over count too (their changes are in the
//! system), so the list runs from the oldest version it ran to the newest one
//! downloaded or offered.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::notes::Release;

/// How long the cached release list is used before asking GitHub again.
pub const FRESH_FOR: Duration = Duration::from_secs(60 * 60);

/// A fresh list that lacks a version this computer has or is offered is
/// asked for again once it is this old (a release published since).
pub const RECHECK_AFTER: Duration = Duration::from_secs(10 * 60);

/// One version on the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub version: String,
    /// When it was published (RFC 3339); empty for one without a release.
    pub date: String,
    /// "current", "staged", "available", "ran" (this computer ran it) or ""
    /// (it came in a later update this computer installed).
    pub state: String,
    /// When this computer first started it (RFC 3339); empty if it never did.
    pub first_booted: String,
    pub html: String,
    pub plain: String,
}

/// A version this computer ran, from its history.
pub struct Ran<'a> {
    pub version: &'a str,
    pub first_booted: &'a str,
}

/// Orders 44.20261002-style versions by their numbers. A part that isn't a
/// number sorts after one that is, and among its kind by text: a total
/// order, which sorting needs.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let mut x = a.split(['.', '-']);
    let mut y = b.split(['.', '-']);
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// The page's list, newest first. `current`, `staged` and `available` are
/// the versions bootc shows (empty when there is none).
pub fn merge(
    releases: &[Release],
    ran: &[Ran],
    current: &str,
    staged: &str,
    available: &str,
) -> Vec<Item> {
    // The oldest version this computer ran is where its story starts.
    let floor = ran
        .iter()
        .map(|r| r.version)
        .chain([current])
        .filter(|v| !v.is_empty())
        .min_by(|a, b| compare_versions(a, b));
    // The newest one it has or is offered is where it ends.
    let ceiling = [current, staged, available]
        .into_iter()
        .filter(|v| !v.is_empty())
        .max_by(|a, b| compare_versions(a, b));
    let in_range = |v: &str| {
        floor.is_none_or(|f| compare_versions(v, f) != Ordering::Less)
            && ceiling.is_none_or(|c| compare_versions(v, c) != Ordering::Greater)
    };
    let mut versions: Vec<&str> = releases
        .iter()
        .map(|r| r.version.as_str())
        .filter(|v| in_range(v))
        .chain(ran.iter().map(|r| r.version))
        .chain([current, staged, available])
        .filter(|v| !v.is_empty())
        .collect();
    versions.sort_by(|a, b| compare_versions(b, a));
    versions.dedup();
    versions
        .into_iter()
        .map(|v| {
            let release = releases.iter().find(|r| r.version == v);
            // the earliest start, if it ran more than once (gone back to)
            let first_booted = ran
                .iter()
                .filter(|r| r.version == v)
                .map(|r| r.first_booted)
                .min()
                .unwrap_or("");
            let state = if v == current {
                "current"
            } else if v == staged {
                "staged"
            } else if v == available {
                "available"
            } else if !first_booted.is_empty() {
                "ran"
            } else {
                ""
            };
            Item {
                version: v.to_string(),
                date: release.map(|r| r.date.clone()).unwrap_or_default(),
                state: state.to_string(),
                first_booted: first_booted.to_string(),
                html: release.map(|r| r.html.clone()).unwrap_or_default(),
                plain: release.map(|r| r.plain.clone()).unwrap_or_default(),
            }
        })
        .collect()
}

/// Where the release list is kept between runs.
pub fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("atlas-updater/releases.json"))
}

/// A cached list: its text and how old it is.
pub struct Cached {
    pub text: String,
    pub age: Option<Duration>,
}

impl Cached {
    /// Whether to use it without asking GitHub. `wanted` are the versions
    /// this computer has or is offered.
    pub fn fresh(&self, wanted: &[&str]) -> bool {
        let Some(age) = self.age else { return false };
        if age >= FRESH_FOR {
            return false;
        }
        let have = |v: &&str| {
            v.is_empty()
                || crate::notes::parse_releases(&self.text)
                    .iter()
                    .any(|r| r.version == *v)
        };
        age < RECHECK_AFTER || wanted.iter().all(have)
    }
}

/// The list cached for `url` (the file's first line), if any.
pub fn read_cache(path: &Path, url: &str, now: SystemTime) -> Option<Cached> {
    let file = std::fs::read_to_string(path).ok()?;
    let (head, text) = file.split_once('\n')?;
    if head != url {
        return None;
    }
    let age = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok());
    Some(Cached {
        text: text.to_string(),
        age,
    })
}

/// Keeps the list GitHub gave for `url`, if it is one (a JSON array); a
/// failure only costs a fetch next time.
pub fn write_cache(path: &Path, url: &str, text: &str) -> bool {
    if !serde_json::from_str::<serde_json::Value>(text).is_ok_and(|v| v.is_array()) {
        return false;
    }
    let tmp = path.with_extension("tmp");
    path.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&tmp, format!("{url}\n{text}")))
        .and_then(|()| std::fs::rename(&tmp, path))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(v: &str) -> Release {
        Release {
            version: v.into(),
            date: format!("{v}-date"),
            html: format!("<p>{v}</p>"),
            plain: v.into(),
        }
    }

    #[test]
    fn versions_order_by_number() {
        assert_eq!(
            compare_versions("44.20261002", "44.20260924"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("44.9", "44.10"), Ordering::Less);
        assert_eq!(compare_versions("45.1", "44.99"), Ordering::Greater);
        assert_eq!(compare_versions("44.1", "44.1"), Ordering::Equal);
        assert_eq!(compare_versions("44.1", "44.1.1"), Ordering::Less);
        // builds numbered within the day, and the older plain day versions
        assert_eq!(
            compare_versions("44.20261008-10", "44.20261008-2"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("44.20261008", "44.20261008-1"),
            Ordering::Less
        );
        assert_eq!(
            compare_versions("44.20261009-1", "44.20261008-5"),
            Ordering::Greater
        );
    }

    #[test]
    fn list_runs_from_the_first_version_ran_to_the_newest_offered() {
        let releases = [
            "44.20261009",
            "44.20261002",
            "44.20260925",
            "44.20260918",
            "44.20260911",
        ]
        .map(rel);
        let ran = [
            Ran {
                version: "44.20260918",
                first_booted: "2026-09-19T08:00:00Z",
            },
            Ran {
                version: "44.20261002",
                first_booted: "2026-10-03T08:00:00Z",
            },
        ];
        let items = merge(&releases, &ran, "44.20261002", "", "44.20261009");
        let got: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.version.as_str(), i.state.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("44.20261009", "available"),
                ("44.20261002", "current"),
                // skipped over: its changes came with 44.20261002
                ("44.20260925", ""),
                ("44.20260918", "ran"),
            ]
        );
        assert_eq!(items[1].first_booted, "2026-10-03T08:00:00Z");
        assert_eq!(items[0].html, "<p>44.20261009</p>");
        assert_eq!(items[0].date, "44.20261009-date");
    }

    #[test]
    fn versions_without_a_release_still_show() {
        // a build with no GitHub release (local builds, or one whose release
        // was never written)
        let ran = [Ran {
            version: "44.20261030",
            first_booted: "2026-10-30T08:00:00Z",
        }];
        let items = merge(
            &[rel("44.20261002")],
            &ran,
            "44.20261031",
            "44.20261032",
            "",
        );
        let got: Vec<(&str, &str, bool)> = items
            .iter()
            .map(|i| (i.version.as_str(), i.state.as_str(), i.html.is_empty()))
            .collect();
        assert_eq!(
            got,
            [
                ("44.20261032", "staged", true),
                ("44.20261031", "current", true),
                ("44.20261030", "ran", true),
            ]
        );
    }

    #[test]
    fn a_version_gone_back_to_keeps_its_first_start() {
        let ran = [
            Ran {
                version: "44.2",
                first_booted: "2026-10-05T00:00:00Z",
            },
            Ran {
                version: "44.3",
                first_booted: "2026-10-04T00:00:00Z",
            },
            Ran {
                version: "44.2",
                first_booted: "2026-10-03T00:00:00Z",
            },
        ];
        let items = merge(&[], &ran, "44.2", "", "");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].first_booted, "2026-10-03T00:00:00Z");
        assert_eq!(items[0].state, "ran");
    }

    #[test]
    fn no_history_lists_only_what_this_computer_has() {
        let releases = ["44.3", "44.2", "44.1"].map(rel);
        let items = merge(&releases, &[], "44.2", "", "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].version, "44.2");
        assert!(merge(&releases, &[], "", "", "").len() == 3);
    }

    #[test]
    fn versions_order_totally() {
        // numbers before words: no cycle like 9 < 10 < 1a < 9
        let mut v = vec!["44.9", "44.1a", "44.10", "44.b", "44.2"];
        v.sort_by(|a, b| compare_versions(a, b));
        assert_eq!(v, ["44.2", "44.9", "44.10", "44.1a", "44.b"]);
    }

    #[test]
    fn cache_round_trip_and_freshness() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a/releases.json");
        let url = "https://example.test/releases";
        let list = r#"[{"tag_name":"44.2","body":"x"}]"#;
        assert!(read_cache(&p, url, SystemTime::now()).is_none());
        assert!(write_cache(&p, url, list));
        let now = SystemTime::now();
        let c = read_cache(&p, url, now).unwrap();
        assert_eq!(c.text, list);
        assert!(c.fresh(&["44.2", ""]));
        // another repository's list is not used
        assert!(read_cache(&p, "https://other.test", now).is_none());
        let later = now + FRESH_FOR + Duration::from_secs(1);
        assert!(!read_cache(&p, url, later).unwrap().fresh(&[]));
    }

    #[test]
    fn a_list_missing_the_offered_version_is_asked_again_after_a_while() {
        let c = |mins: u64| Cached {
            text: r#"[{"tag_name":"44.2","body":"x"}]"#.into(),
            age: Some(Duration::from_secs(mins * 60)),
        };
        assert!(c(5).fresh(&["44.3"]));
        assert!(!c(11).fresh(&["44.3"]));
        assert!(c(11).fresh(&["44.2"]));
    }

    #[test]
    fn only_a_list_is_cached() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("releases.json");
        assert!(write_cache(&p, "u", "[]"));
        // an error page or a 404 must not replace a good list
        assert!(!write_cache(&p, "u", r#"{"message":"Not Found"}"#));
        assert!(!write_cache(&p, "u", "<html>"));
        assert_eq!(read_cache(&p, "u", SystemTime::now()).unwrap().text, "[]");
    }
}
