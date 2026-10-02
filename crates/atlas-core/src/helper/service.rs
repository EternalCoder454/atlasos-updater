//! D-Bus glue: the `SystemHelper1` interface, polkit checks and the idle exit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::message::Header;
use zbus::zvariant::Value;

use super::{BootcRunner, Core, HelperError, Op};
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

/// Logind, for the shutdown delay inhibitor.
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    gen_blocking = false
)]
trait Login1Manager {
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;
}

/// Hold a "delay" shutdown inhibitor (best effort) until dropped, so a
/// shutdown waits for a staging operation instead of killing it half way.
async fn inhibit_shutdown(conn: &zbus::Connection) -> Option<zbus::zvariant::OwnedFd> {
    Login1ManagerProxy::new(conn)
        .await
        .ok()?
        .inhibit(
            "shutdown",
            "Atlas system helper",
            "Staging an operating system change",
            "delay",
        )
        .await
        .ok()
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
        let mut st = self.0.lock().unwrap();
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
        let mut st = self.0.lock().unwrap();
        if st.active == 0 && st.last.unwrap_or(started).elapsed() >= timeout {
            st.closing = true;
        }
        st.closing
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut st = self.activity.0.lock().unwrap();
        st.active -= 1;
        if self.touch {
            st.last = Some(Instant::now());
        }
    }
}

pub struct Service {
    core: Arc<Core>,
    activity: Arc<Activity>,
}

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

    /// Run an already authorized operation on a worker thread.
    pub async fn run(
        &self,
        op: Op,
        inhibitor: Option<&zbus::Connection>,
    ) -> Result<String, HelperError> {
        let _active = self
            .activity
            .enter(op != Op::Status)
            .ok_or_else(|| HelperError::Busy("the helper is shutting down, try again".into()))?;
        op.validate()?;
        let _inhibit = match inhibitor.filter(|_| op.changes_system()) {
            Some(c) => inhibit_shutdown(c).await,
            None => None,
        };
        let core = self.core.clone();
        tokio::task::spawn_blocking(move || core.execute(&op))
            .await
            .map_err(|e| HelperError::Failed(format!("worker failed: {e}")))?
    }

    async fn handle(
        &self,
        header: &Header<'_>,
        conn: &zbus::Connection,
        op: Op,
    ) -> Result<String, HelperError> {
        // count the call while polkit is asked, so we do not exit under it
        let _active = self
            .activity
            .enter(op != Op::Status)
            .ok_or_else(|| HelperError::Busy("the helper is shutting down, try again".into()))?;
        op.validate()?;
        let sender = header
            .sender()
            .ok_or_else(|| HelperError::NotAuthorized("call has no sender".into()))?
            .to_string();
        authorize(conn, &sender, op.action_id()).await?;
        self.run(op, Some(conn)).await
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

/// Serve until idle for `idle_timeout`, then release the bus name and
/// return, so D-Bus starts a fresh helper for the next call.
pub async fn serve(
    builder: zbus::connection::Builder<'static>,
    runner: Arc<dyn BootcRunner>,
    idle_timeout: Duration,
    events: Option<PathBuf>,
) -> zbus::Result<()> {
    let service = Service::new(runner, events);
    let activity = service.activity.clone();
    let started = Instant::now();
    let conn = builder
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()
        .await?;
    let tick = (idle_timeout / 4).max(Duration::from_millis(50));
    loop {
        tokio::time::sleep(tick).await;
        if activity.close_if_idle(idle_timeout, started) {
            let _ = conn.release_name(BUS_NAME).await;
            return Ok(());
        }
    }
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
        s.run(Op::Rollback, None).await.unwrap();
        s.run(Op::Status, None).await.unwrap();
        let names: Vec<_> = events::read(&p).into_iter().map(|e| e.event).collect();
        assert_eq!(names, ["rollback-requested"]);
        // without an events path nothing is written
        let s = Service::new(Arc::new(Fixed(PLAIN)), None);
        s.run(Op::Rollback, None).await.unwrap();
    }
}
