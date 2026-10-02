//! Rust side of the app. C++ (`main.cpp`) only starts Qt and loads the QML;
//! everything else lives here as QObjects exposed to QML.

mod backend;

use std::ffi::c_void;

/// Called once from `main.cpp`. Returns the `Backend` QObject, which C++ hands
/// to the QML engine. Ownership passes to the caller (a QObject with no parent).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_backend_new() -> *mut c_void {
    backend::qobject::backend_make_unique().into_raw().cast()
}
