//! The privileged system helper: the logic behind `atlas-system-helper`.
//!
//! Not for apps; they use [`crate::helper_client`]. The D-Bus and polkit glue
//! is in [`service`]; everything else here is plain code that unit tests drive
//! with a fake [`BootcRunner`].

pub mod events;
pub mod service;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::bootc::{Channel, Status};
use crate::history;

/// bootc is always run by absolute path.
pub const BOOTC: &str = "/usr/bin/bootc";
const STDERR_TAIL: usize = 4096;

/// Errors returned over D-Bus as `net.eterneon.atlas.Error.*`.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "net.eterneon.atlas.Error")]
pub enum HelperError {
    #[zbus(error)]
    ZBus(zbus::Error),
    InvalidArgument(String),
    NotAuthorized(String),
    Busy(String),
    Failed(String),
}

/// Runs bootc with the given argv (without the program name) and returns its
/// stdout, or its stderr tail on failure.
pub trait BootcRunner: Send + Sync + 'static {
    fn run(&self, args: &[&str]) -> Result<String, String>;
}

/// The real runner: `/usr/bin/bootc` with a cleared environment.
pub struct SystemBootc;

impl BootcRunner for SystemBootc {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new(BOOTC)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin")
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run {BOOTC}: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(tail(&String::from_utf8_lossy(&out.stderr), STDERR_TAIL))
        }
    }
}

/// The last `max` bytes of `s`, cut on a char boundary.
fn tail(s: &str, max: usize) -> String {
    let s = s.trim_end();
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

/// The five operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Status,
    CheckForUpdate,
    Upgrade,
    Rollback,
    SwitchChannel(String),
}

impl Op {
    /// The polkit action that guards this operation.
    pub fn action_id(&self) -> &'static str {
        match self {
            Op::Status => "net.eterneon.atlas.system.status",
            Op::CheckForUpdate => "net.eterneon.atlas.system.check",
            Op::Upgrade => "net.eterneon.atlas.system.upgrade",
            Op::Rollback => "net.eterneon.atlas.system.rollback",
            Op::SwitchChannel(_) => "net.eterneon.atlas.system.switch-channel",
        }
    }

    /// Reject bad arguments before anything else happens.
    pub fn validate(&self) -> Result<(), HelperError> {
        if let Op::SwitchChannel(c) = self {
            c.parse::<Channel>()
                .map_err(|e| HelperError::InvalidArgument(e.to_string()))?;
        }
        Ok(())
    }
}

/// Runs operations one at a time on a [`BootcRunner`].
pub struct Core {
    runner: Arc<dyn BootcRunner>,
    busy: AtomicBool,
    events: Option<PathBuf>,
}

struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Core {
    pub fn new(runner: Arc<dyn BootcRunner>) -> Self {
        Core {
            runner,
            busy: AtomicBool::new(false),
            events: None,
        }
    }

    /// Also record update and rollback events to this file.
    pub fn with_events(mut self, path: PathBuf) -> Self {
        self.events = Some(path);
        self
    }

    fn event(&self, event: &str, version: Option<String>, error: Option<&str>) {
        if let Some(p) = &self.events {
            let _ = events::append(p, &events::Event::new(event, version, error));
        }
    }

    /// Record the outcome of the operations that change the system.
    fn record_outcome(&self, op: &Op, result: &Result<String, HelperError>) {
        let (ok, fail) = match op {
            Op::Upgrade => ("update-staged", "update-failed"),
            Op::Rollback => ("rollback-requested", "rollback-failed"),
            Op::SwitchChannel(_) => ("channel-switched", "channel-switch-failed"),
            _ => return,
        };
        match result {
            Ok(json) => {
                let staged = Status::from_json(json)
                    .ok()
                    .and_then(|s| s.status.staged)
                    .and_then(|b| b.version().map(str::to_string));
                match op {
                    // nothing staged: there was no update, nothing to record
                    Op::Upgrade if staged.is_none() => {}
                    Op::Upgrade | Op::Rollback => self.event(ok, staged, None),
                    _ => self.event(ok, None, None),
                }
            }
            Err(HelperError::Failed(m)) => self.event(fail, None, Some(m)),
            Err(_) => {}
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// Run `op` and return `bootc status --json` after it. Blocking.
    pub fn execute(&self, op: &Op) -> Result<String, HelperError> {
        op.validate()?;
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(HelperError::Busy("another operation is running".into()));
        }
        let _guard = BusyGuard(&self.busy);
        let result = self.run_op(op);
        self.record_outcome(op, &result);
        result
    }

    fn run_op(&self, op: &Op) -> Result<String, HelperError> {
        match op {
            Op::Status => {}
            Op::CheckForUpdate => {
                self.bootc(&["upgrade", "--check"])?;
            }
            Op::Upgrade => {
                // stages only: never --apply
                self.bootc(&["upgrade"])?;
            }
            Op::Rollback => {
                self.bootc(&["rollback"])?;
            }
            Op::SwitchChannel(channel) => {
                let current = self.status()?;
                let booted = current.booted_ref().ok_or_else(|| {
                    HelperError::Failed("bootc reports no booted image reference".into())
                })?;
                let new = booted
                    .with_channel(channel)
                    .map_err(|e| HelperError::InvalidArgument(e.to_string()))?;
                self.bootc(&[
                    "switch",
                    "--transport",
                    new.transport_or_default(),
                    &new.image,
                ])?;
            }
        }
        self.status_json()
    }

    fn bootc(&self, args: &[&str]) -> Result<String, HelperError> {
        self.runner.run(args).map_err(HelperError::Failed)
    }

    fn status_json(&self) -> Result<String, HelperError> {
        self.bootc(&["status", "--json"])
    }

    fn status(&self) -> Result<Status, HelperError> {
        Status::from_json(&self.status_json()?)
            .map_err(|e| HelperError::Failed(format!("cannot parse bootc status: {e}")))
    }

    /// `record-boot`: append the booted image to the history at `path`.
    pub fn record_boot(&self, path: &Path) -> Result<bool, HelperError> {
        let status = self.status()?;
        let prior = history::read(path).ok().and_then(|h| h.into_iter().next());
        let wrote = history::record_boot(path, &status, &history::now_rfc3339())
            .map_err(|e| HelperError::Failed(format!("cannot write history: {e}")))?;
        if let (true, Some(prior)) = (wrote, prior) {
            let now = status
                .status
                .booted
                .as_ref()
                .and_then(|b| b.version().map(str::to_string));
            self.boot_event(&prior, now);
        }
        Ok(wrote)
    }

    /// `update-applied`, `rollback-applied` or `automatic-rollback`, from the
    /// versions of the previous and the current boot. An older version
    /// without a `rollback-requested` since the previous boot is automatic
    /// (greenboot gave up and ostree went back).
    fn boot_event(&self, prior: &history::Entry, now: Option<String>) {
        let (Some(old), Some(new)) = (prior.version.as_deref(), now.as_deref()) else {
            return;
        };
        if new > old {
            self.event("update-applied", now.clone(), None);
        } else if new < old {
            let requested = self.events.as_deref().is_some_and(|p| {
                events::read(p)
                    .iter()
                    .any(|e| e.event == "rollback-requested" && e.time >= prior.first_booted)
            });
            self.event(
                if requested {
                    "rollback-applied"
                } else {
                    "automatic-rollback"
                },
                now.clone(),
                None,
            );
        }
    }

    /// `record-event`: append one of the fixed [`events::CLI_EVENTS`].
    pub fn record_event(&self, name: &str) -> Result<(), HelperError> {
        if !events::CLI_EVENTS.contains(&name) {
            return Err(HelperError::InvalidArgument(format!(
                "unknown event {name:?}"
            )));
        }
        self.event(name, None, None);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootc::fixtures::{BOOTED_WITH_UPDATE, PLAIN};
    use std::sync::Mutex;
    use std::sync::mpsc;

    /// Records argv; answers `status --json` with a fixture.
    struct Fake {
        calls: Mutex<Vec<Vec<String>>>,
        status: String,
        fail_on: Option<&'static str>,
    }

    impl Fake {
        fn new(status: &str) -> Arc<Fake> {
            Arc::new(Fake {
                calls: Mutex::default(),
                status: status.into(),
                fail_on: None,
            })
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl BootcRunner for Fake {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            if self.fail_on == Some(args[0]) {
                return Err("boom".into());
            }
            if args == ["status", "--json"] {
                return Ok(self.status.clone());
            }
            Ok(String::new())
        }
    }

    fn core(f: &Arc<Fake>) -> Core {
        Core::new(f.clone())
    }

    #[test]
    fn fixed_argv_for_each_operation() {
        let f = Fake::new(PLAIN);
        let c = core(&f);
        for op in [Op::Status, Op::CheckForUpdate, Op::Upgrade, Op::Rollback] {
            let out = c.execute(&op).unwrap();
            assert_eq!(out, PLAIN);
        }
        let want: Vec<Vec<&str>> = vec![
            vec!["status", "--json"],
            vec!["upgrade", "--check"],
            vec!["status", "--json"],
            vec!["upgrade"],
            vec!["status", "--json"],
            vec!["rollback"],
            vec!["status", "--json"],
        ];
        assert_eq!(f.calls(), want);
        assert!(
            f.calls()
                .iter()
                .all(|a| !a.contains(&"--apply".to_string()))
        );
    }

    #[test]
    fn switch_rewrites_only_the_tag_of_the_booted_ref() {
        let f = Fake::new(BOOTED_WITH_UPDATE); // booted :stable
        core(&f)
            .execute(&Op::SwitchChannel("testing".into()))
            .unwrap();
        let calls = f.calls();
        assert_eq!(calls[0], ["status", "--json"]);
        assert_eq!(
            calls[1],
            [
                "switch",
                "--transport",
                "registry",
                "ghcr.io/eternalcoder454/atlasos:testing"
            ]
        );
    }

    #[test]
    fn switch_rejects_anything_but_stable_or_testing_without_running_bootc() {
        let f = Fake::new(PLAIN);
        let c = core(&f);
        for bad in [
            "latest",
            "",
            "STABLE",
            "stable;rm",
            "--apply",
            "ghcr.io/x:y",
            "44.20261001",
        ] {
            let e = c.execute(&Op::SwitchChannel(bad.into())).unwrap_err();
            assert!(matches!(e, HelperError::InvalidArgument(_)), "{bad:?}");
        }
        assert!(f.calls().is_empty());
    }

    #[test]
    fn switch_keeps_oci_transport() {
        let json = PLAIN
            .replace(
                "ghcr.io/eternalcoder454/atlasos:testing",
                "/var/img/atlasos:testing",
            )
            .replace("\"registry\"", "\"oci\"");
        let f = Fake::new(&json);
        core(&f)
            .execute(&Op::SwitchChannel("stable".into()))
            .unwrap();
        assert_eq!(
            f.calls()[1],
            ["switch", "--transport", "oci", "/var/img/atlasos:stable"]
        );
    }

    #[test]
    fn bootc_failure_becomes_failed_with_stderr() {
        let f = Arc::new(Fake {
            calls: Mutex::default(),
            status: PLAIN.into(),
            fail_on: Some("upgrade"),
        });
        match core(&f).execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => assert_eq!(m, "boom"),
            other => panic!("{other:?}"),
        }
        // and the helper is free again afterwards
        let c = core(&f);
        assert!(c.execute(&Op::Status).is_ok());
        assert!(!c.is_busy());
    }

    /// A runner that blocks until released, to hold the helper busy.
    struct Slow {
        started: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl BootcRunner for Slow {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            if args == ["upgrade"] {
                self.started.lock().unwrap().send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            Ok(PLAIN.into())
        }
    }

    #[test]
    fn second_call_while_busy_gets_busy() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let c = Arc::new(Core::new(Arc::new(Slow {
            started: Mutex::new(started_tx),
            release: Mutex::new(release_rx),
        })));
        let c2 = c.clone();
        let t = std::thread::spawn(move || c2.execute(&Op::Upgrade));
        started_rx.recv().unwrap();
        assert!(c.is_busy());
        assert!(matches!(c.execute(&Op::Status), Err(HelperError::Busy(_))));
        assert!(matches!(
            c.execute(&Op::Rollback),
            Err(HelperError::Busy(_))
        ));
        // bad arguments are still reported as such, not as Busy
        assert!(matches!(
            c.execute(&Op::SwitchChannel("x".into())),
            Err(HelperError::InvalidArgument(_))
        ));
        release_tx.send(()).unwrap();
        t.join().unwrap().unwrap();
        assert!(!c.is_busy());
        assert!(c.execute(&Op::Status).is_ok());
    }

    #[test]
    fn record_boot_appends_once_per_digest() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("history.jsonl");
        let f = Fake::new(BOOTED_WITH_UPDATE);
        let c = core(&f);
        assert!(c.record_boot(&p).unwrap());
        assert!(!c.record_boot(&p).unwrap());
        assert_eq!(history::read(&p).unwrap().len(), 1);
    }

    #[test]
    fn record_boot_off_a_bootc_host_fails() {
        let d = tempfile::tempdir().unwrap();
        let f = Fake::new(crate::bootc::fixtures::NOT_BOOTC);
        assert!(core(&f).record_boot(&d.path().join("h")).is_err());
    }

    fn with_ev(f: &Arc<Fake>, d: &tempfile::TempDir) -> Core {
        Core::new(f.clone()).with_events(d.path().join("events.jsonl"))
    }

    fn names(d: &tempfile::TempDir) -> Vec<String> {
        events::read(&d.path().join("events.jsonl"))
            .into_iter()
            .map(|e| e.event)
            .collect()
    }

    #[test]
    fn operations_record_events() {
        let d = tempfile::tempdir().unwrap();
        let ok = Fake::new(BOOTED_WITH_UPDATE); // has a staged deployment
        let c = with_ev(&ok, &d);
        c.execute(&Op::Upgrade).unwrap();
        c.execute(&Op::Rollback).unwrap();
        c.execute(&Op::SwitchChannel("testing".into())).unwrap();
        c.execute(&Op::Status).unwrap();
        assert_eq!(
            names(&d),
            ["update-staged", "rollback-requested", "channel-switched"]
        );
        assert_eq!(
            events::read(&d.path().join("events.jsonl"))[0]
                .version
                .as_deref(),
            Some("44.20261008")
        );
        // no staged deployment: nothing was updated
        let d2 = tempfile::tempdir().unwrap();
        with_ev(&Fake::new(PLAIN), &d2)
            .execute(&Op::Upgrade)
            .unwrap();
        assert!(names(&d2).is_empty());
        // failures
        let d3 = tempfile::tempdir().unwrap();
        for (what, op) in [("upgrade", Op::Upgrade), ("rollback", Op::Rollback)] {
            let f = Arc::new(Fake {
                calls: Mutex::default(),
                status: PLAIN.into(),
                fail_on: Some(what),
            });
            assert!(with_ev(&f, &d3).execute(&op).is_err());
        }
        assert_eq!(names(&d3), ["update-failed", "rollback-failed"]);
    }

    #[test]
    fn record_boot_events_update_rollback_and_automatic() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("history.jsonl");
        let json_for = |v: &str, digest: &str| {
            BOOTED_WITH_UPDATE
                .replace("44.20261001", v)
                .replace(&"1".repeat(64), digest)
        };
        let boot = |v: &str, digest: &str| {
            with_ev(&Fake::new(&json_for(v, digest)), &d)
                .record_boot(&p)
                .unwrap()
        };
        assert!(boot("44.20261001", &"a".repeat(64))); // first line: no event
        assert!(names(&d).is_empty());
        assert!(boot("44.20261008", &"b".repeat(64)));
        assert_eq!(names(&d), ["update-applied"]);
        // back to older without a request: automatic
        assert!(boot("44.20261001", &"a".repeat(64)));
        assert_eq!(names(&d), ["update-applied", "automatic-rollback"]);
        // update again, request a rollback, then boot the older one
        assert!(boot("44.20261008", &"b".repeat(64)));
        with_ev(&Fake::new(BOOTED_WITH_UPDATE), &d)
            .execute(&Op::Rollback)
            .unwrap();
        assert!(boot("44.20261001", &"a".repeat(64)));
        assert_eq!(
            names(&d).last().map(String::as_str),
            Some("rollback-applied")
        );
    }

    #[test]
    fn record_event_accepts_only_fixed_words() {
        let d = tempfile::tempdir().unwrap();
        let c = with_ev(&Fake::new(PLAIN), &d);
        c.record_event("health-check-failed").unwrap();
        c.record_event("health-check-passed").unwrap();
        for bad in ["", "update-staged", "x\ny", "automatic-rollback"] {
            assert!(matches!(
                c.record_event(bad),
                Err(HelperError::InvalidArgument(_))
            ));
        }
        assert_eq!(names(&d), ["health-check-failed", "health-check-passed"]);
    }

    #[test]
    fn tail_cuts_on_char_boundary() {
        assert_eq!(tail("abc\n", 10), "abc");
        assert_eq!(tail("aébc", 3), "bc");
        let long = "x".repeat(5000);
        assert_eq!(tail(&long, STDERR_TAIL).len(), STDERR_TAIL);
    }

    #[test]
    fn action_ids_are_the_five_policy_actions() {
        let ids: Vec<_> = [
            Op::Status,
            Op::CheckForUpdate,
            Op::Upgrade,
            Op::Rollback,
            Op::SwitchChannel("stable".into()),
        ]
        .iter()
        .map(Op::action_id)
        .collect();
        assert_eq!(
            ids,
            [
                "net.eterneon.atlas.system.status",
                "net.eterneon.atlas.system.check",
                "net.eterneon.atlas.system.upgrade",
                "net.eterneon.atlas.system.rollback",
                "net.eterneon.atlas.system.switch-channel"
            ]
        );
    }
}
