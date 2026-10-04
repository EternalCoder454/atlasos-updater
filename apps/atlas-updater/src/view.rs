//! Turns a bootc `Status` into the plain values the QML screens show.

use atlas_framework_system::bootc::{BootEntry, Status};

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
    /// [`Status::available_update`]: the newest image a check found, when it
    /// is neither booted nor staged.
    pub available: Slot,
    /// `stable`, `testing`, or empty for a custom ref.
    pub channel: String,
    /// A restart will switch to something new (staged, or a queued rollback).
    pub restart_needed: bool,
    /// A rollback is queued for the next restart.
    pub rollback_queued: bool,
    /// The version a queued rollback goes back to (empty when none is queued).
    pub rollback_target: String,
    /// The available image is the one the rollback deployment holds: the
    /// version the user went back from.
    pub available_is_rollback: bool,
    /// The available image failed its boot health checks on this machine and
    /// was rolled back.
    pub available_is_bad: bool,
    /// The rollback image failed its boot health checks on this machine.
    pub rollback_is_bad: bool,
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
    let available = match st.available_update() {
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
    };
    let rollback = slot(st.status.rollback.as_ref());
    let queued = st.status.rollback_queued;
    View {
        available_is_rollback: available.present
            && rollback.present
            && !available.digest.is_empty()
            && available.digest == rollback.digest,
        available_is_bad: available.present && st.is_bad_image(&available.digest),
        rollback_is_bad: rollback.present && st.is_bad_image(&rollback.digest),
        rollback_queued: queued,
        rollback_target: if queued {
            rollback.version.clone()
        } else {
            String::new()
        },
        current: slot(booted),
        staged: slot(st.status.staged.as_ref()),
        rollback,
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
    fn a_queued_rollback_and_the_image_we_went_back_from() {
        let json = r#"{"apiVersion":"org.containers.bootc/v1","kind":"BootcHost",
          "spec":{"image":{"image":"ghcr.io/e/atlasos:stable","transport":"registry"}},
          "status":{
            "booted":{"image":{"image":{"image":"ghcr.io/e/atlasos:stable","transport":"registry"},
                      "version":"44.1","imageDigest":"sha256:aaa"},
                      "cachedUpdate":{"image":{"image":"ghcr.io/e/atlasos:stable","transport":"registry"},
                      "version":"44.2","imageDigest":"sha256:bbb"}},
            "staged": null,
            "rollback":{"image":{"image":{"image":"ghcr.io/e/atlasos:stable","transport":"registry"},
                      "version":"44.2","imageDigest":"sha256:bbb"}},
            "rollbackQueued": QUEUED, "type":"bootcHost"}}"#;
        let v = from_status(&st(&json.replace("QUEUED", "false")));
        assert!(v.available_is_rollback && !v.rollback_queued);
        assert_eq!(v.rollback_target, "");
        let v = from_status(&st(&json.replace("QUEUED", "true")));
        assert!(v.rollback_queued && v.restart_needed);
        assert_eq!(v.rollback_target, "44.2");
        // a different newest image is just an update
        let other = json.replace("QUEUED", "false").replacen(
            "\"imageDigest\":\"sha256:bbb\"",
            "\"imageDigest\":\"sha256:ccc\"",
            1,
        );
        let v = from_status(&st(&other));
        assert!(!v.available_is_rollback);
    }

    #[test]
    fn an_update_that_failed_its_health_checks() {
        let cached = r#","cachedUpdate":{"image":{"image":"ghcr.io/eternalcoder454/atlasos:stable","transport":"registry"},
                       "version":"44.20261009","timestamp":"2026-10-09T04:00:00Z","imageDigest":"sha256:bbb"}"#;
        let mut s = build(cached, "null");
        assert!(!from_status(&s).available_is_bad);
        s.bad_image_digests = vec!["sha256:zzz".into(), "sha256:bbb".into()];
        let v = from_status(&s);
        assert!(v.available.present && v.available_is_bad && !v.rollback_is_bad);
        // the booted image being listed doesn't make the update bad
        s.bad_image_digests = vec!["sha256:aaa".into()];
        assert!(!from_status(&s).available_is_bad);
    }

    #[test]
    fn a_bad_image_left_behind_by_the_automatic_rollback() {
        // greenboot booted 44.1 again; 44.2 is the rollback entry and still
        // the newest image on the registry.
        let json = r#"{"apiVersion":"org.containers.bootc/v1","kind":"BootcHost",
          "spec":{"image":{"image":"ghcr.io/e/atlasos:stable","transport":"registry"}},
          "status":{
            "booted":{"image":{"image":{"image":"ghcr.io/e/atlasos:stable","transport":"registry"},
                      "version":"44.1","imageDigest":"sha256:aaa"},
                      "cachedUpdate":{"image":{"image":"ghcr.io/e/atlasos:stable","transport":"registry"},
                      "version":"44.2","imageDigest":"sha256:bbb"}},
            "staged": null,
            "rollback":{"image":{"image":{"image":"ghcr.io/e/atlasos:stable","transport":"registry"},
                      "version":"44.2","imageDigest":"sha256:bbb"}},
            "rollbackQueued": false, "type":"bootcHost"}}"#;
        let mut s = st(json);
        s.bad_image_digests = vec!["sha256:bbb".into()];
        let v = from_status(&s);
        assert!(v.available_is_bad && v.available_is_rollback && v.rollback_is_bad);
    }

    #[test]
    fn staged_means_restart() {
        let v = from_status(&build("", NEW_IMG));
        assert_eq!(v.staged.version, "44.20261009");
        assert!(v.restart_needed);
    }
}
