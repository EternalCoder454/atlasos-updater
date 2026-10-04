//! The Updater's own settings, `~/.config/atlas-updaterrc` (KConfig INI
//! format): the scheduled restart, the last check and the staged update we
//! already told the user about. The file handling is the framework's
//! [`Settings`]: locked, atomic and KConfig-compatible.

use atlas_framework_core::settings::{Settings, config_dir};

fn settings() -> Settings {
    Settings::at(config_dir().join("atlas-updaterrc"))
}

pub fn get(group: &str, key: &str) -> Option<String> {
    settings().get(group, key)
}

/// `None` removes the key. A failed write is ignored: these values are
/// conveniences, and the app works without them.
pub fn set(group: &str, key: &str, value: Option<&str>) {
    let _ = settings().set(group, key, value);
}
