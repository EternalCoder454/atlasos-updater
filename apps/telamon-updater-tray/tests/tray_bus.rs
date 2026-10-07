//! The real tray program on private buses: a private session bus (what
//! Settings talks to) and a private "system" bus (where the system helper's
//! `Progress` is). Its programs are fakes that write down what they were
//! asked: a `telamon-settings` that records its arguments and a
//! `telamon-updater-glow` that records when it starts and when it is asked
//! to end.
//!
//! Needs `dbus-daemon`. Without it the tests say so and return, unless
//! `TELAMON_REQUIRE_DBUS_TESTS` is set (the container checks set it).

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use telamon_updater_base::tray;
use zbus::zvariant::Value;

// ---- a private bus ----

struct Bus {
    child: Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl Bus {
    fn start() -> Option<Bus> {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={sock}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                sock = dir.path().join("bus").display(),
            ),
        )
        .unwrap();
        let spawned = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                assert!(
                    std::env::var_os("TELAMON_REQUIRE_DBUS_TESTS").is_none(),
                    "TELAMON_REQUIRE_DBUS_TESTS is set but dbus-daemon cannot start: {e}"
                );
                eprintln!("skipped: no dbus-daemon ({e})");
                return None;
            }
        };
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_string();
        assert!(address.starts_with("unix:"), "dbus-daemon said {address:?}");
        Some(Bus {
            child,
            address,
            _dir: dir,
        })
    }

    fn builder(&self) -> zbus::connection::Builder<'static> {
        let address: zbus::Address = self.address.parse().unwrap();
        zbus::connection::Builder::address(address).unwrap()
    }

    async fn connect(&self) -> zbus::Connection {
        self.builder().build().await.unwrap()
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// The condition stays true (or false) for `for_` : used for "nothing happens".
async fn stays(what: &str, for_: Duration, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + for_;
    while Instant::now() < end {
        assert!(ok(), "{what}");
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

// ---- the system helper's Progress, as a service on the "system" bus ----

struct Progress(String);

#[zbus::interface(name = "net.eterneon.telamon.SystemHelper1")]
impl Progress {
    #[zbus(property)]
    fn progress(&self) -> String {
        self.0.clone()
    }
}

struct LegacyProgress(String);

#[zbus::interface(name = "net.eterneon.atlas.SystemHelper1")]
impl LegacyProgress {
    #[zbus(property)]
    fn progress(&self) -> String {
        self.0.clone()
    }
}

const UPGRADING: &str = r#"{"op":"upgrade","stage":"downloading","done":1,"total":9,"detail":""}"#;

struct FakeHelper {
    conn: zbus::Connection,
}

impl FakeHelper {
    async fn start(system: &Bus, progress: &str) -> FakeHelper {
        let conn = system
            .builder()
            .serve_at(
                "/net/eterneon/telamon/SystemHelper",
                Progress(progress.into()),
            )
            .unwrap()
            .serve_at(
                "/net/eterneon/atlas/SystemHelper",
                LegacyProgress(progress.into()),
            )
            .unwrap()
            .name("net.eterneon.telamon.SystemHelper")
            .unwrap()
            .name("net.eterneon.atlas.SystemHelper")
            .unwrap()
            .build()
            .await
            .unwrap();
        FakeHelper { conn }
    }

    /// The new identity's `Progress` becomes `to`, and says so.
    async fn set(&self, to: &str) {
        let r = self
            .conn
            .object_server()
            .interface::<_, Progress>("/net/eterneon/telamon/SystemHelper")
            .await
            .unwrap();
        r.get_mut().await.0 = to.into();
        r.get()
            .await
            .progress_changed(r.signal_emitter())
            .await
            .unwrap();
    }

    async fn set_legacy(&self, to: &str) {
        let r = self
            .conn
            .object_server()
            .interface::<_, LegacyProgress>("/net/eterneon/atlas/SystemHelper")
            .await
            .unwrap();
        r.get_mut().await.0 = to.into();
        r.get()
            .await
            .progress_changed(r.signal_emitter())
            .await
            .unwrap();
    }
}

// ---- the tray, with its fake programs ----

struct Rig {
    dir: tempfile::TempDir,
    session: Bus,
    system: Bus,
    tray: Option<Child>,
}

impl Rig {
    fn new() -> Option<Rig> {
        Some(Rig {
            dir: tempfile::tempdir().unwrap(),
            session: Bus::start()?,
            system: Bus::start()?,
            tray: None,
        })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn script(&self, name: &str, body: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// A glow that writes `start <pid>` and, when asked to end, `stop <pid>`
    /// to `glow.log`.
    fn well_behaved_glow(&self) -> PathBuf {
        self.script(
            "glow",
            r#"echo "start $$" >> "$GLOW_LOG"
trap 'echo "stop $$" >> "$GLOW_LOG"; exit 0' TERM
while :; do sleep 0.1; done"#,
        )
    }

    fn settings_log(&self) -> PathBuf {
        self.path("settings.log")
    }

    fn glow_log(&self) -> PathBuf {
        self.path("glow.log")
    }

    fn start_tray(&mut self, glow: &Path) {
        let settings = self.script(
            "telamon-settings",
            r#"echo "$* token=${XDG_ACTIVATION_TOKEN-unset}" >> "$SETTINGS_LOG""#,
        );
        for d in ["home", "config", "state", "data", "cache", "run"] {
            std::fs::create_dir_all(self.path(d)).unwrap();
        }
        std::fs::set_permissions(self.path("run"), std::fs::Permissions::from_mode(0o700)).unwrap();
        let log = std::fs::File::create(self.path("tray.log")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_telamon-updater-tray"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_CACHE_HOME", self.path("cache"))
            .env("XDG_RUNTIME_DIR", self.path("run"))
            .env("DBUS_SESSION_BUS_ADDRESS", &self.session.address)
            .env("DBUS_SYSTEM_BUS_ADDRESS", &self.system.address)
            .env("TELAMON_UPDATER_GLOW_BIN", glow)
            .env("TELAMON_SETTINGS_BIN", settings)
            .env("GLOW_LOG", self.glow_log())
            .env("SETTINGS_LOG", self.settings_log())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        self.tray = Some(child);
    }

    async fn wait_for_tray(&self) -> zbus::Connection {
        let conn = self.session.connect().await;
        let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if dbus
                .name_has_owner(tray::BUS_NAME.try_into().unwrap())
                .await
                .unwrap()
                && dbus
                    .name_has_owner(tray::LEGACY_BUS_NAME.try_into().unwrap())
                    .await
                    .unwrap()
            {
                // the follower tasks connect and look once
                tokio::time::sleep(Duration::from_millis(400)).await;
                return conn;
            }
            assert!(
                Instant::now() < end,
                "the tray did not take its names:\n{}",
                self.tray_log()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn tray_log(&self) -> String {
        std::fs::read_to_string(self.path("tray.log")).unwrap_or_default()
    }

    /// The glow log: ("start"|"stop", pid) in order.
    fn glow_events(&self) -> Vec<(String, u32)> {
        std::fs::read_to_string(self.glow_log())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let (w, pid) = l.split_once(' ')?;
                Some((w.to_string(), pid.parse().ok()?))
            })
            .collect()
    }

    fn starts(&self) -> usize {
        self.glow_events().iter().filter(|e| e.0 == "start").count()
    }

    fn stops(&self) -> usize {
        self.glow_events().iter().filter(|e| e.0 == "stop").count()
    }

    /// Never more than one glow at a time, as far as the log shows.
    fn assert_one_at_a_time(&self) {
        let mut live: Vec<u32> = Vec::new();
        for (what, pid) in self.glow_events() {
            if what == "start" {
                live.push(pid);
                assert!(
                    live.len() <= 1,
                    "two glows at once: {:?}",
                    self.glow_events()
                );
            } else {
                live.retain(|p| *p != pid);
            }
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(mut t) = self.tray.take() {
            let _ = t.kill();
            let _ = t.wait();
        }
    }
}

fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
        && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| s.contains(") Z "))
            .unwrap_or(true)
}

async fn set_working(conn: &zbus::Connection, on: bool) {
    tray::call_set_working(conn, on).await.unwrap();
}

// ---- tests ----

#[tokio::test(flavor = "current_thread")]
async fn set_working_starts_and_stops_the_glow_and_a_vanished_caller_stops_it() {
    let Some(mut rig) = Rig::new() else { return };
    let helper = FakeHelper::start(&rig.system, "").await;
    let glow = rig.well_behaved_glow();
    rig.start_tray(&glow);
    let _watch = rig.wait_for_tray().await;
    stays("idle: no glow", Duration::from_millis(400), || {
        rig.starts() == 0
    })
    .await;

    // SetWorking(true) starts it, once, however often it is said
    let settings = rig.session.connect().await;
    set_working(&settings, true).await;
    wait_until("the glow starts", || rig.starts() == 1).await;
    set_working(&settings, true).await;
    stays("still one glow", Duration::from_millis(500), || {
        rig.starts() == 1 && rig.stops() == 0
    })
    .await;

    // a second caller holding on: releasing the first changes nothing
    let other = rig.session.connect().await;
    set_working(&other, true).await;
    set_working(&settings, false).await;
    stays(
        "the other caller keeps it",
        Duration::from_millis(500),
        || rig.stops() == 0,
    )
    .await;

    // the other one crashes (its connection closes): the glow ends
    other.graceful_shutdown().await;
    wait_until("the glow ends when the last caller left", || {
        rig.stops() == 1
    })
    .await;

    // a caller that releases
    set_working(&settings, true).await;
    wait_until("started again", || rig.starts() == 2).await;
    set_working(&settings, false).await;
    wait_until("stopped again", || rig.stops() == 2).await;
    rig.assert_one_at_a_time();
    drop(helper);
}

#[tokio::test(flavor = "current_thread")]
async fn the_helpers_progress_shows_the_glow_with_settings_closed() {
    let Some(mut rig) = Rig::new() else { return };
    let helper = FakeHelper::start(&rig.system, "").await;
    let glow = rig.well_behaved_glow();
    rig.start_tray(&glow);
    let session = rig.wait_for_tray().await;

    helper.set(UPGRADING).await;
    wait_until("progress starts the glow", || rig.starts() == 1).await;
    // more progress: nothing new
    helper
        .set(&UPGRADING.replace("\"done\":1", "\"done\":5"))
        .await;
    // the legacy name says the same: one glow
    helper.set_legacy(UPGRADING).await;
    stays("one glow", Duration::from_millis(500), || {
        rig.starts() == 1 && rig.stops() == 0
    })
    .await;
    // a claim in addition, then the progress ends: the claim holds it
    let settings = rig.session.connect().await;
    set_working(&settings, true).await;
    helper.set("").await;
    helper.set_legacy("").await;
    stays("the claim holds it", Duration::from_millis(500), || {
        rig.stops() == 0
    })
    .await;
    set_working(&settings, false).await;
    wait_until("then it ends", || rig.stops() == 1).await;

    // the legacy name alone does it too
    helper.set_legacy(UPGRADING).await;
    wait_until("legacy progress starts it", || rig.starts() == 2).await;
    helper.set_legacy("").await;
    wait_until("and ends it", || rig.stops() == 2).await;

    // the helper leaving the bus in the middle of an upgrade ends it
    helper.set(UPGRADING).await;
    wait_until("started", || rig.starts() == 3).await;
    helper.conn.graceful_shutdown().await;
    wait_until("the helper vanished", || rig.stops() == 3).await;
    rig.assert_one_at_a_time();
    drop(session);
}

#[tokio::test(flavor = "current_thread")]
async fn an_upgrade_already_running_when_the_tray_starts_is_seen() {
    let Some(mut rig) = Rig::new() else { return };
    let _helper = FakeHelper::start(&rig.system, UPGRADING).await;
    let glow = rig.well_behaved_glow();
    rig.start_tray(&glow);
    let _session = rig.wait_for_tray().await;
    wait_until("started from the first read", || rig.starts() == 1).await;
}

#[tokio::test(flavor = "current_thread")]
async fn only_the_helpers_own_word_counts() {
    let Some(mut rig) = Rig::new() else { return };
    let _helper = FakeHelper::start(&rig.system, "").await;
    let glow = rig.well_behaved_glow();
    rig.start_tray(&glow);
    let _session = rig.wait_for_tray().await;

    // another program sends the same signal from the same path
    let spoof = rig.system.connect().await;
    let changed = std::collections::HashMap::from([("Progress", Value::from(UPGRADING))]);
    spoof
        .emit_signal(
            None::<&str>,
            "/net/eterneon/telamon/SystemHelper",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(
                "net.eterneon.telamon.SystemHelper1",
                changed,
                Vec::<&str>::new(),
            ),
        )
        .await
        .unwrap();
    stays("no glow for a stranger", Duration::from_millis(800), || {
        rig.starts() == 0
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn only_one_boolean_is_accepted_and_only_by_the_new_name() {
    let Some(mut rig) = Rig::new() else { return };
    let _helper = FakeHelper::start(&rig.system, "").await;
    let glow = rig.well_behaved_glow();
    rig.start_tray(&glow);
    let conn = rig.wait_for_tray().await;

    let call = |iface: &'static str, member: &'static str, body: Value<'static>| {
        let conn = conn.clone();
        async move {
            conn.call_method(Some(tray::BUS_NAME), tray::PATH, Some(iface), member, &body)
                .await
        }
    };
    // a string, a number, two arguments, none: refused, nothing starts
    for body in [Value::from("true"), Value::from(1u32)] {
        assert!(
            call(tray::INTERFACE, "SetWorking", body).await.is_err(),
            "accepted something that is not a boolean"
        );
    }
    let none = conn
        .call_method(
            Some(tray::BUS_NAME),
            tray::PATH,
            Some(tray::INTERFACE),
            "SetWorking",
            &(),
        )
        .await;
    assert!(none.is_err());
    let two = conn
        .call_method(
            Some(tray::BUS_NAME),
            tray::PATH,
            Some(tray::INTERFACE),
            "SetWorking",
            &(true, "x"),
        )
        .await;
    assert!(two.is_err());
    // the old interface has Reload only
    let old = conn
        .call_method(
            Some(tray::LEGACY_BUS_NAME),
            tray::LEGACY_PATH,
            Some(tray::LEGACY_INTERFACE),
            "SetWorking",
            &(true,),
        )
        .await;
    assert!(old.is_err());
    // (and the new name's path is the old interface's no more)
    let reload_old = conn
        .call_method(
            Some(tray::LEGACY_BUS_NAME),
            tray::LEGACY_PATH,
            Some(tray::LEGACY_INTERFACE),
            "Reload",
            &(),
        )
        .await;
    assert!(reload_old.is_ok());
    stays(
        "none of that started the glow",
        Duration::from_millis(600),
        || rig.starts() == 0,
    )
    .await;
    // and a real one still works
    set_working(&conn, true).await;
    wait_until("a boolean starts it", || rig.starts() == 1).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_glow_that_ignores_the_term_is_killed_and_one_that_dies_is_given_up_on() {
    let Some(mut rig) = Rig::new() else { return };
    let _helper = FakeHelper::start(&rig.system, "").await;
    // ignores SIGTERM
    let stubborn = rig.script(
        "stubborn",
        r#"echo "start $$" >> "$GLOW_LOG"
trap '' TERM
while :; do sleep 0.1; done"#,
    );
    rig.start_tray(&stubborn);
    let conn = rig.wait_for_tray().await;
    set_working(&conn, true).await;
    wait_until("started", || rig.starts() == 1).await;
    let pid = rig.glow_events()[0].1;
    set_working(&conn, false).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(process_alive(pid), "killed before the grace time was over");
    wait_until("killed after the grace time", || !process_alive(pid)).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_glow_that_keeps_dying_is_restarted_three_times_and_then_left() {
    let Some(mut rig) = Rig::new() else { return };
    let _helper = FakeHelper::start(&rig.system, "").await;
    let dies = rig.script(
        "dies",
        r#"echo "start $$" >> "$GLOW_LOG"
exit 1"#,
    );
    rig.start_tray(&dies);
    let conn = rig.wait_for_tray().await;
    set_working(&conn, true).await;
    // 1 start, then restarts after 1 s, 2 s and 4 s
    wait_until("the fourth start", || rig.starts() == 4).await;
    stays("no fifth", Duration::from_secs(3), || rig.starts() == 4).await;
    // idle once, then wanted again: a fresh start
    set_working(&conn, false).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    set_working(&conn, true).await;
    wait_until("fresh start after idle", || rig.starts() == 5).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_glow_that_cannot_start_is_logged_once() {
    let Some(mut rig) = Rig::new() else { return };
    let _helper = FakeHelper::start(&rig.system, "").await;
    rig.start_tray(Path::new("/nonexistent/telamon-updater-glow"));
    let conn = rig.wait_for_tray().await;
    for _ in 0..3 {
        set_working(&conn, true).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        set_working(&conn, false).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let log = rig.tray_log();
    assert_eq!(
        log.matches("cannot start the update glow").count(),
        1,
        "{log}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn quitting_the_tray_ends_the_glow() {
    let Some(mut rig) = Rig::new() else { return };
    let _helper = FakeHelper::start(&rig.system, "").await;
    let glow = rig.well_behaved_glow();
    rig.start_tray(&glow);
    let conn = rig.wait_for_tray().await;
    set_working(&conn, true).await;
    wait_until("started", || rig.starts() == 1).await;
    // the panel menu's Quit
    conn.call_method(
        Some(tray::BUS_NAME),
        "/MenuBar",
        Some("com.canonical.dbusmenu"),
        "Event",
        &(6i32, "clicked", Value::from(0i32), 0u32),
    )
    .await
    .unwrap();
    wait_until("the glow is told to end", || rig.stops() == 1).await;
    let mut tray = rig.tray.take().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = tray.try_wait().unwrap() {
            assert!(status.success(), "{status:?}\n{}", rig.tray_log());
            break;
        }
        assert!(Instant::now() < end, "the tray did not end");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn the_icon_and_the_menu_open_telamon_settings_on_the_right_page() {
    let Some(mut rig) = Rig::new() else { return };
    let _helper = FakeHelper::start(&rig.system, "").await;
    let glow = rig.well_behaved_glow();
    rig.start_tray(&glow);
    let conn = rig.wait_for_tray().await;
    let click = |id: i32| {
        let conn = conn.clone();
        async move {
            conn.call_method(
                Some(tray::BUS_NAME),
                "/MenuBar",
                Some("com.canonical.dbusmenu"),
                "Event",
                &(id, "clicked", Value::from(0i32), 0u32),
            )
            .await
            .unwrap();
        }
    };
    let lines = |rig: &Rig| -> Vec<String> {
        std::fs::read_to_string(rig.settings_log())
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    };
    // the menu's "Open Updates"
    click(1).await;
    wait_until("open", || lines(&rig).len() == 1).await;
    // "Check for Updates"
    click(2).await;
    wait_until("check", || lines(&rig).len() == 2).await;
    // a click on the icon, with the activation token Plasma sent before it
    conn.call_method(
        Some(tray::BUS_NAME),
        "/StatusNotifierItem",
        Some("org.kde.StatusNotifierItem"),
        "ProvideXdgActivationToken",
        &("tok-1",),
    )
    .await
    .unwrap();
    conn.call_method(
        Some(tray::BUS_NAME),
        "/StatusNotifierItem",
        Some("org.kde.StatusNotifierItem"),
        "Activate",
        &(0i32, 0i32),
    )
    .await
    .unwrap();
    wait_until("activate", || lines(&rig).len() == 3).await;
    assert_eq!(
        lines(&rig),
        [
            "updates token=unset",
            "updates check token=unset",
            "updates token=tok-1"
        ]
    );
}
