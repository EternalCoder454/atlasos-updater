//! `atlas-updater --worker <job>`: an app round or an app update for
//! atlas-updater-tray, run without Qt. libflatpak is loaded for the length of
//! the job only, so the resident tray stays small. The result goes to stdout
//! as one line of JSON (atlas_updater_base::worker::Outcome); the window
//! picks up what changed from the settings file.

use std::ffi::{CStr, c_char};
use std::io::Write;

use atlas_updater_base::worker::{
    APPS_NOTIFIED, APPS_ROUND, APPS_UPDATE, Notice, Outcome, TIME_LIMIT_SECS,
};

use crate::{apphistory, apps, lock, rc, schedule};

/// `RoundError` in [`rc::APPS`]: what the window shows until something works.
pub const ROUND_ERROR: &str = "RoundError";
/// `RoundAt` in [`rc::APPS`]: when a worker last changed the apps; an open
/// window lists them again.
pub const ROUND_AT: &str = "RoundAt";

/// What to do with the settings after a round. Kept apart from the round so
/// the rules can be tested without Flatpak.
#[derive(Debug, Default, PartialEq, Eq)]
struct Verdict {
    outcome: Outcome,
    /// `Some(None)` removes the key.
    round_error: Option<Option<String>>,
    /// Only ever cleared here: the tray saves a notice's key once shown.
    clear_notified: bool,
}

/// `notified`: the key of the last app notice.
fn judge(res: &Result<apps::Round, String>, notified: Option<&str>) -> Verdict {
    let round = match res {
        Ok(r) => r,
        Err(e) => {
            return Verdict {
                outcome: Outcome {
                    failed: true,
                    ..Default::default()
                },
                round_error: Some(Some(format!("Could not check app updates: {e}"))),
                clear_notified: false,
            };
        }
    };
    let failed = round.done.error.is_some();
    let mut v = Verdict {
        outcome: Outcome {
            failed,
            waited: round.waited.is_some(),
            ..Default::default()
        },
        ..Default::default()
    };
    // Not listed: a metered or offline connection. Nothing to say.
    let Some(rows) = &round.rows else {
        return v;
    };
    if rows.is_empty() {
        // Nothing waits: the next set, even the same apps again, is news.
        v.clear_notified = true;
    }
    v.round_error = Some(if let Some(e) = &round.done.error {
        Some(format!("Could not update apps in the background: {e}"))
    } else {
        round
            .unchecked
            .as_ref()
            .map(|e| format!("Could not check which app updates ask for new permissions: {e}"))
    });
    // A low-battery wait is quiet: it tries again later.
    if rows.is_empty() || round.waited.is_some() {
        return v;
    }
    // Each notice once: the same apps, held or failed the same way, are not news.
    let key = apps::notice_key(
        rows,
        &round.done.held_back,
        failed || round.unchecked.is_some(),
    );
    if notified == Some(key.as_str()) {
        return v;
    }
    v.outcome.notice = Some(Notice {
        key: Some(key),
        text: apps::ready_text(rows, &round.done.held_back, failed, round.auto),
        // Nothing asking for new permissions, or not knowing, installs from
        // a notification: the user sees it on the Updates page first.
        can_update: round.done.held_back.is_empty() && round.unchecked.is_none(),
    });
    v
}

fn apply(v: &Verdict) {
    if let Some(e) = &v.round_error {
        rc::set(rc::APPS, ROUND_ERROR, e.as_deref());
    }
    if v.clear_notified {
        rc::set(rc::APPS, APPS_NOTIFIED, None);
    }
    rc::set(rc::APPS, ROUND_AT, Some(&schedule::unix_now().to_string()));
}

/// The scheduled round: look for app updates, and install them if the user
/// turned that on.
fn round() -> Outcome {
    let _held = match lock::take(lock::APPS, false) {
        Ok(Some(h)) => h,
        Ok(None) => {
            return Outcome {
                busy: true,
                ..Default::default()
            };
        }
        Err(e) => {
            eprintln!("atlas-updater: cannot take the app update lock: {e}");
            return Outcome {
                failed: true,
                ..Default::default()
            };
        }
    };
    // Asked again just before installing: turning the switch off mid-round counts.
    let res = apps::background(rc::apps_automatic, None);
    if let Ok(r) = &res {
        if let Err(e) = apphistory::record(&r.done.updated, None) {
            eprintln!("atlas-updater: could not save the app update history: {e}");
        }
        if let Some(why) = r.waited {
            eprintln!("atlas-updater: app updates wait: {why}");
        }
        if let Some(e) = &r.done.error {
            eprintln!("atlas-updater: background app update failed: {e}");
        }
        if let Some(e) = &r.unchecked {
            eprintln!("atlas-updater: could not check what app updates ask for: {e}");
        }
    }
    let v = judge(&res, rc::get(rc::APPS, APPS_NOTIFIED).as_deref());
    if let Some(Some(e)) = &v.round_error {
        eprintln!("atlas-updater: {e}");
    }
    apply(&v);
    v.outcome
}

/// "Update Apps" on a notification: what asks for new permissions waits, as
/// the check behind the notice may be hours old.
fn update() -> Outcome {
    // The user asked: wait for a round or the window's operation to end.
    let _held = match lock::take(lock::APPS, true) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("atlas-updater: cannot take the app update lock: {e}");
            None
        }
    };
    let done = apps::update(None, false, true, |_| {});
    if let Err(e) = apphistory::record(&done.updated, None) {
        eprintln!("atlas-updater: could not save the app update history: {e}");
    }
    let left = apps::list(false, true, None).unwrap_or_default();
    let failed = done.error.is_some();
    let mut out = Outcome {
        failed,
        ..Default::default()
    };
    let v = Verdict {
        round_error: Some(
            done.error
                .as_ref()
                .map(|e| format!("Could not update apps: {e}")),
        ),
        ..Default::default()
    };
    if let Some(e) = &done.error {
        eprintln!("atlas-updater: app update failed: {e}");
    }
    if !done.held_back.is_empty() {
        // Say which apps were left out, and not again for the same set.
        // Not "on its own": the user started it.
        out.notice = Some(Notice {
            key: Some(apps::notice_key(&left, &done.held_back, failed)),
            text: apps::ready_text(&left, &done.held_back, failed, false),
            can_update: false,
        });
    } else if let Some(e) = &done.error {
        // The user asked from a notification: say it did not work.
        out.notice = Some(Notice {
            key: None,
            text: format!(
                "{}. Open Atlas Updater to try again.",
                e.trim_end_matches('.')
            ),
            can_update: false,
        });
    }
    apply(&v);
    out
}

/// Called from `main.cpp` for `--worker <job>`, before Qt starts. Returns
/// the exit code.
///
/// # Safety
/// `job` must be null or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atlas_worker(job: *const c_char) -> i32 {
    let job = if job.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(job) }
            .to_string_lossy()
            .into_owned()
    };
    // A hung Flatpak or network call must not hold the app update lock
    // forever: SIGALRM ends the process (the lock goes with it).
    // SAFETY: alarm only sets a timer.
    unsafe {
        libc::alarm(TIME_LIMIT_SECS);
    }
    let run = match job.as_str() {
        APPS_ROUND => round,
        APPS_UPDATE => update,
        other => {
            eprintln!("atlas-updater: unknown worker job {other:?}");
            return 2;
        }
    };
    let out = std::panic::catch_unwind(run).unwrap_or_else(|_| Outcome {
        failed: true,
        ..Default::default()
    });
    // Not println!: it panics, and so aborts, if the tray has gone away.
    let Ok(line) = serde_json::to_string(&out) else {
        return 1;
    };
    match writeln!(std::io::stdout(), "{line}") {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> apps::Row {
        apps::Row {
            name: name.into(),
            id: format!("org.example.{name}"),
            branch: "stable".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_failed_round_says_so_and_backs_off() {
        let v = judge(&Err("no network".into()), None);
        assert!(v.outcome.failed);
        assert_eq!(
            v.round_error,
            Some(Some("Could not check app updates: no network".into()))
        );
        assert!(!v.clear_notified);
    }

    #[test]
    fn an_unlisted_round_changes_nothing() {
        let r = apps::Round {
            waited: Some("metered"),
            ..Default::default()
        };
        let v = judge(&Ok(r), Some("k"));
        assert!(v.outcome.waited && !v.outcome.failed);
        assert_eq!(v.round_error, None);
        assert!(!v.clear_notified);
    }

    #[test]
    fn nothing_waiting_clears_the_notice_and_the_error() {
        let r = apps::Round {
            rows: Some(Vec::new()),
            ..Default::default()
        };
        let v = judge(&Ok(r), Some("old"));
        assert!(v.clear_notified);
        assert_eq!(v.round_error, Some(None));
        assert_eq!(v.outcome.notice, None);
    }

    #[test]
    fn each_set_is_announced_once() {
        let r = || apps::Round {
            rows: Some(vec![row("Firefox")]),
            ..Default::default()
        };
        let v = judge(&Ok(r()), None);
        assert!(!v.clear_notified);
        let n = v.outcome.notice.expect("a notice");
        let key = n.key.clone().expect("a key");
        assert!(n.can_update);
        assert!(n.text.contains("Firefox"), "{}", n.text);
        let again = judge(&Ok(r()), Some(&key));
        assert_eq!(again.outcome.notice, None);
        assert!(!again.clear_notified);
    }

    #[test]
    fn unknown_permissions_mean_no_update_button() {
        let r = apps::Round {
            rows: Some(vec![row("Firefox")]),
            unchecked: Some("timed out".into()),
            ..Default::default()
        };
        let v = judge(&Ok(r), None);
        assert!(!v.outcome.notice.expect("a notice").can_update);
        assert_eq!(
            v.round_error,
            Some(Some(
                "Could not check which app updates ask for new permissions: timed out".into()
            ))
        );
    }

    #[test]
    fn a_battery_wait_is_quiet() {
        let r = apps::Round {
            rows: Some(vec![row("Firefox")]),
            waited: Some("battery"),
            ..Default::default()
        };
        let v = judge(&Ok(r), None);
        assert!(v.outcome.waited);
        assert_eq!(v.outcome.notice, None);
        assert!(!v.clear_notified);
    }
}
