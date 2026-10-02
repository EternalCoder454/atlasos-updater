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
        #[qproperty(QString, notes_state, cxx_name = "notesState")]
        #[qproperty(QString, notes_text, cxx_name = "notesText")]
        #[qproperty(QString, notes_version, cxx_name = "notesVersion")]
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
        #[qinvokable]
        #[cxx_name = "switchChannel"]
        fn switch_channel(self: Pin<&mut Backend>, channel: &QString);
        #[qinvokable]
        #[cxx_name = "dismissMessages"]
        fn dismiss_messages(self: Pin<&mut Backend>);

        #[qinvokable]
        #[cxx_name = "loadNotes"]
        fn load_notes(self: Pin<&mut Backend>);
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
        /// "Send": the user saw the exact data. `index` is into `reportsJson`.
        #[qinvokable]
        #[cxx_name = "sendReport"]
        fn send_report(self: Pin<&mut Backend>, index: i32);
        /// "Don't send".
        #[qinvokable]
        #[cxx_name = "discardReport"]
        fn discard_report(self: Pin<&mut Backend>, index: i32);
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
use crate::errors::OpError;
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
    notes_state: QString,
    notes_text: QString,
    notes_version: QString,
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

    // Not exposed to QML.
    config: Config,
    fixtures: Option<PathBuf>,
    schedule: Schedule,
    started: bool,
    status_inflight: bool,
    view: View,
    pending: Vec<Report>,
}

fn q(s: &str) -> QString {
    QString::from(s)
}

impl qobject::Backend {
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
        if let Some(at) = rc::get(RC_RESTART, "ScheduledAt").and_then(|v| v.parse::<i64>().ok()) {
            // A time that passed while we were not running is dropped: never
            // restart the machine unexpectedly at login.
            if at > schedule::unix_now() {
                self.as_mut().set_scheduled_at(at);
                schedule.set_restart(Some(at));
            } else {
                rc::set(RC_RESTART, "ScheduledAt", None);
            }
        }
        let qt = self.qt_thread();
        let sched = schedule.clone();
        let _ = std::thread::Builder::new()
            .name("atlas-schedule".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                sched.run(move |ev| {
                    let _ = qt.queue(move |obj| obj.on_event(ev));
                });
            });
        // Crash reports are opt-in: read the setting, and only when on look for new ones.
        let on = match &self.rust().fixtures {
            Some(d) => config::read_fixture(d, "crash-enabled").is_some(),
            None => atlas_core::crash::Settings::load().enabled,
        };
        self.as_mut().set_crash_enabled(on);
        if on {
            self.as_mut().collect_reports();
        }
        self.refresh_status();
    }

    pub fn shutdown(self: Pin<&mut Self>) {
        self.rust().schedule.stop();
    }

    fn on_event(self: Pin<&mut Self>, ev: Event) {
        match ev {
            Event::Poll => self.refresh_status(),
            Event::RestartWarning => {
                self.restart_soon();
            }
            Event::RestartDue => {
                let mut this = self;
                this.as_mut().clear_schedule_state();
                if *this.restart_needed() {
                    this.restart_now();
                }
            }
        }
    }

    fn clear_schedule_state(mut self: Pin<&mut Self>) {
        rc::set(RC_RESTART, "ScheduledAt", None);
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
    pub fn switch_channel(self: Pin<&mut Self>, channel: &QString) {
        match channel.to_string().parse::<Channel>() {
            Ok(c) => self.spawn_op(Op::Switch(c), true),
            Err(e) => {
                let mut this = self;
                this.as_mut().set_error_text(q(&e.to_string()));
            }
        }
    }

    pub fn dismiss_messages(mut self: Pin<&mut Self>) {
        self.as_mut().set_error_text(QString::default());
        self.as_mut().set_info_text(QString::default());
    }

    /// `foreground` operations show progress and errors; the silent status
    /// read does neither, and gives way to a foreground one.
    fn spawn_op(mut self: Pin<&mut Self>, op: Op, foreground: bool) {
        if foreground {
            if *self.busy() {
                return;
            }
            self.as_mut().set_error_text(QString::default());
            self.as_mut().set_info_text(QString::default());
            self.as_mut().set_busy_text(q(op.label()));
            self.as_mut().set_busy(true);
        } else {
            if self.rust().status_inflight {
                return;
            }
            self.as_mut().rust_mut().status_inflight = true;
        }
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let res = ops::run(&op, fixtures.as_deref());
            let _ = qt.queue(move |obj| obj.finish_op(op, foreground, res));
        });
    }

    fn finish_op(mut self: Pin<&mut Self>, op: Op, foreground: bool, res: Result<Status, OpError>) {
        if foreground {
            self.as_mut().set_busy(false);
        } else {
            self.as_mut().rust_mut().status_inflight = false;
        }
        match res {
            Ok(st) => {
                self.as_mut().apply_status(&st);
                if foreground {
                    let v = self.rust().view.clone();
                    let msg = match op {
                        Op::Check if v.staged.present => {
                            "An update is downloaded and waiting for a restart.".to_string()
                        }
                        Op::Check if v.available.present => {
                            format!("Version {} is available.", v.available.version)
                        }
                        Op::Check => "You are up to date.".to_string(),
                        Op::Upgrade => {
                            "The update is downloaded. Restart to finish installing it.".to_string()
                        }
                        Op::Rollback => {
                            "Done. Restart to go back to the previous version.".to_string()
                        }
                        Op::Switch(c) => format!("Switched to the {c} channel. Restart to finish."),
                        Op::Status => String::new(),
                    };
                    self.as_mut().set_info_text(q(&msg));
                }
            }
            Err(OpError::Cancelled) => {}
            Err(OpError::Message(m)) => {
                if foreground {
                    self.as_mut().set_error_text(q(&m));
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
        let staged = v.staged.clone();
        // Tell the user once per staged image, even across restarts of the tray.
        if staged.present
            && !staged.digest.is_empty()
            && rc::get(RC_NOTIFIED, "StagedDigest").as_deref() != Some(staged.digest.as_str())
        {
            rc::set(RC_NOTIFIED, "StagedDigest", Some(&staged.digest));
            self.as_mut().update_staged(q(&staged.version));
        }
        // Notes follow the version once the window has asked for them.
        if !self.notes_version().is_empty() {
            self.as_mut().load_notes();
        }
        if !staged.present && !*self.restart_needed() && *self.scheduled_at() != 0 {
            // The staged update is gone (rebooted or cleaned): the plan is moot.
            self.as_mut().cancel_restart();
        }
    }

    // ---- release notes ----

    pub fn load_notes(mut self: Pin<&mut Self>) {
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
            self.as_mut().set_notes_text(QString::default());
            self.as_mut().set_notes_version(QString::default());
            return;
        }
        let state = self.notes_state().to_string();
        if self.notes_version().to_string() == target
            && (state == "ready" || state == "loading" || state == "missing")
        {
            return;
        }
        self.as_mut().set_notes_version(q(&target));
        self.as_mut().set_notes_state(q("loading"));
        self.as_mut().set_notes_text(QString::default());
        let template = self.rust().config.release_notes_url.clone();
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let res = match &fixtures {
                Some(dir) => Ok(config::read_fixture(dir, "notes.json")
                    .map(|t| notes::parse_body(&t))
                    .unwrap_or(Notes::Missing)),
                None => notes::fetch(&template, &target),
            };
            let _ = qt.queue(move |mut obj| {
                // Ignore an answer for a version we no longer show.
                if obj.notes_version().to_string() != target {
                    return;
                }
                match res {
                    Ok(Notes::Found(text)) => {
                        obj.as_mut().set_notes_text(q(&text));
                        obj.as_mut().set_notes_state(q("ready"));
                    }
                    Ok(Notes::Missing) => obj.as_mut().set_notes_state(q("missing")),
                    Err(_) => obj.as_mut().set_notes_state(q("error")),
                }
            });
        });
    }

    // ---- history ----

    pub fn load_history(self: Pin<&mut Self>) {
        let fixtures = self.rust().fixtures.clone();
        let current = self.rust().view.current.digest.clone();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
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
        self.as_mut().set_apps_error(QString::default());
        self.as_mut().set_apps_status(q("Looking for app updates…"));
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let res = apps::list(true, fixtures.as_deref());
            let _ = qt.queue(move |obj| obj.apps_listed(res));
        });
    }

    fn apps_listed(mut self: Pin<&mut Self>, res: Result<Vec<apps::Row>, String>) {
        self.as_mut().set_apps_busy(false);
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
        self.as_mut().set_apps_error(QString::default());
        self.as_mut().set_apps_status(q("Updating apps…"));
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let qt_progress = qt.clone();
            let res = apps::update_all(fixtures.as_deref(), move |line| {
                let _ = qt_progress.queue(move |mut obj| obj.as_mut().set_apps_status(q(&line)));
            })
            .and_then(|()| apps::list(false, fixtures.as_deref()));
            let _ = qt.queue(move |mut obj| {
                if let Err(e) = &res {
                    obj.as_mut().set_apps_busy(false);
                    obj.as_mut().set_apps_status(QString::default());
                    obj.as_mut()
                        .set_apps_error(q(&format!("Could not update apps: {e}")));
                } else {
                    obj.apps_listed(res);
                    // Fixture runs keep their list; a real run is now empty.
                }
            });
        });
    }

    // ---- restart ----

    pub fn restart_now(mut self: Pin<&mut Self>) {
        if self.rust().fixtures.is_some() {
            self.as_mut()
                .set_info_text(q("Developer fixtures: restart skipped."));
            return;
        }
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            if let Err(e) = restart::logout_and_reboot() {
                let _ = qt.queue(move |mut obj| {
                    obj.as_mut().set_error_text(q(&format!(
                        "Could not ask Plasma to restart the computer: {e}"
                    )));
                });
            }
        });
    }

    pub fn schedule_restart(mut self: Pin<&mut Self>, at: i64) {
        if !*self.restart_needed() || at <= schedule::unix_now() {
            return;
        }
        rc::set(RC_RESTART, "ScheduledAt", Some(&at.to_string()));
        self.as_mut().set_scheduled_at(at);
        self.rust().schedule.set_restart(Some(at));
    }

    pub fn cancel_restart(mut self: Pin<&mut Self>) {
        self.rust().schedule.set_restart(None);
        self.as_mut().clear_schedule_state();
    }

    // ---- crash reports (opt-in; see crash.rs) ----

    pub fn enable_crash_reports(mut self: Pin<&mut Self>, on: bool) {
        if self.rust().fixtures.is_none() {
            let _ = atlas_core::crash::Settings { enabled: on }.save();
        }
        self.as_mut().set_crash_enabled(on);
        if on {
            self.as_mut().collect_reports();
            self.load_reports();
        }
    }

    pub fn load_reports(self: Pin<&mut Self>) {
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let reports = match &fixtures {
                Some(dir) => fixture_reports(dir, "crash-pending.json"),
                None => atlas_core::crash::pending(),
            };
            let has_server = fixtures.is_some()
                && config::read_fixture(fixtures.as_deref().unwrap(), "crash-server").is_some()
                || fixtures.is_none() && atlas_core::crash::Endpoint::load().is_some();
            let _ = qt.queue(move |obj| obj.reports_loaded(reports, has_server));
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
        std::thread::spawn(move || {
            let mut sent = match &fixtures {
                Some(dir) => fixture_reports(dir, "crash-sent.json"),
                None => atlas_core::crash::sent(),
            };
            sent.reverse(); // newest first
            let views: Vec<_> = sent.iter().map(|r| crash::view(r, false)).collect();
            let text = serde_json::Value::Array(views).to_string();
            let _ = qt.queue(move |mut obj| obj.as_mut().set_sent_json(q(&text)));
        });
    }

    pub fn collect_reports(self: Pin<&mut Self>) {
        if !*self.crash_enabled() {
            return;
        }
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            if fixtures.is_some() {
                return;
            }
            let mut new = atlas_core::crash::collect_coredumps(None);
            new.extend(atlas_core::crash::collect_events(None));
            let first = new
                .first()
                .map(|r| (r.app_name.clone(), r.report_type.clone()));
            let _ = qt.queue(move |mut obj| {
                obj.as_mut().load_reports();
                if let Some((app, kind)) = first {
                    obj.as_mut().report_found(q(&app), q(&kind));
                }
            });
        });
    }

    pub fn send_report(mut self: Pin<&mut Self>, index: i32) {
        let Some(r) = self.rust().pending.get(index as usize).cloned() else {
            return;
        };
        if *self.busy() {
            return;
        }
        self.as_mut().set_error_text(QString::default());
        self.as_mut().set_busy_text(q("Sending the crash report…"));
        self.as_mut().set_busy(true);
        let fixtures = self.rust().fixtures.is_some();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let res = if fixtures {
                Ok(())
            } else {
                atlas_core::crash::send(&r)
            };
            let _ = qt.queue(move |mut obj| {
                obj.as_mut().set_busy(false);
                match res {
                    Ok(()) => {
                        obj.as_mut().drop_pending(index);
                        obj.as_mut().set_info_text(q("Crash report sent. Thank you."));
                    }
                    Err(e) if e.to_string().contains("no endpoint") => obj.as_mut().set_error_text(q(
                        "No crash report server is set up on this system, so the report can't be sent. It stays here until you decide.",
                    )),
                    Err(e) => obj.as_mut().set_error_text(q(&format!("Could not send the crash report: {e}"))),
                }
            });
        });
    }

    pub fn discard_report(self: Pin<&mut Self>, index: i32) {
        if let Some(r) = self.rust().pending.get(index as usize) {
            if self.rust().fixtures.is_none() {
                let _ = atlas_core::crash::discard(r);
            }
        }
        self.drop_pending(index);
    }

    /// Remove one pending report from the list the UI shows.
    fn drop_pending(self: Pin<&mut Self>, index: i32) {
        let mut this = self;
        let has_server = *this.crash_has_server();
        let mut list = this.rust().pending.clone();
        if (index as usize) < list.len() {
            list.remove(index as usize);
        }
        this.as_mut().reports_loaded(list, has_server);
    }
}

fn fixture_reports(dir: &std::path::Path, name: &str) -> Vec<Report> {
    config::read_fixture(dir, name)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}
