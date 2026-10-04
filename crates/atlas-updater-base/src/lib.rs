//! What Atlas Updater's two processes share. The window (`atlas-updater`,
//! Qt) runs the user's operations; the tray (`atlas-updater-tray`, no Qt)
//! stays resident for the schedule, the restart and the notifications.
//! Nothing here links Qt or libflatpak.

pub mod config;
pub mod crash;
pub mod errors;
pub mod lock;
pub mod notify;
pub mod ops;
pub mod rc;
pub mod restart;
pub mod schedule;
pub mod tray;
pub mod view;
pub mod worker;
