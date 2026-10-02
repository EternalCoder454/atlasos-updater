//! D-Bus glue: the `SystemHelper1` interface, polkit checks and the idle exit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::message::Header;
use zbus::zvariant::Value;

use super::{BootcRunner, Core, HelperError, Op, lock};
use crate::helper_client::{BUS_NAME, OBJECT_PATH};

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

pub struct Service {
    core: Arc<Core>,
    activity: Arc<Activity>,
}

const SHUTTING_DOWN: &str = "the helper is shutting down, try again";

impl Service {
    /// `events` is where update and rollback events go (`None` for none).
    pub fn new(runner: Arc<dyn BootcRunner>, events: Option<PathBuf>) -> Service {
        let mut core = Core::new(runner);
        if let Some(p) = events {
            core = core.with_events(p);
        }
        Service {
            core: Arc::new(core),
            activity: Arc::new(Activity::default()),
        }
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

    /// Run `op` without a polkit check (tests).
    pub async fn run(&self, op: Op) -> Result<String, HelperError> {
        let guard = self.begin(&op)?;
        self.execute(op, guard).await
    }

    async fn handle(
        &self,
        header: &Header<'_>,
        conn: &zbus::Connection,
        op: Op,
    ) -> Result<String, HelperError> {
        // count the call while polkit is asked, so we do not exit under it
        let guard = self.begin(&op)?;
        op.validate()?;
        let sender = header
            .sender()
            .ok_or_else(|| HelperError::NotAuthorized("call has no sender".into()))?
            .to_string();
        authorize(conn, &sender, op.action_id()).await?;
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

#[zbus::interface(name = "net.eterneon.atlas.SystemHelper1")]
impl Service {
    async fn status(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::Status).await
    }

    async fn check_for_update(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::CheckForUpdate).await
    }

    async fn upgrade(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::Upgrade).await
    }

    async fn rollback(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::Rollback).await
    }

    async fn switch_channel(
        &self,
        channel: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        self.handle(&header, conn, Op::SwitchChannel(channel)).await
    }
}

/// How long a running bootc may finish after SIGTERM before we ask it to
/// stop (the unit gives the helper 60 s in all).
const TERM_GRACE: Duration = Duration::from_secs(40);

/// Serve until idle for `idle_timeout` or SIGTERM. Either way: refuse new
/// calls, release the bus name (so D-Bus starts a fresh helper for the next
/// call), then wait for calls in flight and return.
pub async fn serve(
    builder: zbus::connection::Builder<'static>,
    service: Service,
    idle_timeout: Duration,
) -> zbus::Result<()> {
    let activity = service.activity.clone();
    let started = Instant::now();
    let conn = builder
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()
        .await?;
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
    let _ = conn.release_name(BUS_NAME).await;
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootc::fixtures::{BOOTED_WITH_UPDATE, PLAIN};
    use crate::helper::events;

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
}
