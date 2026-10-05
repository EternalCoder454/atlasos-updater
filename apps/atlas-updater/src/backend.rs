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
        /// fwupd is on this system: the Firmware section shows (no section
        /// and no error when it is not).
        #[qproperty(bool, firmware_available, cxx_name = "firmwareAvailable")]
        /// The listing as JSON: `{updates, pending, note}` (firmware::View).
        #[qproperty(QString, firmware_json, cxx_name = "firmwareJson")]
        #[qproperty(bool, firmware_busy, cxx_name = "firmwareBusy")]
        /// "checkFirmware" or "installFirmware" while `firmwareBusy`, else "".
        #[qproperty(QString, firmware_op, cxx_name = "firmwareOp")]
        /// The device being installed (its id in `firmwareJson`), else "".
        #[qproperty(QString, firmware_device, cxx_name = "firmwareDevice")]
        /// fwupd's status word, "Writing", "Verifying"...
        #[qproperty(QString, firmware_status, cxx_name = "firmwareStatus")]
        /// 0 to 100, or -1 when fwupd gave none.
        #[qproperty(i32, firmware_percent, cxx_name = "firmwarePercent")]
        /// What the device asks of the user ("Unplug the device..."), or "".
        #[qproperty(QString, firmware_request, cxx_name = "firmwareRequest")]
        #[qproperty(QString, firmware_error, cxx_name = "firmwareError")]
        /// An install finished that finishes at the next restart.
        #[qproperty(bool, firmware_restart, cxx_name = "firmwareRestart")]
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

        /// A restart did not happen; the Updates page shows why.
        #[qsignal]
        #[cxx_name = "restartProblem"]
        fn restart_problem(self: Pin<&mut Backend>, text: QString);

        /// Reads the settings and the first status.
        #[qinvokable]
        fn start(self: Pin<&mut Backend>);
        /// The settings file changed (the tray cleared a scheduled restart,
        /// a background round ran): show what it says now.
        #[qinvokable]
        #[cxx_name = "reloadSettings"]
        fn reload_settings(self: Pin<&mut Backend>);

        /// Silent status read (inotify hit, window opened).
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
        /// The "Download app updates in the background" switch.
        #[qinvokable]
        #[cxx_name = "enableBackgroundApps"]
        fn enable_background_apps(self: Pin<&mut Backend>, on: bool);

        /// List firmware updates again (fwupd's local state, no download).
        #[qinvokable]
        #[cxx_name = "checkFirmware"]
        fn check_firmware(self: Pin<&mut Backend>);
        /// Install the update shown for `device_id` at `version` with the
        /// shown `checksum` (refused if the release changed since).
        #[qinvokable]
        #[cxx_name = "installFirmware"]
        fn install_firmware(
            self: Pin<&mut Backend>,
            device_id: &QString,
            version: &QString,
            checksum: &QString,
        );

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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use atlas_framework_system::bootc::{Channel, Status};
use atlas_framework_system::crash::Report;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::json;

use crate::config::{self, Config};
use crate::errors::{self, OpError};
use crate::notes::{self, Notes};
use crate::ops::{self, Op};
use crate::schedule;
use crate::view::{self, View};
use crate::worker::{ROUND_AT, ROUND_ERROR};
use crate::{apphistory, apps, changelog, crash, firmware, lock, notify, rc, restart, tray};
use atlas_updater_base::fwupd;

const RC_RESTART: &str = rc::RESTART;
const RC_CHECKED: &str = rc::CHECKED;
const RC_APPS: &str = rc::APPS;

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
    firmware_available: bool,
    firmware_json: QString,
    firmware_busy: bool,
    firmware_op: QString,
    firmware_device: QString,
    firmware_status: QString,
    firmware_percent: i32,
    firmware_request: QString,
    firmware_error: QString,
    firmware_restart: bool,
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

    // Not exposed to QML.
    config: Config,
    fixtures: Option<PathBuf>,
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
    /// "Update Apps" arrived while another app operation ran: run it after.
    apps_update_requested: bool,
    /// "Check for updates" was pressed during another app operation.
    apps_check_requested: bool,
    /// The error shown came from a background round (the tray's worker):
    /// it goes when the user acts, or a later round works.
    apps_round_error: bool,
    /// When a background round last changed the apps (`RoundAt`).
    apps_round_at: Option<String>,
    /// Apps a background round held back (`apps::row_key`) → what they ask
    /// for, shown on their rows until they are updated.
    apps_held: std::collections::HashMap<String, apps::HeldApp>,
    /// Devices whose install this session said the computer must shut down.
    firmware_shutdown: std::collections::HashSet<String>,
}

/// This machine's history, newest first (developer fixtures: theirs).
fn read_history(fixtures: Option<&std::path::Path>) -> Vec<atlas_framework_system::history::Entry> {
    match fixtures {
        Some(dir) => config::read_fixture(dir, "history.jsonl")
            .map(|t| {
                let mut v: Vec<atlas_framework_system::history::Entry> = t
                    .lines()
                    .filter_map(|l| serde_json::from_str(l).ok())
                    .collect();
                v.reverse();
                v
            })
            .unwrap_or_default(),
        None => atlas_framework_system::history::read_default().unwrap_or_default(),
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

/// Runs `f` to the end on a throw-away runtime (worker threads only). The
/// IO driver is on: zbus needs it.
fn run_async<T>(f: impl std::future::Future<Output = Result<T, OpError>>) -> Result<T, OpError> {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt.block_on(f),
        Err(e) => {
            eprintln!("atlas-updater: cannot start a runtime: {e}");
            Err(OpError::Message(INTERNAL.into()))
        }
    }
}

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

/// Holds the app operation lock (atlas_updater_base::lock) for the length
/// of a window's own check or update, waiting for a background round to end.
fn take_apps_lock(fixtures: bool) -> Option<lock::Held> {
    if fixtures {
        return None;
    }
    match lock::take(lock::APPS, true) {
        Ok(h) => h,
        Err(e) => {
            // Not worth refusing the user's request over.
            eprintln!("atlas-updater: cannot take the app update lock: {e}");
            None
        }
    }
}

/// Holds the firmware lock for the length of a window's own install.
fn take_firmware_lock(fixtures: bool) -> Option<lock::Held> {
    if fixtures {
        return None;
    }
    match lock::take(lock::FIRMWARE, true) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("atlas-updater: cannot take the firmware lock: {e}");
            None
        }
    }
}

const INTERNAL: &str = "Atlas Updater hit an internal error. Please try again.";

/// Reload calls to the tray still on their way, and whether one failed: the
/// window may quit right after a change (see `atlas_tray_flush`).
static TRAY_CALLS: AtomicUsize = AtomicUsize::new(0);
static TRAY_MISSED: AtomicBool = AtomicBool::new(false);
/// Notifications still being sent.
static NOTIFY_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Called from `main.cpp` as the process ends: lets notifications being sent
/// finish, and if the tray may not have heard about a change yet, tells it
/// now (blocking, bounded by the calls' own timeouts).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_tray_flush() {
    // send_blocking gives up after 10 s.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(12);
    while NOTIFY_CALLS.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if TRAY_CALLS.load(Ordering::SeqCst) == 0 && !TRAY_MISSED.load(Ordering::SeqCst) {
        return;
    }
    if let Err(e) = guarded(tray::reload).unwrap_or_else(|| Err(INTERNAL.into())) {
        eprintln!("atlas-updater: the tray did not hear about the last change: {e}");
    }
}

fn q(s: &str) -> QString {
    QString::from(s)
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
        if let Some(logo) = atlas_framework_core::osrelease::logo_icon() {
            self.as_mut().set_os_logo(QString::from(logo.as_str()));
        }
        {
            let mut r = self.as_mut().rust_mut();
            r.started = true;
            r.config = cfg;
            r.fixtures = fixtures;
        }
        // Fixture mode: `last-checked` (Unix seconds), else never.
        let checked = match &self.rust().fixtures {
            Some(d) => config::read_fixture(d, "last-checked"),
            None => rc::get(RC_CHECKED, "At"),
        };
        if let Some(t) = checked.and_then(|v| v.trim().parse::<i64>().ok()) {
            self.as_mut().set_last_checked(t);
        }
        // Fixture mode takes the schedule and the switch from its files and
        // leaves the user's own settings alone.
        match self.rust().fixtures.clone() {
            Some(d) => {
                let at = config::fixture_schedule(&d).filter(|t| *t > schedule::unix_now());
                self.as_mut().set_scheduled_at(at.unwrap_or(0));
                let auto = config::read_fixture(&d, "apps-auto").is_some();
                self.as_mut().set_apps_auto(auto);
            }
            None => {
                self.as_mut().rust_mut().apps_round_at = rc::get(RC_APPS, ROUND_AT);
                self.as_mut().reload_settings();
                // The tray keeps the schedule and the notifications: make sure
                // it runs (D-Bus starts it if the user quit it).
                if !spawn_named("atlas-tray", || {
                    if let Err(e) = tray::reload() {
                        eprintln!("atlas-updater: {e}");
                    }
                }) {
                    eprintln!("atlas-updater: could not start a thread to wake the tray");
                }
            }
        }
        // Crash reports are opt-in; the tray collects them, the window lists them.
        let on = match &self.rust().fixtures {
            Some(d) => config::read_fixture(d, "crash-enabled").is_some(),
            None => atlas_framework_system::crash::Settings::load().enabled,
        };
        self.as_mut().set_crash_enabled(on);
        let fix = self.rust().fixtures.is_some();
        self.as_mut().set_fixtures_active(fix);
        self.refresh_status();
    }

    /// What the settings file says now about the scheduled restart, the
    /// background app switch and the last background round.
    pub fn reload_settings(mut self: Pin<&mut Self>) {
        if self.rust().fixtures.is_some() {
            return;
        }
        // A time that passed is not shown: the tray drops it (or is
        // restarting the computer right now).
        let at = rc::scheduled_at()
            .filter(|t| *t > schedule::unix_now())
            .unwrap_or(0);
        if *self.scheduled_at() != at {
            self.as_mut().set_scheduled_at(at);
        }
        let auto = rc::apps_automatic();
        if *self.apps_auto() != auto {
            self.as_mut().set_apps_auto(auto);
        }
        let round_error = rc::get(RC_APPS, ROUND_ERROR).unwrap_or_default();
        if round_error.is_empty() {
            if self.rust().apps_round_error {
                self.as_mut().rust_mut().apps_round_error = false;
                self.as_mut().set_apps_error(QString::default());
            }
        } else if !*self.apps_busy()
            && (self.rust().apps_round_error || self.apps_error().is_empty())
            && self.apps_error().to_string() != round_error
        {
            // An error from something the user did stays until they act.
            self.as_mut().set_apps_error(q(&round_error));
            self.as_mut().rust_mut().apps_round_error = true;
        }
        let round_at = rc::get(RC_APPS, ROUND_AT);
        if round_at != self.rust().apps_round_at {
            self.as_mut().rust_mut().apps_round_at = round_at;
            // A background round changed the apps: list them again quietly.
            if self.rust().window_open {
                self.as_mut().load_history();
                if *self.apps_busy() {
                    self.as_mut().rust_mut().apps_check_requested = true;
                } else {
                    self.as_mut().start_check(false);
                }
            }
        }
    }

    /// Tells the tray about a setting the window changed. `what` names it
    /// in the error, should the tray not answer.
    fn tell_tray(self: Pin<&mut Self>, what: &'static str) {
        if self.rust().fixtures.is_some() {
            return;
        }
        let qt = self.qt_thread();
        TRAY_CALLS.fetch_add(1, Ordering::SeqCst);
        let started = spawn_named("atlas-tray", move || {
            let res = guarded(tray::reload).unwrap_or_else(|| Err(INTERNAL.into()));
            // The tray reads the whole file: a call that worked covers
            // any earlier one that did not.
            TRAY_MISSED.store(res.is_err(), Ordering::SeqCst);
            TRAY_CALLS.fetch_sub(1, Ordering::SeqCst);
            if let Err(e) = res {
                eprintln!("atlas-updater: {e}");
                let _ = qt.queue(move |mut obj| {
                    obj.as_mut().set_info_text(q(&format!(
                        "The {what} is saved, but Atlas Updater's background service did not answer ({e}). It takes effect when the service starts again, at the latest after you log in."
                    )));
                });
            }
        });
        if !started {
            eprintln!("atlas-updater: could not start a thread to tell the tray");
            TRAY_CALLS.fetch_sub(1, Ordering::SeqCst);
            TRAY_MISSED.store(true, Ordering::SeqCst);
        }
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
    /// Before anything was loaded, the status is read again: with the failed
    /// first read's error gone and nothing running, the Updates page would
    /// say "Reading the system state" forever. A failure shows it again.
    pub fn dismiss_messages(mut self: Pin<&mut Self>) {
        self.as_mut().set_error("", QString::default());
        self.as_mut().set_info_text(QString::default());
        if !*self.loaded() {
            self.refresh_status();
        }
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
    fn show_progress(mut self: Pin<&mut Self>, p: Option<atlas_update_engine::progress::Progress>) {
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
        // Notes follow the version, but only while a window is open.
        if self.rust().window_open {
            self.as_mut().load_notes();
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
                // One app operation at a time, the tray's rounds included.
                let _held = take_apps_lock(fixtures.is_some());
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
                    rc::set(RC_APPS, atlas_updater_base::worker::APPS_NOTIFIED, None);
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
                let _held = take_apps_lock(fixtures.is_some());
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
                        let left = rows.clone().unwrap_or_default();
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
        self.as_mut().forget_round_error();
        self.as_mut().set_apps_error(QString::default());
    }

    /// An error from something the user did: it stays until they act again.
    fn user_error(mut self: Pin<&mut Self>, text: QString) {
        self.as_mut().forget_round_error();
        self.as_mut().set_apps_error(text);
    }

    /// A background round's error was replaced or dismissed: the settings
    /// file stops offering it to the next window.
    fn forget_round_error(mut self: Pin<&mut Self>) {
        if std::mem::take(&mut self.as_mut().rust_mut().apps_round_error)
            && self.rust().fixtures.is_none()
        {
            rc::set(RC_APPS, ROUND_ERROR, None);
        }
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
        }
        self.as_mut().set_apps_auto(on);
        // The tray runs the rounds: the first one a minute after "on".
        self.tell_tray("app update setting");
    }

    /// An update the user started from a notice left apps out: say which,
    /// and don't say it again for the same set.
    fn held_notice(self: Pin<&mut Self>, rows: &[apps::Row], held: &[apps::HeldApp], failed: bool) {
        if self.rust().fixtures.is_some() {
            return;
        }
        let key = apps::notice_key(rows, held, failed);
        // Not "on its own": the user started it. The rows show it too, so
        // the notification has no buttons.
        let n = notify::Note {
            event: "appUpdatesReady",
            title: "App updates ready".into(),
            text: notify::escape(&apps::ready_text(rows, held, failed, false)),
            icon: String::new(),
            actions: Vec::new(),
            urgency: None,
            persistent: false,
        };
        // Counted: the window may quit right after (see atlas_tray_flush).
        NOTIFY_CALLS.fetch_add(1, Ordering::SeqCst);
        let started = spawn_named("atlas-notify", move || {
            // guarded: the count must come down even if the send panics.
            match guarded(|| atlas_updater_base::notifier().send_blocking(&n))
                .unwrap_or_else(|| Err(INTERNAL.into()))
            {
                // Told: not again for the same set.
                Ok(()) => rc::set(
                    RC_APPS,
                    atlas_updater_base::worker::APPS_NOTIFIED,
                    Some(&key),
                ),
                Err(e) => eprintln!("atlas-updater: cannot show a notification: {e}"),
            }
            NOTIFY_CALLS.fetch_sub(1, Ordering::SeqCst);
        });
        if !started {
            eprintln!("atlas-updater: could not start a thread to show a notification");
            NOTIFY_CALLS.fetch_sub(1, Ordering::SeqCst);
        }
    }

    // ---- firmware ----

    pub fn check_firmware(mut self: Pin<&mut Self>) {
        if *self.firmware_busy() {
            return;
        }
        self.as_mut().set_firmware_busy(true);
        self.as_mut().set_firmware_op(q("checkFirmware"));
        self.as_mut().set_firmware_error(QString::default());
        let fixtures = self.rust().fixtures.clone();
        let qt = self.qt_thread();
        if !spawn_named("atlas-firmware", move || {
            if fixtures.is_some() && config::fixture_hold("checkFirmware") {
                config::hold_forever();
            }
            let res = guarded(|| match fixtures.as_deref() {
                Some(d) => firmware::fixture_listing(d).map_err(OpError::Message),
                None => run_async(firmware::list()),
            })
            .unwrap_or_else(|| Err(OpError::Message(INTERNAL.into())));
            let _ = qt.queue(move |mut obj| obj.as_mut().firmware_listed(res, None));
        }) {
            self.firmware_listed(
                Err(OpError::Message("could not start a worker thread".into())),
                None,
            );
        }
    }

    /// A listing arrived (`install_error`: an install ended with it).
    fn firmware_listed(
        mut self: Pin<&mut Self>,
        res: Result<Option<fwupd::Listing>, OpError>,
        install_error: Option<String>,
    ) {
        self.as_mut().firmware_idle();
        match res {
            Ok(None) => {
                // After a failed install keep the page, so the error shows.
                if install_error.is_none() {
                    self.as_mut().set_firmware_available(false);
                    self.as_mut().set_firmware_json(QString::default());
                }
            }
            Ok(Some(l)) => {
                self.as_mut().set_firmware_available(true);
                let v = firmware::view(&l, &self.rust().firmware_shutdown);
                let keep: std::collections::HashSet<&str> =
                    l.pending.iter().map(|p| p.device_id.as_str()).collect();
                self.as_mut()
                    .rust_mut()
                    .firmware_shutdown
                    .retain(|d| keep.contains(d.as_str()));
                let text = serde_json::to_string(&v)
                    .unwrap_or_else(|_| r#"{"updates":[],"pending":[],"note":""}"#.into());
                self.as_mut().set_firmware_json(q(&text));
                // The restart row follows fwupd's state, not this session's.
                let restart = v.pending.iter().any(|p| p.state == "reboot");
                self.as_mut().set_firmware_restart(restart);
            }
            Err(e) => {
                // The page keeps the old list and says why it is old.
                let text = match e {
                    OpError::Message(m) => format!("Could not check firmware updates: {m}"),
                    OpError::Cancelled => "Could not check firmware updates.".to_string(),
                };
                self.as_mut().set_firmware_available(true);
                self.as_mut().set_firmware_error(q(&text));
            }
        }
        if let Some(e) = install_error {
            self.as_mut().set_firmware_error(q(&e));
        }
    }

    fn firmware_idle(mut self: Pin<&mut Self>) {
        self.as_mut().set_firmware_busy(false);
        self.as_mut().set_firmware_op(QString::default());
        self.as_mut().set_firmware_device(QString::default());
        self.as_mut().set_firmware_status(QString::default());
        self.as_mut().set_firmware_percent(-1);
        self.as_mut().set_firmware_request(QString::default());
    }

    /// The row's Install button, after the user confirmed.
    pub fn install_firmware(
        mut self: Pin<&mut Self>,
        device_id: &QString,
        version: &QString,
        checksum: &QString,
    ) {
        if *self.firmware_busy() {
            return;
        }
        let device_id = device_id.to_string();
        let version = version.to_string();
        let checksum = checksum.to_string();
        let fixtures = self.rust().fixtures.clone();
        self.as_mut().set_firmware_busy(true);
        self.as_mut().set_firmware_op(q("installFirmware"));
        self.as_mut().set_firmware_device(q(&device_id));
        self.as_mut().set_firmware_error(QString::default());
        self.as_mut().set_firmware_percent(-1);
        self.as_mut().set_firmware_request(QString::default());
        self.as_mut().set_firmware_status(q("Starting"));
        let qt = self.qt_thread();
        let qt_fail = qt.clone();
        if !spawn_named("atlas-firmware-install", move || {
            let qt_progress = qt.clone();
            if fixtures.is_some() {
                if config::fixture_hold("installFirmware") {
                    config::hold_forever();
                }
                let _ = qt.queue(|mut obj| {
                    obj.as_mut().firmware_idle();
                    obj.as_mut()
                        .set_info_text(q("Developer fixtures: firmware install skipped."));
                });
                return;
            }
            let dev = device_id.clone();
            let res = guarded(|| {
                // One firmware operation at a time.
                let _held = take_firmware_lock(false);
                let mut last_status = "";
                let mut request: Option<String> = None;
                let r = run_async(firmware::install(&device_id, &version, &checksum, |p| {
                    if p.request.is_some() {
                        request = p.request.clone();
                    } else if p.status != last_status {
                        request = None;
                    }
                    last_status = p.status;
                    let req = request.clone().unwrap_or_default();
                    let _ = qt_progress.queue(move |mut obj| {
                        obj.as_mut().set_firmware_status(q(p.status));
                        obj.as_mut()
                            .set_firmware_percent(p.percent.map_or(-1, i32::from));
                        obj.as_mut().set_firmware_request(q(&req));
                    });
                }));
                // Whatever happened, list again: fwupd knows the state.
                let listing = run_async(firmware::list());
                (r, listing)
            })
            .unwrap_or_else(|| {
                (
                    Err(OpError::Message(INTERNAL.into())),
                    Err(OpError::Message(INTERNAL.into())),
                )
            });
            let _ = qt.queue(move |mut obj| {
                let (r, listing) = res;
                let mut err = None;
                match r {
                    Ok(done) => {
                        if done.needs_shutdown {
                            obj.as_mut().rust_mut().firmware_shutdown.insert(dev);
                        }
                        if done.needs_reboot {
                            obj.as_mut().set_firmware_restart(true);
                        }
                    }
                    Err(OpError::Cancelled) => {
                        obj.as_mut().set_info_text(q(
                            "The firmware update was cancelled. Nothing was changed.",
                        ));
                    }
                    Err(OpError::Message(m)) => {
                        err = Some(format!("Could not install the firmware update: {m}"));
                    }
                }
                obj.as_mut().firmware_listed(listing, err);
            });
        }) {
            let _ = qt_fail.queue(|mut obj| {
                obj.as_mut().firmware_idle();
                obj.as_mut().set_firmware_error(q(
                    "Could not install the firmware update: could not start a worker thread",
                ));
            });
        }
    }

    // ---- restart ----

    /// "Restart Now" in the window. (A scheduled restart is the tray's.)
    pub fn restart_now(mut self: Pin<&mut Self>) {
        if *self.restarting() {
            return;
        }
        // Never restart in the middle of a firmware flash.
        if *self.firmware_busy() && self.firmware_op().to_string() == "installFirmware" {
            self.as_mut().set_info_text(q(
                "Wait for the firmware update to finish before restarting.",
            ));
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
        let clear = |mut obj: Pin<&mut qobject::Backend>| obj.as_mut().set_restarting(false);
        let fail = move |mut obj: Pin<&mut qobject::Backend>, e: String| {
            // The Updates page shows the error.
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
        if self.as_mut().save_schedule(Some(at)) {
            self.as_mut().set_scheduled_at(at);
            self.tell_tray("restart time");
        }
    }

    pub fn cancel_restart(mut self: Pin<&mut Self>) {
        if self.as_mut().save_schedule(None) {
            self.as_mut().set_scheduled_at(0);
            self.tell_tray("canceled restart");
        }
    }

    /// Saves (or clears) the scheduled restart for the tray, which keeps
    /// it. Fixture mode never touches the user's real settings file.
    fn save_schedule(mut self: Pin<&mut Self>, at: Option<i64>) -> bool {
        if self.rust().fixtures.is_some() {
            return true;
        }
        let value = at.map(|t| t.to_string());
        match rc::try_set(RC_RESTART, "ScheduledAt", value.as_deref()) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("atlas-updater: could not save the restart time: {e}");
                self.as_mut().set_error(
                    "restart",
                    q(&format!("Could not save the restart time: {e}")),
                );
                false
            }
        }
    }

    // ---- crash reports (opt-in; see crash.rs) ----

    pub fn enable_crash_reports(mut self: Pin<&mut Self>, on: bool) {
        if self.rust().fixtures.is_none()
            && let Err(e) = (atlas_framework_system::crash::Settings { enabled: on }).save()
        {
            // Not saved: collection would still see "off", so do not claim "on".
            self.as_mut().set_error(
                "crashSetting",
                q(&format!("Could not save the crash report setting: {e}")),
            );
            return;
        }
        self.as_mut().set_crash_enabled(on);
        // The tray watches for crashes only while this is on.
        self.as_mut().tell_tray("crash report setting");
        if on {
            self.as_mut().collect_reports();
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
                    None => atlas_framework_system::crash::pending(),
                };
                let has_server = match &fixtures {
                    Some(d) => config::read_fixture(d, "crash-server").is_some(),
                    None => atlas_framework_system::crash::Endpoint::load().is_some(),
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
                    None => atlas_framework_system::crash::sent(),
                };
                sent.reverse(); // newest first
                let views: Vec<_> = sent.iter().map(|r| crash::view(r, false)).collect();
                serde_json::Value::Array(views).to_string()
            })
            .unwrap_or_else(|| "[]".into());
            let _ = qt.queue(move |mut obj| obj.as_mut().set_sent_json(q(&text)));
        });
    }

    /// Reports that came in while crash reports were off are looked for
    /// when they are turned on, then listed. The tray collects the rest.
    fn collect_reports(self: Pin<&mut Self>) {
        if !*self.crash_enabled() {
            return;
        }
        if self.rust().fixtures.is_some() {
            self.load_reports();
            return;
        }
        let qt = self.qt_thread();
        let started = spawn_named("atlas-collect", move || {
            let _ = guarded(atlas_updater_base::crash::collect);
            let _ = qt.queue(move |mut obj| {
                // Switched off while collecting: no list.
                if *obj.crash_enabled() {
                    obj.as_mut().load_reports();
                }
            });
        });
        if !started {
            eprintln!("atlas-updater: could not start a thread to collect crash reports");
        }
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
                    atlas_framework_system::crash::send(&r)
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
            && let Err(e) = atlas_framework_system::crash::discard(r)
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
    fn run_async_has_the_io_driver_zbus_needs() {
        // Without the IO driver this panics; with it, the connect fails
        // with an error.
        let r = run_async(async {
            let b = zbus::connection::Builder::address("unix:path=/nonexistent-atlas-test/bus")
                .map_err(|e| OpError::Message(e.to_string()))?;
            b.build()
                .await
                .map(|_| ())
                .map_err(|e| OpError::Message(e.to_string()))
        });
        assert!(matches!(r, Err(OpError::Message(m)) if !m.is_empty()));
    }
}
