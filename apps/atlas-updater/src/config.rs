//! `/etc/atlas-updater/updater.toml` and the developer fixtures switch.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const CONFIG_PATH: &str = "/etc/atlas-updater/updater.toml";
pub const DEFAULT_NOTES_URL: &str =
    "https://api.github.com/repos/EternalCoder454/AtlasOS/releases/tags/{version}";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `{version}` is replaced. `https://`, `http://` and `file://` work.
    pub release_notes_url: String,
}

#[derive(Deserialize, Default)]
struct FileConfig {
    release_notes_url: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            release_notes_url: DEFAULT_NOTES_URL.to_string(),
        }
    }
}

impl Config {
    /// A missing or broken file gives the defaults.
    pub fn load() -> Config {
        std::fs::read_to_string(CONFIG_PATH)
            .map(|t| Config::parse(&t))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Config {
        let file: FileConfig = toml::from_str(text).unwrap_or_default();
        let non_empty =
            |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Config {
            release_notes_url: non_empty(file.release_notes_url)
                .unwrap_or_else(|| DEFAULT_NOTES_URL.to_string()),
        }
    }
}

/// Hidden developer option: `ATLAS_UPDATER_FIXTURES=<dir>` makes the app read
/// `status.json`, `history.jsonl`, `notes.json`, `flatpak.json` and
/// `crash-pending.json`, `crash-sent.json` from that directory instead of D-Bus, the history file and the
/// network. Nothing else looks at it.
pub fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("ATLAS_UPDATER_FIXTURES")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn read_fixture(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::parse("");
        assert_eq!(c.release_notes_url, DEFAULT_NOTES_URL);
    }

    #[test]
    fn overrides() {
        let c = Config::parse("release_notes_url = \"file:///var/notes/{version}.json\"\n");
        assert_eq!(c.release_notes_url, "file:///var/notes/{version}.json");
    }

    #[test]
    fn broken_file_gives_defaults() {
        assert_eq!(Config::parse("this is = = not toml"), Config::default());
        assert_eq!(
            Config::parse("release_notes_url = \"  \""),
            Config::default()
        );
    }
}
