//! Shared library for Atlas apps on AtlasOS.
//!
//! - [`bootc`]: types for `bootc status --json` and the channel tag rewrite.
//! - [`helper_client`]: async client for the `atlas-system-helper` D-Bus service.
//! - [`history`]: the list of versions this machine has booted.
//! - [`progress`]: the live progress of an update and the parsers behind it.
//! - `flatpak` (cargo feature `flatpak`): Flatpak update listing and updating.
//! - [`crash`]: opt-in crash reports, the only telemetry Atlas apps may have.
//!
//! The crate also builds the `atlas-system-helper` binary; its logic lives in
//! [`helper`] so it can be unit tested.

pub mod bootc;
pub mod crash;
#[cfg(feature = "flatpak")]
pub mod flatpak;
mod fsutil;
pub mod helper;
pub mod helper_client;
pub mod history;
pub mod progress;
