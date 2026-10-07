//! Opt-in crash reports (telamon_framework_system::crash). Off unless the user turns them
//! on in Settings; every report is shown before it is sent.

use std::ffi::{CStr, c_char};

use serde_json::{Value, json};
use telamon_framework_system::crash::{self, Report};

pub use telamon_updater_base::crash::{REPO, app_info, display_name};

/// Called first thing from `main.cpp`: panics save a report, but only when
/// the user enabled crash reports (telamon-framework-system checks the setting).
#[unsafe(no_mangle)]
pub extern "C" fn telamon_crash_install() {
    crash::install(app_info());
}

/// Called from the C++ Qt message handler on `QtFatalMsg`, before abort.
///
/// # Safety
/// `msg` must be null or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn telamon_crash_fatal(msg: *const c_char) {
    let text = if msg.is_null() {
        String::from("Qt fatal message")
    } else {
        unsafe { CStr::from_ptr(msg) }
            .to_string_lossy()
            .into_owned()
    };
    let _ = crash::record_fatal(&text);
}

pub fn uptime_text(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (d, h) {
        (0, 0) => format!("{m} min"),
        (0, _) => format!("{h} h {m} min"),
        _ => format!("{d} d {h} h"),
    }
}

fn or_unknown(v: &Option<String>) -> String {
    v.clone().unwrap_or_default()
}

/// Everything the review and history screens show, in plain fields.
pub fn view(r: &Report, with_github: bool) -> Value {
    json!({
        "eventId": r.event_id,
        "sentEventId": r.sent_event_id.clone().unwrap_or_default(),
        "issueUrl": r.issue_url.clone().filter(|u| crash::is_issue_url(u)).unwrap_or_default(),
        "type": r.report_type,
        "time": r.time,
        "appName": display_name(&r.app_name),
        "appVersion": or_unknown(&r.app_version),
        "category": r.category,
        "atlasosVersion": or_unknown(&r.atlasos_version),
        "channel": or_unknown(&r.channel),
        "previousVersion": or_unknown(&r.previous_version),
        "kernel": or_unknown(&r.kernel),
        "gpu": or_unknown(&r.gpu),
        "gpuDriver": or_unknown(&r.gpu_driver),
        "uptime": uptime_text(r.uptime_secs),
        "message": r.message,
        "stacktrace": r.stacktrace,
        "payload": serde_json::to_string_pretty(&r.payload()).unwrap_or_default(),
        "githubUrl": if with_github { crash::github_issue_url(r, REPO) } else { String::new() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime() {
        assert_eq!(uptime_text(90), "1 min");
        assert_eq!(uptime_text(3 * 3600 + 120), "3 h 2 min");
        assert_eq!(uptime_text(2 * 86_400 + 5 * 3600), "2 d 5 h");
    }
}
