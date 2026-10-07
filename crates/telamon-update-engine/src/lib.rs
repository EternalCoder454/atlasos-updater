//! Telamon Updater's update engine: the root `telamon-system-helper` and its
//! clients.
//!
//! - [`helper`]: the helper's logic, here so it can be unit tested; the
//!   crate builds the `telamon-system-helper` binary from it.
//! - [`helper_client`]: async client for the `telamon-system-helper` D-Bus service.
//! - [`progress`]: the live progress of an update and the parsers behind it.
//!
//! What every Telamon app shares (bootc status, history, events, crash reports,
//! os-release, Flatpak) is in the Telamon framework
//! (github.com/EternalCoder454/atlas-framework); use its crates directly.

pub mod helper;
pub mod helper_client;
pub mod identity;
pub mod progress;

#[cfg(test)]
mod testbus;

/// `bootc status --json` samples for the helper's tests.
#[cfg(test)]
pub(crate) mod fixtures {
    pub const BOOTED_WITH_UPDATE: &str = include_str!("../tests/fixtures/status-update.json");
    pub const PLAIN: &str = include_str!("../tests/fixtures/status-plain.json");
    pub const NOT_BOOTC: &str = include_str!("../tests/fixtures/status-container.json");
}
