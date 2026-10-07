//! `telamon-updater`: what is left of Telamon Updater's window.
//!
//! The window is Telamon Settings' Updates page now. What stays here:
//!
//! - `--worker <job>`: an app round or an app update, for the tray, without
//!   Qt (libflatpak is loaded for the length of the job only, so the
//!   resident tray stays small);
//! - `--tray`: from older autostart entries, hands over to
//!   `telamon-updater-tray`;
//! - anything else, the way it was started before (`--page updates|settings|
//!   reports|sent`, `--check`, nothing): starts `telamon-settings` on the
//!   page that has what that opened. A program that still starts
//!   `atlas-updater` or `telamon-updater` (the Launcher, a pin, a script)
//!   lands in the right place. The activation token stays in the
//!   environment, so the window comes to the front.
//!
//! The same program runs as `atlas-updater` (a link, for this release).

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use telamon_updater_core::base::crash;

/// A program next to this one (`/usr/bin`), else `/usr/bin`.
fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(format!("/usr/bin/{name}")))
}

/// What `telamon-settings` is asked for when this program was started with
/// `args` (without the program name): `updates` for the Updates page,
/// `updates check` for "Check for Updates" (the tray's menu), `privacy
/// crash-review` for the crash reports waiting. Unknown words are ignored,
/// as the old window ignored them.
fn settings_args(args: &[String]) -> Vec<&'static str> {
    let mut check = false;
    let mut page = "updates";
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--check" => check = true,
            "--page" => {
                page = match it.next().map(String::as_str) {
                    Some("reports" | "sent") => "reports",
                    _ => "updates",
                };
            }
            _ => {}
        }
    }
    if check {
        vec!["updates", "check"]
    } else if page == "reports" {
        vec!["privacy", "crash-review"]
    } else {
        vec!["updates"]
    }
}

fn main() -> ExitCode {
    // Panics save a crash report, when the user turned them on.
    telamon_framework_system::crash::install(crash::app_info());
    let args: Vec<String> = std::env::args().skip(1).collect();

    if let Some(i) = args.iter().position(|a| a == "--worker") {
        let job = args.get(i + 1).map_or("", String::as_str);
        let code = telamon_updater_core::worker::run(job);
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    if args.iter().any(|a| a == "--tray") {
        let tray = sibling("telamon-updater-tray");
        let err = Command::new(&tray).exec();
        eprintln!("telamon-updater: cannot start {}: {err}", tray.display());
        return ExitCode::FAILURE;
    }

    // For tests and developers; the installed program is /usr/bin's.
    let settings = std::env::var_os("TELAMON_SETTINGS_BIN")
        .map(PathBuf::from)
        .filter(|_| cfg!(debug_assertions))
        .unwrap_or_else(|| PathBuf::from("/usr/bin/telamon-settings"));
    let err = Command::new(&settings).args(settings_args(&args)).exec();
    eprintln!(
        "telamon-updater: cannot start {}: {err}",
        settings.display()
    );
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_old_ways_to_open_the_window_open_the_right_page() {
        assert_eq!(settings_args(&args(&[])), ["updates"]);
        assert_eq!(settings_args(&args(&["--page", "updates"])), ["updates"]);
        assert_eq!(settings_args(&args(&["--page", "settings"])), ["updates"]);
        assert_eq!(
            settings_args(&args(&["--page", "reports"])),
            ["privacy", "crash-review"]
        );
        assert_eq!(
            settings_args(&args(&["--page", "sent"])),
            ["privacy", "crash-review"]
        );
        assert_eq!(settings_args(&args(&["--check"])), ["updates", "check"]);
        // Nothing else is passed on.
        assert_eq!(
            settings_args(&args(&["--page", "../../x", "--evil", "-rf"])),
            ["updates"]
        );
        assert_eq!(settings_args(&args(&["--page"])), ["updates"]);
    }
}
