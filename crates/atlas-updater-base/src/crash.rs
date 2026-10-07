//! Crash report helpers both processes use.

use telamon_framework_system::crash::AppInfo;

pub const APP_ID: &str = "net.eterneon.atlas.updater";
pub const REPO: &str = "atlasos-updater";

pub fn app_info() -> AppInfo {
    AppInfo {
        name: "Atlas Updater".into(),
        id: APP_ID.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        repo: REPO.into(),
    }
}

/// The name to show for a report's app. A program run from outside a package
/// is named by its scrubbed path ("/var/home/USER/crashtest"); its file name
/// reads better. The report itself keeps the full name.
pub fn display_name(app_name: &str) -> &str {
    match app_name.rsplit_once('/') {
        Some((_, file)) if !file.is_empty() => file,
        _ => app_name,
    }
}

/// Collects new crash reports (coredumps and helper events), one collector at
/// a time across processes: the framework's markers are not safe against two
/// runs at once. Returns the first new report's app name and type.
/// Blocking.
pub fn collect() -> Option<(String, String)> {
    let _held = crate::lock::take(crate::lock::CRASH, true).ok().flatten();
    let mut new = telamon_framework_system::crash::collect_coredumps(None);
    new.extend(telamon_framework_system::crash::collect_events(None));
    new.first()
        .map(|r| (display_name(&r.app_name).to_string(), r.report_type.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_drops_the_path() {
        assert_eq!(display_name("/var/home/USER/crashtest"), "crashtest");
        assert_eq!(
            display_name("net.eterneon.atlas.updater"),
            "net.eterneon.atlas.updater"
        );
        assert_eq!(display_name("dir/"), "dir/");
        assert_eq!(display_name(""), "");
    }
}
