//! The privileged system helper: the logic behind `atlas-system-helper`.
//!
//! Not for apps; they use [`crate::helper_client`]. The D-Bus and polkit glue
//! is in [`service`]; everything else here is plain code that unit tests drive
//! with a fake [`BootcRunner`].

pub mod events;
pub mod service;

use std::cmp::Ordering as Cmp;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::bootc::{Channel, Status};
use crate::history;

/// bootc is always run by absolute path.
pub const BOOTC: &str = "/usr/bin/bootc";
const STDERR_TAIL: usize = 4096;
/// Most output kept from bootc (stdout and stderr each).
const OUTPUT_CAP: usize = 4 * 1024 * 1024;
const SHORT_TIMEOUT: Duration = Duration::from_secs(120);
const LONG_TIMEOUT: Duration = Duration::from_secs(3600);

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

/// The real runner: `/usr/bin/bootc` with a cleared environment, a wall-clock
/// timeout (2 minutes for status and check, 60 for upgrade, rollback and
/// switch) and capped output.
pub struct SystemBootc;

impl BootcRunner for SystemBootc {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let short = matches!(args, ["status", ..] | ["upgrade", "--check"]);
        let timeout = if short { SHORT_TIMEOUT } else { LONG_TIMEOUT };
        run_limited(Path::new(BOOTC), args, timeout, OUTPUT_CAP)
    }
}

/// Read `r` to the end, keeping at most `cap` bytes; the flag says if more
/// came (the rest is drained so the child never blocks on a full pipe).
fn read_capped(mut r: impl Read, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut over = false;
    let mut buf = [0u8; 16 * 1024];
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            break;
        }
        let room = cap.saturating_sub(kept.len());
        kept.extend_from_slice(&buf[..n.min(room)]);
        over |= n > room;
    }
    (kept, over)
}

/// Run `program` with a cleared environment; kill it after `timeout`; fail if
/// it prints more than `cap` bytes on stdout.
fn run_limited(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    cap: usize,
) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    let out = child.stdout.take().ok_or("no stdout")?;
    let err = child.stderr.take().ok_or("no stderr")?;
    let out_t = std::thread::spawn(move || read_capped(out, cap));
    let err_t = std::thread::spawn(move || read_capped(err, cap));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                // readers end when the pipes close; do not wait for stragglers
                return Err(format!(
                    "bootc did not finish within {} s and was stopped",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("waiting for bootc failed: {e}")),
        }
    };
    let (stdout, over) = out_t.join().map_err(|_| "reader failed")?;
    let (stderr, _) = err_t.join().map_err(|_| "reader failed")?;
    if status.success() {
        if over {
            return Err("bootc printed too much output".into());
        }
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        Err(tail(&String::from_utf8_lossy(&stderr), STDERR_TAIL))
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

/// Compare dotted numeric versions like `44.20261008`; `None` if either has a
/// non-numeric part.
fn version_cmp(a: &str, b: &str) -> Option<Cmp> {
    let parse = |v: &str| {
        v.split('.')
            .map(|p| p.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    let (mut a, mut b) = (parse(a)?, parse(b)?);
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    Some(a.cmp(&b))
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

    /// True for operations that change the system (they hold a shutdown
    /// inhibitor in the service).
    pub fn changes_system(&self) -> bool {
        matches!(self, Op::Upgrade | Op::Rollback | Op::SwitchChannel(_))
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

/// Runs operations on a [`BootcRunner`]: one changing operation at a time;
/// `Status` is read-only and never takes the busy flag.
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
    /// `staged_before` is the staged digest before an upgrade.
    fn record_outcome(
        &self,
        op: &Op,
        staged_before: Option<&str>,
        result: &Result<String, HelperError>,
    ) {
        let (ok, fail) = match op {
            Op::Upgrade => ("update-staged", "update-failed"),
            Op::Rollback => ("rollback-requested", "rollback-failed"),
            Op::SwitchChannel(_) => ("channel-switched", "channel-switch-failed"),
            _ => return,
        };
        match result {
            Ok(json) => {
                let staged = Status::from_json(json).ok().and_then(|s| s.status.staged);
                let version = staged
                    .as_ref()
                    .and_then(|b| b.version().map(str::to_string));
                match op {
                    // nothing new staged: no update, or the same one as before
                    Op::Upgrade => {
                        let digest = staged.as_ref().and_then(|b| b.digest());
                        if digest.is_some() && digest != staged_before {
                            self.event(ok, version, None);
                        }
                    }
                    Op::Rollback => self.event(ok, version, None),
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
        if *op == Op::Status {
            return self.status_json();
        }
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(HelperError::Busy("another operation is running".into()));
        }
        let _guard = BusyGuard(&self.busy);
        let before = (self.events.is_some() && *op == Op::Upgrade)
            .then(|| self.status().ok())
            .flatten()
            .and_then(|s| s.status.staged)
            .and_then(|b| b.digest().map(str::to_string));
        let result = self.run_op(op);
        self.record_outcome(op, before.as_deref(), &result);
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
                let mut args = vec!["switch", "--transport", new.transport_or_default()];
                match new.signature.as_ref() {
                    None => {}
                    Some(serde_json::Value::String(s)) if s == "insecure" => {}
                    Some(serde_json::Value::String(s)) if s == "containerPolicy" => {
                        args.push("--enforce-container-sigpolicy");
                    }
                    Some(_) => {
                        return Err(HelperError::Failed(
                            "the booted image uses a signature setting this helper cannot carry over; use bootc switch by hand".into(),
                        ));
                    }
                }
                args.push(&new.image);
                self.bootc(&args)?;
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
            let booted = status.status.booted.as_ref();
            let version = booted.and_then(|b| b.version().map(str::to_string));
            let image = booted
                .and_then(|b| b.image.as_ref())
                .map(|i| i.image.image.clone());
            self.boot_event(&prior, version, image.as_deref());
        }
        Ok(wrote)
    }

    /// `update-applied`, `rollback-applied`, `automatic-rollback` or
    /// `channel-switch-applied`, from the previous and the current boot.
    /// A different image name or tag is a channel switch, not an update. An
    /// older version without a `rollback-requested` since the previous boot
    /// is automatic (greenboot gave up and ostree went back). Versions that
    /// are not dotted numbers record nothing.
    fn boot_event(&self, prior: &history::Entry, now: Option<String>, image: Option<&str>) {
        if image.is_some_and(|i| !prior.image.is_empty() && i != prior.image) {
            self.event("channel-switch-applied", now, None);
            return;
        }
        let (Some(old), Some(new)) = (prior.version.as_deref(), now.as_deref()) else {
            return;
        };
        match version_cmp(new, old) {
            Some(Cmp::Greater) => self.event("update-applied", now.clone(), None),
            Some(Cmp::Less) => {
                let requested = self.events.as_deref().is_some_and(|p| {
                    events::read(p)
                        .iter()
                        .any(|e| e.event == "rollback-requested" && e.time >= prior.first_booted)
                });
                let name = if requested {
                    "rollback-applied"
                } else {
                    "automatic-rollback"
                };
                self.event(name, now.clone(), None);
            }
            _ => {}
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
        after_upgrade: Option<String>,
    }

    impl Fake {
        fn new(status: &str) -> Arc<Fake> {
            Arc::new(Fake {
                calls: Mutex::default(),
                status: status.into(),
                fail_on: None,
                after_upgrade: None,
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
                let upgraded = self.calls.lock().unwrap().iter().any(|c| c == &["upgrade"]);
                return Ok(match (&self.after_upgrade, upgraded) {
                    (Some(a), true) => a.clone(),
                    _ => self.status.clone(),
                });
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
            after_upgrade: None,
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
        // Status is read-only and never takes the busy flag
        assert!(c.execute(&Op::Status).is_ok());
        assert!(matches!(
            c.execute(&Op::CheckForUpdate),
            Err(HelperError::Busy(_))
        ));
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
        // nothing staged before the upgrade, a staged deployment after it
        let ok = Arc::new(Fake {
            calls: Mutex::default(),
            status: PLAIN.into(),
            fail_on: None,
            after_upgrade: Some(BOOTED_WITH_UPDATE.into()),
        });
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
        // the same deployment already staged: not a new event
        let dd = tempfile::tempdir().unwrap();
        let cc = with_ev(&Fake::new(BOOTED_WITH_UPDATE), &dd);
        cc.execute(&Op::Upgrade).unwrap();
        assert!(names(&dd).is_empty());
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
                after_upgrade: None,
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

    #[test]
    fn dotted_version_compare() {
        assert_eq!(
            version_cmp("44.20261008", "44.20261001"),
            Some(Cmp::Greater)
        );
        assert_eq!(version_cmp("44.9", "44.10"), Some(Cmp::Less));
        assert_eq!(version_cmp("44.1", "44.1.0"), Some(Cmp::Equal));
        assert_eq!(version_cmp("44.x", "44.1"), None);
    }

    #[test]
    fn image_change_between_boots_is_a_channel_switch() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("history.jsonl");
        let testing = BOOTED_WITH_UPDATE.replace("atlasos:stable", "atlasos:testing");
        with_ev(&Fake::new(BOOTED_WITH_UPDATE), &d)
            .record_boot(&p)
            .unwrap();
        let newer = testing
            .replace("44.20261001", "44.20261003")
            .replace(&"1".repeat(64), &"9".repeat(64));
        with_ev(&Fake::new(&newer), &d).record_boot(&p).unwrap();
        assert_eq!(names(&d), ["channel-switch-applied"]);
    }

    /// The fixture with `signature` added to the booted ref (the third
    /// `"transport"`: spec, staged, booted).
    fn signed(sig: &str) -> String {
        let pat = "\"transport\": \"registry\"";
        let (i, _) = BOOTED_WITH_UPDATE.match_indices(pat).nth(2).unwrap();
        let end = i + pat.len();
        format!(
            "{}, \"signature\": {sig}{}",
            &BOOTED_WITH_UPDATE[..end],
            &BOOTED_WITH_UPDATE[end..]
        )
    }

    #[test]
    fn switch_carries_the_signature_policy_or_refuses() {
        let f = Fake::new(&signed("\"containerPolicy\""));
        core(&f)
            .execute(&Op::SwitchChannel("testing".into()))
            .unwrap();
        assert!(f.calls()[1].contains(&"--enforce-container-sigpolicy".to_string()));
        assert_eq!(
            f.calls()[1].last().unwrap(),
            "ghcr.io/eternalcoder454/atlasos:testing"
        );
        let f = Fake::new(&signed("\"insecure\""));
        core(&f)
            .execute(&Op::SwitchChannel("testing".into()))
            .unwrap();
        assert!(!f.calls()[1].contains(&"--enforce-container-sigpolicy".to_string()));
        let f = Fake::new(&signed("{\"ostreeRemoteSignature\": \"fedora\"}"));
        assert!(matches!(
            core(&f).execute(&Op::SwitchChannel("testing".into())),
            Err(HelperError::Failed(_))
        ));
        assert_eq!(f.calls().len(), 1, "refused before switching");
    }

    #[test]
    fn runner_times_out_and_caps_output() {
        let sh = Path::new("/bin/sh");
        let t = Instant::now();
        let e = run_limited(sh, &["-c", "sleep 30"], Duration::from_millis(300), 1000).unwrap_err();
        assert!(e.contains("was stopped") && t.elapsed() < Duration::from_secs(10));
        let e = run_limited(
            sh,
            &["-c", "yes | head -c 3000000"],
            Duration::from_secs(20),
            1000,
        )
        .unwrap_err();
        assert!(e.contains("too much"));
        assert_eq!(
            run_limited(
                sh,
                &["-c", "echo hi; echo $LANG"],
                Duration::from_secs(20),
                1000
            )
            .unwrap(),
            "hi\nC.UTF-8\n"
        );
        let e = run_limited(
            sh,
            &["-c", "echo bad >&2; exit 3"],
            Duration::from_secs(20),
            1000,
        )
        .unwrap_err();
        assert_eq!(e, "bad");
    }
}
