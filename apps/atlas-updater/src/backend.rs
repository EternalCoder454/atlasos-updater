//! The one QObject behind every screen. State lives here; slow work (D-Bus,
//! network, flatpak, files) runs on worker threads and posts back through
//! `qt_thread()`, so the GUI thread never blocks.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qproperty(bool, busy, cxx_name = "busy")]
        #[qproperty(QString, busy_text, cxx_name = "busyText")]
        /// The system operation that set `busy` ("check", "download",
        /// "rollback", "cancelRollback", "switch" or "sendReport"),
        /// or "" when none runs. Only that operation clears it.
        #[qproperty(QString, busy_op, cxx_name = "busyOp")]
        /// The app operation behind `appsBusy` ("checkApps" or "updateApps"),
        /// or "". Separate from `busyOp`: the two can overlap.
        #[qproperty(QString, apps_op, cxx_name = "appsOp")]
        /// A restart of the computer has been asked for and has not ended.
        #[qproperty(bool, restarting, cxx_name = "restarting")]
        /// The operation whose failure set `errorText` (the `busyOp` names,
        /// plus "status", "restart", "crashSetting", "discardReport" and "timer", which
        /// only ever appear here); "" when `errorText` is empty.
        #[qproperty(QString, error_op, cxx_name = "errorOp")]
        #[qproperty(bool, loaded, cxx_name = "loaded")]
        #[qproperty(QString, error_text, cxx_name = "errorText")]
        #[qproperty(QString, info_text, cxx_name = "infoText")]
        #[qproperty(QString, current_version, cxx_name = "currentVersion")]
        #[qproperty(QString, current_date, cxx_name = "currentDate")]
        #[qproperty(bool, has_staged, cxx_name = "hasStaged")]
        #[qproperty(QString, staged_version, cxx_name = "stagedVersion")]
        #[qproperty(QString, staged_date, cxx_name = "stagedDate")]
        #[qproperty(bool, has_rollback, cxx_name = "hasRollback")]
        #[qproperty(QString, rollback_version, cxx_name = "rollbackVersion")]
        #[qproperty(QString, rollback_date, cxx_name = "rollbackDate")]
        #[qproperty(bool, update_available, cxx_name = "updateAvailable")]
        #[qproperty(QString, available_version, cxx_name = "availableVersion")]
        #[qproperty(QString, available_date, cxx_name = "availableDate")]
        #[qproperty(QString, channel, cxx_name = "channel")]
        #[qproperty(bool, restart_needed, cxx_name = "restartNeeded")]
        /// A rollback is queued for the next restart.
        #[qproperty(bool, rollback_queued, cxx_name = "rollbackQueued")]
        /// The version the queued rollback goes back to (empty if none).
        #[qproperty(QString, rollback_target, cxx_name = "rollbackTarget")]
        /// The available image is the one the user went back from.
        #[qproperty(bool, available_is_rollback, cxx_name = "availableIsRollback")]
        /// The available image failed its boot health checks on this machine.
        #[qproperty(bool, available_is_bad, cxx_name = "availableIsBad")]
        /// The rollback image failed its boot health checks on this machine.
        #[qproperty(bool, rollback_is_bad, cxx_name = "rollbackIsBad")]
        #[qproperty(QString, notes_state, cxx_name = "notesState")]
        /// The release notes as an HTML fragment (empty when there are none).
        #[qproperty(QString, notes_html, cxx_name = "notesHtml")]
        /// The release notes as plain text (for the accessible name).
        #[qproperty(QString, notes_plain, cxx_name = "notesPlain")]
        #[qproperty(QString, notes_version, cxx_name = "notesVersion")]
        /// Plain-language reason when `notesState` is "error".
        #[qproperty(QString, notes_error, cxx_name = "notesError")]
        #[qproperty(QString, apps_json, cxx_name = "appsJson")]
        #[qproperty(i32, apps_count, cxx_name = "appsCount")]
        #[qproperty(bool, apps_busy, cxx_name = "appsBusy")]
        #[qproperty(QString, apps_status, cxx_name = "appsStatus")]
        #[qproperty(QString, apps_error, cxx_name = "appsError")]
        /// "Download app updates in the background": off by default, saved
        /// per user.
        #[qproperty(bool, apps_auto, cxx_name = "appsAuto")]
        /// App updates installed here (apphistory::Entry as JSON), newest first.
        #[qproperty(QString, app_history_json, cxx_name = "appHistoryJson")]
        #[qproperty(QString, history_json, cxx_name = "historyJson")]
        /// The Changelog page's versions (changelog::Item as JSON), newest first.
        #[qproperty(QString, changelog_json, cxx_name = "changelogJson")]
        /// "", "loading", "ready" or "error".
        #[qproperty(QString, changelog_state, cxx_name = "changelogState")]
        /// Plain-language reason for "error", or a note that the list shown
        /// is a saved one (offline) with "ready".
        #[qproperty(QString, changelog_note, cxx_name = "changelogNote")]
        /// While an update downloads: "downloading" (bytes) or "installing"
        /// (steps), else "". From the helper's Progress property.
        #[qproperty(QString, progress_stage, cxx_name = "progressStage")]
        /// Bytes or steps done and in all (`progressTotal` 0: unknown).
        /// Doubles: QML numbers, and byte counts pass i32.
        #[qproperty(f64, progress_done, cxx_name = "progressDone")]
        #[qproperty(f64, progress_total, cxx_name = "progressTotal")]
        /// The step running ("Deploying Image"), may be empty.
        #[qproperty(QString, progress_detail, cxx_name = "progressDetail")]
        /// When this app last checked for updates (Unix seconds; 0: never).
        #[qproperty(i64, last_checked, cxx_name = "lastChecked")]
        #[qproperty(i64, scheduled_at, cxx_name = "scheduledAt")]
        #[qproperty(bool, crash_enabled, cxx_name = "crashEnabled")]
        #[qproperty(bool, crash_has_server, cxx_name = "crashHasServer")]
        #[qproperty(QString, reports_json, cxx_name = "reportsJson")]
        #[qproperty(i32, reports_count, cxx_name = "reportsCount")]
        #[qproperty(QString, sent_json, cxx_name = "sentJson")]
        /// The icon name from `LOGO=` in os-release, or empty.
        #[qproperty(QString, os_logo, cxx_name = "osLogo")]
        /// `ATLAS_UPDATER_FIXTURES` is set: everything shown is fake. The UI
        /// shows a permanent banner.
        #[qproperty(bool, fixtures_active, cxx_name = "fixturesActive")]
        #[namespace = "atlas_updater"]
        type Backend = super::BackendRust;

        /// A staged update we have not told the user about yet.
        #[qsignal]
        #[cxx_name = "updateStaged"]
        fn update_staged(self: Pin<&mut Backend>, version: QString);

        /// App updates wait for the user (background updates off, or some
        /// held back): the tray shows `text` as a notification. Once per set.
        #[qsignal]
        #[cxx_name = "appUpdatesReady"]
        /// `can_update`: false when an app asks for new permissions, which
        /// the user should see before agreeing (no "Update Apps" button then).
        fn app_updates_ready(self: Pin<&mut Backend>, text: QString, can_update: bool);

        /// A new report is waiting for review (tray notification).
        #[qsignal]
        #[cxx_name = "reportFound"]
        fn report_found(self: Pin<&mut Backend>, app_name: QString, report_type: QString);

        /// The scheduled restart is 5 minutes away (`scheduledAt` has the time).
        #[qsignal]
        #[cxx_name = "restartSoon"]
        fn restart_soon(self: Pin<&mut Backend>);

        /// A scheduled restart did not happen (missed while asleep, or Plasma
        /// refused it); the tray shows this as a notification.
        #[qsignal]
        #[cxx_name = "restartProblem"]
        fn restart_problem(self: Pin<&mut Backend>, text: QString);

        /// Starts the scheduler thread and the first status read.
        #[qinvokable]
        fn start(self: Pin<&mut Backend>);
        /// Stops the scheduler thread (before the app quits).
        #[qinvokable]
        fn shutdown(self: Pin<&mut Backend>);

        /// Silent status read (inotify hit, 6 h poll, window opened).
        #[qinvokable]
        #[cxx_name = "refreshStatus"]
        fn refresh_status(self: Pin<&mut Backend>);
        #[qinvokable]
        #[cxx_name = "checkForUpdate"]
        fn check_for_update(self: Pin<&mut Backend>);
        #[qinvokable]
        #[cxx_name = "downloadUpdate"]
        fn download_update(self: Pin<&mut Backend>);
        #[qinvokable]
        fn rollback(self: Pin<&mut Backend>);
        /// Cancel a queued rollback (nothing is changed if none is queued).
        #[qinvokable]
        #[cxx_name = "cancelRollback"]
        fn cancel_rollback(self: Pin<&mut Backend>);
        #[qinvokable]
        #[cxx_name = "switchChannel"]
        fn switch_channel(self: Pin<&mut Backend>, channel: &QString);
        #[qinvokable]
        #[cxx_name = "dismissMessages"]
        fn dismiss_messages(self: Pin<&mut Backend>);
        #[qinvokable]
        #[cxx_name = "dismissInfo"]
        fn dismiss_info(self: Pin<&mut Backend>);

        #[qinvokable]
        #[cxx_name = "loadNotes"]
        fn load_notes(self: Pin<&mut Backend>);
        /// The window exists: release notes may be fetched.
        #[qinvokable]
        #[cxx_name = "windowOpened"]
        fn window_opened(self: Pin<&mut Backend>);
        /// The window is gone: forget the notes, fetch nothing more.
        #[qinvokable]
        #[cxx_name = "windowClosed"]
        fn window_closed(self: Pin<&mut Backend>);
        /// True for `https://` links: the only ones release notes may open.
        #[qinvokable]
        #[cxx_name = "isSafeLink"]
        fn is_safe_link(self: &Backend, link: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "loadHistory"]
        fn load_history(self: Pin<&mut Backend>);
        /// The Changelog page opened (or asked again): read the release list
        /// (cached for an hour) and the history.
        #[qinvokable]
        #[cxx_name = "loadChangelog"]
        fn load_changelog(self: Pin<&mut Backend>);

        #[qinvokable]
        #[cxx_name = "checkApps"]
        fn check_apps(self: Pin<&mut Backend>);
        #[qinvokable]
        #[cxx_name = "updateApps"]
        fn update_apps(self: Pin<&mut Backend>);
        /// "Update Apps" on a notification: what asks for new permissions
        /// waits, as the check behind the notice may be hours old.
        #[qinvokable]
        #[cxx_name = "updateAppsChecked"]
        fn update_apps_checked(self: Pin<&mut Backend>);
        /// The "Download app updates in the background" switch.
        #[qinvokable]
        #[cxx_name = "enableBackgroundApps"]
        fn enable_background_apps(self: Pin<&mut Backend>, on: bool);

        #[qinvokable]
        #[cxx_name = "restartNow"]
        fn restart_now(self: Pin<&mut Backend>);
        /// `at` is Unix seconds.
        #[qinvokable]
        #[cxx_name = "scheduleRestart"]
        fn schedule_restart(self: Pin<&mut Backend>, at: i64);
        #[qinvokable]
        #[cxx_name = "cancelRestart"]
        fn cancel_restart(self: Pin<&mut Backend>);

        /// The "Send crash reports" switch. Off by default; saved per user.
        #[qinvokable]
        #[cxx_name = "enableCrashReports"]
        fn enable_crash_reports(self: Pin<&mut Backend>, on: bool);
        /// Window opened or review screen shown: read the pending reports.
        #[qinvokable]
        #[cxx_name = "loadReports"]
        fn load_reports(self: Pin<&mut Backend>);
        #[qinvokable]
        #[cxx_name = "loadSentReports"]
        fn load_sent_reports(self: Pin<&mut Backend>);
        /// Tray: new systemd-coredump entries and helper events (only when on).
        #[qinvokable]
        #[cxx_name = "collectReports"]
        fn collect_reports(self: Pin<&mut Backend>);
        /// "Send": the user saw the exact data. `event_id` is the report's
        /// `eventId` in `reportsJson`.
        #[qinvokable]
        #[cxx_name = "sendReport"]
        fn send_report(self: Pin<&mut Backend>, event_id: &QString);
        /// "Don't send". Ignored while a send is running.
        #[qinvokable]
        #[cxx_name = "discardReport"]
        fn discard_report(self: Pin<&mut Backend>, event_id: &QString);
    }

    // Lets worker threads post closures back to the Qt thread.
    impl cxx_qt::Threading for Backend {}

    // Lets Rust create the object (see `atlas_backend_new` in lib.rs).
    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn backend_make_unique() -> UniquePtr<Backend>;
    }
}

use core::pin::Pin;
use std::path::PathBuf;

use atlas_core::bootc::{Channel, Status};
use atlas_core::crash::Report;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::json;

use crate::config::{self, Config};
use crate::errors::{self, OpError};
use crate::notes::{self, Notes};
use crate::ops::{self, Op};
use crate::schedule::{self, Event, Schedule};
use crate::view::{self, View};
use crate::{apphistory, apps, changelog, crash, rc, restart};

const RC_RESTART: &str = "Restart";
const RC_NOTIFIED: &str = "Notified";
const RC_CHECKED: &str = "Checked";
const RC_APPS: &str = "AppUpdates";
/// The first app round after the switch is turned on.
const APPS_SOON: std::time::Duration = std::time::Duration::from_secs(60);
/// The next app round after one that had to wait or failed.
const APPS_RETRY: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// When to try again after `failures` failed background rounds in a row:
/// 30 minutes, doubling each time, at most 6 hours.
fn apps_retry_after(failures: u32) -> std::time::Duration {
    let doubled = APPS_RETRY.saturating_mul(1 << failures.clamp(1, 5).saturating_sub(1));
    doubled.min(std::time::Duration::from_secs(6 * 3600))
}

#[derive(Default)]
pub struct BackendRust {
    busy: bool,
    busy_text: QString,
    loaded: bool,
    error_text: QString,
    info_text: QString,
    current_version: QString,
    current_date: QString,
    has_staged: bool,
    staged_version: QString,
    staged_date: QString,
    has_rollback: bool,
    rollback_version: QString,
    rollback_date: QString,
    update_available: bool,
    available_version: QString,
    available_date: QString,
    channel: QString,
    restart_needed: bool,
    rollback_queued: bool,
    rollback_target: QString,
    available_is_rollback: bool,
    available_is_bad: bool,
    rollback_is_bad: bool,
    notes_state: QString,
    notes_html: QString,
    notes_plain: QString,
    busy_op: QString,
    apps_op: QString,
    restarting: bool,
    error_op: QString,
    notes_version: QString,
    notes_error: QString,
    apps_json: QString,
    apps_count: i32,
    apps_busy: bool,
    apps_status: QString,
    apps_error: QString,
    apps_auto: bool,
    app_history_json: QString,
    history_json: QString,
    changelog_json: QString,
    changelog_state: QString,
    changelog_note: QString,
    progress_stage: QString,
    progress_done: f64,
    progress_total: f64,
    progress_detail: QString,
    last_checked: i64,
    scheduled_at: i64,
    crash_enabled: bool,
    crash_has_server: bool,
    reports_json: QString,
    reports_count: i32,
    sent_json: QString,
    os_logo: QString,
    fixtures_active: bool,
    fixture_notified: String,

    // Not exposed to QML.
    config: Config,
    fixtures: Option<PathBuf>,
    schedule: Schedule,
    started: bool,
    /// Bumped whenever the pending list changes by another route, so a
    /// slower `load_reports` result is not shown over it.
    reports_gen: u64,
    status_inflight: bool,
    view: View,
    pending: Vec<Report>,
    window_open: bool,
    /// Failed notes lookups: version and when, so a failure is not retried
    /// on every status refresh.
    notes_failed: Option<(String, std::time::Instant)>,
    /// A Changelog load is running.
    changelog_inflight: bool,
    /// Another was asked for meanwhile (a version changed): run it after.
    changelog_dirty: bool,
    /// When fetching the release list last failed.
    changelog_failed: Option<std::time::Instant>,
    /// `apps_auto` for worker threads, which read it just before installing.
    apps_auto_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// "Update Apps" arrived while another app operation ran: run it after.
    apps_update_requested: bool,
    /// The scheduled app round came while another app operation ran.
    apps_round_pending: bool,
    /// "Check for updates" was pressed during another app operation.
    apps_check_requested: bool,
    /// Background rounds that failed in a row, for the back-off.
    apps_failures: u32,
    /// The error shown came from a background round: the next round that
    /// works takes it away.
    apps_round_error: bool,
    /// Apps a background round held back (`apps::row_key`) → what they ask
    /// for, shown on their rows until they are updated.
    apps_held: std::collections::HashMap<String, apps::HeldApp>,
}

/// This machine's history, newest first (developer fixtures: theirs).
fn read_history(fixtures: Option<&std::path::Path>) -> Vec<atlas_core::history::Entry> {
    match fixtures {
        Some(dir) => config::read_fixture(dir, "history.jsonl")
            .map(|t| {
                let mut v: Vec<atlas_core::history::Entry> = t
                    .lines()
                    .filter_map(|l| serde_json::from_str(l).ok())
                    .collect();
                v.reverse();
                v
            })
            .unwrap_or_default(),
        None => atlas_core::history::read_default().unwrap_or_default(),
    }
}

/// The release list for the Changelog page.
struct ReleaseList {
    releases: Vec<notes::Release>,
    /// Shown above the list (a saved list used offline), or the error.
    note: String,
    /// GitHub was asked and failed: wait before asking again.
    fetch_failed: bool,
}

/// The cache while fresh, else GitHub (then cached), else a stale cache with
/// a note saying so. `may_fetch` false (a fetch failed a moment ago) uses
/// whatever is cached. `wanted`: the versions this computer has or is offered.
fn release_list(
    fixtures: Option<&std::path::Path>,
    template: &str,
    wanted: &[&str],
    may_fetch: bool,
) -> Result<ReleaseList, ReleaseList> {
    let list = |text: &str, note: &str, fetch_failed| ReleaseList {
        releases: notes::parse_releases(text),
        note: note.to_string(),
        fetch_failed,
    };
    if let Some(dir) = fixtures {
        let text = config::read_fixture(dir, "releases.json").unwrap_or_default();
        return Ok(list(&text, "", false));
    }
    const SAVED: &str = "Showing the release notes saved earlier: the newest could not be loaded.";
    let url = notes::releases_url(template);
    let cache = changelog::cache_path();
    let cached = cache
        .as_deref()
        .and_then(|p| changelog::read_cache(p, &url, std::time::SystemTime::now()));
    if let Some(c) = &cached
        && (c.fresh(wanted) || !may_fetch)
    {
        let note = if c.fresh(wanted) { "" } else { SAVED };
        return Ok(list(&c.text, note, false));
    }
    if !may_fetch {
        return Err(list(
            "",
            "The release notes could not be loaded. Try again in a few minutes.",
            false,
        ));
    }
    let fetched = notes::fetch_releases(template);
    if let Ok(Some(text)) = &fetched
        && cache
            .as_deref()
            .is_some_and(|p| changelog::write_cache(p, &url, text))
    {
        return Ok(list(text, "", false));
    }
    match (fetched, cached) {
        // a list we could not save: still good to show
        (Ok(Some(text)), _) if serde_json::from_str::<Vec<serde_json::Value>>(&text).is_ok() => {
            Ok(list(&text, "", false))
        }
        (_, Some(c)) => Ok(list(&c.text, SAVED, true)),
        (Err(e), None) => Err(list("", &e.text(), true)),
        (Ok(_), None) => Err(list("", "The release notes could not be loaded.", true)),
    }
}

/// How long the Changelog leaves GitHub alone after a failed fetch (its
/// limit for unauthenticated requests is 60 an hour).
const CHANGELOG_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// How long a failed (or missing) notes lookup is left alone.
const NOTES_BACKOFF: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Runs `f` on a new named thread. `false` if the OS refused a thread.
fn spawn_named(name: &str, f: impl FnOnce() + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .is_ok()
}

/// Runs `f`; a panic inside becomes `None` so the caller can still report
/// back (a dead worker must never leave a spinner on).
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

const INTERNAL: &str = "Atlas Updater hit an internal error. Please try again.";

fn q(s: &str) -> QString {
    QString::from(s)
}

/// Stores (or clears) the scheduled restart time. Fixture mode never touches
/// the user's real settings file.
fn save_schedule(fixtures: bool, at: Option<i64>) {
    if !fixtures {
        rc::set(
            RC_RESTART,
            "ScheduledAt",
            at.map(|t| t.to_string()).as_deref(),
        );
    }
}

fn timer_failed(mut obj: Pin<&mut qobject::Backend>) {
    obj.as_mut().set_error("timer", q(
        "Atlas Updater could not start its background timer. Update checks and scheduled restarts will not run until it is reopened.",
    ));
}

impl qobject::Backend {
    /// Show `text` as the error of the operation `op` (a QML-side name).
    fn set_error(mut self: Pin<&mut Self>, op: &str, text: QString) {
        // never leave `errorOp` naming an error whose text is not there yet
        // (or no longer is): text first when setting, op first when clearing
        if text.is_empty() {
            self.as_mut().set_error_op(QString::default());
            self.as_mut().set_error_text(text);
        } else {
            self.as_mut().set_error_text(text);
            self.as_mut().set_error_op(q(op));
        }
    }

    /// `busyOp` follows `busy`: set when a system operation starts, cleared
    /// by that same operation when it ends.
    fn begin_op(mut self: Pin<&mut Self>, name: &str) {
        self.as_mut().set_busy_op(q(name));
    }

    fn end_op(mut self: Pin<&mut Self>, name: &str) {
        if self.busy_op().to_string() == name {
            self.as_mut().set_busy_op(QString::default());
        }
    }

    pub fn start(mut self: Pin<&mut Self>) {
        if self.rust().started {
            return;
        }
        let cfg = Config::load();
        let fixtures = config::fixtures_dir();
        let schedule = self.rust().schedule.clone();
        if let Some(logo) = atlas_core::osrelease::logo_icon() {
            self.as_mut().set_os_logo(QString::from(logo.as_str()));
        }
        {
            let mut r = self.as_mut().rust_mut();
            r.started = true;
            r.config = cfg;
            r.fixtures = fixtures;
        }
        // Fixture mode takes the schedule from `schedule.json` and leaves the
        // user's own settings file alone.
        let saved = match &self.rust().fixtures {
            Some(d) => config::fixture_schedule(d),
            None => rc::get(RC_RESTART, "ScheduledAt").and_then(|v| v.parse::<i64>().ok()),
        };
        // Fixture mode: `last-checked` (Unix seconds), else never.
        let checked = match &self.rust().fixtures {
            Some(d) => config::read_fixture(d, "last-checked"),
            None => rc::get(RC_CHECKED, "At"),
        };
        if let Some(t) = checked.and_then(|v| v.trim().parse::<i64>().ok()) {
            self.as_mut().set_last_checked(t);
        }
        // Background app updates are off unless the user turned them on.
        let auto = match &self.rust().fixtures {
            Some(d) => config::read_fixture(d, "apps-auto").is_some(),
            None => rc::get(RC_APPS, "Automatic").as_deref() == Some("true"),
        };
        self.as_mut().set_apps_auto(auto);
        self.rust()
            .apps_auto_flag
            .store(auto, std::sync::atomic::Ordering::Relaxed);
        if let Some(at) = saved {
            // A time that passed while we were not running is dropped: never
            // restart the machine unexpectedly at login.
            if at > schedule::unix_now() {
                self.as_mut().set_scheduled_at(at);
                schedule.restore_restart(at);
            } else {
                save_schedule(self.rust().fixtures.is_some(), None);
            }
        }
        let qt = self.qt_thread();
        let sched = schedule.clone();
        let qt_run = qt.clone();
        let started = std::thread::Builder::new()
            .name("atlas-schedule".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let ended = guarded(|| {
                    sched.run(move |ev| {
                        let _ = qt_run.queue(move |obj| obj.on_event(ev));
                    })
                });
                if ended.is_none() {
                    let _ = qt.queue(timer_failed);
                }
            });
        if started.is_err() {
            let _ = self.qt_thread().queue(timer_failed);
        }
        // Crash reports are opt-in: read the setting, and only when on look for new ones.
        let on = match &self.rust().fixtures {
            Some(d) => config::read_fixture(d, "crash-enabled").is_some(),
            None => atlas_core::crash::Settings::load().enabled,
        };
        self.as_mut().set_crash_enabled(on);
        let fix = self.rust().fixtures.is_some();
        self.as_mut().set_fixtures_active(fix);
        if on {
            self.as_mut().collect_reports();
        }
        self.refresh_status();
    }

    pub fn shutdown(self: Pin<&mut Self>) {
        self.rust().schedule.stop();
    }

    fn on_event(self: Pin<&mut Self>, ev: Event) {
        // An event can sit in the queue while the user cancels or picks
        // another time: act only if it is still the scheduled one.
        let current = |this: &Self, t: i64| *this.scheduled_at() == t;
        match ev {
            Event::Poll => self.refresh_status(),
            Event::Apps => self.apps_round(),
            Event::RestartWarning(t) => {
                if current(&self, t) {
                    self.restart_soon();
                }
            }
            Event::RestartDue(t) => {
                let mut this = self;
                if !current(&this, t) {
                    return;
                }
                save_schedule(this.rust().fixtures.is_some(), None);
                if *this.restarting() {
                    // a restart is already in flight: just drop the plan
                    this.as_mut().clear_schedule_state();
                } else if *this.restart_needed() && this.rust().fixtures.is_none() {
                    // scheduledAt stays set while the restart is in flight: the
                    // shell must not quit a window-less instance before the
                    // logout call finishes (start_restart clears it after).
                    this.start_restart(Some(t));
                } else {
                    this.as_mut().clear_schedule_state();
                }
            }
            Event::RestartMissed(t) => {
                let mut this = self;
                if !current(&this, t) {
                    return;
                }
                this.as_mut().clear_schedule_state();
                let text = "The scheduled restart did not happen because the computer was asleep at that time. The update is still waiting. Restart when you are ready.";
                this.as_mut().set_info_text(q(text));
                this.restart_problem(q(text));
            }
        }
    }

    fn clear_schedule_state(mut self: Pin<&mut Self>) {
        save_schedule(self.rust().fixtures.is_some(), None);
        self.as_mut().set_scheduled_at(0);
    }

    // ---- status and helper operations ----

    pub fn refresh_status(self: Pin<&mut Self>) {
        self.spawn_op(Op::Status, false);
    }
    pub fn check_for_update(self: Pin<&mut Self>) {
        self.spawn_op(Op::Check, true);
    }
    pub fn download_update(self: Pin<&mut Self>) {
        self.spawn_op(Op::Upgrade, true);
    }
    pub fn rollback(self: Pin<&mut Self>) {
        self.spawn_op(Op::Rollback, true);
    }
    pub fn cancel_rollback(self: Pin<&mut Self>) {
        self.spawn_op(Op::CancelRollback, true);
    }
    pub fn switch_channel(self: Pin<&mut Self>, channel: &QString) {
        match channel.to_string().parse::<Channel>() {
            Ok(c) => self.spawn_op(Op::Switch(c), true),
            Err(e) => {
                let mut this = self;
                this.as_mut().set_error("switch", q(&e.to_string()));
            }
        }
    }

    /// Clears the error (and `errorOp`) and the info message. It leaves
    /// `busyOp` and `appsOp` alone: they belong to operations still running.
    pub fn dismiss_messages(mut self: Pin<&mut Self>) {
        self.as_mut().set_error("", QString::default());
        self.as_mut().set_info_text(QString::default());
    }

    /// Clears only the info message: a one-off confirmation ("Crash report
    /// sent") that shouldn't follow the user to other sections. Errors stay.
    pub fn dismiss_info(self: Pin<&mut Self>) {
        self.set_info_text(QString::default());
    }

    /// `foreground` operations show progress and errors; the silent status
    /// read does neither, and gives way to a foreground one.
    fn spawn_op(mut self: Pin<&mut Self>, op: Op, foreground: bool) {
        if foreground {
            if *self.busy() {
                return;
            }
            self.as_mut().set_error("", QString::default());
            self.as_mut().set_info_text(QString::default());
            self.as_mut().set_busy_text(q(op.label()));
            self.as_mut().begin_op(op.name());
            self.as_mut().set_busy(true);
        } else {
            if self.rust().status_inflight {
                return;
            }
            self.as_mut().rust_mut().status_inflight = true;
        }
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        let op2 = op.clone();
        let pq = qt.clone();
        let on_progress: ops::OnProgress = Box::new(move |p| {
            let _ = pq.queue(move |obj| obj.show_progress(p));
        });
        if !spawn_named("atlas-op", move || {
            let res = guarded(|| ops::run(&op2, fixtures.as_deref(), on_progress))
                .unwrap_or_else(|| Err(OpError::Message(INTERNAL.into())));
            let _ = qt.queue(move |obj| obj.finish_op(op2, foreground, res));
        }) {
            self.finish_op(
                op,
                foreground,
                Err(OpError::Message(
                    "Could not start a worker thread. The system may be out of resources.".into(),
                )),
            );
        }
    }

    /// The helper's progress, while a foreground op runs (a late one,
    /// queued before the op finished, is dropped).
    fn show_progress(mut self: Pin<&mut Self>, p: Option<atlas_core::progress::Progress>) {
        let p = p.filter(|_| *self.busy()).unwrap_or_default();
        self.as_mut().set_progress_stage(q(&p.stage));
        self.as_mut().set_progress_done(p.done as f64);
        self.as_mut().set_progress_total(p.total as f64);
        self.as_mut().set_progress_detail(q(&p.detail));
    }

    fn finish_op(mut self: Pin<&mut Self>, op: Op, foreground: bool, res: Result<Status, OpError>) {
        if foreground {
            self.as_mut().show_progress(None);
            self.as_mut().set_busy(false);
            self.as_mut().end_op(op.name());
        } else {
            self.as_mut().rust_mut().status_inflight = false;
        }
        match res {
            Ok(st) => {
                // a good read ends the "status" error a failed first read left
                if self.error_op().to_string() == "status" {
                    self.as_mut().set_error("", QString::default());
                }
                self.as_mut().apply_status(&st);
                if matches!(op, Op::Check | Op::Upgrade) {
                    let now = schedule::unix_now();
                    self.as_mut().set_last_checked(now);
                    if self.rust().fixtures.is_none() {
                        rc::set(RC_CHECKED, "At", Some(&now.to_string()));
                    }
                }
                if foreground {
                    let v = self.rust().view.clone();
                    // No banner where the Updates page's hero already
                    // shows the new state; it would say the same thing twice.
                    let msg = match op {
                        Op::Check if v.staged.present || v.available.present => String::new(),
                        Op::Check => "You are up to date.".to_string(),
                        Op::Upgrade if v.staged.present => String::new(),
                        Op::Upgrade => "No new update was downloaded.".to_string(),
                        Op::Rollback if v.rollback_queued => String::new(),
                        Op::Rollback => {
                            "Going back did not take effect. Nothing was changed.".to_string()
                        }
                        Op::CancelRollback => "The rollback was canceled.".to_string(),
                        Op::Switch(c) => format!("Switched to the {c} channel. Restart to finish."),
                        Op::Status => String::new(),
                    };
                    self.as_mut().set_info_text(q(&msg));
                }
            }
            Err(OpError::Cancelled) => {
                if foreground {
                    let action = match op {
                        Op::Check => "check for updates",
                        Op::Upgrade => "download updates",
                        Op::Rollback => "roll back the update",
                        Op::CancelRollback => "cancel the rollback",
                        Op::Switch(_) => "switch channels",
                        Op::Status => "read the update state",
                    };
                    self.as_mut()
                        .set_error(op.name(), q(&errors::denied_text(action)));
                }
            }
            Err(OpError::Message(m)) => {
                // A silent read that fails before anything was ever loaded
                // would leave "Reading the system state" up forever.
                if foreground || !*self.loaded() {
                    self.as_mut().set_error(op.name(), q(&m));
                }
            }
        }
    }

    fn apply_status(mut self: Pin<&mut Self>, st: &Status) {
        let v = view::from_status(st);
        // Before the setters: their signals make QML call back into load_notes().
        self.as_mut().rust_mut().view = v.clone();
        self.as_mut().set_loaded(true);
        self.as_mut().set_current_version(q(&v.current.version));
        self.as_mut().set_current_date(q(&v.current.date));
        self.as_mut().set_has_staged(v.staged.present);
        self.as_mut().set_staged_version(q(&v.staged.version));
        self.as_mut().set_staged_date(q(&v.staged.date));
        self.as_mut().set_has_rollback(v.rollback.present);
        self.as_mut().set_rollback_version(q(&v.rollback.version));
        self.as_mut().set_rollback_date(q(&v.rollback.date));
        self.as_mut().set_update_available(v.available.present);
        self.as_mut().set_available_version(q(&v.available.version));
        self.as_mut().set_available_date(q(&v.available.date));
        self.as_mut().set_channel(q(&v.channel));
        self.as_mut().set_restart_needed(v.restart_needed);
        self.as_mut().set_rollback_queued(v.rollback_queued);
        self.as_mut().set_rollback_target(q(&v.rollback_target));
        self.as_mut()
            .set_available_is_rollback(v.available_is_rollback);
        self.as_mut().set_available_is_bad(v.available_is_bad);
        self.as_mut().set_rollback_is_bad(v.rollback_is_bad);
        let staged = v.staged.clone();
        // Tell the user once per staged image, even across restarts of the tray.
        // (fixture mode keeps this in memory and never touches real settings)
        let fix = self.rust().fixtures.is_some();
        let seen = if fix {
            Some(self.rust().fixture_notified.clone())
        } else {
            rc::get(RC_NOTIFIED, "StagedDigest")
        };
        if staged.present
            && !staged.digest.is_empty()
            && seen.as_deref() != Some(staged.digest.as_str())
        {
            if fix {
                self.as_mut().rust_mut().fixture_notified = staged.digest.clone();
            } else {
                rc::set(RC_NOTIFIED, "StagedDigest", Some(&staged.digest));
            }
            self.as_mut().update_staged(q(&staged.version));
        }
        // Notes follow the version, but only while a window is open.
        if self.rust().window_open {
            self.as_mut().load_notes();
        }
        if !staged.present && !*self.restart_needed() && *self.scheduled_at() != 0 {
            // The staged update is gone (rebooted or cleaned): the plan is moot.
            self.as_mut().cancel_restart();
        }
    }

    // ---- release notes ----

    pub fn window_opened(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().window_open = true;
    }

    pub fn window_closed(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().window_open = false;
        // Tray mode keeps no notes in memory and makes no requests.
        self.as_mut().set_notes_state(q("none"));
        self.as_mut().set_notes_html(QString::default());
        self.as_mut().set_notes_plain(QString::default());
        self.as_mut().set_notes_version(QString::default());
        self.as_mut().set_notes_error(QString::default());
    }

    pub fn is_safe_link(&self, link: &QString) -> bool {
        notes::is_safe_link(&link.to_string())
    }

    pub fn load_notes(mut self: Pin<&mut Self>) {
        if !self.rust().window_open {
            return;
        }
        let v = self.rust().view.clone();
        let target = if v.staged.present {
            v.staged.version.clone()
        } else if v.available.present {
            v.available.version.clone()
        } else {
            String::new()
        };
        if target.is_empty() {
            self.as_mut().set_notes_state(q("none"));
            self.as_mut().set_notes_html(QString::default());
            self.as_mut().set_notes_plain(QString::default());
            self.as_mut().set_notes_version(QString::default());
            return;
        }
        let same = self.notes_version().to_string() == target;
        let state = self.notes_state().to_string();
        if same && (state == "ready" || state == "loading" || state == "missing") {
            return;
        }
        // A failure is not retried for a while, so a rate-limited or offline
        // machine does not hammer the server on every status refresh.
        if same
            && state == "error"
            && self
                .rust()
                .notes_failed
                .as_ref()
                .is_some_and(|(ver, at)| *ver == target && at.elapsed() < NOTES_BACKOFF)
        {
            return;
        }
        self.as_mut().set_notes_version(q(&target));
        self.as_mut().set_notes_state(q("loading"));
        self.as_mut().set_notes_html(QString::default());
        self.as_mut().set_notes_plain(QString::default());
        self.as_mut().set_notes_error(QString::default());
        let template = self.rust().config.release_notes_url.clone();
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        let target2 = target.clone();
        let started = spawn_named("atlas-notes", move || {
            let res = guarded(|| match &fixtures {
                Some(dir) => Ok(config::read_fixture(dir, "notes.json")
                    .map(|t| notes::parse_body(&t))
                    .unwrap_or(Notes::Missing)),
                None => notes::fetch(&template, &target2),
            })
            .unwrap_or_else(|| Err(notes::FetchError::Other("internal error".into())));
            let _ = qt.queue(move |mut obj| {
                // Ignore an answer for a version we no longer show, or after
                // the window closed.
                if obj.notes_version().to_string() != target2 || !obj.rust().window_open {
                    return;
                }
                match res {
                    Ok(Notes::Found(text, plain)) => {
                        obj.as_mut().set_notes_html(q(&text));
                        obj.as_mut().set_notes_plain(q(&plain));
                        obj.as_mut().set_notes_state(q("ready"));
                    }
                    Ok(Notes::Missing) => obj.as_mut().set_notes_state(q("missing")),
                    Err(e) => {
                        obj.as_mut().rust_mut().notes_failed =
                            Some((target2.clone(), std::time::Instant::now()));
                        obj.as_mut().set_notes_error(q(&e.text()));
                        obj.as_mut().set_notes_state(q("error"));
                    }
                }
            });
        });
        if !started {
            self.as_mut().set_notes_state(q("error"));
        }
    }

    // ---- changelog ----

    pub fn load_changelog(mut self: Pin<&mut Self>) {
        if !self.rust().window_open {
            return;
        }
        if self.rust().changelog_inflight {
            self.as_mut().rust_mut().changelog_dirty = true;
            return;
        }
        let may_fetch = self
            .rust()
            .changelog_failed
            .is_none_or(|t| t.elapsed() >= CHANGELOG_BACKOFF);
        let fixtures = self.rust().fixtures.clone();
        let template = self.rust().config.release_notes_url.clone();
        let v = &self.rust().view;
        let slot = |s: &view::Slot| {
            if s.present {
                s.version.clone()
            } else {
                String::new()
            }
        };
        let (current, staged, available) = (slot(&v.current), slot(&v.staged), slot(&v.available));
        if self.changelog_json().is_empty() {
            self.as_mut().set_changelog_state(q("loading"));
        }
        self.as_mut().rust_mut().changelog_inflight = true;
        let qt = self.qt_thread();
        let started = spawn_named("atlas-changelog", move || {
            let history = read_history(fixtures.as_deref());
            let wanted = [current.as_str(), staged.as_str(), available.as_str()];
            let res = release_list(fixtures.as_deref(), &template, &wanted, may_fetch);
            let failed = match &res {
                Ok(l) | Err(l) => l.fetch_failed,
            };
            let res = res
                .map_err(|l| l.note)
                .map(|ReleaseList { releases, note, .. }| {
                    let ran: Vec<changelog::Ran> = history
                        .iter()
                        .filter_map(|e| {
                            Some(changelog::Ran {
                                version: e.version.as_deref()?,
                                first_booted: &e.first_booted,
                            })
                        })
                        .collect();
                    let items = changelog::merge(&releases, &ran, &current, &staged, &available);
                    (
                        serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()),
                        note,
                    )
                });
            let _ = qt.queue(move |mut obj| {
                obj.as_mut().rust_mut().changelog_inflight = false;
                if failed {
                    obj.as_mut().rust_mut().changelog_failed = Some(std::time::Instant::now());
                }
                let again = std::mem::take(&mut obj.as_mut().rust_mut().changelog_dirty);
                match res {
                    Ok((text, note)) => {
                        obj.as_mut().set_changelog_json(q(&text));
                        obj.as_mut().set_changelog_note(q(&note));
                        obj.as_mut().set_changelog_state(q("ready"));
                    }
                    Err(e) => {
                        obj.as_mut().set_changelog_note(q(&e));
                        obj.as_mut().set_changelog_state(q("error"));
                    }
                }
                if again {
                    obj.load_changelog();
                }
            });
        });
        if !started {
            self.as_mut().rust_mut().changelog_inflight = false;
            self.as_mut().set_changelog_note(q(
                "Could not start a worker thread. The system may be out of resources.",
            ));
            self.as_mut().set_changelog_state(q("error"));
        }
    }

    // ---- history ----

    pub fn load_history(self: Pin<&mut Self>) {
        let fixtures = self.rust().fixtures.clone();
        let current = self.rust().view.current.digest.clone();
        let qt = self.qt_thread();
        spawn_named("atlas-history", move || {
            let entries = read_history(fixtures.as_deref());
            let rows: Vec<_> = entries
                .iter()
                .map(|e| {
                    let channel = e.image.rsplit_once(':').map(|x| x.1).unwrap_or("");
                    json!({
                        "version": e.version.clone().unwrap_or_else(|| e.digest.chars().take(19).collect()),
                        "built": e.timestamp.clone().unwrap_or_default(),
                        "booted": e.first_booted,
                        "channel": channel,
                        "current": !current.is_empty() && e.digest == current,
                    })
                })
                .collect();
            let text = serde_json::Value::Array(rows).to_string();
            let apps_text = serde_json::to_string(&apphistory::read(fixtures.as_deref()))
                .unwrap_or_else(|_| "[]".into());
            let _ = qt.queue(move |mut obj| {
                obj.as_mut().set_history_json(q(&text));
                obj.as_mut().set_app_history_json(q(&apps_text));
            });
        });
    }

    // ---- flatpak ----

    pub fn check_apps(mut self: Pin<&mut Self>) {
        if *self.apps_busy() {
            self.as_mut().rust_mut().apps_check_requested = true;
            return;
        }
        self.start_check(true);
    }

    /// `fresh`: the user asked just now, so an old error goes. A check
    /// queued behind another operation keeps the error that one left.
    fn start_check(mut self: Pin<&mut Self>, fresh: bool) {
        self.as_mut().set_apps_busy(true);
        self.as_mut().set_apps_op(q("checkApps"));
        if fresh {
            self.as_mut().clear_apps_error();
        }
        self.as_mut().set_apps_status(q("Looking for app updates…"));
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        if !spawn_named("atlas-apps", move || {
            if fixtures.is_some() && config::fixture_hold("checkApps") {
                config::hold_forever();
            }
            let res = guarded(|| {
                let rows = apps::list(true, false, fixtures.as_deref())?;
                // What the waiting updates ask for, shown before the user
                // presses "Update Apps".
                // (Fixture runs keep the notes they were given.)
                let checked = (!rows.is_empty() && fixtures.is_none())
                    .then(|| apps::check(fixtures.as_deref()));
                if let Some(e) = checked.as_ref().and_then(|c| c.error.as_ref()) {
                    eprintln!("atlas-updater: could not check what app updates ask for: {e}");
                }
                Ok((rows, checked))
            })
            .unwrap_or_else(|| Err(INTERNAL.to_string()));
            let _ = qt.queue(move |mut obj| {
                let mut unchecked = None;
                let res = res.map(|(rows, checked)| {
                    if let Some(c) = checked {
                        obj.as_mut().apps_checked(&c);
                        unchecked = c.error;
                    }
                    rows
                });
                obj.as_mut().apps_listed(res);
                if let Some(e) = unchecked {
                    obj.as_mut().user_error(q(&format!(
                        "Could not check which app updates ask for new permissions: {e}"
                    )));
                }
                obj.after_apps();
            });
        }) {
            self.as_mut()
                .apps_listed(Err("could not start a worker thread".into()));
            self.after_apps();
        }
    }

    /// An app operation ended: run what was asked for meanwhile.
    fn after_apps(mut self: Pin<&mut Self>) {
        if *self.apps_busy() {
            return;
        }
        if std::mem::take(&mut self.as_mut().rust_mut().apps_update_requested) {
            // Pressed while something else ran: on a notice, whose check
            // may be out of date or have failed.
            self.start_update(true);
        } else if std::mem::take(&mut self.as_mut().rust_mut().apps_check_requested) {
            self.start_check(false);
        } else if std::mem::take(&mut self.as_mut().rust_mut().apps_round_pending) {
            self.apps_round();
        }
    }

    /// Notes what a check found asking for new permissions. One that ran
    /// to the end replaces the earlier notes.
    fn apps_checked(mut self: Pin<&mut Self>, done: &apps::Done) {
        if done.error.is_none() {
            self.as_mut().rust_mut().apps_held.clear();
        }
        self.as_mut().apps_noted(&done.held_back);
    }

    /// Notes apps that ask for new permissions, for their rows.
    fn apps_noted(mut self: Pin<&mut Self>, held: &[apps::HeldApp]) {
        let mut this = self.as_mut().rust_mut();
        for h in held {
            this.apps_held.insert(h.key.clone(), h.clone());
        }
        if !held.is_empty() {
            // An "Update Apps" asked for before these were found would
            // install them before the user saw what they ask for.
            this.apps_update_requested = false;
        }
    }

    fn apps_listed(mut self: Pin<&mut Self>, res: Result<Vec<apps::Row>, String>) {
        self.as_mut().set_apps_busy(false);
        self.as_mut().set_apps_op(QString::default());
        self.as_mut().set_apps_status(QString::default());
        match res {
            Ok(mut rows) => {
                // Nothing waits: the next set, even the same apps again, is news.
                if rows.is_empty() && self.rust().fixtures.is_none() {
                    rc::set(RC_APPS, "Notified", None);
                }
                // Held apps keep their note until they are updated.
                let keys: std::collections::HashSet<String> =
                    rows.iter().map(apps::row_key).collect();
                let mut this = self.as_mut().rust_mut();
                this.apps_held.retain(|k, _| keys.contains(k));
                for r in &mut rows {
                    if let Some(h) = this.apps_held.get(&apps::row_key(r)) {
                        r.asks = apps::asks_text(&h.asks);
                    }
                }
                self.as_mut().set_apps_count(rows.len() as i32);
                let text = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
                self.as_mut().set_apps_json(q(&text));
            }
            Err(e) => {
                self.as_mut()
                    .user_error(q(&format!("Could not check app updates: {e}")));
            }
        }
    }

    /// The Updates page's button: the user sees each row's note there.
    pub fn update_apps(mut self: Pin<&mut Self>) {
        if *self.apps_busy() {
            self.as_mut().rust_mut().apps_update_requested = true;
            return;
        }
        self.start_update(false);
    }

    pub fn update_apps_checked(mut self: Pin<&mut Self>) {
        if *self.apps_busy() {
            // The notification's button during a background round, say.
            self.as_mut().rust_mut().apps_update_requested = true;
            return;
        }
        self.start_update(true);
    }

    /// `hold`: leave out what asks for new permissions, and say so.
    fn start_update(mut self: Pin<&mut Self>, hold: bool) {
        self.as_mut().set_apps_busy(true);
        self.as_mut().set_apps_op(q("updateApps"));
        self.as_mut().clear_apps_error();
        self.as_mut().set_apps_status(q("Updating apps…"));
        let fixtures = self.rust().fixtures.clone();
        // What the page showed when the user pressed it.
        let shown = self.rust().apps_held.clone();
        let qt = self.qt_thread();
        let qt_fail = qt.clone();
        if !spawn_named("atlas-apps-update", move || {
            let qt_progress = qt.clone();
            if fixtures.is_some() && config::fixture_hold("updateApps") {
                config::hold_forever();
            }
            let res = guarded(|| {
                // The page's notes may be hours old: look again, and install
                // nothing if something now asks for more than it showed.
                let unseen = if hold || fixtures.is_some() {
                    None
                } else {
                    apps::unseen(apps::check(None), &shown)
                };
                let done = match unseen {
                    Some(done) => done,
                    None => apps::update(fixtures.as_deref(), false, hold, move |line| {
                        let _ = qt_progress
                            .queue(move |mut obj| obj.as_mut().set_apps_status(q(&line)));
                    }),
                };
                if let Err(e) = apphistory::record(&done.updated, fixtures.as_deref()) {
                    eprintln!("atlas-updater: could not save the app update history: {e}");
                }
                // Fixture runs keep their list; a real run lists what is left.
                let rows = apps::list(false, false, fixtures.as_deref());
                (done, rows)
            });
            let _ = qt.queue(move |mut obj| {
                match res {
                    None => {
                        obj.as_mut().apps_idle();
                        obj.as_mut()
                            .user_error(q(&format!("Could not update apps: {INTERNAL}")));
                    }
                    Some((done, rows)) => {
                        let failed = done.error.clone();
                        // Noted before listing, so the rows show it.
                        obj.as_mut().apps_noted(&done.held_back);
                        let left = rows.as_ref().map(Vec::clone).unwrap_or_default();
                        obj.as_mut().apps_listed(rows);
                        if !done.held_back.is_empty() {
                            obj.as_mut()
                                .held_notice(&left, &done.held_back, failed.is_some());
                        }
                        if let Some(e) = failed {
                            obj.as_mut()
                                .user_error(q(&format!("Could not update apps: {e}")));
                        }
                        if !done.updated.is_empty() {
                            obj.as_mut().load_history();
                        }
                    }
                }
                obj.after_apps();
            });
        }) {
            let _ = qt_fail.queue(|mut obj| {
                obj.as_mut().apps_idle();
                obj.as_mut()
                    .user_error(q("Could not update apps: could not start a worker thread"));
                obj.after_apps();
            });
        }
    }

    fn clear_apps_error(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().apps_round_error = false;
        self.as_mut().set_apps_error(QString::default());
    }

    /// An error from something the user did: it stays until they act again.
    fn user_error(mut self: Pin<&mut Self>, text: QString) {
        self.as_mut().rust_mut().apps_round_error = false;
        self.as_mut().set_apps_error(text);
    }

    /// An error from a background round, which nobody is waiting on.
    fn round_error(mut self: Pin<&mut Self>, text: String) {
        eprintln!("atlas-updater: {text}");
        self.as_mut().set_apps_error(q(&text));
        self.as_mut().rust_mut().apps_round_error = true;
    }

    fn apps_idle(mut self: Pin<&mut Self>) {
        self.as_mut().set_apps_busy(false);
        self.as_mut().set_apps_op(QString::default());
        self.as_mut().set_apps_status(QString::default());
    }

    pub fn enable_background_apps(mut self: Pin<&mut Self>, on: bool) {
        if self.rust().fixtures.is_none() {
            let value = Some(if on { "true" } else { "false" });
            if let Err(e) = rc::try_set(RC_APPS, "Automatic", value) {
                // The switch goes back to what is in effect.
                eprintln!("atlas-updater: could not save the app update setting: {e}");
                self.as_mut()
                    .user_error(q(&format!("Could not save the setting: {e}")));
                return;
            }
            if on {
                // Don't make the user wait up to 6 hours to see it work.
                self.rust().schedule.apps_in(APPS_SOON);
            }
        }
        self.rust()
            .apps_auto_flag
            .store(on, std::sync::atomic::Ordering::Relaxed);
        self.as_mut().set_apps_auto(on);
    }

    /// The scheduled app round: look for app updates, and install them if
    /// the user turned that on. Whatever is left waiting for the user is
    /// announced once per set.
    fn apps_round(mut self: Pin<&mut Self>) {
        // Fixture mode never acts on its own.
        if self.rust().fixtures.is_some() {
            return;
        }
        if *self.apps_busy() {
            // Run when the user's own operation ends.
            self.as_mut().rust_mut().apps_round_pending = true;
            return;
        }
        let auto = *self.apps_auto();
        let flag = self.rust().apps_auto_flag.clone();
        self.as_mut().set_apps_busy(true);
        self.as_mut().set_apps_op(q("autoApps"));
        // An error already shown stays until something replaces it.
        self.as_mut().set_apps_status(q(if auto {
            "Updating apps in the background…"
        } else {
            "Looking for app updates…"
        }));
        let qt = self.qt_thread();
        let qt_fail = qt.clone();
        if !spawn_named("atlas-apps-auto", move || {
            let res = guarded(|| {
                let round =
                    apps::background(|| flag.load(std::sync::atomic::Ordering::Relaxed), None);
                if let Ok(r) = &round {
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
                round
            })
            .unwrap_or_else(|| Err(INTERNAL.to_string()));
            let _ = qt.queue(move |obj| obj.apps_round_done(res));
        }) {
            let _ = qt_fail.queue(|mut obj| {
                obj.as_mut().apps_idle();
                obj.as_mut().round_error(
                    "Could not check app updates: could not start a worker thread".into(),
                );
                obj.as_mut().apps_failed();
                obj.after_apps();
            });
        }
    }

    fn apps_round_done(mut self: Pin<&mut Self>, res: Result<apps::Round, String>) {
        self.as_mut().apps_round_finish(res);
        self.after_apps();
    }

    /// A background round failed: try again later, later each time.
    fn apps_failed(mut self: Pin<&mut Self>) {
        let mut this = self.as_mut().rust_mut();
        this.apps_failures = this.apps_failures.saturating_add(1);
        this.schedule.apps_in(apps_retry_after(this.apps_failures));
    }

    fn apps_round_finish(mut self: Pin<&mut Self>, res: Result<apps::Round, String>) {
        let round = match res {
            Ok(r) => r,
            Err(e) => {
                self.as_mut().apps_idle();
                self.as_mut()
                    .round_error(format!("Could not check app updates: {e}"));
                self.as_mut().apps_failed();
                return;
            }
        };
        let failed = round.done.error.is_some();
        if failed {
            self.as_mut().apps_failed();
        } else {
            self.as_mut().rust_mut().apps_failures = 0;
            if round.waited.is_some() {
                self.rust().schedule.apps_in(APPS_RETRY);
            }
        }
        if !round.done.updated.is_empty() {
            self.as_mut().load_history();
        }
        if round.ran {
            // This run checked every update: its held set is the whole one.
            self.as_mut().rust_mut().apps_held.clear();
        }
        self.as_mut().apps_noted(&round.done.held_back);
        let Some(rows) = round.rows else {
            // Not listed: a metered or offline connection. Nothing to say.
            self.as_mut().apps_idle();
            return;
        };
        self.as_mut().apps_listed(Ok(rows.clone()));
        if let Some(e) = &round.done.error {
            // Logged by the worker already.
            self.as_mut()
                .set_apps_error(q(&format!("Could not update apps in the background: {e}")));
            self.as_mut().rust_mut().apps_round_error = true;
        } else if let Some(e) = &round.unchecked {
            self.as_mut().set_apps_error(q(&format!(
                "Could not check which app updates ask for new permissions: {e}"
            )));
            self.as_mut().rust_mut().apps_round_error = true;
        } else if self.rust().apps_round_error {
            // This round worked: an earlier round's error is out of date.
            self.as_mut().clear_apps_error();
        }
        // A low-battery wait is quiet: it tries again later.
        if rows.is_empty() || round.waited.is_some() {
            return;
        }
        // Each notice once: the same apps, held or failed the same way, are
        // not news.
        let key = apps::notice_key(
            &rows,
            &round.done.held_back,
            failed || round.unchecked.is_some(),
        );
        if rc::get(RC_APPS, "Notified").as_deref() == Some(key.as_str()) {
            return;
        }
        rc::set(RC_APPS, "Notified", Some(&key));
        let text = apps::ready_text(&rows, &round.done.held_back, failed, round.auto);
        // Nothing asking for new permissions, or not knowing, installs from
        // a notification: the user sees it on the Updates page first.
        let can_update = round.done.held_back.is_empty() && round.unchecked.is_none();
        self.app_updates_ready(q(&text), can_update);
    }

    /// An update the user started from a notice left apps out: say which,
    /// and don't say it again for the same set.
    fn held_notice(
        mut self: Pin<&mut Self>,
        rows: &[apps::Row],
        held: &[apps::HeldApp],
        failed: bool,
    ) {
        if self.rust().fixtures.is_none() {
            let key = apps::notice_key(rows, held, failed);
            rc::set(RC_APPS, "Notified", Some(&key));
        }
        // Not "on its own": the user started it.
        let text = apps::ready_text(rows, held, failed, false);
        self.app_updates_ready(q(&text), false);
    }

    // ---- restart ----

    pub fn restart_now(self: Pin<&mut Self>) {
        self.start_restart(None);
    }

    /// `scheduled` is the time of the scheduled restart that fired, if this is
    /// one: only that restart's end (a failure, or a logout that did not
    /// happen) clears the schedule; a manual restart leaves it alone.
    fn start_restart(mut self: Pin<&mut Self>, scheduled: Option<i64>) {
        if *self.restarting() {
            return;
        }
        if self.rust().fixtures.is_some() {
            if config::fixture_hold("restart") {
                // screenshot hook: look like a restart in progress, for good
                self.as_mut().set_restarting(true);
                return;
            }
            self.as_mut()
                .set_info_text(q("Developer fixtures: restart skipped."));
            return;
        }
        // stays set until the session ends, or the logout is canceled
        self.as_mut().set_restarting(true);
        let qt = self.qt_thread();
        let qt_fail = qt.clone();
        let qt_ok = qt.clone();
        let clear = move |mut obj: Pin<&mut qobject::Backend>| {
            obj.as_mut().set_restarting(false);
            if let Some(t) = scheduled
                && *obj.scheduled_at() == t
            {
                obj.as_mut().clear_schedule_state();
            }
        };
        let fail = move |mut obj: Pin<&mut qobject::Backend>, e: String| {
            // The Updates page shows the error; restartProblem also lets the
            // shell notify when the window is not active (a scheduled
            // restart in the tray, for example).
            if e == restart::CANCELED {
                obj.as_mut().set_info_text(q(&e));
                clear(obj);
                return;
            }
            if e == restart::NO_ANSWER {
                // Plasma may still act on the request: say so, no notification.
                obj.as_mut().set_error("restart", q(&e));
                clear(obj);
                return;
            }
            let text = format!(
                "Could not restart the computer: {e}. The update is still waiting. Restart it yourself when you are ready."
            );
            obj.as_mut().set_error("restart", q(&text));
            clear(obj.as_mut());
            obj.restart_problem(q(&text));
        };
        if !spawn_named("atlas-restart", move || {
            let res =
                guarded(restart::logout_and_reboot).unwrap_or_else(|| Err("internal error".into()));
            match res {
                Err(e) => {
                    let _ = qt.queue(move |obj| fail(obj, e));
                }
                // Not reached when the restart happens (the session ends
                // first); a canceled one comes back as restart::CANCELED.
                Ok(()) => {
                    let _ = qt_ok.queue(clear);
                }
            }
        }) {
            let _ = qt_fail.queue(move |obj| fail(obj, "could not start a worker thread".into()));
        }
    }

    pub fn schedule_restart(mut self: Pin<&mut Self>, at: i64) {
        if !*self.restart_needed() {
            self.as_mut()
                .set_info_text(q("There is no update waiting for a restart."));
            return;
        }
        if at <= schedule::unix_now() {
            self.as_mut()
                .set_info_text(q("That time has already passed. Pick a later time."));
            return;
        }
        save_schedule(self.rust().fixtures.is_some(), Some(at));
        self.as_mut().set_scheduled_at(at);
        self.rust().schedule.set_restart(Some(at));
    }

    pub fn cancel_restart(mut self: Pin<&mut Self>) {
        self.rust().schedule.set_restart(None);
        self.as_mut().clear_schedule_state();
    }

    // ---- crash reports (opt-in; see crash.rs) ----

    pub fn enable_crash_reports(mut self: Pin<&mut Self>, on: bool) {
        if self.rust().fixtures.is_none()
            && let Err(e) = (atlas_core::crash::Settings { enabled: on }).save()
        {
            // Not saved: collection would still see "off", so do not claim "on".
            self.as_mut().set_error(
                "crashSetting",
                q(&format!("Could not save the crash report setting: {e}")),
            );
            return;
        }
        self.as_mut().set_crash_enabled(on);
        if on {
            self.as_mut().collect_reports();
            self.load_reports();
        } else {
            // Off means off: nothing stays queued on screen.
            let has_server = *self.crash_has_server();
            self.as_mut().reports_loaded(Vec::new(), has_server);
            self.as_mut().set_sent_json(q("[]"));
        }
    }

    pub fn load_reports(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().reports_gen += 1;
        let gen_now = self.rust().reports_gen;
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        spawn_named("atlas-reports", move || {
            let (reports, has_server) = guarded(|| {
                let reports = match &fixtures {
                    Some(dir) => fixture_reports(dir, "crash-pending.json"),
                    None => atlas_core::crash::pending(),
                };
                let has_server = match &fixtures {
                    Some(d) => config::read_fixture(d, "crash-server").is_some(),
                    None => atlas_core::crash::Endpoint::load().is_some(),
                };
                (reports, has_server)
            })
            .unwrap_or_default();
            let _ = qt.queue(move |mut obj| {
                // Whether a server is set up doesn't depend on the switch:
                // Settings explains it either way.
                obj.as_mut().set_crash_has_server(has_server);
                // Switched off while we were reading: show nothing.
                if *obj.crash_enabled() && obj.rust().reports_gen == gen_now {
                    obj.reports_loaded(reports, has_server);
                }
            });
        });
    }

    fn reports_loaded(mut self: Pin<&mut Self>, reports: Vec<Report>, has_server: bool) {
        let views: Vec<_> = reports
            .iter()
            .map(|r| crash::view(r, !has_server))
            .collect();
        self.as_mut().set_crash_has_server(has_server);
        self.as_mut().set_reports_count(reports.len() as i32);
        self.as_mut()
            .set_reports_json(q(&serde_json::Value::Array(views).to_string()));
        self.as_mut().rust_mut().pending = reports;
    }

    pub fn load_sent_reports(self: Pin<&mut Self>) {
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        spawn_named("atlas-sent", move || {
            let text = guarded(|| {
                let mut sent = match &fixtures {
                    Some(dir) => fixture_reports(dir, "crash-sent.json"),
                    None => atlas_core::crash::sent(),
                };
                sent.reverse(); // newest first
                let views: Vec<_> = sent.iter().map(|r| crash::view(r, false)).collect();
                serde_json::Value::Array(views).to_string()
            })
            .unwrap_or_else(|| "[]".into());
            let _ = qt.queue(move |mut obj| obj.as_mut().set_sent_json(q(&text)));
        });
    }

    pub fn collect_reports(self: Pin<&mut Self>) {
        if !*self.crash_enabled() {
            return;
        }
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        spawn_named("atlas-collect", move || {
            if fixtures.is_some() {
                return;
            }
            let first = guarded(|| {
                let mut new = atlas_core::crash::collect_coredumps(None);
                new.extend(atlas_core::crash::collect_events(None));
                new.first().map(|r| {
                    (
                        crate::crash::display_name(&r.app_name).to_string(),
                        r.report_type.clone(),
                    )
                })
            })
            .flatten();
            let _ = qt.queue(move |mut obj| {
                // Switched off while collecting: no list, no notification.
                if !*obj.crash_enabled() {
                    return;
                }
                obj.as_mut().load_reports();
                if let Some((app, kind)) = first {
                    obj.as_mut().report_found(q(&app), q(&kind));
                }
            });
        });
    }

    pub fn send_report(mut self: Pin<&mut Self>, event_id: &QString) {
        let id = event_id.to_string();
        if *self.busy() {
            return; // one send at a time; also blocks discard (see below)
        }
        let Some(r) = self
            .rust()
            .pending
            .iter()
            .find(|r| r.event_id == id)
            .cloned()
        else {
            return;
        };
        self.as_mut().set_error("", QString::default());
        self.as_mut().set_busy_text(q("Sending the crash report…"));
        self.as_mut().begin_op("sendReport");
        self.as_mut().set_busy(true);
        let fixtures = self.rust().fixtures.is_some();
        let qt = self.qt_thread();
        let qt_fail = qt.clone();
        let done = move |mut obj: Pin<&mut qobject::Backend>,
                         id: String,
                         res: std::io::Result<()>| {
            obj.as_mut().set_busy(false);
            obj.as_mut().end_op("sendReport");
            match res {
                Ok(()) => {
                    obj.as_mut().drop_pending(&id);
                    // an earlier load, dropped as stale, may have found newer
                    // reports: read the folder again
                    if !fixtures {
                        obj.as_mut().load_reports();
                    }
                    obj.as_mut().set_info_text(q("Crash report sent to the AtlasOS GitHub project. Thank you."));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => obj.as_mut().set_error("sendReport", q(
                    "No crash report server is set up on this system, so the report can't be sent. It stays here until you decide.",
                )),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    obj.as_mut().set_error("sendReport", q("Crash reports are turned off, so nothing was sent."))
                }
                Err(e) => obj.as_mut().set_error("sendReport", q(&format!("Could not send the crash report: {e}"))),
            }
        };
        let id2 = id.clone();
        if !spawn_named("atlas-send", move || {
            let res = guarded(|| {
                if fixtures {
                    if config::fixture_hold("sendReport") {
                        config::hold_forever();
                    }
                    Ok(())
                } else {
                    atlas_core::crash::send(&r)
                }
            })
            .unwrap_or_else(|| Err(std::io::Error::other(INTERNAL)));
            let _ = qt.queue(move |obj| done(obj, id2, res));
        }) {
            let _ = qt_fail.queue(move |obj| {
                done(
                    obj,
                    id,
                    Err(std::io::Error::other("could not start a worker thread")),
                )
            });
        }
    }

    pub fn discard_report(mut self: Pin<&mut Self>, event_id: &QString) {
        if *self.busy() {
            return; // a send is running: do not change the list under it
        }
        let id = event_id.to_string();
        if let Some(r) = self.rust().pending.iter().find(|r| r.event_id == id)
            && self.rust().fixtures.is_none()
            && let Err(e) = atlas_core::crash::discard(r)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            // Still on disk: keep it listed rather than let it come back later.
            self.as_mut().set_error(
                "discardReport",
                q(&format!("Could not delete the crash report: {e}")),
            );
            return;
        }
        self.as_mut().drop_pending(&id);
        if self.rust().fixtures.is_none() {
            self.load_reports();
        }
    }

    /// Remove one pending report, by id, from the list the UI shows.
    fn drop_pending(self: Pin<&mut Self>, id: &str) {
        let mut this = self;
        let has_server = *this.crash_has_server();
        this.as_mut().rust_mut().reports_gen += 1;
        let mut list = this.rust().pending.clone();
        list.retain(|r| r.event_id != id);
        this.as_mut().reports_loaded(list, has_server);
    }
}

fn fixture_reports(dir: &std::path::Path, name: &str) -> Vec<Report> {
    config::read_fixture(dir, name)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_worker_is_reported_not_lost() {
        assert_eq!(guarded(|| 7), Some(7));
        assert_eq!(guarded(|| -> i32 { panic!("boom") }), None);
    }

    #[test]
    fn failed_rounds_back_off() {
        let m = |n| apps_retry_after(n).as_secs() / 60;
        assert_eq!(
            [m(0), m(1), m(2), m(3), m(4), m(5), m(99)],
            [30, 30, 60, 120, 240, 360, 360]
        );
    }
}
