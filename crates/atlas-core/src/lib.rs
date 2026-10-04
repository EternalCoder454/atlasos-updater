//! Atlas Updater's update engine: the root `atlas-system-helper` and its
//! clients.
//!
//! - [`helper`]: the helper's logic, here so it can be unit tested; the
//!   crate builds the `atlas-system-helper` binary from it.
//! - [`helper_client`]: async client for the `atlas-system-helper` D-Bus service.
//! - [`progress`]: the live progress of an update and the parsers behind it.
//!
//! What every Atlas app shares lives in atlas-framework
//! (github.com/EternalCoder454/atlas-framework); it is re-exported here under
//! its old paths:
//!
//! - [`bootc`], [`history`], [`crash`] (and `helper::events`):
//!   atlas-framework-system.
//! - [`osrelease`] comes from atlas-framework-core.
//! - `flatpak` (cargo feature `flatpak`): atlas-framework-flatpak.

pub use atlas_framework_core::osrelease;
#[cfg(feature = "flatpak")]
pub use atlas_framework_flatpak as flatpak;
pub use atlas_framework_system::{bootc, crash, history};
pub mod helper;
pub mod helper_client;
pub mod progress;

/// `bootc status --json` samples for the helper's tests.
#[cfg(test)]
pub(crate) mod fixtures {
    pub const BOOTED_WITH_UPDATE: &str = include_str!("../tests/fixtures/status-update.json");
    pub const PLAIN: &str = include_str!("../tests/fixtures/status-plain.json");
    pub const NOT_BOOTC: &str = include_str!("../tests/fixtures/status-container.json");
}
