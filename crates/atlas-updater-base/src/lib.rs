//! What Atlas Updater's two processes share. The window (`atlas-updater`,
//! Qt) runs the user's operations; the tray (`atlas-updater-tray`, no Qt)
//! stays resident for the schedule, the restart and the notifications.
//! Nothing here links Qt or libflatpak.

pub mod config;
pub mod crash;
pub mod errors;
pub mod fwupd;
pub mod lock;
pub mod ops;
pub mod rc;
pub mod restart;
pub mod schedule;
pub mod tray;
pub mod view;
pub mod worker;

/// Desktop notifications: the framework's sender (feature `notify`).
pub use atlas_framework_system::notify;

/// Sends notifications under the app's names, as atlas-updater.notifyrc
/// declares them.
pub fn notifier() -> notify::Notifier {
    notify::Notifier::new(&crash::app_info())
}

#[cfg(test)]
mod tests {
    /// Plasma files the notifications by these names: they must stay what
    /// atlas-updater.notifyrc and the desktop file are called.
    #[test]
    fn notifications_go_out_under_the_apps_names() {
        let n = super::notifier();
        assert_eq!(n.component(), "atlas-updater");
        assert_eq!(n.desktop_entry(), "net.eterneon.atlas.updater");
        assert_eq!(n.app_name(), "Atlas Updater");
        assert_eq!(n.app_icon(), "net.eterneon.atlas.updater");
    }
}
