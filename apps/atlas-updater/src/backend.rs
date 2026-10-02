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
        #[qproperty(QString, history_json, cxx_name = "historyJson")]
        #[qproperty(i64, scheduled_at, cxx_name = "scheduledAt")]
        #[qproperty(bool, crash_enabled, cxx_name = "crashEnabled")]
        #[qproperty(bool, crash_has_server, cxx_name = "crashHasServer")]
        #[qproperty(QString, reports_json, cxx_name = "reportsJson")]
        #[qproperty(i32, reports_count, cxx_name = "reportsCount")]
        #[qproperty(QString, sent_json, cxx_name = "sentJson")]
        /// `ATLAS_UPDATER_FIXTURES` is set: everything shown is fake. The UI
        /// shows a permanent banner.
        #[qproperty(bool, fixtures_active, cxx_name = "fixturesActive")]
        #[namespace = "atlas_updater"]
        type Backend = super::BackendRust;

        /// A staged update we have not told the user about yet.
        #[qsignal]
        #[cxx_name = "updateStaged"]
        fn update_staged(self: Pin<&mut Backend>, version: QString);

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

        #[qinvokable]
        #[cxx_name = "checkApps"]
        fn check_apps(self: Pin<&mut Backend>);
        #[qinvokable]
        #[cxx_name = "updateApps"]
        fn update_apps(self: Pin<&mut Backend>);

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
use crate::{apps, crash, rc, restart};

const RC_RESTART: &str = "Restart";
const RC_NOTIFIED: &str = "Notified";

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
    history_json: QString,
    scheduled_at: i64,
    crash_enabled: bool,
    crash_has_server: bool,
    reports_json: QString,
    reports_count: i32,
    sent_json: QString,
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
}

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
        if !spawn_named("atlas-op", move || {
            let res = guarded(|| ops::run(&op2, fixtures.as_deref()))
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

    fn finish_op(mut self: Pin<&mut Self>, op: Op, foreground: bool, res: Result<Status, OpError>) {
        if foreground {
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
                        Op::CancelRollback => "The rollback was cancelled.".to_string(),
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

    // ---- history ----

    pub fn load_history(self: Pin<&mut Self>) {
        let fixtures = self.rust().fixtures.clone();
        let current = self.rust().view.current.digest.clone();
        let qt = self.qt_thread();
        spawn_named("atlas-history", move || {
            let entries = match &fixtures {
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
            };
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
            let _ = qt.queue(move |mut obj| obj.as_mut().set_history_json(q(&text)));
        });
    }

    // ---- flatpak ----

    pub fn check_apps(mut self: Pin<&mut Self>) {
        if *self.apps_busy() {
            return;
        }
        self.as_mut().set_apps_busy(true);
        self.as_mut().set_apps_op(q("checkApps"));
        self.as_mut().set_apps_error(QString::default());
        self.as_mut().set_apps_status(q("Looking for app updates…"));
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        if !spawn_named("atlas-apps", move || {
            if fixtures.is_some() && config::fixture_hold("checkApps") {
                config::hold_forever();
            }
            let res = guarded(|| apps::list(true, fixtures.as_deref()))
                .unwrap_or_else(|| Err(INTERNAL.to_string()));
            let _ = qt.queue(move |obj| obj.apps_listed(res));
        }) {
            self.apps_listed(Err("could not start a worker thread".into()));
        }
    }

    fn apps_listed(mut self: Pin<&mut Self>, res: Result<Vec<apps::Row>, String>) {
        self.as_mut().set_apps_busy(false);
        self.as_mut().set_apps_op(QString::default());
        self.as_mut().set_apps_status(QString::default());
        match res {
            Ok(rows) => {
                self.as_mut().set_apps_count(rows.len() as i32);
                let text = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
                self.as_mut().set_apps_json(q(&text));
            }
            Err(e) => {
                self.as_mut()
                    .set_apps_error(q(&format!("Could not check app updates: {e}")));
            }
        }
    }

    pub fn update_apps(mut self: Pin<&mut Self>) {
        if *self.apps_busy() {
            return;
        }
        self.as_mut().set_apps_busy(true);
        self.as_mut().set_apps_op(q("updateApps"));
        self.as_mut().set_apps_error(QString::default());
        self.as_mut().set_apps_status(q("Updating apps…"));
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        let qt_fail = qt.clone();
        if !spawn_named("atlas-apps-update", move || {
            let qt_progress = qt.clone();
            if fixtures.is_some() && config::fixture_hold("updateApps") {
                config::hold_forever();
            }
            let res = guarded(|| {
                apps::update_all(fixtures.as_deref(), move |line| {
                    let _ =
                        qt_progress.queue(move |mut obj| obj.as_mut().set_apps_status(q(&line)));
                })
                .and_then(|()| apps::list(false, fixtures.as_deref()))
            })
            .unwrap_or_else(|| Err(INTERNAL.to_string()));
            let _ = qt.queue(move |mut obj| {
                if let Err(e) = &res {
                    obj.as_mut().set_apps_busy(false);
                    obj.as_mut().set_apps_op(QString::default());
                    obj.as_mut().set_apps_status(QString::default());
                    obj.as_mut()
                        .set_apps_error(q(&format!("Could not update apps: {e}")));
                } else {
                    obj.apps_listed(res);
                    // Fixture runs keep their list; a real run is now empty.
                }
            });
        }) {
            let _ = qt_fail.queue(|mut obj| {
                obj.as_mut().set_apps_busy(false);
                obj.as_mut().set_apps_op(QString::default());
                obj.as_mut().set_apps_status(QString::default());
                obj.as_mut()
                    .set_apps_error(q("Could not update apps: could not start a worker thread"));
            });
        }
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
                // The request went through. If the session still runs (the
                // logout was dismissed) the time must not stay set.
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
                    obj.as_mut().set_info_text(q("Crash report sent. Thank you."));
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
}
