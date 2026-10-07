//! Blocking operations the backend runs on worker threads.

use std::path::Path;

use telamon_framework_system::bootc::{Channel, Status};
use atlas_update_engine::helper_client::HelperClient;
use atlas_update_engine::progress::Progress;

use crate::config;
use crate::errors::{self, OpError};

#[derive(Debug, Clone)]
pub enum Op {
    Status,
    Check,
    Upgrade,
    Rollback,
    CancelRollback,
    Switch(Channel),
}

impl Op {
    /// The name QML knows this operation by (`busyOp`, `errorOp`).
    pub fn name(&self) -> &'static str {
        match self {
            Op::Status => "status",
            Op::Check => "check",
            Op::Upgrade => "download",
            Op::Rollback => "rollback",
            Op::CancelRollback => "cancelRollback",
            Op::Switch(_) => "switch",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Op::Status => "Reading the system state…",
            Op::Check => "Checking for updates…",
            Op::Upgrade => "Downloading the update…",
            Op::Rollback => "Going back to the previous version…",
            Op::CancelRollback => "Canceling the rollback…",
            Op::Switch(_) => "Switching channel…",
        }
    }
}

/// Called with the helper's progress while an upgrade or switch runs, and
/// with `None` when it goes back to nothing.
pub type OnProgress = Box<dyn Fn(Option<Progress>) + Send + 'static>;

/// Talks to the system helper, or reads `status.json` under the fixtures switch.
/// A short-lived current-thread runtime and a fresh connection per call: nothing
/// stays allocated while the app sits idle in the tray.
pub fn run(op: &Op, fixtures: Option<&Path>, on_progress: OnProgress) -> Result<Status, OpError> {
    if let Some(dir) = fixtures {
        return fixture_status(op, dir, &on_progress);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| OpError::Message(format!("Could not start a worker: {e}")))?;
    rt.block_on(async {
        let client = HelperClient::connect()
            .await
            .map_err(|e| errors::friendly(&e))?;
        // Subscribed before the call starts, so no early update is missed.
        // Progress is extra: without it the bar is simply indeterminate.
        let follow = match op {
            Op::Upgrade | Op::Switch(_) => client.progress_changes().await.ok().map(|mut s| {
                tokio::spawn(async move {
                    while let Some(p) = std::future::poll_fn(|cx| {
                        zbus::export::futures_core::Stream::poll_next(
                            std::pin::Pin::new(&mut s),
                            cx,
                        )
                    })
                    .await
                    {
                        on_progress(p);
                    }
                })
            }),
            _ => None,
        };
        let st = match op {
            Op::Status => client.status().await,
            Op::Check => client.check_for_update().await,
            Op::Upgrade => client.upgrade().await,
            Op::Rollback => client.rollback().await,
            Op::CancelRollback => client.cancel_rollback().await,
            Op::Switch(c) => client.switch_channel(*c).await,
        };
        if let Some(task) = follow {
            task.abort();
        }
        st.map_err(|e| errors::friendly(&e))
    })
}

fn fixture_status(op: &Op, dir: &Path, on_progress: &OnProgress) -> Result<Status, OpError> {
    // Ops with a visible effect read their own file when it exists, so a
    // fixture directory can show "before" and "after".
    let name = match op {
        Op::Status | Op::Check => "status.json",
        Op::Upgrade => "status-after-upgrade.json",
        Op::Rollback => "status-after-rollback.json",
        Op::CancelRollback => "status-after-cancel-rollback.json",
        Op::Switch(_) => "status-after-switch.json",
    };
    let text = config::read_fixture(dir, name)
        .or_else(|| config::read_fixture(dir, "status.json"))
        .ok_or_else(|| OpError::Message("No status.json in the fixtures directory.".into()))?;
    if let Some(msg) = text.strip_prefix("ERROR:") {
        return Err(OpError::Message(msg.trim().to_string()));
    }
    // Screenshot hook: stay busy until the app quits, showing the progress
    // in `progress.json` if there is one.
    if config::fixture_hold(op.name()) {
        if let Some(p) = config::read_fixture(dir, "progress.json")
            .and_then(|t| serde_json::from_str::<Progress>(&t).ok())
        {
            on_progress(Some(p));
        }
        config::hold_forever();
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    let mut st =
        Status::from_json(&text).map_err(|e| OpError::Message(format!("Bad fixture: {e}")))?;
    st.bad_image_digests =
        telamon_framework_system::bootc::bad_image_digests(&dir.join("bad-image-digests"));
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_names_are_the_ones_qml_uses() {
        let names: Vec<_> = [
            Op::Status,
            Op::Check,
            Op::Upgrade,
            Op::Rollback,
            Op::CancelRollback,
            Op::Switch(Channel::Stable),
        ]
        .iter()
        .map(Op::name)
        .collect();
        assert_eq!(
            names,
            [
                "status",
                "check",
                "download",
                "rollback",
                "cancelRollback",
                "switch"
            ]
        );
    }
}
