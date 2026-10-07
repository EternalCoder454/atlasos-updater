//! The helper's command line is the same whatever name it is started by:
//! the OS image's scripts call it as `atlas-system-helper` (a link to
//! `telamon-system-helper`).

use std::os::unix::fs::symlink;
use std::process::Command;

fn run_as(name: &str, args: &[&str]) -> (Option<i32>, String) {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join(name);
    symlink(env!("CARGO_BIN_EXE_telamon-system-helper"), &link).unwrap();
    let out = Command::new(&link).args(args).output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn an_unknown_mode_prints_the_usage_under_both_names() {
    for name in ["telamon-system-helper", "atlas-system-helper"] {
        let (code, err) = run_as(name, &["no-such-mode"]);
        assert_eq!(code, Some(2), "{name}");
        assert!(
            err.contains("usage: telamon-system-helper"),
            "{name}: {err}"
        );
        assert!(err.contains("record-event"), "{name}: {err}");
    }
}

#[test]
fn record_event_refuses_an_unknown_event_under_both_names() {
    // (the events file is root's: this must fail on the name before writing)
    for name in ["telamon-system-helper", "atlas-system-helper"] {
        let (code, err) = run_as(name, &["record-event", "no-such-event"]);
        assert_eq!(code, Some(2), "{name}: {err}");
        assert!(err.starts_with("telamon-system-helper:"), "{name}: {err}");
    }
}
