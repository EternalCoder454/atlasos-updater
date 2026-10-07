//! `/etc/telamon-updater/updater.toml` and the developer fixtures switch.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const CONFIG_PATH: &str = "/etc/telamon-updater/updater.toml";
/// Where the file was until 0.3.0: read when the new one is not there.
pub const LEGACY_CONFIG_PATH: &str = "/etc/atlas-updater/updater.toml";
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
        Config::load_from(&[CONFIG_PATH, LEGACY_CONFIG_PATH])
    }

    /// The first of `paths` that exists (a file that exists but can't be
    /// read counts: an administrator's file is not skipped over).
    pub fn load_from(paths: &[&str]) -> Config {
        for path in paths {
            match std::fs::read_to_string(path) {
                Ok(t) => return Config::parse(&t),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Config::default(),
            }
        }
        Config::default()
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

/// The developer variable `TELAMON_UPDATER_<name>`; when it is not set, the
/// name it had until 0.3.0, `ATLAS_UPDATER_<name>`. The new name wins.
pub fn env(name: &str) -> Option<OsString> {
    env_in(name, |k| std::env::var_os(k))
}

fn env_in(name: &str, get: impl Fn(&str) -> Option<OsString>) -> Option<OsString> {
    get(&format!("TELAMON_UPDATER_{name}")).or_else(|| get(&format!("ATLAS_UPDATER_{name}")))
}

/// Hidden developer option: `TELAMON_UPDATER_FIXTURES=<dir>` makes the app read
/// `status.json`, `history.jsonl`, `notes.json`, `flatpak.json` and
/// `crash-pending.json`, `crash-sent.json` from that directory instead of D-Bus, the history file and the
/// network. Nothing else looks at it. Only debug builds and builds with the
/// `fixtures` cargo feature honour it: a stray variable must not make the
/// shipped app show fake data.
#[cfg(any(debug_assertions, test, feature = "fixtures"))]
pub fn fixtures_dir() -> Option<PathBuf> {
    env("FIXTURES").filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(not(any(debug_assertions, test, feature = "fixtures")))]
pub fn fixtures_dir() -> Option<PathBuf> {
    None
}

pub fn read_fixture(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

/// Fixture-mode hook: `TELAMON_UPDATER_FIXTURE_HOLD=<op>` makes the next run of
/// that operation (names as in `busyOp`) stay busy until the app quits, so the
/// busy state can be photographed. Only fixture code paths ask.
#[cfg_attr(
    not(any(debug_assertions, test, feature = "fixtures")),
    allow(dead_code)
)]
static HOLD_TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg_attr(
    not(any(debug_assertions, test, feature = "fixtures")),
    allow(dead_code)
)]
fn hold_matches(var: Option<&str>, name: &str, taken: &std::sync::atomic::AtomicBool) -> bool {
    var == Some(name) && !taken.swap(true, std::sync::atomic::Ordering::AcqRel)
}

/// `true` once, for the first run of the operation named in the variable.
#[cfg(not(any(debug_assertions, test, feature = "fixtures")))]
pub fn fixture_hold(_name: &str) -> bool {
    false
}

#[cfg(any(debug_assertions, test, feature = "fixtures"))]
pub fn fixture_hold(name: &str) -> bool {
    hold_matches(
        std::env::var("TELAMON_UPDATER_FIXTURE_HOLD")
            .ok()
            .as_deref(),
        name,
        &HOLD_TAKEN,
    )
}

/// Park this worker thread for good (the process exit ends it).
pub fn hold_forever() -> ! {
    loop {
        std::thread::park();
    }
}

/// Unix seconds from an RFC 3339 time (`2026-10-02T04:00:00Z`, `+02:00`
/// offsets and fractions accepted).
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 20 || !s.is_char_boundary(19) {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    if b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_len = match mo {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=12).contains(&mo) || !(1..=month_len).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        let n = r.chars().take_while(char::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        rest = &r[n..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        r if r.len() == 6
            && (r.starts_with('+') || r.starts_with('-'))
            && r.as_bytes()[3] == b':' =>
        {
            let sign = if r.starts_with('-') { -1 } else { 1 };
            let (oh, om) = (
                r.get(1..3)?.parse::<i64>().ok()?,
                r.get(4..6)?.parse::<i64>().ok()?,
            );
            sign * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    // days since 1970-01-01 (Howard Hinnant's civil-days algorithm)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + sec - offset)
}

/// Fixture-mode scheduled restart: `schedule.json` is
/// `{"scheduledAt": "<RFC 3339>"}` (a Unix number works too).
pub fn fixture_schedule(dir: &Path) -> Option<i64> {
    let v: serde_json::Value = serde_json::from_str(&read_fixture(dir, "schedule.json")?).ok()?;
    match v.get("scheduledAt")? {
        serde_json::Value::String(t) => parse_rfc3339(t),
        serde_json::Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_variable_wins_and_the_old_one_still_works() {
        fn get<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
            move |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| OsString::from(v))
            }
        }
        let both = [
            ("TELAMON_UPDATER_FIXTURES", "/new"),
            ("ATLAS_UPDATER_FIXTURES", "/old"),
        ];
        let old = [("ATLAS_UPDATER_FIXTURES", "/old")];
        assert_eq!(env_in("FIXTURES", get(&both)), Some("/new".into()));
        assert_eq!(env_in("FIXTURES", get(&old)), Some("/old".into()));
        assert_eq!(env_in("FIXTURES", get(&[])), None);
        assert_eq!(env_in("PAGE", get(&both)), None);
        // set but empty: the new name still wins (an empty value turns it off)
        let empty = [
            ("TELAMON_UPDATER_FIXTURES", ""),
            ("ATLAS_UPDATER_FIXTURES", "/old"),
        ];
        assert_eq!(env_in("FIXTURES", get(&empty)), Some("".into()));
    }

    #[test]
    fn the_new_config_file_is_read_first_and_then_the_old_one() {
        let d = tempfile::tempdir().unwrap();
        let (new, old) = (d.path().join("new.toml"), d.path().join("old.toml"));
        let missing = d.path().join("missing.toml");
        let paths = |a: &Path, b: &Path| {
            [
                a.to_str().unwrap().to_string(),
                b.to_str().unwrap().to_string(),
            ]
        };
        let load = |p: [String; 2]| Config::load_from(&[p[0].as_str(), p[1].as_str()]);
        std::fs::write(&old, "release_notes_url = \"file:///old/{version}\"\n").unwrap();
        // only the old file: it counts
        assert_eq!(
            load(paths(&missing, &old)).release_notes_url,
            "file:///old/{version}"
        );
        // both: the new one wins
        std::fs::write(&new, "release_notes_url = \"file:///new/{version}\"\n").unwrap();
        assert_eq!(
            load(paths(&new, &old)).release_notes_url,
            "file:///new/{version}"
        );
        // a new file that is broken gives the defaults, not the old file
        std::fs::write(&new, "= = =").unwrap();
        assert_eq!(load(paths(&new, &old)), Config::default());
        // neither
        assert_eq!(load(paths(&missing, &missing)), Config::default());
    }

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

    #[test]
    fn rfc3339_parses() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-02T04:00:00Z"), Some(1_790_913_600));
        assert_eq!(
            parse_rfc3339("2026-10-02T06:00:00+02:00"),
            Some(1_790_913_600)
        );
        assert_eq!(
            parse_rfc3339("2026-10-02T04:00:00.250Z"),
            Some(1_790_913_600)
        );
        assert_eq!(parse_rfc3339("2000-02-29T00:00:00Z"), Some(951_782_400));
        for bad in [
            "2026-02-29T00:00:00Z",
            "2026-02-31T00:00:00Z",
            "1900-02-29T00:00:00Z",
            "2026-04-31T00:00:00Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
        assert!(parse_rfc3339("2024-02-29T00:00:00Z").is_some());
        for bad in [
            "",
            "tomorrow",
            "2026-13-01T00:00:00Z",
            "2026-10-02T04:00:00",
            "2026-10-02T04:00:00+0200",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn schedule_fixture_is_read() {
        let d = std::env::temp_dir().join(format!("telamon-sched-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(fixture_schedule(&d), None);
        std::fs::write(
            d.join("schedule.json"),
            r#"{"scheduledAt":"2026-10-02T04:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(fixture_schedule(&d), Some(1_790_913_600));
        std::fs::write(d.join("schedule.json"), r#"{"scheduledAt":1790913600}"#).unwrap();
        assert_eq!(fixture_schedule(&d), Some(1_790_913_600));
        std::fs::write(d.join("schedule.json"), "nonsense").unwrap();
        assert_eq!(fixture_schedule(&d), None);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn hold_applies_once_and_only_to_the_named_op() {
        use std::sync::atomic::AtomicBool;
        let taken = AtomicBool::new(false);
        assert!(!hold_matches(None, "check", &taken));
        assert!(!hold_matches(Some("download"), "check", &taken));
        assert!(hold_matches(Some("check"), "check", &taken));
        assert!(
            !hold_matches(Some("check"), "check", &taken),
            "only the next run"
        );
    }
}
