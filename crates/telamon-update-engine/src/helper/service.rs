//! D-Bus glue: the `SystemHelper1` interface, polkit checks and the idle exit.
//!
//! One [`Service`] answers under two identities (see [`crate::identity`]):
//! `net.eterneon.telamon.SystemHelper` and, for this release, the old
//! `net.eterneon.atlas.SystemHelper` ([`LegacyService`], the same methods on
//! the old interface, object path and error names, authorized against the
//! old polkit action ids).

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::message::Header;
use zbus::zvariant::Value;

use super::{BootcRunner, Core, HelperError, LegacyHelperError, Op, lock};
use crate::identity::Identity;

/// Exit after this long with no calls.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

const POLKIT_ALLOW_USER_INTERACTION: u32 = 1;

#[zbus::proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority",
    gen_blocking = false
)]
trait Authority {
    #[allow(clippy::type_complexity)]
    fn check_authorization(
        &self,
        subject: &(&str, HashMap<&str, Value<'_>>),
        action_id: &str,
        details: &HashMap<&str, &str>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1",
    gen_blocking = false
)]
trait Systemd {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

/// The only unit the helper ever asks systemd to start. (The old name,
/// `atlas-drivers.service`, is an alias of it that the package ships.)
const DRIVERS_UNIT: &str = "telamon-drivers.service";
/// systemd answers a start request at once (it does not wait for the job).
const START_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct State {
    active: usize,
    last: Option<Instant>,
    closing: bool,
}

/// Calls in flight and the time of the last one. One lock, so a call cannot
/// slip in between the idle check and the decision to exit.
#[derive(Default)]
pub struct Activity(Mutex<State>);

pub struct ActivityGuard {
    activity: Arc<Activity>,
    touch: bool,
}

impl Activity {
    /// Register a call; `None` once the helper is shutting down. A call with
    /// `touch == false` (Status) does not postpone the idle exit.
    pub fn enter(self: &Arc<Self>, touch: bool) -> Option<ActivityGuard> {
        let mut st = lock(&self.0);
        if st.closing {
            return None;
        }
        st.active += 1;
        Some(ActivityGuard {
            activity: self.clone(),
            touch,
        })
    }

    /// If nothing is in flight and the last touching call ended `timeout`
    /// ago, mark the helper as closing and return true. From then on
    /// [`enter`](Self::enter) refuses.
    pub fn close_if_idle(&self, timeout: Duration, started: Instant) -> bool {
        let mut st = lock(&self.0);
        if st.active == 0 && st.last.unwrap_or(started).elapsed() >= timeout {
            st.closing = true;
        }
        st.closing
    }

    /// Refuse new calls from now on (SIGTERM).
    pub fn begin_close(&self) {
        lock(&self.0).closing = true;
    }

    /// Wait until no call is in flight; false if `limit` ran out first.
    pub async fn wait_drained(&self, limit: Duration) -> bool {
        let end = Instant::now() + limit;
        while lock(&self.0).active > 0 {
            if Instant::now() >= end {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        true
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut st = lock(&self.activity.0);
        st.active = st.active.saturating_sub(1);
        if self.touch {
            st.last = Some(Instant::now());
        }
    }
}

/// What decides whether a caller may run an operation. The helper asks
/// polkit ([`Polkit`]); tests put their own here.
pub trait Authorizer: Send + Sync + 'static {
    /// `sender`: the caller's unique bus name, `None` when the call has none.
    /// `action`: the polkit action id of the identity the call came through.
    fn check<'a>(
        &'a self,
        conn: &'a zbus::Connection,
        sender: Option<&'a str>,
        action: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), HelperError>> + Send + 'a>>;
}

/// The real authorizer: polkit's `CheckAuthorization` for the caller.
pub struct Polkit;

impl Authorizer for Polkit {
    fn check<'a>(
        &'a self,
        conn: &'a zbus::Connection,
        sender: Option<&'a str>,
        action: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), HelperError>> + Send + 'a>> {
        Box::pin(async move {
            let sender =
                sender.ok_or_else(|| HelperError::NotAuthorized("call has no sender".into()))?;
            authorize(conn, sender, action).await
        })
    }
}

#[derive(Clone)]
pub struct Service {
    core: Arc<Core>,
    activity: Arc<Activity>,
    authorizer: Arc<dyn Authorizer>,
}

/// The same service under the old identity: the interface
/// `net.eterneon.atlas.SystemHelper1` at `/net/eterneon/atlas/SystemHelper`.
#[derive(Clone)]
pub struct LegacyService(pub Service);

const SHUTTING_DOWN: &str = "the helper is shutting down, try again";

impl Service {
    /// `events` is where update and rollback events go (`None` for none).
    pub fn new(runner: Arc<dyn BootcRunner>, events: Option<PathBuf>) -> Service {
        let mut core = Core::new(runner);
        if let Some(p) = events {
            core = core.with_events(p);
        }
        Service::from_core(core)
    }

    pub fn from_core(core: Core) -> Service {
        Service {
            core: Arc::new(core),
            activity: Arc::new(Activity::default()),
            authorizer: Arc::new(Polkit),
        }
    }

    /// Ask `authorizer` instead of polkit (tests).
    pub fn with_authorizer(mut self, authorizer: Arc<dyn Authorizer>) -> Self {
        self.authorizer = authorizer;
        self
    }

    /// Count the call; refused while shutting down.
    fn begin(&self, op: &Op) -> Result<ActivityGuard, HelperError> {
        self.activity
            .enter(*op != Op::Status)
            .ok_or_else(|| HelperError::ShuttingDown(SHUTTING_DOWN.into()))
    }

    /// Run an already authorized operation. The activity guard moves into the
    /// worker, so the call counts until bootc has really finished, even if the
    /// caller goes away.
    async fn execute(&self, op: Op, guard: ActivityGuard) -> Result<String, HelperError> {
        op.validate()?;
        let core = self.core.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            core.execute(&op)
        })
        .await
        .map_err(|e| HelperError::Failed(format!("worker failed: {e}")))?
    }

    /// If the machine needs another image for its hardware, have systemd run
    /// `telamon-drivers.service` (the pull happens there, not in this call).
    /// Never fails the caller: a problem is logged.
    async fn start_drivers(&self, conn: &zbus::Connection) {
        let core = self.core.clone();
        let wanted = tokio::task::spawn_blocking(move || core.drivers_wanted())
            .await
            .unwrap_or(false);
        if !wanted {
            return;
        }
        let start = async {
            SystemdProxy::new(conn)
                .await?
                .start_unit(DRIVERS_UNIT, "replace")
                .await
        };
        match tokio::time::timeout(START_TIMEOUT, start).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("telamon-system-helper: cannot start {DRIVERS_UNIT}: {e}"),
            Err(_) => eprintln!("telamon-system-helper: starting {DRIVERS_UNIT} timed out"),
        }
    }

    /// Run `op` without a polkit check (tests).
    pub async fn run(&self, op: Op) -> Result<String, HelperError> {
        let guard = self.begin(&op)?;
        self.execute(op, guard).await
    }

    /// A call that came in through `identity`: checked against that
    /// identity's polkit action, then run.
    async fn handle(
        &self,
        header: &Header<'_>,
        conn: &zbus::Connection,
        op: Op,
        identity: Identity,
    ) -> Result<String, HelperError> {
        // count the call while polkit is asked, so we do not exit under it
        let guard = self.begin(&op)?;
        op.validate()?;
        let sender = header.sender().map(|s| s.to_string());
        self.authorizer
            .check(conn, sender.as_deref(), op.action_id_for(identity))
            .await?;
        if op == Op::CheckForUpdate {
            self.start_drivers(conn).await;
        }
        self.execute(op, guard).await
    }
}

async fn authorize(conn: &zbus::Connection, sender: &str, action: &str) -> Result<(), HelperError> {
    let denied = |m: String| HelperError::NotAuthorized(m);
    let authority = AuthorityProxy::new(conn)
        .await
        .map_err(|e| denied(format!("cannot reach polkit: {e}")))?;
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Value::from(sender))]),
    );
    let (authorized, _challenge, _details) = authority
        .check_authorization(
            &subject,
            action,
            &HashMap::new(),
            POLKIT_ALLOW_USER_INTERACTION,
            "",
        )
        .await
        .map_err(|e| denied(format!("polkit check failed: {e}")))?;
    if authorized {
        Ok(())
    } else {
        Err(denied(format!("not authorized for {action}")))
    }
}

#[zbus::interface(name = "net.eterneon.telamon.SystemHelper1")]
impl Service {
    /// JSON of the current [`Progress`](crate::progress::Progress) while an
    /// upgrade or switch runs, `""` otherwise.
    #[zbus(property)]
    async fn progress(&self) -> String {
        self.core.progress_json()
    }

    async fn status(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::Status, Identity::Telamon)
            .await
    }

    async fn check_for_update(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::CheckForUpdate, Identity::Telamon)
            .await
    }

    async fn upgrade(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::Upgrade, Identity::Telamon)
            .await
    }

    async fn rollback(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::Rollback, Identity::Telamon)
            .await
    }

    async fn cancel_rollback(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::CancelRollback, Identity::Telamon)
            .await
    }

    async fn switch_channel(
        &self,
        channel: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::SwitchChannel(channel), Identity::Telamon)
            .await
    }
}

// The old identity: the same six methods and the property, nothing more,
// with the old error names and the old polkit actions. Remove with the
// legacy names (see DESIGN.md).
#[zbus::interface(name = "net.eterneon.atlas.SystemHelper1")]
impl LegacyService {
    #[zbus(property)]
    async fn progress(&self) -> String {
        self.0.core.progress_json()
    }

    async fn status(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, LegacyHelperError> {
        self.0
            .handle(&header, conn, Op::Status, Identity::Legacy)
            .await
            .map_err(Into::into)
    }

    async fn check_for_update(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, LegacyHelperError> {
        self.0
            .handle(&header, conn, Op::CheckForUpdate, Identity::Legacy)
            .await
            .map_err(Into::into)
    }

    async fn upgrade(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, LegacyHelperError> {
        self.0
            .handle(&header, conn, Op::Upgrade, Identity::Legacy)
            .await
            .map_err(Into::into)
    }

    async fn rollback(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, LegacyHelperError> {
        self.0
            .handle(&header, conn, Op::Rollback, Identity::Legacy)
            .await
            .map_err(Into::into)
    }

    async fn cancel_rollback(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, LegacyHelperError> {
        self.0
            .handle(&header, conn, Op::CancelRollback, Identity::Legacy)
            .await
            .map_err(Into::into)
    }

    async fn switch_channel(
        &self,
        channel: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, LegacyHelperError> {
        self.0
            .handle(&header, conn, Op::SwitchChannel(channel), Identity::Legacy)
            .await
            .map_err(Into::into)
    }
}

/// How long a running bootc may finish after SIGTERM before we ask it to
/// stop (the unit gives the helper 60 s in all).
const TERM_GRACE: Duration = Duration::from_secs(40);

/// Serve until idle for `idle_timeout` or SIGTERM. Either way: refuse new
/// calls, release the bus names (so D-Bus starts a fresh helper for the next
/// call), then wait for calls in flight and return.
///
/// Both identities are served: the new name is the one the unit waits for
/// (`BusName=`); the old one is taken right after, and if something else owns
/// it (a helper from before the rename, still running after the upgrade) this
/// helper serves the new name only.
pub async fn serve(
    builder: zbus::connection::Builder<'static>,
    service: Service,
    idle_timeout: Duration,
) -> zbus::Result<()> {
    let activity = service.activity.clone();
    let progress = service.core.progress_watch();
    let started = Instant::now();
    let [new, old] = Identity::ALL;
    let conn = builder
        .name(new.bus_name())?
        .serve_at(new.object_path(), service.clone())?
        .serve_at(old.object_path(), LegacyService(service))?
        .build()
        .await?;
    let mut names = vec![new.bus_name()];
    match conn
        .request_name_with_flags(old.bus_name(), RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
            names.push(old.bus_name());
        }
        Ok(_) | Err(zbus::Error::NameTaken) => eprintln!(
            "telamon-system-helper: {} is taken by another helper; serving {} only",
            old.bus_name(),
            new.bus_name()
        ),
        Err(e) => eprintln!("telamon-system-helper: cannot take {}: {e}", old.bus_name()),
    }
    // announce changes of the Progress property, at most ~4 a second
    let new_iface = conn
        .object_server()
        .interface::<_, Service>(new.object_path())
        .await?;
    let old_iface = conn
        .object_server()
        .interface::<_, LegacyService>(old.object_path())
        .await?;
    let announcer = tokio::spawn(super::live::announce_changes(progress, move || {
        let (new_iface, old_iface) = (new_iface.clone(), old_iface.clone());
        async move {
            let _ = new_iface
                .get()
                .await
                .progress_changed(new_iface.signal_emitter())
                .await;
            let _ = old_iface
                .get()
                .await
                .progress_changed(old_iface.signal_emitter())
                .await;
        }
    }));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let tick = (idle_timeout / 4).max(Duration::from_millis(50));
    let terminated = loop {
        tokio::select! {
            _ = term.recv() => break true,
            _ = tokio::time::sleep(tick) => {
                if activity.close_if_idle(idle_timeout, started) {
                    break false;
                }
            }
        }
    };
    activity.begin_close();
    if terminated {
        super::stop_retrying();
    }
    for name in names {
        let _ = conn.release_name(name).await;
    }
    let limit = if terminated {
        TERM_GRACE
    } else {
        Duration::from_secs(3600)
    };
    if !activity.wait_drained(limit).await {
        // still running after the grace period: ask bootc to stop
        super::terminate_running();
        activity.wait_drained(Duration::from_secs(10)).await;
    }
    announcer.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{BOOTED_WITH_UPDATE, PLAIN};
    use crate::helper::events;

    #[test]
    fn the_unit_the_helper_starts_is_the_new_one_and_the_package_has_it() {
        assert_eq!(DRIVERS_UNIT, "telamon-drivers.service");
        let unit = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/systemd")
            .join(DRIVERS_UNIT);
        assert!(unit.is_file(), "{}", unit.display());
    }

    #[test]
    fn closing_only_when_idle_and_then_refuses_calls() {
        let a = Arc::new(Activity::default());
        let start = Instant::now();
        assert!(!a.close_if_idle(Duration::from_secs(60), start));
        let g = a.enter(true).unwrap();
        assert!(!a.close_if_idle(Duration::ZERO, start), "call in flight");
        drop(g);
        assert!(a.close_if_idle(Duration::ZERO, start));
        assert!(a.enter(true).is_none(), "refused once closing");
    }

    #[test]
    fn status_calls_do_not_postpone_the_idle_exit() {
        let a = Arc::new(Activity::default());
        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(60));
        drop(a.enter(false).unwrap());
        assert!(a.close_if_idle(Duration::from_millis(50), start));
        let b = Arc::new(Activity::default());
        drop(b.enter(true).unwrap());
        assert!(!b.close_if_idle(Duration::from_millis(50), start));
    }

    struct Fixed(&'static str);
    impl BootcRunner for Fixed {
        fn run(&self, _args: &[&str]) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    #[tokio::test]
    async fn service_path_records_events() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("events.jsonl");
        let s = Service::new(Arc::new(Fixed(BOOTED_WITH_UPDATE)), Some(p.clone()));
        s.run(Op::Rollback).await.unwrap();
        s.run(Op::Status).await.unwrap();
        let names: Vec<_> = events::read(&p).into_iter().map(|e| e.event).collect();
        assert_eq!(names, ["rollback-requested"]);
        // without an events path nothing is written
        let s = Service::new(Arc::new(Fixed(PLAIN)), None);
        s.run(Op::Rollback).await.unwrap();
    }

    struct Gate {
        started: Mutex<std::sync::mpsc::Sender<()>>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl BootcRunner for Gate {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            if args == ["upgrade"] {
                lock(&self.started).send(()).unwrap();
                lock(&self.release).recv().unwrap();
            }
            Ok(PLAIN.to_string())
        }
    }

    #[tokio::test]
    async fn status_is_answered_while_an_upgrade_runs() {
        let (st_tx, st_rx) = std::sync::mpsc::channel();
        let (rel_tx, rel_rx) = std::sync::mpsc::channel();
        let s = Arc::new(Service::new(
            Arc::new(Gate {
                started: Mutex::new(st_tx),
                release: Mutex::new(rel_rx),
            }),
            None,
        ));
        let s2 = s.clone();
        let up = tokio::spawn(async move { s2.run(Op::Upgrade).await });
        tokio::task::spawn_blocking(move || st_rx.recv().unwrap())
            .await
            .unwrap();
        for _ in 0..5 {
            assert!(
                s.run(Op::Status).await.is_ok(),
                "Status must not wait for or collide with Upgrade"
            );
        }
        assert!(matches!(
            s.run(Op::Rollback).await,
            Err(HelperError::Busy(_))
        ));
        // the upgrade still counts as in flight, so the helper does not exit
        assert!(!s.activity.close_if_idle(Duration::ZERO, Instant::now()));
        rel_tx.send(()).unwrap();
        up.await.unwrap().unwrap();
        assert!(s.activity.wait_drained(Duration::from_secs(1)).await);
        assert!(s.activity.close_if_idle(Duration::ZERO, Instant::now()));
        assert!(matches!(
            s.run(Op::Status).await,
            Err(HelperError::ShuttingDown(_))
        ));
    }

    #[tokio::test]
    async fn guard_lives_until_the_worker_finishes_even_if_the_caller_is_dropped() {
        let (st_tx, st_rx) = std::sync::mpsc::channel();
        let (rel_tx, rel_rx) = std::sync::mpsc::channel();
        let s = Arc::new(Service::new(
            Arc::new(Gate {
                started: Mutex::new(st_tx),
                release: Mutex::new(rel_rx),
            }),
            None,
        ));
        let s2 = s.clone();
        let up = tokio::spawn(async move { s2.run(Op::Upgrade).await });
        tokio::task::spawn_blocking(move || st_rx.recv().unwrap())
            .await
            .unwrap();
        up.abort(); // the handler future is dropped, bootc keeps running
        let _ = up.await;
        assert!(!s.activity.wait_drained(Duration::from_millis(200)).await);
        rel_tx.send(()).unwrap();
        assert!(s.activity.wait_drained(Duration::from_secs(5)).await);
    }

    // ---- the two identities, on a private bus ----

    use crate::helper_client::HelperClient;
    use crate::testbus::{TestBus, wait_for_owner};

    /// Records every action it is asked about; answers `allow`.
    struct Recorder {
        asked: Mutex<Vec<String>>,
        allow: bool,
    }

    impl Recorder {
        fn new(allow: bool) -> Arc<Recorder> {
            Arc::new(Recorder {
                asked: Mutex::default(),
                allow,
            })
        }

        fn asked(&self) -> Vec<String> {
            lock(&self.asked).clone()
        }
    }

    impl Authorizer for Recorder {
        fn check<'a>(
            &'a self,
            _conn: &'a zbus::Connection,
            sender: Option<&'a str>,
            action: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), HelperError>> + Send + 'a>> {
            Box::pin(async move {
                assert!(sender.is_some_and(|s| s.starts_with(':')), "{sender:?}");
                lock(&self.asked).push(action.to_string());
                if self.allow {
                    Ok(())
                } else {
                    Err(HelperError::NotAuthorized(format!(
                        "not authorized for {action}"
                    )))
                }
            })
        }
    }

    /// Serves a helper on `bus` with `auth` and waits until both names are taken.
    async fn serve_on(
        bus: &TestBus,
        auth: Arc<Recorder>,
    ) -> (tokio::task::JoinHandle<zbus::Result<()>>, zbus::Connection) {
        let service = Service::new(Arc::new(Fixed(PLAIN)), None).with_authorizer(auth);
        let task = tokio::spawn(serve(bus.builder(), service, Duration::from_secs(300)));
        let client = bus.connect().await;
        for id in Identity::ALL {
            wait_for_owner(&client, id.bus_name()).await;
        }
        (task, client)
    }

    /// The name of the D-Bus error a raw call to `identity` ends in.
    async fn error_name(
        conn: &zbus::Connection,
        identity: Identity,
        method: &str,
        body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) -> String {
        match conn
            .call_method(
                Some(identity.bus_name()),
                identity.object_path(),
                Some(identity.interface()),
                method,
                body,
            )
            .await
        {
            Err(zbus::Error::MethodError(name, ..)) => name.to_string(),
            other => panic!("{method} through {identity:?}: {other:?}"),
        }
    }

    #[tokio::test]
    async fn each_identity_asks_polkit_about_its_own_action_and_fails_under_its_own_name() {
        let Some(bus) = TestBus::start(&[]) else {
            return;
        };
        let auth = Recorder::new(false);
        let (task, conn) = serve_on(&bus, auth.clone()).await;
        for id in Identity::ALL {
            let p = id.action_prefix();
            let e = id.error_prefix();
            for (method, suffix) in [
                ("Status", "status"),
                ("CheckForUpdate", "check"),
                ("Upgrade", "upgrade"),
                ("Rollback", "rollback"),
                ("CancelRollback", "rollback"),
            ] {
                assert_eq!(
                    error_name(&conn, id, method, &()).await,
                    format!("{e}.NotAuthorized"),
                    "{method}"
                );
                assert_eq!(auth.asked().pop().unwrap(), format!("{p}.{suffix}"));
            }
            assert_eq!(
                error_name(&conn, id, "SwitchChannel", &("stable",)).await,
                format!("{e}.NotAuthorized")
            );
            assert_eq!(auth.asked().pop().unwrap(), format!("{p}.switch-channel"));
            // a bad argument is refused before anyone is asked
            let before = auth.asked().len();
            assert_eq!(
                error_name(&conn, id, "SwitchChannel", &("nightly",)).await,
                format!("{e}.InvalidArgument")
            );
            assert_eq!(auth.asked().len(), before);
        }
        // nothing but those: the old identity has exactly the six methods too
        for id in Identity::ALL {
            let xml = conn
                .call_method(
                    Some(id.bus_name()),
                    id.object_path(),
                    Some("org.freedesktop.DBus.Introspectable"),
                    "Introspect",
                    &(),
                )
                .await
                .unwrap()
                .body()
                .deserialize::<String>()
                .unwrap();
            let iface = xml
                .split(&format!("<interface name=\"{}\">", id.interface()))
                .nth(1)
                .and_then(|r| r.split("</interface>").next())
                .unwrap();
            let mut methods: Vec<_> = iface
                .split("<method name=\"")
                .skip(1)
                .map(|m| m.split('"').next().unwrap())
                .collect();
            methods.sort_unstable();
            assert_eq!(
                methods,
                [
                    "CancelRollback",
                    "CheckForUpdate",
                    "Rollback",
                    "Status",
                    "SwitchChannel",
                    "Upgrade"
                ],
                "{id:?}"
            );
            assert!(iface.contains("<property name=\"Progress\""), "{id:?}");
        }
        task.abort();
    }

    #[tokio::test]
    async fn both_identities_answer_with_the_same_implementation() {
        let Some(bus) = TestBus::start(&[]) else {
            return;
        };
        let auth = Recorder::new(true);
        let (task, conn) = serve_on(&bus, auth.clone()).await;
        for id in Identity::ALL {
            let client = HelperClient::with_identity(&conn, id).await.unwrap();
            assert_eq!(client.identity(), id);
            client.status().await.unwrap();
            assert_eq!(client.progress().await.unwrap(), None, "{id:?}");
        }
        assert_eq!(
            auth.asked()
                .iter()
                .filter(|a| a.ends_with(".system.status"))
                .cloned()
                .collect::<Vec<_>>(),
            [
                "net.eterneon.telamon.system.status",
                "net.eterneon.atlas.system.status"
            ]
        );
        // the client picks the new identity when both are there
        let client = HelperClient::with_connection(&conn).await.unwrap();
        assert_eq!(client.identity(), Identity::Telamon);
        task.abort();
    }

    #[tokio::test]
    async fn an_old_helper_holding_the_old_name_leaves_the_new_one_served() {
        let Some(bus) = TestBus::start(&[]) else {
            return;
        };
        // a helper from before the rename: only the old name
        let old = bus.connect().await;
        old.request_name(Identity::Legacy.bus_name()).await.unwrap();
        let service =
            Service::new(Arc::new(Fixed(PLAIN)), None).with_authorizer(Recorder::new(true));
        let task = tokio::spawn(serve(bus.builder(), service, Duration::from_secs(300)));
        let conn = bus.connect().await;
        wait_for_owner(&conn, Identity::Telamon.bus_name()).await;
        let client = HelperClient::with_connection(&conn).await.unwrap();
        assert_eq!(client.identity(), Identity::Telamon);
        assert!(client.status().await.is_ok());
        task.abort();
    }
}
