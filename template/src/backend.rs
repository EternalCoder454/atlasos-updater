//! One example QObject. Copy it, rename it and add your own properties and
//! invokables. Never block the GUI thread: run slow work on a thread and post
//! the result back with `qt_thread().queue(..)`.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qproperty(QString, status)]
        #[qproperty(bool, busy)]
        #[namespace = "atlas_app"]
        type Backend = super::BackendRust;

        /// Example invokable: does some work on a worker thread.
        #[qinvokable]
        fn refresh(self: Pin<&mut Backend>);
    }

    // Lets worker threads post closures back to the Qt thread.
    impl cxx_qt::Threading for Backend {}

    // Lets Rust create the object (see `atlas_backend_new` in lib.rs).
    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn backend_make_unique() -> UniquePtr<Backend>;
    }
}

use core::pin::Pin;
use cxx_qt::Threading;
use cxx_qt_lib::QString;

pub struct BackendRust {
    status: QString,
    busy: bool,
}

impl Default for BackendRust {
    fn default() -> Self {
        Self {
            status: QString::from("Ready"),
            busy: false,
        }
    }
}

impl qobject::Backend {
    pub fn refresh(mut self: Pin<&mut Self>) {
        if *self.busy() {
            return;
        }
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from("Working…"));
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            // Replace with a call into atlas-core (helper_client, history, ...).
            let text = format!("atlas-core {}", env!("CARGO_PKG_VERSION"));
            let _ = qt.queue(move |mut obj| {
                obj.as_mut().set_status(QString::from(text.as_str()));
                obj.as_mut().set_busy(false);
            });
        });
    }
}
