//! Atlas Updater, Rust side. `cpp/` holds the thin Qt glue (tray, notifications,
//! window lifecycle); every QObject QML talks to is in `backend.rs`.

mod apps;
mod backend;
mod config;
mod crash;
mod errors;
mod notes;
mod ops;
mod rc;
mod restart;
mod schedule;
mod view;

use std::ffi::c_void;

/// Called once from `main.cpp`. Returns the `Backend` QObject (no parent;
/// the caller owns it).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_backend_new() -> *mut c_void {
    backend::qobject::backend_make_unique().into_raw().cast()
}
