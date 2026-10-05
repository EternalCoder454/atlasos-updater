//! The Updater's own settings, `~/.config/atlas-updaterrc` (KConfig INI
//! format): the scheduled restart, the last check, what the user was already
//! told about, background app updates. The window and the tray both use it;
//! the file handling is the framework's [`Settings`]: locked, atomic and
//! KConfig-compatible.

use atlas_framework_core::settings::{Settings, config_dir};

/// `ScheduledAt`: Unix seconds of the scheduled restart.
pub const RESTART: &str = "Restart";
/// `StagedDigest`: the staged update the user was told about.
pub const NOTIFIED: &str = "Notified";
/// `At`: the last check for an OS update.
pub const CHECKED: &str = "Checked";
/// `Automatic` (`true`/`false`): background app updates; `Notified`: the
/// last app notice's key; `RoundError`: what went wrong in the background,
/// for the window to show until something works again.
pub const APPS: &str = "AppUpdates";
/// `Notified`: the key of the firmware set the user was last told about
/// (`fwupd::notice_key`); removed when no firmware update waits.
pub const FIRMWARE: &str = "Firmware";

fn settings() -> Settings {
    Settings::at(config_dir().join("atlas-updaterrc"))
}

pub fn get(group: &str, key: &str) -> Option<String> {
    settings().get(group, key)
}

/// `None` removes the key. A failed write is ignored: these values are
/// conveniences, and the app works without them.
pub fn set(group: &str, key: &str, value: Option<&str>) {
    let _ = try_set(group, key, value);
}

/// [`set`] for a choice the user made: the error says why it wasn't saved.
pub fn try_set(group: &str, key: &str, value: Option<&str>) -> std::io::Result<()> {
    settings().set(group, key, value)
}

/// The scheduled restart, if one is saved.
pub fn scheduled_at() -> Option<i64> {
    get(RESTART, "ScheduledAt").and_then(|v| v.trim().parse::<i64>().ok())
}

/// Whether the user turned background app updates on (off by default).
pub fn apps_automatic() -> bool {
    get(APPS, "Automatic").as_deref() == Some("true")
}
