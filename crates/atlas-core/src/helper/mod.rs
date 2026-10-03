//! The privileged system helper: the logic behind `atlas-system-helper`.
//!
//! Not for apps; they use [`crate::helper_client`]. The D-Bus and polkit glue
//! is in [`service`]; everything else here is plain code that unit tests drive
//! with a fake [`BootcRunner`].

pub mod events;
pub mod layered;
pub mod service;

use std::cmp::Ordering as Cmp;
use std::collections::HashSet;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use crate::bootc::{Channel, Status};
use crate::helper_client::{
    NO_ROLLBACK_QUEUED, ROLLBACK_ALREADY_QUEUED, STATE_UNREAD, STATE_UNREAD_CANCEL,
};
use crate::history;

/// bootc is always run by absolute path, and so are rpm-ostree and skopeo,
/// which stand in for it on a system with local rpm-ostree changes (see
/// [`layered`]).
pub const BOOTC: &str = "/usr/bin/bootc";
pub const RPM_OSTREE: &str = "/usr/bin/rpm-ostree";
pub const SKOPEO: &str = "/usr/bin/skopeo";
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
/// stdout, or its stderr tail on failure. rpm-ostree and skopeo likewise, for
/// a system with local rpm-ostree changes; a runner without them fails those.
pub trait BootcRunner: Send + Sync + 'static {
    fn run(&self, args: &[&str]) -> Result<String, String>;

    fn rpm_ostree(&self, _args: &[&str]) -> Result<String, String> {
        Err("rpm-ostree is not available".into())
    }

    fn skopeo(&self, _args: &[&str]) -> Result<String, String> {
        Err("skopeo is not available".into())
    }
}

/// The real runner: `/usr/bin/bootc` (and rpm-ostree, skopeo) with a cleared
/// environment, a wall-clock timeout (2 minutes for status and check, 60 for
/// upgrade, rollback and switch) and capped output.
pub struct SystemBootc;

impl BootcRunner for SystemBootc {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let short = matches!(args, ["status", ..] | ["upgrade", "--check"]);
        let timeout = if short { SHORT_TIMEOUT } else { LONG_TIMEOUT };
        run_limited(Path::new(BOOTC), args, timeout, OUTPUT_CAP)
    }

    fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
        let short = matches!(args, ["status", ..]);
        let timeout = if short { SHORT_TIMEOUT } else { LONG_TIMEOUT };
        run_limited(Path::new(RPM_OSTREE), args, timeout, OUTPUT_CAP)
    }

    fn skopeo(&self, args: &[&str]) -> Result<String, String> {
        run_limited(Path::new(SKOPEO), args, SHORT_TIMEOUT, OUTPUT_CAP)
    }
}

/// Lock, ignoring poisoning (a panicking thread must not wedge the helper).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Process groups of the bootc processes running now (an upgrade and a
/// status can run at the same time; each call adds and removes its own).
static RUNNING: Mutex<Option<HashSet<u32>>> = Mutex::new(None);

/// Set once the helper asks bootc to stop; a bootc that dies after that was
/// interrupted, not broken.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// What a stopped bootc reports (and what is never logged as an update failure).
pub const INTERRUPTED: &str = "bootc was interrupted before it finished";

/// Send `signal` to a whole process group (`false` if there was none).
fn kill_group(pgid: u32, signal: rustix::process::Signal) -> bool {
    i32::try_from(pgid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .is_some_and(|pid| rustix::process::kill_process_group(pid, signal).is_ok())
}

/// Ask every running bootc (and its children) to stop; used at shutdown.
pub fn terminate_running() {
    CLOSING.store(true, Ordering::Release);
    stop_running();
}

fn stop_running() {
    let pgids: Vec<u32> = lock(&RUNNING).iter().flatten().copied().collect();
    for pgid in pgids {
        kill_group(pgid, rustix::process::Signal::TERM);
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
    lock(&RUNNING).get_or_insert_with(HashSet::new).insert(pgid);
    // The helper may have started closing between its check and the spawn.
    if CLOSING.load(Ordering::Acquire) {
        kill_group(pgid, rustix::process::Signal::TERM);
    }
    let name = program
        .file_name()
        .map_or("bootc".into(), |n| n.to_string_lossy());
    let result = supervise(&mut child, pgid, timeout, cap, &name);
    if let Some(set) = lock(&RUNNING).as_mut() {
        set.remove(&pgid);
    }
    result
}

fn supervise(
    child: &mut std::process::Child,
    pgid: u32,
    timeout: Duration,
    cap: usize,
    name: &str,
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
                kill_group(pgid, rustix::process::Signal::KILL);
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{name} did not finish within {} s and was stopped",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                kill_group(pgid, rustix::process::Signal::KILL);
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("waiting for {name} failed: {e}"));
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
        kill_group(pgid, rustix::process::Signal::KILL);
    }
    let (stdout, over) = {
        let b = lock(&out_buf);
        (b.data.clone(), b.over)
    };
    if status.success() {
        if over {
            return Err(format!("{name} printed too much output"));
        }
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        let stderr = lock(&err_buf).data.clone();
        let text = tail(&String::from_utf8_lossy(&stderr), STDERR_TAIL);
        if CLOSING.load(Ordering::Acquire) && (status.signal().is_some() || text.is_empty()) {
            return Err(INTERRUPTED.into());
        }
        Err(match (text.is_empty(), status.signal(), status.code()) {
            (false, _, _) => text,
            (true, Some(sig), _) => format!("{name} was stopped by signal {sig}"),
            (true, None, Some(code)) => {
                format!("{name} failed (exit status {code}) without a message")
            }
            (true, None, None) => format!("{name} failed without a message"),
        })
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
    /// Undo a queued rollback (runs `bootc rollback` again).
    CancelRollback,
    SwitchChannel(String),
}

impl Op {
    /// The polkit action that guards this operation.
    pub fn action_id(&self) -> &'static str {
        match self {
            Op::Status => "net.eterneon.atlas.system.status",
            Op::CheckForUpdate => "net.eterneon.atlas.system.check",
            Op::Upgrade => "net.eterneon.atlas.system.upgrade",
            Op::Rollback | Op::CancelRollback => "net.eterneon.atlas.system.rollback",
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

/// Runs operations on a [`BootcRunner`]: one changing operation at a time;
/// `Status` is read-only and never takes the busy flag.
pub struct Core {
    runner: Arc<dyn BootcRunner>,
    busy: AtomicBool,
    events: Option<PathBuf>,
    /// Where the last update check's result is kept on a system with local
    /// rpm-ostree changes (see [`layered`]); without it, it isn't kept.
    update_file: Option<PathBuf>,
    /// `bootc status` shared between callers; see [`Core::cached_status`].
    status_cache: StatusCache,
}

/// The last `bootc status` result (failures too, so a broken bootc is not
/// asked again by every caller in turn) and who is refreshing it. The lock is
/// never held while bootc runs.
#[derive(Default)]
struct StatusState {
    entry: Option<(Instant, Result<String, String>)>,
    refreshing: bool,
    waiters: usize,
    /// Bumped when a changing operation starts or ends: a status that began
    /// before is not stored.
    generation: u64,
}

#[derive(Default)]
struct StatusCache {
    state: Mutex<StatusState>,
    done: Condvar,
}

/// Most callers that wait for one running `bootc status`.
const MAX_STATUS_WAITERS: usize = 8;

struct RefreshGuard<'a>(&'a StatusCache);

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        lock(&self.0.state).refreshing = false;
        self.0.done.notify_all();
    }
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
            update_file: None,
            status_cache: StatusCache::default(),
        }
    }

    /// Keep the update check's result on a system with local rpm-ostree
    /// changes in this file ([`layered::UPDATE_FILE`]).
    pub fn with_update_file(mut self, path: PathBuf) -> Self {
        self.update_file = Some(path);
        self
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
                    _ => self.event(ok, None, None),
                }
            }
            // A bootc stopped at shutdown is not a failed update.
            Err(HelperError::Failed(m)) if m == INTERRUPTED => {}
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
        self.invalidate_status();
        let before = (self.events.is_some() && *op == Op::Upgrade)
            // a direct read: the shared one may refuse under a status flood
            .then(|| Status::from_json(&self.status_json().ok()?).ok())
            .flatten()
            .and_then(|s| s.status.staged)
            .and_then(|b| b.digest().map(str::to_string));
        let result = self.run_op(op);
        self.invalidate_status();
        // (the rollback operations record their own events)
        if !matches!(op, Op::Rollback | Op::CancelRollback) {
            self.record_outcome(op, before.as_deref(), &result);
        }
        result
    }

    fn run_op(&self, op: &Op) -> Result<String, HelperError> {
        match op {
            Op::Status => {}
            // bootc refuses both on a system with local rpm-ostree changes;
            // there they go through skopeo and rpm-ostree (see `layered`).
            // If the system turns out not to be one, or can't be asked,
            // bootc's own error stands.
            Op::CheckForUpdate => {
                if let Err(e) = self.bootc(&["upgrade", "--check"]) {
                    let Ok(Some(origin)) = self.layered_origin() else {
                        return Err(e);
                    };
                    self.layered_check(&origin)?;
                }
            }
            Op::Upgrade => {
                // stages only: never --apply (rpm-ostree: never --reboot)
                if let Err(e) = self.bootc(&["upgrade"]) {
                    let Ok(Some(_)) = self.layered_origin() else {
                        return Err(e);
                    };
                    self.rpm_ostree(&["upgrade"])?;
                    self.forget_update();
                }
            }
            Op::Rollback | Op::CancelRollback => return self.toggle_rollback(op),
            Op::SwitchChannel(channel) => {
                let json = self.bootc(&["status", "--json"])?;
                // rebase keeps the local changes, which bootc can't switch
                if let Some(origin) = self.origin_if_layered(&json)? {
                    let target = layered::origin_with_channel(&origin, channel)
                        .map_err(HelperError::InvalidArgument)?;
                    self.rpm_ostree(&["rebase", &target])?;
                    self.forget_update();
                    return self.status_json();
                }
                let current = Status::from_json(&json)
                    .map_err(|e| HelperError::Failed(format!("cannot parse bootc status: {e}")))?;
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

    /// `bootc rollback` toggles: with one queued, a second call cancels it. So
    /// Rollback and CancelRollback read the state first and refuse the wrong
    /// one. They record their own events: a refusal or a failed read before
    /// anything ran records nothing, and once bootc has changed the state the
    /// event is recorded even if the final status read then fails.
    fn toggle_rollback(&self, op: &Op) -> Result<String, HelperError> {
        let cancel = *op == Op::CancelRollback;
        let current = self.status()?.status;
        let queued = current.rollback_queued;
        // On a system with local rpm-ostree changes a second `bootc rollback`
        // leaves the first queued; `rpm-ostree rollback` toggles there.
        let layered = [&current.booted, &current.staged]
            .into_iter()
            .flatten()
            .any(|e| e.incompatible);
        if queued && !cancel {
            return Err(HelperError::Failed(ROLLBACK_ALREADY_QUEUED.into()));
        }
        if !queued && cancel {
            return Err(HelperError::Failed(NO_ROLLBACK_QUEUED.into()));
        }
        let (ok, fail) = if cancel {
            ("rollback-cancelled", "rollback-failed")
        } else {
            ("rollback-requested", "rollback-failed")
        };
        let res = if layered {
            // rpm-ostree won't roll back past a staged update; bootc drops it
            // itself, so do the same first
            if current.staged.is_some() {
                self.rpm_ostree(&["cleanup", "-p"])
            } else {
                Ok(String::new())
            }
            .and_then(|_| self.rpm_ostree(&["rollback"]))
        } else {
            self.bootc(&["rollback"])
        };
        match res {
            Err(HelperError::Failed(m)) if m == INTERRUPTED => {
                return Err(HelperError::Failed(m));
            }
            Err(HelperError::Failed(m)) => {
                self.event(fail, None, Some(&m));
                return Err(HelperError::Failed(m));
            }
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        let res = self.status_json();
        let version = (!cancel)
            .then(|| {
                let st = Status::from_json(res.as_ref().ok()?).ok()?;
                st.status.staged?.version().map(str::to_string)
            })
            .flatten();
        self.event(ok, version, None);
        // bootc did it; only reading the new state failed. Say so: the caller
        // must not take this for a failed rollback.
        res.map_err(|e| match e {
            HelperError::Failed(m) => HelperError::Failed(format!(
                "{} {m}",
                if cancel {
                    STATE_UNREAD_CANCEL
                } else {
                    STATE_UNREAD
                }
            )),
            e => e,
        })
    }

    /// Forget the cached status (an operation started or ended). Never waits
    /// for a running status.
    fn invalidate_status(&self) {
        let mut st = lock(&self.status_cache.state);
        st.entry = None;
        st.generation += 1;
    }

    /// `bootc status --json`: one process at a time, its result (an error too)
    /// shared for 2 s. Callers that arrive meanwhile wait for it, up to
    /// [`MAX_STATUS_WAITERS`] (then `Busy`); the lock is not held while bootc
    /// runs, and a changing operation drops the entry (`invalidate_status`).
    /// Only the `Status` call goes through here: the reads an operation makes
    /// for itself (`status`, `status_json`) always run bootc.
    fn cached_status(&self) -> Result<String, HelperError> {
        let cache = &self.status_cache;
        let mut st = lock(&cache.state);
        loop {
            if let Some((at, res)) = &st.entry
                && at.elapsed() < STATUS_CACHE
            {
                return res.clone().map_err(HelperError::Failed);
            }
            if !st.refreshing {
                break;
            }
            if st.waiters >= MAX_STATUS_WAITERS {
                return Err(HelperError::Busy("too many status requests".into()));
            }
            st.waiters += 1;
            st = cache.done.wait(st).unwrap_or_else(|e| e.into_inner());
            st.waiters -= 1;
        }
        st.refreshing = true;
        let generation = st.generation;
        drop(st);
        // Clears `refreshing` and wakes the waiters even if bootc's runner
        // panics: nobody may wait for a refresh that no longer runs.
        let _refresh = RefreshGuard(cache);
        let res = self.status_json();
        let mut st = lock(&cache.state);
        if st.generation == generation {
            let stored = match &res {
                Ok(j) => Ok(j.clone()),
                Err(HelperError::Failed(m)) => Err(m.clone()),
                Err(e) => Err(format!("{e:?}")),
            };
            st.entry = Some((Instant::now(), stored));
        }
        res
    }

    fn bootc(&self, args: &[&str]) -> Result<String, HelperError> {
        self.runner.run(args).map_err(HelperError::Failed)
    }

    fn rpm_ostree(&self, args: &[&str]) -> Result<String, HelperError> {
        self.runner.rpm_ostree(args).map_err(HelperError::Failed)
    }

    /// `bootc status --json`, on a system with local rpm-ostree changes with
    /// what bootc leaves out filled in from rpm-ostree (see [`layered`]); if
    /// rpm-ostree can't be read there, bootc's status as it is.
    fn status_json(&self) -> Result<String, HelperError> {
        let json = self.bootc(&["status", "--json"])?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else {
            return Ok(json);
        };
        if !layered::is_layered(&v) {
            return Ok(json);
        }
        let Ok(rpm) = self.rpm_ostree(&["status", "--json"]) else {
            return Ok(json);
        };
        Ok(layered::fill(v, &rpm, self.saved_update().as_ref()).unwrap_or(json))
    }

    /// The image rpm-ostree follows on a system with local rpm-ostree
    /// changes; `None` on one without, which bootc handles.
    fn layered_origin(&self) -> Result<Option<String>, HelperError> {
        self.origin_if_layered(&self.bootc(&["status", "--json"])?)
    }

    /// [`Core::layered_origin`] from bootc's status `json`.
    fn origin_if_layered(&self, json: &str) -> Result<Option<String>, HelperError> {
        let v: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| HelperError::Failed(format!("cannot parse bootc status: {e}")))?;
        if !layered::is_layered(&v) {
            return Ok(None);
        }
        let rpm = self.rpm_ostree(&["status", "--json"])?;
        layered::followed_origin(&rpm)
            .map(Some)
            .map_err(HelperError::Failed)
    }

    /// `upgrade --check` for a system with local rpm-ostree changes: ask the
    /// registry with skopeo and keep the answer.
    fn layered_check(&self, origin: &str) -> Result<(), HelperError> {
        let r = layered::parse_origin(origin)
            .ok_or_else(|| HelperError::Failed(format!("unusable rpm-ostree origin {origin:?}")))?;
        let out = self
            .runner
            .skopeo(&["inspect", "--", &layered::skopeo_ref(&r)])
            .map_err(HelperError::Failed)?;
        let found = layered::image_from_skopeo(&out, &r).map_err(HelperError::Failed)?;
        let Some(path) = &self.update_file else {
            return Ok(());
        };
        let text = serde_json::to_string(&found).map_err(|e| HelperError::Failed(e.to_string()))?;
        let tmp = path.with_extension("tmp");
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = std::fs::File::create(&tmp)?;
            std::io::Write::write_all(&mut f, text.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)
        };
        write().map_err(|e| HelperError::Failed(format!("cannot save the update check: {e}")))
    }

    /// Drop the last check's result once something else is staged (it
    /// would be shown as news again after switching back).
    fn forget_update(&self) {
        if let Some(path) = &self.update_file {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The last [`Core::layered_check`] result.
    fn saved_update(&self) -> Option<crate::bootc::ImageStatus> {
        let text = std::fs::read_to_string(self.update_file.as_ref()?).ok()?;
        serde_json::from_str(&text).ok()
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
        // a rollback that was requested and not cancelled afterwards
        let rollback_queued = self.events.as_deref().is_some_and(|p| {
            events::read(p)
                .iter()
                .filter(|e| e.time >= prior.first_booted)
                .fold(false, |q, e| match e.event.as_str() {
                    "rollback-requested" => true,
                    "rollback-cancelled" => false,
                    _ => q,
                })
        });
        let image_changed = now
            .image
            .as_deref()
            .is_some_and(|i| !prior.image.is_empty() && i != prior.image);
        let switched = image_changed && since("channel-switched");
        let name = match order {
            Cmp::Greater if switched => "channel-switch-applied",
            Cmp::Greater => "update-applied",
            Cmp::Less if rollback_queued => "rollback-applied",
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
            vec!["status", "--json"], // the rollback-queued check
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

    fn queued() -> String {
        let mut v: serde_json::Value = serde_json::from_str(PLAIN).unwrap();
        v["status"]["rollbackQueued"] = true.into();
        v.to_string()
    }

    /// bootc whose `status` fails from the `fail_status_from`-th call on.
    struct StatusBreaks {
        calls: Mutex<usize>,
        fail_status_from: usize,
    }
    impl BootcRunner for StatusBreaks {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            if args[0] != "status" {
                return Ok(String::new());
            }
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n >= self.fail_status_from {
                Err("status broke".into())
            } else {
                Ok(PLAIN.into())
            }
        }
    }

    #[test]
    fn rollback_events_follow_what_actually_happened() {
        let d = tempfile::tempdir().unwrap();
        let ev = |r| Core::new(r).with_events(d.path().join("events.jsonl"));
        // the pre-check cannot read the state: nothing ran, nothing recorded
        let c = ev(Arc::new(StatusBreaks {
            calls: Mutex::new(0),
            fail_status_from: 1,
        }));
        assert!(c.execute(&Op::Rollback).is_err());
        assert!(c.execute(&Op::CancelRollback).is_err());
        assert!(names(&d).is_empty());
        // the rollback ran, then the final status read failed: still recorded
        let c = ev(Arc::new(StatusBreaks {
            calls: Mutex::new(0),
            fail_status_from: 2,
        }));
        let e = c.execute(&Op::Rollback).unwrap_err();
        assert!(
            matches!(&e, HelperError::Failed(m) if m.starts_with(STATE_UNREAD)),
            "{e:?}"
        );
        assert_eq!(names(&d), ["rollback-requested"]);
    }

    #[test]
    fn a_second_rollback_is_refused_and_cancel_undoes_the_first() {
        // queued already: Rollback must not run bootc (it would cancel it)
        let d = tempfile::tempdir().unwrap();
        let f = Fake::new(&queued());
        let c = with_ev(&f, &d);
        assert!(matches!(
            c.execute(&Op::Rollback),
            Err(HelperError::Failed(m)) if m == ROLLBACK_ALREADY_QUEUED
        ));
        assert!(!f.calls().iter().any(|a| a == &["rollback"]));
        assert!(names(&d).is_empty(), "a refusal is not a failure event");
        // CancelRollback runs it once and records the cancel
        c.execute(&Op::CancelRollback).unwrap();
        assert_eq!(f.calls().iter().filter(|a| *a == &["rollback"]).count(), 1);
        assert_eq!(names(&d), ["rollback-cancelled"]);
        // nothing queued: nothing to cancel, and bootc is not run
        let f = Fake::new(PLAIN);
        let c = with_ev(&f, &d);
        assert!(matches!(
            c.execute(&Op::CancelRollback),
            Err(HelperError::Failed(m)) if m == NO_ROLLBACK_QUEUED
        ));
        assert!(!f.calls().iter().any(|a| a == &["rollback"]));
        assert_eq!(Op::CancelRollback.action_id(), Op::Rollback.action_id());
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
        assert_eq!(*r.0.lock().unwrap(), 5); // queued check + rollback + its status + a fresh status
    }

    #[test]
    fn a_panicking_status_does_not_wedge_later_callers() {
        struct Panicky(Mutex<bool>);
        impl BootcRunner for Panicky {
            fn run(&self, _a: &[&str]) -> Result<String, String> {
                let mut first = self.0.lock().unwrap_or_else(|e| e.into_inner());
                if std::mem::replace(&mut *first, false) {
                    panic!("runner bug");
                }
                Ok(PLAIN.into())
            }
        }
        let c = Arc::new(Core::new(Arc::new(Panicky(Mutex::new(true)))));
        let c2 = c.clone();
        assert!(
            std::thread::spawn(move || c2.execute(&Op::Status))
                .join()
                .is_err()
        );
        assert_eq!(c.execute(&Op::Status).unwrap(), PLAIN);
    }

    #[test]
    fn failed_status_is_cached_briefly_and_does_not_block_operations() {
        struct Failing(Mutex<usize>);
        impl BootcRunner for Failing {
            fn run(&self, a: &[&str]) -> Result<String, String> {
                if a[0] == "status" {
                    *self.0.lock().unwrap() += 1;
                    std::thread::sleep(Duration::from_millis(100));
                    return Err("boom".into());
                }
                Ok(String::new())
            }
        }
        let r = Arc::new(Failing(Mutex::new(0)));
        let c = Arc::new(Core::new(r.clone()));
        let hs: Vec<_> = (0..6)
            .map(|_| {
                let c = c.clone();
                std::thread::spawn(move || c.execute(&Op::Status))
            })
            .collect();
        for h in hs {
            assert!(matches!(h.join().unwrap(), Err(HelperError::Failed(m)) if m == "boom"));
        }
        assert_eq!(*r.0.lock().unwrap(), 1, "one bootc run, failure shared");
        // invalidating never waits for a running status
        let c2 = c.clone();
        let slow = std::thread::spawn(move || c2.execute(&Op::Status));
        std::thread::sleep(Duration::from_millis(20));
        let t = Instant::now();
        c.invalidate_status();
        assert!(t.elapsed() < Duration::from_millis(50));
        let _ = slow.join().unwrap();
    }

    #[test]
    fn a_stopped_bootc_is_interrupted_not_a_failed_update() {
        // not through CLOSING (global): a signal death without text still says so
        let e = run_limited(
            Path::new("/bin/sh"),
            &["-c", "kill -KILL $$"],
            Duration::from_secs(5),
            1000,
        )
        .unwrap_err();
        assert!(e.contains("signal 9"), "{e}");
        let e = run_limited(
            Path::new("/bin/sh"),
            &["-c", "exit 3"],
            Duration::from_secs(5),
            1000,
        )
        .unwrap_err();
        assert!(e.contains("exit status 3"), "{e}");
        // the failure event is skipped for an interruption
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("events.jsonl");
        let c = Core::new(Fake::new(PLAIN)).with_events(p.clone());
        c.record_outcome(
            &Op::Upgrade,
            None,
            &Err(HelperError::Failed(INTERRUPTED.into())),
        );
        assert!(events::read(&p).is_empty());
        c.record_outcome(&Op::Upgrade, None, &Err(HelperError::Failed("x".into())));
        assert_eq!(events::read(&p).len(), 1);
    }

    #[test]
    fn two_running_groups_are_tracked_at_once() {
        // (RUNNING is shared with parallel tests, so nothing here signals it)
        let hs: Vec<_> = (0..2)
            .map(|_| {
                std::thread::spawn(|| {
                    run_limited(
                        Path::new("/bin/sleep"),
                        &["30"],
                        Duration::from_secs(1),
                        1000,
                    )
                })
            })
            .collect();
        let t = Instant::now();
        while lock(&RUNNING).as_ref().map_or(0, |s| s.len()) < 2 {
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(20));
        }
        for h in hs {
            assert!(h.join().unwrap().is_err());
        }
        assert!(t.elapsed() < Duration::from_secs(15));
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

    /// A system with local rpm-ostree changes, as the test VM showed it:
    /// bootc refuses to change it, rpm-ostree and skopeo answer.
    struct Layered {
        calls: Mutex<Vec<Vec<String>>>,
        rpm_ostree_status: Result<String, String>,
    }

    const REFUSED: &str = "error: Upgrading: Deployment contains local rpm-ostree modifications; cannot upgrade via bootc.";
    const SKOPEO_OUT: &str = r#"{"Digest":"sha256:ccc","Created":"2026-10-09T04:00:00Z",
        "Labels":{"org.opencontainers.image.version":"44.20261009"}}"#;

    impl Layered {
        fn new() -> Arc<Layered> {
            Arc::new(Layered {
                calls: Mutex::default(),
                rpm_ostree_status: Ok(
                    include_str!("../../tests/fixtures/rpm-ostree-layered.json").into()
                ),
            })
        }
        fn record(&self, program: &str, args: &[&str]) {
            let mut call = vec![program.to_string()];
            call.extend(args.iter().map(|s| s.to_string()));
            self.calls.lock().unwrap().push(call);
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl BootcRunner for Layered {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            self.record("bootc", args);
            match args {
                ["status", "--json"] => {
                    Ok(include_str!("../../tests/fixtures/status-layered.json").into())
                }
                _ => Err(REFUSED.into()),
            }
        }
        fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
            self.record("rpm-ostree", args);
            match args {
                ["status", "--json"] => self.rpm_ostree_status.clone(),
                _ => Ok(String::new()),
            }
        }
        fn skopeo(&self, args: &[&str]) -> Result<String, String> {
            self.record("skopeo", args);
            Ok(SKOPEO_OUT.into())
        }
    }

    fn ran(calls: &[Vec<String>], want: &[&str]) -> bool {
        calls.iter().any(|c| c == want)
    }

    #[test]
    fn layered_status_has_the_image_and_channel() {
        let f = Layered::new();
        let s = Status::from_json(&Core::new(f.clone()).execute(&Op::Status).unwrap()).unwrap();
        assert_eq!(s.channel(), Some(Channel::Stable));
        assert_eq!(
            s.status.booted.as_ref().unwrap().version(),
            Some("44.20261034")
        );
    }

    #[test]
    fn layered_status_without_rpm_ostree_is_bootc_s() {
        let f = Arc::new(Layered {
            calls: Mutex::default(),
            rpm_ostree_status: Err("rpm-ostreed is not running".into()),
        });
        let s = Status::from_json(&Core::new(f).execute(&Op::Status).unwrap()).unwrap();
        assert_eq!(s.channel(), None);
    }

    #[test]
    fn layered_check_asks_the_registry_and_keeps_the_answer() {
        let d = tempfile::tempdir().unwrap();
        let f = Layered::new();
        let c = Core::new(f.clone()).with_update_file(d.path().join("u/layered-update.json"));
        let s = Status::from_json(&c.execute(&Op::CheckForUpdate).unwrap()).unwrap();
        assert!(ran(
            &f.calls(),
            &[
                "skopeo",
                "inspect",
                "--",
                "oci:/var/mnt/atlasreg/registry:stable"
            ]
        ));
        let update = s.available_update().unwrap();
        assert_eq!(update.image_digest, "sha256:ccc");
        assert_eq!(update.version.as_deref(), Some("44.20261009"));
        // a later helper (the first exited when idle) still shows it
        let later =
            Core::new(Layered::new()).with_update_file(d.path().join("u/layered-update.json"));
        let s = Status::from_json(&later.execute(&Op::Status).unwrap()).unwrap();
        assert!(s.update_available());
    }

    #[test]
    fn layered_upgrade_stages_with_rpm_ostree() {
        let d = tempfile::tempdir().unwrap();
        let f = Layered::new();
        let c = Core::new(f.clone()).with_update_file(d.path().join("layered-update.json"));
        c.execute(&Op::CheckForUpdate).unwrap();
        c.execute(&Op::Upgrade).unwrap();
        // what it staged is no longer news
        assert!(!d.path().join("layered-update.json").exists());
        let calls = f.calls();
        assert!(ran(&calls, &["rpm-ostree", "upgrade"]));
        // stages only: no reboot, no apply
        assert!(
            calls
                .iter()
                .flatten()
                .all(|a| !matches!(a.as_str(), "--apply" | "--reboot" | "-r"))
        );
    }

    #[test]
    fn layered_switch_rebases_to_the_other_tag() {
        let f = Layered::new();
        Core::new(f.clone())
            .execute(&Op::SwitchChannel("testing".into()))
            .unwrap();
        let calls = f.calls();
        assert!(ran(
            &calls,
            &[
                "rpm-ostree",
                "rebase",
                "ostree-unverified-image:oci:/var/mnt/atlasreg/registry:testing"
            ]
        ));
        assert!(!calls.iter().any(|c| c[..2] == ["bootc", "switch"]));
    }

    #[test]
    fn layered_rollback_uses_rpm_ostree() {
        let f = Layered::new();
        Core::new(f.clone()).execute(&Op::Rollback).unwrap();
        let calls = f.calls();
        // the fixture has an update staged, which goes first
        let at = |want: &[&str]| calls.iter().position(|c| c == want);
        assert!(
            at(&["rpm-ostree", "cleanup", "-p"]).unwrap()
                < at(&["rpm-ostree", "rollback"]).unwrap()
        );
        assert!(!ran(&f.calls(), &["bootc", "rollback"]));
    }

    #[test]
    fn bootc_s_error_stays_when_rpm_ostree_can_t_be_asked() {
        let f = Arc::new(Layered {
            calls: Mutex::default(),
            rpm_ostree_status: Err("rpm-ostreed is not running".into()),
        });
        for op in [Op::CheckForUpdate, Op::Upgrade] {
            match Core::new(f.clone()).execute(&op) {
                Err(HelperError::Failed(m)) => assert_eq!(m, REFUSED),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_plain_system_s_bootc_error_stays() {
        // bootc fails for some other reason: rpm-ostree is not tried
        let f = Arc::new(Fake {
            calls: Mutex::default(),
            status: PLAIN.into(),
            after_upgrade: None,
            fail_on: Some("upgrade"),
        });
        match core(&f).execute(&Op::CheckForUpdate) {
            Err(HelperError::Failed(m)) => assert_eq!(m, "boom"),
            other => panic!("{other:?}"),
        }
    }
}
