//! Opening Telamon Settings: the Updates page and its sub-pages are Telamon
//! Settings' (the Updater has no window of its own). What the tray was asked
//! to open, as the arguments `telamon-settings` takes, and how it is started.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// What to open. One word per way in: the panel icon, the menu's items and
/// the notifications' actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Open {
    /// The Updates page.
    Updates,
    /// The Updates page, checking for updates ("Check for Updates").
    Check,
    /// The crash reports waiting, for review (Privacy page).
    CrashReview,
    /// The Updates page, at the firmware updates.
    Firmware,
    /// The Updates page, at the app updates.
    Apps,
}

impl Open {
    /// The arguments for `telamon-settings`.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Open::Updates => &["updates"],
            Open::Check => &["updates", "check"],
            Open::CrashReview => &["privacy", "crash-review"],
            Open::Firmware => &["updates", "firmware"],
            Open::Apps => &["updates", "apps"],
        }
    }
}

/// The program: `TELAMON_SETTINGS_BIN` if set (tests and developers: a
/// release build ignores it), else `telamon-settings` next to the tray, else
/// `/usr/bin/telamon-settings`.
pub fn settings_bin() -> PathBuf {
    settings_bin_from(
        std::env::var_os("TELAMON_SETTINGS_BIN")
            .map(PathBuf::from)
            .filter(|_| cfg!(debug_assertions)),
        std::env::current_exe().ok(),
    )
}

fn settings_bin_from(env: Option<PathBuf>, exe: Option<PathBuf>) -> PathBuf {
    if let Some(p) = env.filter(|p| !p.as_os_str().is_empty()) {
        return p;
    }
    exe.and_then(|p| p.parent().map(|d| d.join("telamon-settings")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("/usr/bin/telamon-settings"))
}

/// The command that opens `what`; `token` is the activation token that lets
/// the window come to the front (none: any the tray itself got is not passed
/// on).
pub fn command(program: &std::path::Path, what: Open, token: Option<String>) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(what.args()).stdin(Stdio::null());
    match token {
        Some(t) => cmd.env("XDG_ACTIVATION_TOKEN", t),
        None => cmd.env_remove("XDG_ACTIVATION_TOKEN"),
    };
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn every_way_in_is_one_set_of_arguments() {
        assert_eq!(Open::Updates.args(), ["updates"]);
        assert_eq!(Open::Check.args(), ["updates", "check"]);
        assert_eq!(Open::CrashReview.args(), ["privacy", "crash-review"]);
        assert_eq!(Open::Firmware.args(), ["updates", "firmware"]);
        assert_eq!(Open::Apps.args(), ["updates", "apps"]);
    }

    #[test]
    fn the_program_is_the_override_or_the_one_next_to_the_tray_or_usr_bin() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("telamon-updater-tray");
        let sibling = dir.path().join("telamon-settings");
        // nothing next to the tray: /usr/bin
        assert_eq!(
            settings_bin_from(None, Some(exe.clone())),
            PathBuf::from("/usr/bin/telamon-settings")
        );
        assert_eq!(
            settings_bin_from(None, None),
            PathBuf::from("/usr/bin/telamon-settings")
        );
        std::fs::write(&sibling, "").unwrap();
        assert_eq!(settings_bin_from(None, Some(exe.clone())), sibling);
        // the override wins; an empty one does not count
        assert_eq!(
            settings_bin_from(Some("/x/settings".into()), Some(exe.clone())),
            PathBuf::from("/x/settings")
        );
        assert_eq!(settings_bin_from(Some("".into()), Some(exe)), sibling);
    }

    /// Run `cmd` to the end. A script written a moment ago can answer "text
    /// file busy" while another test's thread has forked with its write
    /// handle still open; that passes in a few milliseconds.
    fn run(cmd: &mut std::process::Command) -> std::process::ExitStatus {
        for _ in 0..200 {
            match cmd.status() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other.unwrap(),
            }
        }
        cmd.status().unwrap()
    }

    /// A `telamon-settings` that writes its arguments (one per line) and
    /// the activation token to `out`.
    fn fake_settings(dir: &std::path::Path) -> (PathBuf, PathBuf) {
        let bin = dir.join("fake-telamon-settings");
        let out = dir.join("argv");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\n{{ for a in \"$@\"; do echo \"arg:$a\"; done; echo \"token:${{XDG_ACTIVATION_TOKEN-unset}}\"; }} > '{}'\n",
                out.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (bin, out)
    }

    /// Spawn level: what the tray starts for each way in, with the fake.
    #[test]
    fn the_tray_starts_telamon_settings_with_exactly_those_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let (bin, out) = fake_settings(dir.path());
        for (what, want) in [
            (Open::Updates, vec!["arg:updates"]),
            (Open::Check, vec!["arg:updates", "arg:check"]),
            (Open::CrashReview, vec!["arg:privacy", "arg:crash-review"]),
            (Open::Firmware, vec!["arg:updates", "arg:firmware"]),
            (Open::Apps, vec!["arg:updates", "arg:apps"]),
        ] {
            let _ = std::fs::remove_file(&out);
            let status = run(&mut command(&bin, what, Some("tok123".into())));
            assert!(status.success());
            let got = std::fs::read_to_string(&out).unwrap();
            let mut want = want;
            want.push("token:tok123");
            assert_eq!(got.lines().collect::<Vec<_>>(), want, "{what:?}");
        }
    }

    #[test]
    fn without_a_token_none_is_passed_on() {
        let dir = tempfile::tempdir().unwrap();
        let (bin, out) = fake_settings(dir.path());
        let cmd = command(&bin, Open::Updates, None);
        // removed for the child, whatever the tray's own environment holds
        let token = cmd
            .get_envs()
            .find(|(k, _)| *k == "XDG_ACTIVATION_TOKEN")
            .map(|(_, v)| v);
        assert_eq!(token, Some(None));
        let mut cmd = cmd;
        assert!(run(&mut cmd).success());
        let got = std::fs::read_to_string(&out).unwrap();
        assert!(got.ends_with("token:unset\n"), "{got}");
    }
}
