//! atlas-updater-tray: the part of Atlas Updater that runs all session. It
//! keeps the schedule (status poll, app rounds, the scheduled restart),
//! sends the notifications and shows the panel icon. No Qt and no
//! libflatpak, so it stays a few MB: the window is `atlas-updater` (started
//! on demand, gone when closed) and app rounds run in short-lived
//! `atlas-updater --worker` processes.

mod icons;
mod sni;
mod timefmt;
mod watch;

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use atlas_framework_system::bootc::Status;
use atlas_updater_base::fwupd::{self, FirmwareUpdate};
use atlas_updater_base::notify::{self, Note, Notifier, Urgency};
use atlas_updater_base::ops::{self, Op};
use atlas_updater_base::schedule::{self, Event, Schedule};
use atlas_updater_base::view::{self, View};
use atlas_updater_base::worker::{self, Outcome};
use atlas_updater_base::{crash, lock, rc, restart, tray};
use tokio::sync::mpsc;
use tokio::time::Instant;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::message::Type as MsgType;
use zbus::{MatchRule, MessageStream};

use sni::{Item, Look, Menu};

pub const APP_ICON: &str = crash::APP_ID;
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// The first app round after background updates are switched on.
const APPS_SOON: Duration = Duration::from_secs(60);
/// The next app round after one that had to wait.
const APPS_RETRY: Duration = Duration::from_secs(30 * 60);
/// The next app round after one that found another app operation running.
const APPS_BUSY_RETRY: Duration = Duration::from_secs(5 * 60);
/// /run/ostree changes come in bursts while ostree stages: wait for quiet.
const OSTREE_SETTLE: Duration = Duration::from_millis(1500);
const CRASH_SETTLE: Duration = Duration::from_millis(3000);
/// Most a worker may print: one JSON line.
/// After the change watch broke: set it up again.
const WATCH_RETRY: Duration = Duration::from_secs(60);
const WORKER_OUTPUT_LIMIT: u64 = 64 * 1024;

/// When to try again after `failures` failed background rounds in a row:
/// 30 minutes, doubling each time, at most 6 hours.
fn apps_retry_after(failures: u32) -> Duration {
    let doubled = APPS_RETRY.saturating_mul(1 << failures.clamp(1, 5).saturating_sub(1));
    doubled.min(Duration::from_secs(6 * 3600))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// The scheduled app round.
    Round,
    /// "Update Apps" from a notification.
    Update,
}

#[derive(Debug)]
pub enum Msg {
    Sched(Event),
    Status(Box<Result<Status, String>>),
    Activate,
    Token(String),
    Menu(i32),
    Reload,
    RestartEnded(Result<(), String>, Option<i64>),
    WorkerDone(Job, Option<Outcome>),
    /// A firmware listing ended: `None` when it failed (logged).
    Firmware(Option<FwFound>),
    Collected(Option<(String, String)>),
}

/// What a notification we sent is about, for its actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Staged,
    Apps,
    Firmware,
    RestartSoon,
    RestartFailed,
    Crash,
}

struct Tray {
    /// A Reload is queued; more calls until it is handled add nothing.
    reload_pending: Arc<AtomicBool>,
    conn: zbus::Connection,
    notifier: Notifier,
    tx: mpsc::UnboundedSender<Msg>,
    schedule: Schedule,
    icons: icons::Icons,
    view: View,
    /// The scheduled restart (Unix seconds), 0 for none.
    scheduled_at: i64,
    apps_auto: bool,
    crash_on: bool,
    restarting: bool,
    /// The last restart failed: urgent icon until the next try.
    restart_failed: bool,
    /// The restart warning is up (urgent icon) …
    soon_shown: bool,
    /// … as this notification, if the server showed it.
    soon_id: Option<u32>,
    /// The notification server that shows the restart warning.
    soon_owner: Option<String>,
    notes: HashMap<u32, Kind>,
    /// Activation tokens the notification server sent, by notification.
    tokens: HashMap<u32, String>,
    /// The notification server's unique name: only its signals count.
    notify_owner: Option<String>,
    /// From ProvideXdgActivationToken, for the next window opened.
    item_token: Option<String>,
    status_inflight: bool,
    status_again: bool,
    last_status_error: Option<String>,
    /// A status was read at least once: `view` is real.
    status_known: bool,
    /// The restart warning could not be shown: the restart must not happen.
    warn_missing: bool,
    /// Watching for changes failed: try again then.
    watch_retry: Option<Instant>,
    worker: Option<Job>,
    round_pending: bool,
    update_pending: bool,
    apps_failures: u32,
    /// A firmware listing is running.
    fw_inflight: bool,
    /// Firmware updates are waiting (from the last listing).
    fw_waiting: bool,
    /// The scheduled restart is waiting for a firmware update to finish.
    fw_postponed: bool,
    collecting: bool,
    collect_again: bool,
    look: Look,
    menu_revision: u32,
}

/// A sibling program in this one's directory (`/usr/bin`).
fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(format!("/usr/bin/{name}")))
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name(name.into())
        .stack_size(256 * 1024)
        .spawn(f)
        .is_ok()
}

impl Tray {
    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }

    // ---- the panel ----

    fn look(&mut self) -> Look {
        let staged = self.view.staged.present;
        let firmware = self.fw_waiting;
        if !staged {
            self.restart_failed = false;
        }
        let at = self.scheduled_at;
        let urgent = staged && ((self.soon_shown && at > 0) || self.restart_failed);
        let mut tip = if staged {
            format!(
                "Update {} is ready. Restart to install it.",
                self.view.staged.version
            )
        } else if firmware {
            "Firmware updates are available.".to_string()
        } else {
            "Your system is up to date.".to_string()
        };
        if at > 0 {
            tip.push_str(&format!("\nRestart scheduled for {}", timefmt::short(at)));
            if self.fw_postponed {
                tip.push_str("\nWaiting for a firmware update to finish first.");
            }
        }
        Look {
            status: if staged || firmware {
                "NeedsAttention"
            } else {
                "Passive"
            },
            icon: self.icons.base.clone(),
            attention: if urgent {
                self.icons.urgent.clone()
            } else {
                self.icons.ready.clone()
            },
            tip,
            restart: staged,
            cancel: at > 0,
        }
    }

    /// Shows the current state in the panel; signals only what changed.
    async fn update_item(&mut self) {
        let new = self.look();
        let old = std::mem::replace(&mut self.look, new.clone());
        if old == new {
            return;
        }
        let server = self.conn.object_server();
        if let Ok(item) = server.interface::<_, Item>(sni::ITEM_PATH).await {
            item.get_mut().await.look = new.clone();
            let e = item.signal_emitter();
            if old.status != new.status {
                let _ = Item::new_status(e, new.status).await;
            }
            if old.icon != new.icon {
                let _ = Item::new_icon(e).await;
            }
            if old.attention != new.attention {
                let _ = Item::new_attention_icon(e).await;
            }
            if old.tip != new.tip {
                let _ = Item::new_tool_tip(e).await;
            }
        }
        if (old.restart, old.cancel) != (new.restart, new.cancel)
            && let Ok(menu) = server.interface::<_, Menu>(sni::MENU_PATH).await
        {
            self.menu_revision += 1;
            let changed = {
                let mut m = menu.get_mut().await;
                m.restart = new.restart;
                m.cancel = new.cancel;
                m.revision = self.menu_revision;
                m.changed()
            };
            let e = menu.signal_emitter();
            let _ = Menu::items_properties_updated(e, changed, Vec::new()).await;
            let _ = Menu::layout_updated(e, self.menu_revision, 0).await;
        }
    }

    async fn register(&self) {
        let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
        let res = self
            .conn
            .call_method(
                Some(WATCHER),
                "/StatusNotifierWatcher",
                Some(WATCHER),
                "RegisterStatusNotifierItem",
                &name,
            )
            .await;
        if let Err(e) = res {
            // No panel yet (or none at all): registered when one appears.
            eprintln!("atlas-updater-tray: no status notifier watcher yet: {e}");
        }
    }

    // ---- the window ----

    async fn window_running(&self) -> bool {
        match zbus::fdo::DBusProxy::new(&self.conn).await {
            Ok(p) => p
                .name_has_owner(tray::WINDOW_BUS_NAME.try_into().expect("a valid bus name"))
                .await
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Opens the window (or raises it: the window is single-instance and
    /// hands a second launch's arguments to the first).
    fn open_window(&self, args: &[&str], token: Option<String>) {
        let mut cmd = Command::new(sibling("atlas-updater"));
        cmd.args(args).stdin(Stdio::null());
        match token {
            Some(t) => cmd.env("XDG_ACTIVATION_TOKEN", t),
            None => cmd.env_remove("XDG_ACTIVATION_TOKEN"),
        };
        match cmd.spawn() {
            Ok(mut child) => {
                // Reaped when it exits; the window may stay open for hours.
                if !spawn_thread("atlas-reap", move || {
                    let _ = child.wait();
                }) {
                    eprintln!("atlas-updater-tray: cannot wait for the window");
                }
            }
            Err(e) => eprintln!("atlas-updater-tray: cannot open the window: {e}"),
        }
    }

    // ---- notifications ----

    async fn notify(&mut self, kind: Kind, n: Note) -> Option<u32> {
        self.try_notify(kind, n).await.ok().flatten()
    }

    /// `Ok(None)`: the user turned this event's popup off. `Err`: it could
    /// not be shown (logged).
    async fn try_notify(&mut self, kind: Kind, n: Note) -> Result<Option<u32>, ()> {
        let sent = match self.notifier.send(&self.conn, &n).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("atlas-updater-tray: cannot show a notification: {e}");
                return Err(());
            }
        };
        let Some(s) = sent else {
            return Ok(None);
        };
        if let Some(owner) = s.server {
            self.notify_owner = Some(owner);
        }
        self.notes.insert(s.id, kind);
        Ok(Some(s.id))
    }

    async fn on_note_signal(&mut self, msg: zbus::Message) {
        let h = msg.header();
        let from = h.sender().map(|s| s.to_string());
        if from.is_none() || from != self.notify_owner {
            return;
        }
        let Some(member) = h.member().map(|m| m.to_string()) else {
            return;
        };
        let body = msg.body();
        match member.as_str() {
            "ActivationToken" => {
                if let Ok((id, token)) = body.deserialize::<(u32, String)>()
                    && self.notes.contains_key(&id)
                {
                    self.tokens.insert(id, token);
                }
            }
            "ActionInvoked" => {
                if let Ok((id, key)) = body.deserialize::<(u32, String)>()
                    && let Some(kind) = self.notes.get(&id).copied()
                {
                    let token = self.tokens.remove(&id);
                    self.on_action(kind, &key, token).await;
                }
            }
            "NotificationClosed" => {
                if let Ok((id, _reason)) = body.deserialize::<(u32, u32)>() {
                    self.notes.remove(&id);
                    self.tokens.remove(&id);
                    if self.soon_id == Some(id) {
                        // Dismissed or closed: drop the urgent icon.
                        self.soon_id = None;
                        self.soon_shown = false;
                        self.update_item().await;
                    }
                }
            }
            _ => {}
        }
    }

    async fn on_action(&mut self, kind: Kind, key: &str, token: Option<String>) {
        match (kind, key) {
            (Kind::RestartFailed, notify::DEFAULT_ACTION) => {
                self.open_window(&["--page", "updates"], token)
            }
            (Kind::Firmware, notify::DEFAULT_ACTION) => {
                self.open_window(&["--page", "updates"], token)
            }
            (_, notify::DEFAULT_ACTION) => self.open_window(&[], token),
            // An old notice can outlive the update it was about.
            (Kind::Staged, "restart") | (Kind::RestartSoon, "restart-now")
                if self.view.staged.present =>
            {
                self.start_restart(None).await
            }
            (Kind::RestartSoon, "cancel-restart") => self.cancel_restart().await,
            (Kind::Apps, "update-apps") => self.update_apps(),
            (Kind::Crash, "review") => self.open_window(&["--page", "reports"], token),
            _ => {}
        }
    }

    // ---- status ----

    fn refresh_status(&mut self) {
        if self.status_inflight {
            self.status_again = true;
            return;
        }
        self.status_inflight = true;
        let tx = self.tx.clone();
        // Debug builds only (ATLAS_UPDATER_FIXTURES): a made-up status, for
        // testing the tray without the system helper.
        let fixtures = atlas_updater_base::config::fixtures_dir();
        if !spawn_thread("atlas-status", move || {
            let res = std::panic::catch_unwind(|| {
                ops::run(&Op::Status, fixtures.as_deref(), Box::new(|_| {})).map_err(|e| match e {
                    atlas_updater_base::errors::OpError::Cancelled => "not allowed".to_string(),
                    atlas_updater_base::errors::OpError::Message(m) => m,
                })
            })
            .unwrap_or_else(|_| Err("internal error".into()));
            let _ = tx.send(Msg::Status(Box::new(res)));
        }) {
            self.status_inflight = false;
        }
    }

    async fn on_status(&mut self, res: Result<Status, String>) {
        self.status_inflight = false;
        match res {
            Ok(st) => {
                self.last_status_error = None;
                self.apply_status(&st).await;
            }
            Err(e) => {
                // Logged once, not every 6 hours.
                if self.last_status_error.as_deref() != Some(e.as_str()) {
                    eprintln!("atlas-updater-tray: cannot read the update state: {e}");
                    self.last_status_error = Some(e);
                }
            }
        }
        if std::mem::take(&mut self.status_again) {
            self.refresh_status();
        }
    }

    async fn apply_status(&mut self, st: &Status) {
        self.view = view::from_status(st);
        self.status_known = true;
        let staged = self.view.staged.clone();
        // Tell the user once per staged image, even across restarts of the
        // tray. While the window is open, the user is looking at it.
        if staged.present
            && !staged.digest.is_empty()
            && rc::get(rc::NOTIFIED, "StagedDigest").as_deref() != Some(staged.digest.as_str())
        {
            rc::set(rc::NOTIFIED, "StagedDigest", Some(&staged.digest));
            if !self.window_running().await {
                let n = Note {
                    event: "updateStaged",
                    title: "Update ready".into(),
                    text: format!(
                        "AtlasOS {} is downloaded. Restart to finish installing it.",
                        notify::escape(&staged.version)
                    ),
                    icon: String::new(),
                    actions: vec![
                        ("restart", "Restart to Update".into()),
                        (notify::DEFAULT_ACTION, "Open Atlas Updater".into()),
                    ],
                    urgency: None,
                    persistent: false,
                };
                self.notify(Kind::Staged, n).await;
            }
        }
        if !staged.present && !self.view.restart_needed && self.scheduled_at != 0 {
            // The staged update is gone (rebooted or cleaned): the plan is moot.
            self.cancel_restart().await;
        }
        self.update_item().await;
    }

    // ---- the scheduled restart ----

    /// Clears the plan, saved and in memory, and its warning.
    async fn clear_schedule(&mut self) {
        rc::set(rc::RESTART, "ScheduledAt", None);
        self.scheduled_at = 0;
        self.schedule.set_restart(None);
        self.soon_owner = None;
        if let Some(id) = self.soon_id.take() {
            let _ = notify::close(&self.conn, id).await;
        }
        self.soon_shown = false;
        self.warn_missing = false;
        self.fw_postponed = false;
        self.update_item().await;
    }

    async fn cancel_restart(&mut self) {
        self.restart_failed = false;
        self.clear_schedule().await;
    }

    async fn on_event(&mut self, ev: Event) {
        // An event can sit in the queue while the user cancels or picks
        // another time: act only if it is still the scheduled one.
        let current = |t: i64| self.scheduled_at == t;
        match ev {
            Event::Poll => self.refresh_status(),
            Event::Apps => self.apps_round(),
            Event::RestartWarning(t) => {
                if current(t) && self.still_saved(t).await {
                    self.restart_soon().await;
                }
            }
            Event::RestartDue(t) => {
                if !current(t) || !self.still_saved(t).await {
                    return;
                }
                rc::set(rc::RESTART, "ScheduledAt", None);
                if self.restarting {
                    self.clear_schedule().await;
                } else if self.warn_missing {
                    // Never restart without the user having been warned.
                    self.clear_schedule().await;
                    self.restart_problem(
                        "The scheduled restart did not happen because its warning could not be shown. The update is still waiting. Restart when you are ready.",
                    )
                    .await;
                } else if self.view.restart_needed && firmware_flashing() {
                    // Never restart in the middle of a firmware flash: look
                    // again in a minute (the warning was already given).
                    let again = schedule::unix_now() + FW_RESTART_RETRY_SECS;
                    eprintln!(
                        "atlas-updater-tray: restart postponed: a firmware update is running"
                    );
                    rc::set(rc::RESTART, "ScheduledAt", Some(&again.to_string()));
                    self.scheduled_at = again;
                    self.fw_postponed = true;
                    self.schedule.set_restart(Some(again));
                    self.update_item().await;
                } else if self.view.restart_needed {
                    // scheduled_at stays set while the restart runs.
                    self.fw_postponed = false;
                    self.start_restart(Some(t)).await;
                } else if !self.status_known || self.last_status_error.is_some() {
                    self.clear_schedule().await;
                    self.restart_problem(
                        "The scheduled restart did not happen because Atlas Updater could not check the update. Open Atlas Updater to see what is waiting.",
                    )
                    .await;
                } else {
                    // Nothing waits any more (the status said so).
                    self.clear_schedule().await;
                }
            }
            Event::RestartMissed(t) => {
                if !current(t) || !self.still_saved(t).await {
                    return;
                }
                self.clear_schedule().await;
                self.restart_problem(
                    "The scheduled restart did not happen because the computer was asleep at that time. The update is still waiting. Restart when you are ready.",
                )
                .await;
            }
        }
    }

    async fn restart_soon(&mut self) {
        // The real time left: a saved time found at login can be much closer.
        let left = self.scheduled_at - schedule::unix_now();
        let minutes = ((left + 59) / 60).max(1);
        let title = if minutes == 1 {
            "Restarting in 1 minute".to_string()
        } else {
            format!("Restarting in {minutes} minutes")
        };
        // The only warning before an automatic restart: it stays until the
        // user acts, and closes itself when the restart is canceled.
        let n = Note {
            event: "restartSoon",
            title,
            text: "Your computer will restart soon to finish updating. Save your work.".into(),
            icon: String::new(),
            actions: vec![
                ("restart-now", "Restart Now".into()),
                ("cancel-restart", "Cancel Restart".into()),
            ],
            urgency: Some(Urgency::Critical),
            persistent: true,
        };
        self.soon_shown = true;
        match self.try_notify(Kind::RestartSoon, n).await {
            Ok(id) => {
                self.soon_id = id;
                self.soon_owner = id.and(self.notify_owner.clone());
                self.warn_missing = false;
            }
            // Also a server that did not answer in 10 s: it may still show
            // the warning later, untracked. Its buttons then do nothing
            // (only tracked ids count), and the restart does not happen
            // unwarned (warn_missing).
            Err(()) => {
                self.soon_id = None;
                self.soon_owner = None;
                self.warn_missing = true;
            }
        }
        self.update_item().await;
    }

    /// Whether `t` is still the time saved in the settings. The window's
    /// Reload may never have arrived (it quit right after the change): the
    /// file decides, never only what this process remembers.
    async fn still_saved(&mut self, t: i64) -> bool {
        if rc::scheduled_at() == Some(t) {
            return true;
        }
        self.reload().await;
        false
    }

    async fn restart_problem(&mut self, text: &str) {
        self.restart_failed = true;
        self.update_item().await;
        let n = Note {
            event: "restartFailed",
            title: "Restart did not happen".into(),
            text: notify::escape(text),
            icon: String::new(),
            actions: vec![(notify::DEFAULT_ACTION, "Open Atlas Updater".into())],
            urgency: Some(Urgency::High),
            persistent: false,
        };
        self.notify(Kind::RestartFailed, n).await;
    }

    /// `scheduled`: the time of the scheduled restart that fired, if this is
    /// one: only that restart's end clears the schedule.
    async fn start_restart(&mut self, scheduled: Option<i64>) {
        if self.restarting {
            return;
        }
        if scheduled.is_none() && firmware_flashing() {
            self.restart_problem(
                "Wait for the firmware update to finish before restarting. The update is still waiting.",
            )
            .await;
            return;
        }
        self.restarting = true;
        self.restart_failed = false;
        self.update_item().await;
        let tx = self.tx.clone();
        if !spawn_thread("atlas-restart", move || {
            let res = std::panic::catch_unwind(restart::logout_and_reboot)
                .unwrap_or_else(|_| Err("internal error".into()));
            let _ = tx.send(Msg::RestartEnded(res, scheduled));
        }) {
            self.send(Msg::RestartEnded(
                Err("could not start a worker thread".into()),
                scheduled,
            ));
        }
    }

    async fn restart_ended(&mut self, res: Result<(), String>, scheduled: Option<i64>) {
        self.restarting = false;
        if let Some(t) = scheduled
            && self.scheduled_at == t
        {
            self.clear_schedule().await;
        }
        match res {
            // Not reached when the restart happens (the session ends first).
            Ok(()) => {}
            Err(e) if e == restart::CANCELED => eprintln!("atlas-updater-tray: {e}"),
            // Plasma may still act on the request: no notification.
            Err(e) if e == restart::NO_ANSWER => eprintln!("atlas-updater-tray: {e}"),
            Err(e) => {
                eprintln!("atlas-updater-tray: restart failed: {e}");
                let text = format!(
                    "Could not restart the computer: {e}. The update is still waiting. Restart it yourself when you are ready."
                );
                self.restart_problem(&text).await;
            }
        }
        self.update_item().await;
    }

    // ---- settings the window changed ----

    async fn reload(&mut self) {
        let saved = rc::scheduled_at();
        let at = match saved {
            // `>=`: a Due event for this very second may be on its way.
            Some(t) if t >= schedule::unix_now() => t,
            Some(_) => {
                // Passed while nobody was looking: never restart unexpectedly.
                rc::set(rc::RESTART, "ScheduledAt", None);
                0
            }
            None => 0,
        };
        if at != self.scheduled_at {
            if at == 0 {
                self.clear_schedule().await;
            } else {
                self.scheduled_at = at;
                // A new time replaces a failed restart and an old warning.
                self.restart_failed = false;
                self.soon_owner = None;
                if let Some(id) = self.soon_id.take() {
                    let _ = notify::close(&self.conn, id).await;
                }
                self.soon_shown = false;
                self.warn_missing = false;
                self.schedule.set_restart(Some(at));
            }
        }
        let auto = rc::apps_automatic();
        if auto && !self.apps_auto {
            // Don't make the user wait up to 6 hours to see it work.
            self.schedule.apps_in(APPS_SOON);
        }
        self.apps_auto = auto;
        self.set_crash(atlas_framework_system::crash::Settings::load().enabled);
        self.update_item().await;
    }

    // ---- crash reports ----

    fn set_crash(&mut self, on: bool) {
        if on == self.crash_on {
            return;
        }
        self.crash_on = on;
        if on {
            self.collect();
        }
    }

    fn collect(&mut self) {
        if !self.crash_on {
            return;
        }
        if self.collecting {
            self.collect_again = true;
            return;
        }
        self.collecting = true;
        let tx = self.tx.clone();
        if !spawn_thread("atlas-collect", move || {
            let first = std::panic::catch_unwind(crash::collect).unwrap_or(None);
            let _ = tx.send(Msg::Collected(first));
        }) {
            self.collecting = false;
        }
    }

    async fn collected(&mut self, first: Option<(String, String)>) {
        self.collecting = false;
        // Switched off while collecting: no notification.
        if self.crash_on
            && let Some((app, kind)) = first
        {
            let crashed = matches!(kind.as_str(), "panic" | "fatal" | "coredump");
            let text = if crashed {
                format!(
                    "{} closed unexpectedly. Review the crash report?",
                    notify::escape(&app)
                )
            } else {
                "Something went wrong with a system update. Review the report?".to_string()
            };
            let n = Note {
                event: "crashReport",
                title: "Crash report ready".into(),
                text,
                icon: "tools-report-bug".into(),
                actions: vec![("review", "Review".into())],
                urgency: None,
                persistent: false,
            };
            self.notify(Kind::Crash, n).await;
        }
        if std::mem::take(&mut self.collect_again) {
            self.collect();
        }
    }

    // ---- app updates (in workers) ----

    fn apps_round(&mut self) {
        self.firmware_round();
        if self.worker.is_some() {
            self.round_pending = true;
            return;
        }
        self.start_worker(Job::Round);
    }

    fn update_apps(&mut self) {
        if self.worker.is_some() {
            self.update_pending = true;
            return;
        }
        self.start_worker(Job::Update);
    }

    fn start_worker(&mut self, job: Job) {
        let mode = match job {
            Job::Round => worker::APPS_ROUND,
            Job::Update => worker::APPS_UPDATE,
        };
        let mut cmd = Command::new(sibling("atlas-updater"));
        cmd.args(["--worker", mode])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .env_remove("XDG_ACTIVATION_TOKEN");
        // SAFETY: prctl is async-signal-safe; nothing else runs between fork
        // and exec. The worker ends with the tray rather than outlive it.
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("atlas-updater-tray: cannot start the app update worker: {e}");
                self.send(Msg::WorkerDone(job, None));
                self.worker = Some(job);
                return;
            }
        };
        self.worker = Some(job);
        let tx = self.tx.clone();
        let stdout = child.stdout.take();
        if !spawn_thread("atlas-worker", move || {
            // Read apart from the wait: something the worker started may
            // keep the pipe open after it ended.
            let (out_tx, out_rx) = std::sync::mpsc::channel();
            if let Some(s) = stdout {
                let read = spawn_thread("atlas-worker-out", move || {
                    let mut out = Vec::new();
                    let _ = s.take(WORKER_OUTPUT_LIMIT).read_to_end(&mut out);
                    let _ = out_tx.send(out);
                });
                if !read {
                    eprintln!("atlas-updater-tray: cannot read the app update worker's result");
                }
            }
            let status = child.wait();
            let out = out_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_default();
            // Libraries may write other bytes there: keep what parses.
            let outcome = Outcome::parse(&String::from_utf8_lossy(&out));
            if outcome.is_none() {
                eprintln!(
                    "atlas-updater-tray: the app update worker ended without a result ({status:?})"
                );
            }
            let _ = tx.send(Msg::WorkerDone(job, outcome));
        }) {
            // Not followed: it still runs, and ends with us.
            eprintln!("atlas-updater-tray: cannot wait for the app update worker");
            self.send(Msg::WorkerDone(job, None));
        }
    }

    async fn worker_done(&mut self, job: Job, out: Option<Outcome>) {
        self.worker = None;
        match job {
            Job::Round => match &out {
                None => self.apps_failed(),
                Some(o) if o.busy => self.schedule.apps_in(APPS_BUSY_RETRY),
                Some(o) if o.failed => self.apps_failed(),
                Some(o) => {
                    self.apps_failures = 0;
                    if o.waited {
                        self.schedule.apps_in(APPS_RETRY);
                    }
                }
            },
            Job::Update => {}
        }
        let update_failed = job == Job::Update && out.as_ref().is_none_or(|o| o.failed);
        // "Update Apps" from a notification must answer, even when the
        // worker died or said nothing.
        let notice = match out.and_then(|o| o.notice) {
            None if update_failed => Some(worker::Notice {
                text: "Atlas Updater could not update the apps. Open Atlas Updater to try again."
                    .into(),
                key: None,
                can_update: false,
            }),
            n => n,
        };
        if let Some(notice) = notice {
            let mut actions = Vec::new();
            // An app asking for new permissions is reviewed in the window first.
            if notice.can_update {
                actions.push(("update-apps", "Update Apps".to_string()));
            }
            actions.push((notify::DEFAULT_ACTION, "Open Atlas Updater".to_string()));
            let n = Note {
                event: "appUpdatesReady",
                title: if update_failed {
                    "Could not update apps".into()
                } else {
                    "App updates ready".into()
                },
                // App names come from Flatpak metadata: never markup.
                text: notify::escape(&notice.text),
                icon: String::new(),
                actions,
                urgency: None,
                persistent: false,
            };
            // Counted as told only once shown (or turned off by the user): a
            // notice that could not be shown comes again with the next round.
            if self.try_notify(Kind::Apps, n).await.is_ok()
                && let Some(key) = &notice.key
            {
                rc::set(rc::APPS, worker::APPS_NOTIFIED, Some(key));
            }
        }
        if std::mem::take(&mut self.update_pending) {
            self.start_worker(Job::Update);
        } else if std::mem::take(&mut self.round_pending) {
            self.start_worker(Job::Round);
        }
    }

    // ---- firmware (listed with each round, never installed here) ----

    /// Lists firmware in a short task: one system-bus connection that is
    /// dropped when done, so nothing idles between rounds. Without fwupd
    /// there is nothing to ask beyond one bus lookup.
    fn firmware_round(&mut self) {
        if std::mem::replace(&mut self.fw_inflight, true) {
            return;
        }
        let tx = self.tx.clone();
        let task = tokio::spawn(async move {
            tokio::time::timeout(FW_TIME_LIMIT, firmware_list())
                .await
                .unwrap_or_else(|_| {
                    eprintln!("atlas-updater-tray: fwupd took too long; firmware not listed");
                    None
                })
        });
        // Awaited apart, so a panic in the task still ends the round (the
        // in-flight flag is cleared by the message).
        tokio::spawn(async move {
            let found = task.await.unwrap_or_else(|e| {
                eprintln!("atlas-updater-tray: the firmware task failed: {e}");
                None
            });
            let _ = tx.send(Msg::Firmware(found));
        });
    }

    async fn firmware_done(&mut self, found: Option<FwFound>) {
        self.fw_inflight = false;
        // A failed listing changes nothing: the next round tries again.
        let Some(found) = found else { return };
        let waiting = !found.updates.is_empty();
        let saved = rc::get(rc::FIRMWARE, FW_NOTIFIED);
        let window = if found.key.is_empty() || saved.as_deref() == Some(found.key.as_str()) {
            false
        } else {
            self.window_running().await
        };
        match firmware_step(&found.key, saved.as_deref(), window) {
            FwStep::Keep => {}
            FwStep::Clear => rc::set(rc::FIRMWARE, FW_NOTIFIED, None),
            FwStep::Notify => {
                let n = Note {
                    event: "firmwareReady",
                    title: "Firmware updates available".into(),
                    text: notify::escape(&firmware_text(&found.updates)),
                    icon: String::new(),
                    actions: vec![(notify::DEFAULT_ACTION, "Open Atlas Updater".into())],
                    urgency: None,
                    persistent: false,
                };
                // Counted as told only once shown (or turned off by the
                // user): one that could not be shown comes with the next round.
                if self.try_notify(Kind::Firmware, n).await.is_ok() {
                    rc::set(rc::FIRMWARE, FW_NOTIFIED, Some(&found.key));
                }
            }
        }
        if self.fw_waiting != waiting {
            self.fw_waiting = waiting;
            self.update_item().await;
        }
    }

    /// A background round failed: try again later, later each time.
    fn apps_failed(&mut self) {
        self.apps_failures = self.apps_failures.saturating_add(1);
        self.schedule.apps_in(apps_retry_after(self.apps_failures));
    }

    // ---- the panel's menu and click ----

    async fn on_menu(&mut self, id: i32) -> bool {
        let token = self.item_token.take();
        match id {
            sni::OPEN => self.open_window(&[], token),
            sni::CHECK => self.open_window(&["--check"], token),
            // Hidden items can still be "clicked" over D-Bus: act only on
            // what the menu shows.
            sni::RESTART if self.view.staged.present => self.start_restart(None).await,
            sni::CANCEL if self.scheduled_at > 0 => self.cancel_restart().await,
            sni::QUIT => return false,
            _ => {}
        }
        true
    }

    async fn handle(&mut self, m: Msg) -> bool {
        match m {
            Msg::Sched(ev) => self.on_event(ev).await,
            Msg::Status(res) => self.on_status(*res).await,
            Msg::Activate => {
                let token = self.item_token.take();
                self.open_window(&[], token);
            }
            Msg::Token(t) => self.item_token = Some(t),
            Msg::Menu(id) => return self.on_menu(id).await,
            Msg::Reload => {
                // Calls that come in from here on are read again.
                self.reload_pending.store(false, Ordering::SeqCst);
                self.reload().await
            }
            Msg::RestartEnded(res, scheduled) => self.restart_ended(res, scheduled).await,
            Msg::WorkerDone(job, out) => self.worker_done(job, out).await,
            Msg::Firmware(found) => self.firmware_done(found).await,
            Msg::Collected(first) => self.collected(first).await,
        }
        true
    }
}

/// What a firmware listing found.
#[derive(Debug)]
pub struct FwFound {
    updates: Vec<FirmwareUpdate>,
    key: String,
}

/// How long a scheduled restart waits when a firmware update is running.
const FW_RESTART_RETRY_SECS: i64 = 60;

/// Whether a firmware install holds the firmware lock (the window's own).
/// Without the lock to look at, nothing is known to run.
fn firmware_flashing() -> bool {
    match lock::take(lock::FIRMWARE, false) {
        Ok(Some(_free)) => false,
        Ok(None) => true,
        Err(e) => {
            eprintln!("atlas-updater-tray: cannot look at the firmware lock: {e}");
            false
        }
    }
}

/// Most a firmware listing may take, connecting included.
const FW_TIME_LIMIT: Duration = Duration::from_secs(90);
/// The key in `[Firmware]` that holds the set the user was told about.
const FW_NOTIFIED: &str = "Notified";
/// Most device names in the notification.
const FW_NAMES_SHOWN: usize = 3;

/// Lists firmware over the system bus. No fwupd (not running and not
/// installed) is an empty list; a failure is logged (every round it happens)
/// and gives `None`.
async fn firmware_list() -> Option<FwFound> {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("atlas-updater-tray: firmware not listed: no system bus: {e}");
            return None;
        }
    };
    match fwupd::available(&conn).await {
        Ok(true) => {}
        Ok(false) => {
            return Some(FwFound {
                updates: Vec::new(),
                key: String::new(),
            });
        }
        Err(e) => {
            eprintln!("atlas-updater-tray: firmware not listed: {}", fw_error(&e));
            return None;
        }
    }
    match fwupd::list(&conn).await {
        Ok(l) => Some(FwFound {
            key: fwupd::notice_key(&l.updates),
            updates: l.updates,
        }),
        Err(e) => {
            eprintln!("atlas-updater-tray: firmware not listed: {}", fw_error(&e));
            None
        }
    }
}

/// What went wrong, in one line for the log.
fn fw_error(e: &fwupd::Error) -> String {
    match e {
        fwupd::Error::Cancelled => "refused".into(),
        fwupd::Error::Message(m) => m.lines().next().unwrap_or_default().to_string(),
    }
}

/// What to do with the saved notice key after a listing.
#[derive(Debug, PartialEq, Eq)]
enum FwStep {
    /// Nothing to say, nothing to change.
    Keep,
    /// Nothing waits any more: forget the set.
    Clear,
    /// A new set: say so, and keep its key once shown.
    Notify,
}

/// `key` is the listed set's (empty for none), `saved` the one the user
/// was told about. While the window is open the user is looking at the
/// list: no notice, and the key stays, so the set is told about later.
fn firmware_step(key: &str, saved: Option<&str>, window_open: bool) -> FwStep {
    if key.is_empty() {
        return if saved.is_some() {
            FwStep::Clear
        } else {
            FwStep::Keep
        };
    }
    if saved == Some(key) || window_open {
        FwStep::Keep
    } else {
        FwStep::Notify
    }
}

/// "Firmware updates are available for A, B and 2 more." (device names
/// are fwupd's: escaped by the caller).
fn firmware_text(updates: &[FirmwareUpdate]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for u in updates {
        let n = u.device.trim();
        if !n.is_empty() && !names.contains(&n) {
            names.push(n);
        }
    }
    if names.is_empty() {
        return "Firmware updates are available.".into();
    }
    let more = names.len().saturating_sub(FW_NAMES_SHOWN);
    names.truncate(FW_NAMES_SHOWN);
    let mut list = names.join(", ");
    if more > 0 {
        list.push_str(&format!(" and {more} more"));
    }
    format!("Firmware updates are available for {list}.")
}

async fn next_message(s: &mut Option<MessageStream>) -> Option<zbus::Message> {
    let Some(s) = s else {
        return std::future::pending().await;
    };
    loop {
        let item = std::future::poll_fn(|cx| {
            zbus::export::futures_core::Stream::poll_next(std::pin::Pin::new(&mut *s), cx)
        })
        .await;
        match item {
            Some(Ok(m)) => return Some(m),
            Some(Err(_)) => continue,
            None => return std::future::pending().await,
        }
    }
}

/// `Err`: the watch broke and was dropped (logged).
async fn next_hits(w: &mut Option<watch::Watcher>) -> Result<Vec<watch::Hit>, ()> {
    let Some(watcher) = w else {
        return std::future::pending().await;
    };
    match watcher.next().await {
        Ok(h) => Ok(h),
        Err(e) => {
            eprintln!("atlas-updater-tray: stopped watching for changes: {e}");
            *w = None;
            Err(())
        }
    }
}

fn new_watcher(crash_on: bool) -> Option<watch::Watcher> {
    match watch::Watcher::new() {
        Ok(mut w) => {
            w.set_crash(crash_on);
            Some(w)
        }
        Err(e) => {
            eprintln!("atlas-updater-tray: cannot watch for staged updates: {e}");
            None
        }
    }
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

async fn stream(
    conn: &zbus::Connection,
    rule: zbus::Result<MatchRule<'static>>,
) -> Option<MessageStream> {
    let rule = rule.ok()?;
    match MessageStream::for_match_rule(rule, conn, Some(64)).await {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("atlas-updater-tray: cannot follow D-Bus signals: {e}");
            None
        }
    }
}

fn owner_changes(name: &'static str) -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder()
        .msg_type(MsgType::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, name)?
        .build())
}

async fn run() -> Result<(), String> {
    let conn = zbus::connection::Builder::session()
        .map_err(|e| e.to_string())?
        .method_timeout(Duration::from_secs(25))
        .build()
        .await
        .map_err(|e| format!("cannot connect to the session bus: {e}"))?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let icons = icons::Icons::load();
    let reload_pending = Arc::new(AtomicBool::new(false));
    let mut t = Tray {
        reload_pending: reload_pending.clone(),
        conn: conn.clone(),
        notifier: atlas_updater_base::notifier(),
        tx: tx.clone(),
        schedule: Schedule::default(),
        look: Look::default(),
        icons,
        view: View::default(),
        scheduled_at: 0,
        apps_auto: rc::apps_automatic(),
        crash_on: false,
        restarting: false,
        restart_failed: false,
        soon_shown: false,
        soon_id: None,
        soon_owner: None,
        notes: HashMap::new(),
        tokens: HashMap::new(),
        notify_owner: None,
        item_token: None,
        status_inflight: false,
        status_again: false,
        last_status_error: None,
        status_known: false,
        warn_missing: false,
        watch_retry: None,
        worker: None,
        round_pending: false,
        update_pending: false,
        apps_failures: 0,
        fw_inflight: false,
        fw_postponed: false,
        fw_waiting: false,
        collecting: false,
        collect_again: false,
        menu_revision: 1,
    };
    let look = t.look();
    t.look = look.clone();
    let server = conn.object_server();
    server
        .at(
            sni::ITEM_PATH,
            Item {
                look: look.clone(),
                tx: tx.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    server
        .at(
            sni::MENU_PATH,
            Menu {
                restart: look.restart,
                cancel: look.cancel,
                revision: t.menu_revision,
                tx: tx.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    server
        .at(
            tray::PATH,
            sni::Control {
                tx: tx.clone(),
                pending: reload_pending.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    // One tray per session. Taken once the objects are served: the window's
    // Reload, which may be what started us, must find them.
    match conn
        .request_name_with_flags(tray::BUS_NAME, RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(RequestNameReply::PrimaryOwner) | Ok(RequestNameReply::AlreadyOwner) => {}
        Ok(_) | Err(zbus::Error::NameTaken) => {
            eprintln!("atlas-updater-tray: already running");
            return Ok(());
        }
        Err(e) => return Err(format!("cannot take {}: {e}", tray::BUS_NAME)),
    }
    let item_name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    conn.request_name(item_name.as_str())
        .await
        .map_err(|e| format!("cannot take {item_name}: {e}"))?;

    // Signals: the panel coming and going, the notification server's.
    let mut watcher_owner = stream(&conn, owner_changes(WATCHER)).await;
    let mut notes_owner = stream(&conn, owner_changes(notify::SERVICE)).await;
    let note_rule = MatchRule::builder()
        .msg_type(MsgType::Signal)
        .interface(notify::INTERFACE)
        .and_then(|b| b.path(notify::PATH))
        .map(|b| b.build());
    let mut note_signals = stream(&conn, note_rule).await;
    t.register().await;

    // A restart saved by an earlier run.
    match rc::scheduled_at() {
        Some(at) if at > schedule::unix_now() => {
            t.scheduled_at = at;
            t.schedule.restore_restart(at);
        }
        // A time that passed while we were not running is dropped: never
        // restart the machine unexpectedly at login.
        Some(_) => rc::set(rc::RESTART, "ScheduledAt", None),
        None => {}
    }

    // Crash reports are opt-in: only when on are the sources watched and read.
    let crash_on = atlas_framework_system::crash::Settings::load().enabled;
    let mut watcher = new_watcher(crash_on);
    if watcher.is_none() {
        t.watch_retry = Some(Instant::now() + WATCH_RETRY);
    }
    t.set_crash(crash_on);

    let sched = t.schedule.clone();
    let sched_tx = tx.clone();
    if !spawn_thread("atlas-schedule", move || {
        let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sched.run(|ev| {
                let _ = sched_tx.send(Msg::Sched(ev));
            })
        }));
        if ended.is_err() {
            eprintln!("atlas-updater-tray: the schedule stopped; restarts and checks will not run");
        }
    }) {
        return Err("cannot start the schedule".into());
    }

    t.refresh_status();
    t.update_item().await;

    let mut ostree_at: Option<Instant> = None;
    let mut crash_at: Option<Instant> = None;
    loop {
        tokio::select! {
            m = rx.recv() => {
                let Some(m) = m else { break };
                if !t.handle(m).await {
                    break;
                }
                if let Some(w) = watcher.as_mut() {
                    w.set_crash(t.crash_on);
                }
            }
            hits = next_hits(&mut watcher) => {
                let Ok(hits) = hits else {
                    // The 6-hour status poll covers the gap.
                    t.watch_retry = Some(Instant::now() + WATCH_RETRY);
                    continue;
                };
                for h in hits {
                    match h {
                        // From the first hit: a steady stream must not put it off forever.
                        watch::Hit::Ostree => {
                            ostree_at.get_or_insert(Instant::now() + OSTREE_SETTLE);
                        }
                        watch::Hit::Crash => {
                            crash_at.get_or_insert(Instant::now() + CRASH_SETTLE);
                        }
                    }
                }
            }
            _ = sleep_until(t.watch_retry) => {
                t.watch_retry = None;
                watcher = new_watcher(t.crash_on);
                if watcher.is_some() {
                    // Whatever happened meanwhile: look again.
                    t.refresh_status();
                    if t.crash_on {
                        t.collect();
                    }
                } else {
                    t.watch_retry = Some(Instant::now() + WATCH_RETRY);
                }
            }
            _ = sleep_until(ostree_at) => {
                ostree_at = None;
                t.refresh_status();
            }
            _ = sleep_until(crash_at) => {
                crash_at = None;
                t.collect();
            }
            m = next_message(&mut note_signals) => {
                if let Some(m) = m {
                    t.on_note_signal(m).await;
                }
            }
            m = next_message(&mut watcher_owner) => {
                if let Some(m) = m
                    && let Ok((_, _, new)) = m.body().deserialize::<(String, String, String)>()
                    && !new.is_empty()
                {
                    // Plasma (re)started: show up in its tray again.
                    t.register().await;
                }
            }
            m = next_message(&mut notes_owner) => {
                if let Some(m) = m
                    && let Ok((_, _, new)) = m.body().deserialize::<(String, String, String)>()
                {
                    let back = !new.is_empty();
                    // Unless a notification already went to it since it came
                    // up, our notifications went with the old server.
                    if !(back && t.notify_owner.as_deref() == Some(new.as_str())) {
                        t.notify_owner = back.then(|| new.clone());
                        t.notes.clear();
                        t.tokens.clear();
                    }
                    let warning_here = back && t.soon_owner.as_deref() == Some(new.as_str());
                    if !warning_here {
                        t.soon_id = None;
                        t.soon_owner = None;
                    }
                    // A restart still to come is warned about again on the
                    // new server (the old warning is gone from the screen).
                    if back
                        && !warning_here
                        && t.soon_shown
                        && !t.restarting
                        && t.scheduled_at > schedule::unix_now()
                    {
                        t.restart_soon().await;
                    }
                }
            }
        }
    }
    t.schedule.stop();
    Ok(())
}

fn main() -> ExitCode {
    // Dates in the tooltip follow the user's locale.
    // SAFETY: called first, before any other thread exists.
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }
    atlas_framework_system::crash::install(crash::app_info());
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("atlas-updater-tray: cannot start: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("atlas-updater-tray: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fw(device: &str) -> FirmwareUpdate {
        FirmwareUpdate {
            device_id: device.into(),
            device: device.into(),
            vendor: String::new(),
            current: "1".into(),
            version: "2".into(),
            summary: String::new(),
            description: String::new(),
            urgency: fwupd::Urgency::Unknown,
            size: 0,
            checksums: Vec::new(),
            locations: Vec::new(),
            remote_id: String::new(),
            trusted: true,
            needs_reboot: false,
            needs_shutdown: false,
            internal: false,
        }
    }

    #[test]
    fn firmware_notified_once_per_set() {
        // New set: tell. Same set: not again. Window open: neither a notice
        // nor a key change, so it is told once the window is closed.
        assert_eq!(firmware_step("a", None, false), FwStep::Notify);
        assert_eq!(firmware_step("a", Some("a"), false), FwStep::Keep);
        assert_eq!(firmware_step("b", Some("a"), false), FwStep::Notify);
        assert_eq!(firmware_step("a", None, true), FwStep::Keep);
        assert_eq!(firmware_step("b", Some("a"), true), FwStep::Keep);
        // Nothing waits: forget, so the same set later is news again.
        assert_eq!(firmware_step("", Some("a"), false), FwStep::Clear);
        assert_eq!(firmware_step("", Some("a"), true), FwStep::Clear);
        assert_eq!(firmware_step("", None, false), FwStep::Keep);
    }

    #[test]
    fn firmware_text_names_devices() {
        assert_eq!(
            firmware_text(&[fw("UEFI dbx"), fw(" "), fw("UEFI dbx")]),
            "Firmware updates are available for UEFI dbx."
        );
        assert_eq!(
            firmware_text(&[fw("A"), fw("B"), fw("C"), fw("D"), fw("E")]),
            "Firmware updates are available for A, B, C and 2 more."
        );
        assert_eq!(firmware_text(&[fw("")]), "Firmware updates are available.");
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
