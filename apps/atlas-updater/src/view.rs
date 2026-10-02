//! Turns a bootc `Status` into the plain values the QML screens show.

use atlas_core::bootc::{BootEntry, Status};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slot {
    pub present: bool,
    pub version: String,
    /// RFC 3339 build time of the image; QML formats it for the locale.
    pub date: String,
    pub digest: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub current: Slot,
    pub staged: Slot,
    pub rollback: Slot,
    /// `booted.cachedUpdate` when it is not staged yet.
    pub available: Slot,
    /// `stable`, `testing`, or empty for a custom ref.
    pub channel: String,
    /// A restart will switch to something new (staged, or a queued rollback).
    pub restart_needed: bool,
}

fn short_digest(d: &str) -> String {
    let d = d.strip_prefix("sha256:").unwrap_or(d);
    d.chars().take(12).collect()
}

fn slot(e: Option<&BootEntry>) -> Slot {
    let Some(e) = e else {
        return Slot::default();
    };
    let digest = e.digest().unwrap_or_default().to_string();
    Slot {
        present: true,
        version: e
            .version()
            .map(str::to_string)
            .unwrap_or_else(|| short_digest(&digest)),
        date: e.timestamp().unwrap_or_default().to_string(),
        digest,
    }
}

pub fn from_status(st: &Status) -> View {
    let booted = st.status.booted.as_ref();
    let available = if st.update_available() {
        let c = booted.and_then(|b| b.cached_update.as_ref());
        match c {
            Some(c) => Slot {
                present: true,
                version: c
                    .version
                    .clone()
                    .unwrap_or_else(|| short_digest(&c.image_digest)),
                date: c.timestamp.clone().unwrap_or_default(),
                digest: c.image_digest.clone(),
            },
            None => Slot::default(),
        }
    } else {
        Slot::default()
    };
    View {
        current: slot(booted),
        staged: slot(st.status.staged.as_ref()),
        rollback: slot(st.status.rollback.as_ref()),
        available,
        channel: st.channel().map(|c| c.to_string()).unwrap_or_default(),
        restart_needed: st.has_staged() || st.status.rollback_queued,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(json: &str) -> Status {
        Status::from_json(json).unwrap()
    }

    const BASE: &str = r#"{"apiVersion":"org.containers.bootc/v1","kind":"BootcHost",
      "spec":{"image":{"image":"ghcr.io/eternalcoder454/atlasos:stable","transport":"registry"}},
      "status":{
        "booted":{"image":{"image":{"image":"ghcr.io/eternalcoder454/atlasos:stable","transport":"registry"},
                  "version":"44.20261002","timestamp":"2026-10-02T04:00:00Z","imageDigest":"sha256:aaa"}
                  __CACHED__},
        "staged": __STAGED__,
        "rollback": null, "rollbackQueued": false, "type":"bootcHost"}}"#;

    fn build(cached: &str, staged: &str) -> Status {
        st(&BASE
            .replace("__CACHED__", cached)
            .replace("__STAGED__", staged))
    }

    const NEW_IMG: &str = r#"{"image":{"image":{"image":"ghcr.io/eternalcoder454/atlasos:stable","transport":"registry"},
                  "version":"44.20261009","timestamp":"2026-10-09T04:00:00Z","imageDigest":"sha256:bbb"}}"#;

    #[test]
    fn plain() {
        let v = from_status(&build("", "null"));
        assert_eq!(v.current.version, "44.20261002");
        assert_eq!(v.channel, "stable");
        assert!(!v.restart_needed && !v.available.present && !v.staged.present);
    }

    #[test]
    fn update_found_not_staged() {
        let cached = r#","cachedUpdate":{"image":{"image":"ghcr.io/eternalcoder454/atlasos:stable","transport":"registry"},
                       "version":"44.20261009","timestamp":"2026-10-09T04:00:00Z","imageDigest":"sha256:bbb"}"#;
        let v = from_status(&build(cached, "null"));
        assert_eq!(v.available.version, "44.20261009");
        assert!(!v.restart_needed);
    }

    #[test]
    fn staged_means_restart() {
        let v = from_status(&build("", NEW_IMG));
        assert_eq!(v.staged.version, "44.20261009");
        assert!(v.restart_needed);
    }
}
