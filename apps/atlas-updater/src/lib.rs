//! Atlas Updater, Rust side. `cpp/` holds the thin Qt glue (window
//! lifecycle); every QObject QML talks to is in `backend.rs`. The panel icon,
//! the schedule and the notifications are atlas-updater-tray's; app rounds it
//! runs in this program as `--worker` (`worker.rs`), without Qt.

mod apphistory;
mod apps;
mod backend;
mod changelog;
mod crash;
mod firmware;
mod notes;
mod power;
mod worker;

pub use atlas_updater_base::{
    config, errors, lock, notify, ops, rc, restart, schedule, tray, view,
};

use std::ffi::c_void;

/// Called once from `main.cpp`. Returns the `Backend` QObject (no parent;
/// the caller owns it).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_backend_new() -> *mut c_void {
    backend::qobject::backend_make_unique().into_raw().cast()
}
