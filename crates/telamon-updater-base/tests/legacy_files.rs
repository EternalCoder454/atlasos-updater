//! What the tray and the worker see when the user's files are still under the
//! names from before 0.3.0: they get the values, the files end up under the
//! new names, and nothing is written to the old ones.
//!
//! One test in this binary: it sets the environment before anything else runs.

use std::fs;
use std::path::Path;

use telamon_updater_base::migrate::{self, Dirs};
use telamon_updater_base::rc;

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn settings_and_state_from_the_old_names_are_read_and_then_live_under_the_new_ones() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    // SAFETY: the only test of this binary, and nothing else has started a
    // thread yet.
    unsafe {
        std::env::set_var("HOME", h);
        std::env::set_var("XDG_CONFIG_HOME", h.join(".config"));
        std::env::set_var("XDG_STATE_HOME", h.join(".local/state"));
        std::env::set_var("XDG_CACHE_HOME", h.join(".cache"));
    }
    let (config, state) = (h.join(".config"), h.join(".local/state"));
    put(
        &config.join("atlas-updaterrc"),
        "[Atlas]\nFormat=1\n\n[Restart]\nScheduledAt=4102444800\n\n[AppUpdates]\nAutomatic=true\nNotified=old-key\n",
    );
    put(
        &state.join("atlas-updater/app-updates.jsonl"),
        "{\"at\":1}\n",
    );

    // the tray's first questions (it also calls migrate at start-up)
    assert_eq!(rc::scheduled_at(), Some(4_102_444_800));
    assert!(rc::apps_automatic());
    assert_eq!(rc::get(rc::APPS, "Notified").as_deref(), Some("old-key"));

    // moved, not copied
    assert!(!config.join("atlas-updaterrc").exists());
    assert!(config.join("telamon-updaterrc").exists());
    assert!(!state.join("atlas-updater").exists());
    assert_eq!(
        fs::read_to_string(state.join("telamon-updater/app-updates.jsonl")).unwrap(),
        "{\"at\":1}\n"
    );

    // a worker's write goes to the new file, and the old name stays absent
    rc::set(rc::APPS, "Notified", Some("new-key"));
    rc::set(rc::RESTART, "ScheduledAt", None);
    assert_eq!(rc::get(rc::APPS, "Notified").as_deref(), Some("new-key"));
    assert_eq!(rc::scheduled_at(), None);
    assert!(!config.join("atlas-updaterrc").exists());
    assert!(!config.join(".atlas-updaterrc.lock").exists());
    let text = fs::read_to_string(config.join("telamon-updaterrc")).unwrap();
    assert!(
        text.contains("Automatic=true") && text.contains("new-key"),
        "{text}"
    );

    // an older tray, still running, writes the old name again: it is left
    // alone, and what the new name holds is never replaced by it
    put(
        &config.join("atlas-updaterrc"),
        "[Restart]\nScheduledAt=1\n",
    );
    let r = migrate::adopt(&Dirs::from_env());
    assert!(r.moved.is_empty(), "{r:?}");
    assert_eq!(rc::scheduled_at(), None);
    assert_eq!(
        fs::read_to_string(config.join("atlas-updaterrc")).unwrap(),
        "[Restart]\nScheduledAt=1\n"
    );
}
