//! What a worker (`telamon-updater --worker <job>`) prints for the tray: one
//! line of JSON on stdout. The tray never runs Flatpak itself, so libflatpak
//! is only ever loaded for the length of a job.

use serde::{Deserialize, Serialize};

pub const APPS_ROUND: &str = "apps-round";
pub const APPS_UPDATE: &str = "apps-update";
/// `Notified` in `[AppUpdates]`: the key of the last app notice shown. The
/// tray writes it once the notice is on screen.
pub const APPS_NOTIFIED: &str = "Notified";
/// The longest a job may run (hung Flatpak or network): it is then ended,
/// which also frees the app update lock.
pub const TIME_LIMIT_SECS: u32 = 6 * 60 * 60;

/// An app notice to show (each set once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub text: String,
    /// What to save as [`APPS_NOTIFIED`] once shown; `None`: nothing.
    #[serde(default)]
    pub key: Option<String>,
    /// "Update Apps" may install from the notification: nothing asks for new
    /// permissions, and every update was checked.
    pub can_update: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// Another app operation held the lock: nothing ran.
    #[serde(default)]
    pub busy: bool,
    /// The round (or update) failed: try again later, later each time.
    #[serde(default)]
    pub failed: bool,
    /// The round waited (battery, metered network): try again in a while.
    #[serde(default)]
    pub waited: bool,
    #[serde(default)]
    pub notice: Option<Notice>,
}

impl Outcome {
    /// The last line of `stdout` that parses. `None`: the worker died first.
    pub fn parse(stdout: &str) -> Option<Outcome> {
        stdout
            .lines()
            .rev()
            .find_map(|l| serde_json::from_str(l.trim()).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_takes_the_last_line() {
        let o = Outcome {
            waited: true,
            notice: Some(Notice {
                text: "Firefox".into(),
                key: Some("k".into()),
                can_update: true,
            }),
            ..Default::default()
        };
        let line = serde_json::to_string(&o).unwrap();
        assert_eq!(Outcome::parse(&format!("noise\n{line}\n")), Some(o));
        assert_eq!(Outcome::parse(""), None);
        assert_eq!(Outcome::parse("not json"), None);
        assert_eq!(Outcome::parse("{}"), Some(Outcome::default()));
    }
}
