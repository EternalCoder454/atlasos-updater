//! Blocking operations the backend runs on worker threads.

use std::path::Path;

use atlas_core::bootc::{Channel, Status};
use atlas_core::helper_client::HelperClient;

use crate::config;
use crate::errors::{self, OpError};

#[derive(Debug, Clone)]
pub enum Op {
    Status,
    Check,
    Upgrade,
    Rollback,
    Switch(Channel),
}

impl Op {
    pub fn label(&self) -> &'static str {
        match self {
            Op::Status => "Reading the system state…",
            Op::Check => "Checking for updates…",
            Op::Upgrade => "Downloading the update…",
            Op::Rollback => "Going back to the previous version…",
            Op::Switch(_) => "Switching channel…",
        }
    }
}

/// Talks to the system helper, or reads `status.json` under the fixtures switch.
/// A short-lived current-thread runtime and a fresh connection per call: nothing
/// stays allocated while the app sits idle in the tray.
pub fn run(op: &Op, fixtures: Option<&Path>) -> Result<Status, OpError> {
    if let Some(dir) = fixtures {
        return fixture_status(op, dir);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| OpError::Message(format!("Could not start a worker: {e}")))?;
    rt.block_on(async {
        let client = HelperClient::connect()
            .await
            .map_err(|e| errors::friendly(&e))?;
        let st = match op {
            Op::Status => client.status().await,
            Op::Check => client.check_for_update().await,
            Op::Upgrade => client.upgrade().await,
            Op::Rollback => client.rollback().await,
            Op::Switch(c) => client.switch_channel(*c).await,
        };
        st.map_err(|e| errors::friendly(&e))
    })
}

fn fixture_status(op: &Op, dir: &Path) -> Result<Status, OpError> {
    // Ops with a visible effect read their own file when it exists, so a
    // fixture directory can show "before" and "after".
    let name = match op {
        Op::Status | Op::Check => "status.json",
        Op::Upgrade => "status-after-upgrade.json",
        Op::Rollback => "status-after-rollback.json",
        Op::Switch(_) => "status-after-switch.json",
    };
    let text = config::read_fixture(dir, name)
        .or_else(|| config::read_fixture(dir, "status.json"))
        .ok_or_else(|| OpError::Message("No status.json in the fixtures directory.".into()))?;
    if let Some(msg) = text.strip_prefix("ERROR:") {
        return Err(OpError::Message(msg.trim().to_string()));
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    Status::from_json(&text).map_err(|e| OpError::Message(format!("Bad fixture: {e}")))
}
