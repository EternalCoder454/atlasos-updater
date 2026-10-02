//! D-Bus glue: the `SystemHelper1` interface, polkit checks and the idle exit.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// Tracks calls in flight and the time of the last one.
#[derive(Default)]
pub struct Activity {
    active: AtomicUsize,
    last: Mutex<Option<Instant>>,
}

pub struct ActivityGuard(Arc<Activity>);

impl Activity {
    pub fn enter(self: &Arc<Self>) -> ActivityGuard {
        self.active.fetch_add(1, Ordering::AcqRel);
        ActivityGuard(self.clone())
    }

    /// True when nothing is in flight and the last call ended `timeout` ago.
    pub fn idle_for(&self, timeout: Duration, since_start: Instant) -> bool {
        if self.active.load(Ordering::Acquire) > 0 {
            return false;
        }
        let last = self.last.lock().unwrap().unwrap_or(since_start);
        last.elapsed() >= timeout
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        *self.0.last.lock().unwrap() = Some(Instant::now());
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct Service {
    core: Arc<Core>,
    activity: Arc<Activity>,
}

impl Service {
    async fn handle(
        &self,
        header: &Header<'_>,
        conn: &zbus::Connection,
        op: Op,
    ) -> Result<String, HelperError> {
        let _active = self.activity.enter();
        op.validate()?;
        let sender = header
            .sender()
            .ok_or_else(|| HelperError::NotAuthorized("call has no sender".into()))?
            .to_string();
        authorize(conn, &sender, op.action_id()).await?;
        let core = self.core.clone();
        tokio::task::spawn_blocking(move || core.execute(&op))
            .await
            .map_err(|e| HelperError::Failed(format!("worker failed: {e}")))?
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

/// Serve on `connection_builder` until idle for [`IDLE_TIMEOUT`].
pub async fn serve(
    builder: zbus::connection::Builder<'static>,
    runner: Arc<dyn BootcRunner>,
    idle_timeout: Duration,
) -> zbus::Result<()> {
    let activity = Arc::new(Activity::default());
    let service = Service {
        core: Arc::new(Core::new(runner)),
        activity: activity.clone(),
    };
    let started = Instant::now();
    let _conn = builder
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()
        .await?;
    let tick = (idle_timeout / 4).max(Duration::from_millis(50));
    loop {
        tokio::time::sleep(tick).await;
        if activity.idle_for(idle_timeout, started) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_idle_only_when_nothing_in_flight() {
        let a = Arc::new(Activity::default());
        let start = Instant::now();
        assert!(a.idle_for(Duration::ZERO, start));
        assert!(!a.idle_for(Duration::from_secs(60), start));
        let g = a.enter();
        assert!(!a.idle_for(Duration::ZERO, start));
        drop(g);
        assert!(a.idle_for(Duration::ZERO, start));
        assert!(!a.idle_for(Duration::from_secs(60), start));
    }
}
