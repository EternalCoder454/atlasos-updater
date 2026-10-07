//! What Telamon Updater's background program and Telamon Settings' Updates
//! page (which was Updater's window) share, without Qt: Flatpak app updates,
//! firmware, release notes, the changelog, the history of app updates, power
//! and metered-connection checks, and the app update worker (`worker::run`,
//! `telamon-updater --worker <job>`). The system helper's client and the
//! progress parser are `telamon-update-engine`; the settings file, schedule,
//! restart, locks, the fwupd client and the tray's names are
//! `telamon-updater-base`, re-exported here.

pub mod apphistory;
pub mod apps;
pub mod changelog;
pub mod firmware;
pub mod notes;
pub mod power;
pub mod worker;

pub use telamon_update_engine as engine;
pub use telamon_updater_base as base;
pub use telamon_updater_base::{
    config, errors, lock, notify, ops, rc, restart, schedule, tray, view,
};
