//! The privileged system helper: the logic behind `atlas-system-helper`.
//!
//! Not for apps; they use [`crate::helper_client`]. The D-Bus and polkit glue
//! is in [`service`]; everything else here is plain code that unit tests drive
//! with a fake [`BootcRunner`].

pub mod events;
pub mod service;

use std::cmp::Ordering as Cmp;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use crate::bootc::{Channel, Status};
use crate::history;

/// bootc is always run by absolute path.
pub const BOOTC: &str = "/usr/bin/bootc";
const STDERR_TAIL: usize = 4096;
/// Most output kept from bootc (stdout and stderr each).
const OUTPUT_CAP: usize = 4 * 1024 * 1024;
const SHORT_TIMEOUT: Duration = Duration::from_secs(120);
const STATUS_CACHE: Duration = Duration::from_secs(2);
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
    /// The helper is exiting; the client retries and D-Bus starts a new one.
    ShuttingDown(String),
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

/// Lock, ignoring poisoning (a panicking thread must not wedge the helper).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Process group of the bootc that is running now (0 if none).
static RUNNING_PGID: AtomicU32 = AtomicU32::new(0);

/// Send `signal` to a whole process group. There is no safe libc call for
/// this, so it uses kill(1).
fn kill_group(pgid: u32, signal: &str) {
    let _ = Command::new("/usr/bin/kill")
        .args([signal, "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Ask a running bootc (and its children) to stop; used at shutdown.
pub fn terminate_running() {
    let pgid = RUNNING_PGID.load(Ordering::Acquire);
    if pgid != 0 {
        kill_group(pgid, "-TERM");
    }
}

#[derive(Default)]
struct Captured {
    data: Vec<u8>,
    over: bool,
}

/// Read `r` into `buf`, keeping at most `cap` bytes; the rest is drained so
/// the child never blocks on a full pipe. Sends on `done` at the end.
fn read_capped(mut r: impl Read, cap: usize, buf: Arc<Mutex<Captured>>, done: mpsc::Sender<()>) {
    let mut chunk = [0u8; 16 * 1024];
    while let Ok(n) = r.read(&mut chunk) {
        if n == 0 {
            break;
        }
        let mut b = lock(&buf);
        let room = cap.saturating_sub(b.data.len());
        b.data.extend_from_slice(&chunk[..n.min(room)]);
        b.over |= n > room;
    }
    let _ = done.send(());
}

/// How long to wait for the output pipes after the child has exited
/// (grandchildren may still hold them).
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// Run `program` in its own process group with a cleared environment; kill
/// the group after `timeout`; fail if it prints more than `cap` bytes on
/// stdout.
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
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    let pgid = child.id();
    RUNNING_PGID.store(pgid, Ordering::Release);
    let result = supervise(&mut child, pgid, timeout, cap);
    RUNNING_PGID.store(0, Ordering::Release);
    result
}

fn supervise(
    child: &mut std::process::Child,
    pgid: u32,
    timeout: Duration,
    cap: usize,
) -> Result<String, String> {
    let out = child.stdout.take().ok_or("no stdout")?;
    let err = child.stderr.take().ok_or("no stderr")?;
    let (out_buf, err_buf) = (
        Arc::new(Mutex::new(Captured::default())),
        Arc::new(Mutex::new(Captured::default())),
    );
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    {
        let b = out_buf.clone();
        std::thread::spawn(move || read_capped(out, cap, b, out_tx));
        let b = err_buf.clone();
        std::thread::spawn(move || read_capped(err, cap, b, err_tx));
    }
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                kill_group(pgid, "-KILL");
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "bootc did not finish within {} s and was stopped",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                kill_group(pgid, "-KILL");
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("waiting for bootc failed: {e}"));
            }
        }
    };
    // A grandchild may keep the pipes open; wait a bounded time, then stop
    // it and use what we have.
    let end = Instant::now() + PIPE_GRACE;
    let mut pipes_closed = true;
    for rx in [&out_rx, &err_rx] {
        let left = end.saturating_duration_since(Instant::now());
        pipes_closed &= rx.recv_timeout(left).is_ok();
    }
    if !pipes_closed {
        kill_group(pgid, "-KILL");
    }
    let (stdout, over) = {
        let b = lock(&out_buf);
        (b.data.clone(), b.over)
    };
    if status.success() {
        if over {
            return Err("bootc printed too much output".into());
        }
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        let stderr = lock(&err_buf).data.clone();
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

/// What `boot_event` knows about the current boot.
struct BootInfo {
    version: Option<String>,
    timestamp: Option<String>,
    image: Option<String>,
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
    /// The last successful `bootc status` and when it ran; the lock is held
    /// while bootc runs, so at most one status process exists and callers
    /// that waited share its result.
    status_cache: Mutex<Option<(Instant, String)>>,
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
            status_cache: Mutex::new(None),
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
            return self.cached_status();
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
        *lock(&self.status_cache) = None;
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

    /// `bootc status --json`, at most one process at a time, shared for 2 s.
    fn cached_status(&self) -> Result<String, HelperError> {
        let mut cache = lock(&self.status_cache);
        if let Some((at, json)) = cache.as_ref()
            && at.elapsed() < STATUS_CACHE
        {
            return Ok(json.clone());
        }
        let json = self.status_json()?;
        *cache = Some((Instant::now(), json.clone()));
        Ok(json)
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
            self.boot_event(
                &prior,
                &BootInfo {
                    version: booted.and_then(|b| b.version().map(str::to_string)),
                    timestamp: booted.and_then(|b| b.timestamp().map(str::to_string)),
                    image: booted
                        .and_then(|b| b.image.as_ref())
                        .map(|i| i.image.image.clone()),
                },
            );
        }
        Ok(wrote)
    }

    /// `update-applied`, `rollback-applied`, `automatic-rollback` or
    /// `channel-switch-applied`, from the previous and the current boot.
    ///
    /// The order of the two boots comes from the dotted version numbers, or
    /// from the image build times when a version is not numeric. Newer: an
    /// update, or a channel switch if a `channel-switched` event followed the
    /// previous boot and the image name or tag changed. Older: a rollback if
    /// `rollback-requested` followed the previous boot; else a switch to an
    /// older channel build if `channel-switched` did and the image changed;
    /// else automatic (greenboot gave up and ostree went back).
    fn boot_event(&self, prior: &history::Entry, now: &BootInfo) {
        let order = match (prior.version.as_deref(), now.version.as_deref()) {
            (Some(old), Some(new)) => version_cmp(new, old),
            _ => None,
        }
        .or_else(
            || match (prior.timestamp.as_deref(), now.timestamp.as_deref()) {
                (Some(old), Some(new)) => Some(new.cmp(old)),
                _ => None,
            },
        );
        let Some(order) = order else { return };
        let since = |name: &str| {
            self.events.as_deref().is_some_and(|p| {
                events::read(p)
                    .iter()
                    .any(|e| e.event == name && e.time >= prior.first_booted)
            })
        };
        let image_changed = now
            .image
            .as_deref()
            .is_some_and(|i| !prior.image.is_empty() && i != prior.image);
        let switched = image_changed && since("channel-switched");
        let name = match order {
            Cmp::Greater if switched => "channel-switch-applied",
            Cmp::Greater => "update-applied",
            Cmp::Less if since("rollback-requested") => "rollback-applied",
            Cmp::Less if switched => "channel-switch-applied",
            Cmp::Less => "automatic-rollback",
            Cmp::Equal if switched => "channel-switch-applied",
            Cmp::Equal => return,
        };
        self.event(name, now.version.clone(), None);
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

    fn boot_with(d: &tempfile::TempDir, p: &Path, image: &str, version: &str, digest: &str) {
        let json = BOOTED_WITH_UPDATE
            .replace("atlasos:stable", image)
            .replace("44.20261001", version)
            .replace(&"1".repeat(64), &digest.repeat(64));
        with_ev(&Fake::new(&json), d).record_boot(p).unwrap();
    }

    fn ev(d: &tempfile::TempDir, name: &str) {
        events::append(
            &d.path().join("events.jsonl"),
            &events::Event::new(name, None, None),
        )
        .unwrap();
    }

    #[test]
    fn channel_events_need_the_matching_request() {
        // a normal switch to a newer testing build
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.jsonl");
        boot_with(&d, &p, "atlasos:stable", "44.20261001", "a");
        ev(&d, "channel-switched");
        boot_with(&d, &p, "atlasos:testing", "44.20261003", "b");
        assert_eq!(names(&d), ["channel-switched", "channel-switch-applied"]);

        // a rollback across channels (rollback requested)
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.jsonl");
        boot_with(&d, &p, "atlasos:testing", "44.20261003", "b");
        ev(&d, "rollback-requested");
        boot_with(&d, &p, "atlasos:stable", "44.20261001", "a");
        assert_eq!(names(&d), ["rollback-requested", "rollback-applied"]);

        // the switch failed, and later an older version boots by itself
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.jsonl");
        boot_with(&d, &p, "atlasos:testing", "44.20261003", "b");
        ev(&d, "channel-switch-failed");
        boot_with(&d, &p, "atlasos:stable", "44.20261001", "a");
        assert_eq!(names(&d), ["channel-switch-failed", "automatic-rollback"]);

        // an image change newer, but nobody switched: an ordinary update
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.jsonl");
        boot_with(&d, &p, "atlasos:stable", "44.20261001", "a");
        boot_with(&d, &p, "atlasos:testing", "44.20261003", "b");
        assert_eq!(names(&d), ["update-applied"]);
    }

    #[test]
    fn non_numeric_versions_fall_back_to_build_time() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.jsonl");
        boot_with(&d, &p, "atlasos:stable", "nightly", "a");
        // same timestamp in the fixture: equal, no event
        boot_with(&d, &p, "atlasos:stable", "nightly-2", "b");
        assert!(names(&d).is_empty());
        let json = BOOTED_WITH_UPDATE
            .replace("44.20261001", "dev-build")
            .replace("2026-10-01T04:12:09Z", "2026-11-01T00:00:00Z")
            .replace(&"1".repeat(64), &"c".repeat(64));
        with_ev(&Fake::new(&json), &d).record_boot(&p).unwrap();
        assert_eq!(names(&d), ["update-applied"]);
    }

    #[test]
    fn status_calls_share_one_bootc_process() {
        struct Counting(Mutex<usize>);
        impl BootcRunner for Counting {
            fn run(&self, _a: &[&str]) -> Result<String, String> {
                *self.0.lock().unwrap() += 1;
                std::thread::sleep(Duration::from_millis(100));
                Ok(PLAIN.into())
            }
        }
        let r = Arc::new(Counting(Mutex::new(0)));
        let c = Arc::new(Core::new(r.clone()));
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let c = c.clone();
                std::thread::spawn(move || c.execute(&Op::Status).unwrap())
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(*r.0.lock().unwrap(), 1);
        // a changing operation drops the cache
        c.execute(&Op::Rollback).unwrap();
        c.execute(&Op::Status).unwrap();
        assert_eq!(*r.0.lock().unwrap(), 4); // rollback + its status + a fresh status
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
    fn runner_times_out_kills_the_group_and_reaps() {
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("pid");
        let script = format!("echo $$ > {}; sleep 30 & sleep 30", pidfile.display());
        let t = Instant::now();
        let e = run_limited(
            Path::new("/bin/sh"),
            &["-c", &script],
            Duration::from_millis(500),
            1000,
        )
        .unwrap_err();
        assert!(e.contains("was stopped") && t.elapsed() < Duration::from_secs(10));
        let pid = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .to_string();
        // reaped: no zombie left behind either
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        assert_eq!(RUNNING_PGID.load(Ordering::Acquire), 0);
    }

    #[test]
    fn grandchild_holding_the_pipe_does_not_hang_the_runner() {
        let t = Instant::now();
        let out = run_limited(
            Path::new("/bin/sh"),
            &["-c", "sleep 30 & echo done"],
            Duration::from_secs(20),
            1000,
        )
        .unwrap();
        assert_eq!(out, "done\n");
        assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
    }

    #[test]
    fn runner_caps_output_and_reports_stderr() {
        let sh = Path::new("/bin/sh");
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
