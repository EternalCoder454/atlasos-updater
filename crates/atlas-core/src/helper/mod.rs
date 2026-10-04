//! The privileged system helper: the logic behind `atlas-system-helper`.
//!
//! Not for apps; they use [`crate::helper_client`]. The D-Bus and polkit glue
//! is in [`service`]; everything else here is plain code that unit tests drive
//! with a fake [`BootcRunner`].

pub use atlas_framework_system::events;
pub mod layered;
pub mod live;
pub mod retry;
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

use crate::bootc::{Channel, Status, version_cmp};
use crate::helper_client::{
    DOWNGRADE_REFUSED, NO_ROLLBACK_QUEUED, ROLLBACK_ALREADY_QUEUED, STATE_UNREAD,
    STATE_UNREAD_CANCEL,
};
use crate::history;
use crate::progress::{BootcParser, RpmOstreeParser};
use live::{ProgressCell, ProgressSink};

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
/// The check before a pull is fail-open, so it gets one short try.
const QUICK_TIMEOUT: Duration = Duration::from_secs(30);
/// bootc's registry credentials, first one found (as the stager's condition).
const AUTH_FILES: [&str; 2] = ["/run/ostree/auth.json", "/etc/ostree/auth.json"];
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

    /// One quick `skopeo` call (about 30 seconds at most) for the check made
    /// before a pull; a runner without its own limit uses [`skopeo`](Self::skopeo).
    fn skopeo_quick(&self, args: &[&str]) -> Result<String, String> {
        self.skopeo(args)
    }

    /// [`run`](Self::run) for `upgrade` and `switch`, reporting progress to
    /// `sink` while it runs. A runner that has none just runs.
    fn run_progress(&self, args: &[&str], _sink: &ProgressSink) -> Result<String, String> {
        self.run(args)
    }

    /// [`rpm_ostree`](Self::rpm_ostree) for `upgrade` and `rebase`, likewise.
    fn rpm_ostree_progress(&self, args: &[&str], _sink: &ProgressSink) -> Result<String, String> {
        self.rpm_ostree(args)
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

    fn run_progress(&self, args: &[&str], sink: &ProgressSink) -> Result<String, String> {
        run_with(
            Path::new(BOOTC),
            args,
            LONG_TIMEOUT,
            OUTPUT_CAP,
            Feed::BootcFd(sink.clone()),
        )
    }

    fn rpm_ostree_progress(&self, args: &[&str], sink: &ProgressSink) -> Result<String, String> {
        run_with(
            Path::new(RPM_OSTREE),
            args,
            LONG_TIMEOUT,
            OUTPUT_CAP,
            Feed::Stdout(sink.clone()),
        )
    }

    fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
        let short = matches!(args, ["status", ..]);
        let timeout = if short { SHORT_TIMEOUT } else { LONG_TIMEOUT };
        run_limited(Path::new(RPM_OSTREE), args, timeout, OUTPUT_CAP)
    }

    fn skopeo(&self, args: &[&str]) -> Result<String, String> {
        run_limited(Path::new(SKOPEO), args, SHORT_TIMEOUT, OUTPUT_CAP)
    }

    fn skopeo_quick(&self, args: &[&str]) -> Result<String, String> {
        run_limited(Path::new(SKOPEO), args, QUICK_TIMEOUT, OUTPUT_CAP)
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

/// Set when the helper is told to stop (SIGTERM): a step waiting to be tried
/// again gives up at once rather than start a fetch that the shutdown would
/// cut off.
static STOP_RETRY: AtomicBool = AtomicBool::new(false);

/// No more retries (see [`retry`]); the running step goes on.
pub fn stop_retrying() {
    STOP_RETRY.store(true, Ordering::Release);
}

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
/// `tap` sees every chunk read, capped or not.
fn read_capped(
    mut r: impl Read,
    cap: usize,
    buf: Arc<Mutex<Captured>>,
    done: mpsc::Sender<()>,
    mut tap: Option<Tap>,
) {
    let mut chunk = [0u8; 16 * 1024];
    while let Ok(n) = r.read(&mut chunk) {
        if n == 0 {
            break;
        }
        if let Some(t) = tap.as_mut() {
            t(&chunk[..n]);
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
    run_with(program, args, timeout, cap, Feed::None)
}

/// Sees each chunk of a program's stdout as it arrives.
type Tap = Box<dyn FnMut(&[u8]) + Send>;

/// Where a running program's progress comes from.
enum Feed {
    None,
    /// bootc: JSON lines on an extra pipe, whose number is the `--progress-fd`
    /// argument (added to `args`, after the subcommand).
    BootcFd(ProgressSink),
    /// rpm-ostree: parsed from stdout as it arrives.
    Stdout(ProgressSink),
}

/// [`run_limited`] with progress. Progress is an addition: if the pipe cannot
/// be made, the program runs without it.
fn run_with(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    cap: usize,
    feed: Feed,
) -> Result<String, String> {
    match feed {
        Feed::BootcFd(sink) => {
            // note whether bootc reported anything: one that did accepted the flag
            let reported = Arc::new(AtomicBool::new(false));
            let (r2, s2) = (reported.clone(), sink);
            let counting: ProgressSink = Arc::new(move |p| {
                r2.store(true, Ordering::Release);
                s2(p)
            });
            let first = run_once(program, args, timeout, cap, Feed::BootcFd(counting));
            // an older bootc without the (hidden) flag: run it once without
            match first {
                Err(e) if !reported.load(Ordering::Acquire) && rejects_progress_fd(&e) => {
                    run_once(program, args, timeout, cap, Feed::None)
                }
                other => other,
            }
        }
        other => run_once(program, args, timeout, cap, other),
    }
}

/// Whether bootc's error says it does not know `--progress-fd` (clap's
/// "unexpected argument '--progress-fd' found", or "unrecognized"/"unknown"
/// next to the flag), as opposed to a failure that merely names it.
fn rejects_progress_fd(stderr: &str) -> bool {
    stderr.contains("unexpected argument '--progress-fd'")
        || stderr.lines().any(|l| {
            l.contains("--progress-fd") && (l.contains("unrecognized") || l.contains("unknown"))
        })
}

fn run_once(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    cap: usize,
    feed: Feed,
) -> Result<String, String> {
    let mut cmd = Command::new(program);
    let mut progress_rx = None;
    let mut stdout_tap: Option<Tap> = None;
    let mut fd_arg = String::new();
    let mut write_end = None;
    match feed {
        Feed::None => {}
        Feed::Stdout(sink) => {
            let mut parser = RpmOstreeParser::new();
            stdout_tap = Some(Box::new(move |c: &[u8]| {
                for p in parser.feed(c) {
                    sink(p);
                }
            }));
        }
        Feed::BootcFd(sink) => {
            if let Ok((r, w)) = std::io::pipe() {
                use std::os::fd::AsRawFd;
                let fd = w.as_raw_fd();
                // a pipe on 0, 1 or 2 would clash with the child's stdio
                if fd <= 2 {
                    return run_once(program, args, timeout, cap, Feed::None);
                }
                fd_arg = fd.to_string();
                // SAFETY: the closure only calls fcntl, which is async-signal-safe.
                // It clears FD_CLOEXEC on the write end in the child alone, so
                // no other program the helper starts inherits it.
                unsafe {
                    cmd.pre_exec(move || {
                        let flags = libc::fcntl(fd, libc::F_GETFD);
                        if flags < 0
                            || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                        {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                write_end = Some(w);
                progress_rx = Some((r, sink));
            }
        }
    }
    // `--progress-fd <n>` goes right after the subcommand (a fixed position;
    // nothing from the D-Bus caller is in it)
    let mut full: Vec<&str> = args.to_vec();
    if !fd_arg.is_empty() && !full.is_empty() {
        full.insert(1, "--progress-fd");
        full.insert(2, &fd_arg);
    }
    let spawned = cmd
        .args(&full)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn();
    // Close our copy of the write end, so the reader sees EOF when the child
    // (and anything it started) is done. (`cmd` holds the fd number only.)
    drop(write_end);
    let mut child = spawned.map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    let pgid = child.id();
    lock(&RUNNING).get_or_insert_with(HashSet::new).insert(pgid);
    // The helper may have started closing between its check and the spawn.
    if CLOSING.load(Ordering::Acquire) {
        kill_group(pgid, rustix::process::Signal::TERM);
    }
    let name = program
        .file_name()
        .map_or("bootc".into(), |n| n.to_string_lossy());
    let result = supervise(
        &mut child,
        pgid,
        timeout,
        cap,
        &name,
        stdout_tap,
        progress_rx,
    );
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
    stdout_tap: Option<Tap>,
    progress: Option<(std::io::PipeReader, ProgressSink)>,
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
        std::thread::spawn(move || read_capped(out, cap, b, out_tx, stdout_tap));
        let b = err_buf.clone();
        std::thread::spawn(move || read_capped(err, cap, b, err_tx, None));
    }
    let (prog_tx, prog_rx) = mpsc::channel();
    match progress {
        Some((mut r, sink)) => {
            std::thread::spawn(move || {
                let mut parser = BootcParser::new();
                let mut chunk = [0u8; 8 * 1024];
                while let Ok(n) = r.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    for p in parser.feed(&chunk[..n]) {
                        sink(p);
                    }
                }
                let _ = prog_tx.send(());
            });
        }
        None => drop(prog_tx),
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
    // the progress reader too, so no update arrives after the call returns
    // (no progress pipe: its sender is gone and this returns at once)
    let left = end.saturating_duration_since(Instant::now());
    if let Err(mpsc::RecvTimeoutError::Timeout) = prog_rx.recv_timeout(left) {
        pipes_closed = false;
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

/// The refusal text for `found`, an image older than what `before` has.
fn downgrade_message(before: &Status, found: &crate::bootc::ImageStatus) -> String {
    let installed = before
        .status
        .booted
        .as_ref()
        .and_then(|b| b.version())
        .unwrap_or("unknown");
    let found = found.version.as_deref().unwrap_or(&found.image_digest);
    format!("{DOWNGRADE_REFUSED} (found {found}, installed {installed})")
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
    /// The containers policy (`policy.json`); a channel switch enforces the
    /// signatures it demands. Without it, the switch keeps the booted image's
    /// signature setting.
    policy: Option<PathBuf>,
    /// `bootc status` shared between callers; see [`Core::cached_status`].
    status_cache: StatusCache,
    /// The `Progress` property while an upgrade or switch runs.
    progress: Arc<ProgressCell>,
    /// Waits between tries of a step that fetched from the registry (see
    /// [`retry`]); false stops the retrying.
    retry_wait: fn(Duration) -> bool,
}

/// Sleep `d`, unless the helper is shutting down: then stop at once (false).
fn wait_unless_closing(d: Duration) -> bool {
    let end = Instant::now() + d;
    loop {
        if CLOSING.load(Ordering::Acquire) || STOP_RETRY.load(Ordering::Acquire) {
            return false;
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(200)));
    }
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

/// What [`Core::read_state`] read.
struct StateRead {
    /// `bootc status --json` as bootc gave it
    bootc: String,
    /// bootc says the system has local rpm-ostree changes
    layered: bool,
    /// `rpm-ostree status --json` on such a system, if it could be read
    rpm: Option<String>,
    /// [`Core::status_json`]'s answer: `bootc`, with `rpm` filled in
    json: String,
}

/// The state an upgrade or switch starts from.
struct Before {
    read: StateRead,
    /// what it must not go back behind (`None` if `read` doesn't parse)
    status: Option<Status>,
}

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
            policy: None,
            status_cache: StatusCache::default(),
            progress: ProgressCell::new(),
            retry_wait: wait_unless_closing,
        }
    }

    /// Wait between retries with `wait` (tests: without sleeping).
    pub fn with_retry_wait(mut self, wait: fn(Duration) -> bool) -> Self {
        self.retry_wait = wait;
        self
    }

    /// Run `step`, a fetch from the registry, again while it fails with a
    /// passing network error (see [`retry`]).
    fn fetching(
        &self,
        step: impl FnMut() -> Result<String, String>,
    ) -> Result<String, HelperError> {
        retry::retrying(step, self.retry_wait).map_err(HelperError::Failed)
    }

    /// The `Progress` property: JSON of the current progress, `""` when none.
    pub fn progress_json(&self) -> String {
        self.progress.current()
    }

    /// Changes of the `Progress` property (the D-Bus service announces them).
    pub fn progress_watch(&self) -> tokio::sync::watch::Receiver<String> {
        self.progress.subscribe()
    }

    /// Keep the update check's result on a system with local rpm-ostree
    /// changes in this file ([`layered::UPDATE_FILE`]).
    pub fn with_update_file(mut self, path: PathBuf) -> Self {
        self.update_file = Some(path);
        self
    }

    /// Read the containers policy from this file
    /// ([`CONTAINERS_POLICY`](crate::bootc::CONTAINERS_POLICY)).
    pub fn with_policy(mut self, path: PathBuf) -> Self {
        self.policy = Some(path);
        self
    }

    /// True when the containers policy demands a signature for the registry
    /// image `image` (see [`crate::bootc::policy_requires_signature`]). False
    /// when it can't be read.
    fn policy_requires_signature(&self, image: &str) -> bool {
        self.policy
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .is_some_and(|v| crate::bootc::policy_requires_signature(&v, image))
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
        // What an upgrade or switch starts from: the staged digest for the
        // event, and the booted and staged images it must not go back behind.
        // The operation reuses this read instead of reading again.
        let before = matches!(op, Op::Upgrade | Op::SwitchChannel(_))
            // a direct read: the shared one may refuse under a status flood
            .then(|| self.read_state().ok())
            .flatten()
            .map(|read| Before {
                status: Status::from_json(&read.json).ok(),
                read,
            });
        let staged_before = before
            .as_ref()
            .and_then(|b| b.status.as_ref())
            .and_then(|s| s.status.staged.as_ref())
            .and_then(|b| b.digest().map(str::to_string));
        let result = self.run_op(op, before.as_ref());
        // the app's Status call right after needn't run bootc again
        self.invalidate_status_with(result.as_ref().ok());
        // (the rollback operations record their own events)
        if !matches!(op, Op::Rollback | Op::CancelRollback) {
            self.record_outcome(op, staged_before.as_deref(), &result);
        }
        result
    }

    fn run_op(&self, op: &Op, before: Option<&Before>) -> Result<String, HelperError> {
        // progress is reported for the two operations that download
        let progress = match op {
            Op::Upgrade => Some(self.progress.begin("upgrade")),
            Op::SwitchChannel(_) => Some(self.progress.begin("switch")),
            _ => None,
        };
        let sink = progress.as_ref().map(|(s, _)| s.clone());
        let result = self.run_op_with(op, sink.as_ref(), before);
        drop(progress); // clears the property
        result
    }

    fn run_op_with(
        &self,
        op: &Op,
        sink: Option<&ProgressSink>,
        before: Option<&Before>,
    ) -> Result<String, HelperError> {
        let before_status = before.and_then(|b| b.status.as_ref());
        match op {
            Op::Status => {}
            // bootc refuses both on a system with local rpm-ostree changes;
            // there they go through skopeo and rpm-ostree (see `layered`).
            // If the system turns out not to be one, or can't be asked,
            // bootc's own error stands.
            Op::CheckForUpdate => {
                if let Err(e) = self.fetching(|| self.runner.run(&["upgrade", "--check"])) {
                    let Ok(Some(origin)) = self.layered_origin() else {
                        return Err(e);
                    };
                    self.layered_check(&origin)?;
                }
            }
            Op::Upgrade => {
                // stages only: never --apply (rpm-ostree: never --reboot)
                // A system the read before shows with local changes goes to
                // rpm-ostree at once, without bootc's refusal first.
                let origin = before.and_then(|b| self.origin_of(&b.read).ok().flatten());
                let mut layered = origin.is_some();
                // An older image is refused before anything is downloaded
                // when the registry can be asked; `refuse_downgrade` below
                // stays as the backstop.
                let followed = if let Some(origin) = &origin {
                    layered::parse_origin(origin)
                } else {
                    before_status.and_then(|s| s.spec.image.clone().or(s.booted_ref().cloned()))
                };
                if let Some(target) = &followed {
                    self.refuse_older_before_pull(before_status, target)?;
                }
                if !layered && let Err(e) = self.bootc_progress(&["upgrade"], sink) {
                    // asked again: the system may have been changed since
                    if !matches!(self.layered_origin(), Ok(Some(_))) {
                        return Err(e);
                    }
                    layered = true;
                }
                if layered {
                    self.rpm_ostree_progress(&["upgrade"], sink)?;
                    self.forget_update();
                }
                return self.refuse_downgrade(before_status, self.status_json()?);
            }
            Op::Rollback | Op::CancelRollback => return self.toggle_rollback(op),
            Op::SwitchChannel(channel) => {
                let fresh;
                let read = match before {
                    Some(b) => &b.read,
                    None => {
                        fresh = self.read_state()?;
                        &fresh
                    }
                };
                // rebase keeps the local changes, which bootc can't switch
                if let Some(origin) = self.origin_of(read)? {
                    let mut target = layered::origin_with_channel(&origin, channel)
                        .map_err(HelperError::InvalidArgument)?;
                    // never record a weaker check than the policy makes
                    if let Some(image) = target
                        .strip_prefix("ostree-unverified-registry:")
                        .or_else(|| target.strip_prefix("ostree-unverified-image:docker://"))
                        && self.policy_requires_signature(image)
                    {
                        target = format!("ostree-image-signed:docker://{image}");
                    }
                    if let Some(r) = layered::parse_origin(&target) {
                        self.refuse_older_before_pull(before_status, &r)?;
                    }
                    self.rpm_ostree_progress(&["rebase", &target], sink)?;
                    self.forget_update();
                    return self.refuse_downgrade(before_status, self.status_json()?);
                }
                let current = Status::from_json(&read.bootc)
                    .map_err(|e| HelperError::Failed(format!("cannot parse bootc status: {e}")))?;
                let booted = current.booted_ref().ok_or_else(|| {
                    HelperError::Failed("bootc reports no booted image reference".into())
                })?;
                let new = booted
                    .with_channel(channel)
                    .map_err(|e| HelperError::InvalidArgument(e.to_string()))?;
                let mut args = vec!["switch", "--transport", new.transport_or_default()];
                // An unverified origin is upgraded to a checked one where the
                // policy demands a signature anyway (as the stager does), so
                // the switch never records a weaker check than the pulls get.
                let unverified = match new.signature.as_ref() {
                    None => true,
                    Some(serde_json::Value::String(s)) => s == "insecure",
                    Some(_) => false,
                };
                match new.signature.as_ref() {
                    _ if unverified
                        && new.transport_or_default() == "registry"
                        && self.policy_requires_signature(&new.image) =>
                    {
                        args.push("--enforce-container-sigpolicy");
                    }
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
                self.refuse_older_before_pull(before_status, &new)?;
                self.bootc_progress(&args, sink)?;
                // the channel's tag, even the one followed now, may have gone back
                return self.refuse_downgrade(before_status, self.status_json()?);
            }
        }
        self.status_json()
    }

    /// After an upgrade or switch: if what is staged now is older than the
    /// image booted or staged before (a tag moved back, see
    /// [`Status::is_downgrade`]), take it out again with `rpm-ostree cleanup
    /// -p` and fail. `json` is the status after it; `before` the one from
    /// before. Without `before`, the booted image (which an upgrade doesn't
    /// change) is compared from `json` alone.
    fn refuse_downgrade(
        &self,
        before: Option<&Status>,
        json: String,
    ) -> Result<String, HelperError> {
        let Ok(after) = Status::from_json(&json) else {
            return Ok(json);
        };
        let before = before.unwrap_or(&after);
        let Some(staged) = after
            .status
            .staged
            .as_ref()
            .and_then(|e| e.image.as_ref())
            .filter(|i| before.is_downgrade(i))
        else {
            return Ok(json);
        };
        let mut msg = downgrade_message(before, staged);
        // `cleanup -p` takes out what was staged before too (an update or a
        // switch to the other channel): it was replaced by the old image.
        let replaced = before
            .status
            .staged
            .as_ref()
            .and_then(|e| e.digest())
            .is_some_and(|d| d != staged.image_digest);
        if replaced {
            msg.push_str(" What was set to install at the next restart before was removed too; check for updates again.");
        }
        // (recorded as failed by `execute`, with this message)
        let mut last = String::new();
        for attempt in 0..retry::ATTEMPTS {
            if attempt > 0 && !(self.retry_wait)(retry::DELAYS[attempt - 1]) {
                break; // the helper is closing
            }
            match self.rpm_ostree(&["cleanup", "-p"]) {
                Ok(_) => return Err(HelperError::Failed(msg)),
                Err(e) => last = tail(&e.to_string(), 300),
            }
        }
        Err(HelperError::Failed(format!(
            "{msg} Removing it failed, so the older update is STILL STAGED and will install at the next restart. Remove it with `sudo rpm-ostree cleanup -p` before restarting: {last}"
        )))
    }

    /// Before an upgrade or switch pulls `target`: ask the registry (skopeo
    /// inspect, manifest and config only) and refuse an image older than the
    /// booted or staged one without downloading it. If the registry can't be
    /// asked, or `before` is unknown, nothing is refused here: the pull goes
    /// on and [`Core::refuse_downgrade`] checks what it staged.
    fn refuse_older_before_pull(
        &self,
        before: Option<&Status>,
        target: &crate::bootc::ImageReference,
    ) -> Result<(), HelperError> {
        let Some(before) = before else {
            return Ok(());
        };
        let image = layered::skopeo_ref(target);
        let mut args = vec!["inspect"];
        if let Some(auth) = AUTH_FILES.iter().find(|p| Path::new(p).is_file()) {
            args.extend(["--authfile", auth]);
        }
        args.extend(["--", &image]);
        let found = match self.runner.skopeo_quick(&args) {
            Ok(out) => layered::image_from_skopeo(&out, target),
            Err(e) => Err(e),
        };
        let found = match found {
            Ok(found) => found,
            Err(e) => {
                eprintln!("atlas-system-helper: pre-pull check skipped: {e}");
                return Ok(());
            }
        };
        if before.is_downgrade(&found) {
            let msg = downgrade_message(before, &found);
            return Err(HelperError::Failed(format!(
                "{msg} Nothing was downloaded."
            )));
        }
        Ok(())
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
        self.invalidate_status_with(None);
    }

    /// [`Core::invalidate_status`], sharing `json` as the cached status in
    /// the same step: an operation's last read, made as it ended.
    fn invalidate_status_with(&self, json: Option<&String>) {
        let mut st = lock(&self.status_cache.state);
        st.entry = json.map(|j| (Instant::now(), Ok(j.clone())));
        st.generation += 1;
    }

    /// `bootc status --json`: one process at a time, its result (an error too)
    /// shared for 2 s. Callers that arrive meanwhile wait for it, up to
    /// [`MAX_STATUS_WAITERS`] (then `Busy`); the lock is not held while bootc
    /// runs, and a changing operation drops the entry (`invalidate_status`)
    /// and shares the status it ended with (`invalidate_status_with`).
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

    /// bootc `upgrade` or `switch`, which fetch: retried (see [`retry`]).
    fn bootc_progress(
        &self,
        args: &[&str],
        sink: Option<&ProgressSink>,
    ) -> Result<String, HelperError> {
        self.fetching(|| match sink {
            Some(s) => self.runner.run_progress(args, s),
            None => self.runner.run(args),
        })
    }

    /// rpm-ostree `upgrade` or `rebase`, which fetch: retried likewise.
    fn rpm_ostree_progress(
        &self,
        args: &[&str],
        sink: Option<&ProgressSink>,
    ) -> Result<String, HelperError> {
        self.fetching(|| match sink {
            Some(s) => self.runner.rpm_ostree_progress(args, s),
            None => self.runner.rpm_ostree(args),
        })
    }

    fn rpm_ostree(&self, args: &[&str]) -> Result<String, HelperError> {
        self.runner.rpm_ostree(args).map_err(HelperError::Failed)
    }

    /// `bootc status --json`, on a system with local rpm-ostree changes with
    /// what bootc leaves out filled in from rpm-ostree (see [`layered`]); if
    /// rpm-ostree can't be read there, bootc's status as it is.
    fn status_json(&self) -> Result<String, HelperError> {
        self.read_state().map(|r| r.json)
    }

    /// One read of bootc's status, and rpm-ostree's where it fills it in.
    fn read_state(&self) -> Result<StateRead, HelperError> {
        let bootc = self.bootc(&["status", "--json"])?;
        let v = serde_json::from_str::<serde_json::Value>(&bootc).ok();
        let layered = v.as_ref().is_some_and(layered::is_layered);
        let rpm = layered
            .then(|| self.rpm_ostree(&["status", "--json"]).ok())
            .flatten();
        let json = match (v, &rpm) {
            (Some(v), Some(rpm)) => layered::fill(v, rpm, self.saved_update().as_ref())
                .unwrap_or_else(|_| bootc.clone()),
            _ => bootc.clone(),
        };
        Ok(StateRead {
            bootc,
            layered,
            rpm,
            json,
        })
    }

    /// The image rpm-ostree follows on a system with local rpm-ostree
    /// changes; `None` on one without, which bootc handles.
    fn layered_origin(&self) -> Result<Option<String>, HelperError> {
        self.origin_of(&self.read_state()?)
    }

    /// [`Core::layered_origin`] from a `read` already made (rpm-ostree is
    /// asked again if it couldn't be read then).
    fn origin_of(&self, read: &StateRead) -> Result<Option<String>, HelperError> {
        if !read.layered {
            return Ok(None);
        }
        let fresh;
        let rpm = match &read.rpm {
            Some(rpm) => rpm,
            None => {
                fresh = self.rpm_ostree(&["status", "--json"])?;
                &fresh
            }
        };
        layered::followed_origin(rpm)
            .map(Some)
            .map_err(HelperError::Failed)
    }

    /// `upgrade --check` for a system with local rpm-ostree changes: ask the
    /// registry with skopeo and keep the answer.
    fn layered_check(&self, origin: &str) -> Result<(), HelperError> {
        let r = layered::parse_origin(origin)
            .ok_or_else(|| HelperError::Failed(format!("unusable rpm-ostree origin {origin:?}")))?;
        let image = layered::skopeo_ref(&r);
        let out = self.fetching(|| self.runner.skopeo(&["inspect", "--", &image]))?;
        let found = layered::image_from_skopeo(&out, &r).map_err(HelperError::Failed)?;
        let Some(path) = &self.update_file else {
            return Ok(());
        };
        let text = serde_json::to_string(&found).map_err(|e| HelperError::Failed(e.to_string()))?;
        let tmp = path.with_extension("tmp");
        let write = || -> std::io::Result<()> {
            use std::os::unix::fs::OpenOptionsExt;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // a stale or planted temp file is replaced, never followed
            match std::fs::remove_file(&tmp) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&tmp)?;
            let done = std::io::Write::write_all(&mut f, text.as_bytes())
                .and_then(|()| f.sync_all())
                .and_then(|()| std::fs::rename(&tmp, path));
            if done.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            done?;
            // make the rename itself durable
            if let Some(dir) = path.parent() {
                std::fs::File::open(dir)?.sync_all()?;
            }
            Ok(())
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
    use crate::fixtures::{BOOTED_WITH_UPDATE, PLAIN};
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

    /// bootc whose first `fails` calls other than `status` fail with `error`.
    struct Flaky {
        calls: Mutex<Vec<Vec<String>>>,
        fails: usize,
        error: &'static str,
    }

    impl Flaky {
        fn new(fails: usize, error: &'static str) -> Arc<Flaky> {
            Arc::new(Flaky {
                calls: Mutex::default(),
                fails,
                error,
            })
        }
        fn count(&self, args: &[&str]) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| *c == args)
                .count()
        }
    }

    impl BootcRunner for Flaky {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(args.iter().map(|s| s.to_string()).collect());
            if args[0] == "status" {
                return Ok(PLAIN.into());
            }
            let n = calls.iter().filter(|c| c[0] != "status").count();
            if n <= self.fails {
                Err(self.error.into())
            } else {
                Ok(String::new())
            }
        }
    }

    const RESET: &str = "error: Upgrading: reading blob sha256:abc: read tcp 10.0.0.2:51234->185.199.108.154:443: read: connection reset by peer";

    #[test]
    fn a_dropped_connection_is_tried_again() {
        let no_sleep: fn(Duration) -> bool = |_| true;
        for (op, args) in [
            (Op::Upgrade, vec!["upgrade"]),
            (Op::CheckForUpdate, vec!["upgrade", "--check"]),
        ] {
            let f = Flaky::new(2, RESET);
            let c = Core::new(f.clone()).with_retry_wait(no_sleep);
            assert!(c.execute(&op).is_ok(), "{op:?}");
            assert_eq!(f.count(&args), 3, "{op:?}");
        }
        // the third failure is the last
        let f = Flaky::new(3, RESET);
        let e = Core::new(f.clone())
            .with_retry_wait(no_sleep)
            .execute(&Op::Upgrade);
        assert!(matches!(e, Err(HelperError::Failed(m)) if m == RESET));
        assert_eq!(f.count(&["upgrade"]), retry::ATTEMPTS);
        // a switch too
        let f = Flaky::new(1, RESET);
        let c = Core::new(f.clone()).with_retry_wait(no_sleep);
        c.execute(&Op::SwitchChannel("stable".into())).unwrap();
        assert_eq!(
            f.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c[0] == "switch")
                .count(),
            2
        );
    }

    #[test]
    fn a_rollback_a_lasting_error_or_a_shutdown_is_not_tried_again() {
        let f = Flaky::new(1, RESET);
        let c = Core::new(f.clone()).with_retry_wait(|_| true);
        assert!(c.execute(&Op::Rollback).is_err());
        assert_eq!(f.count(&["rollback"]), 1);
        let f = Flaky::new(1, "error: Upgrading: no space left on device");
        let c = Core::new(f.clone()).with_retry_wait(|_| true);
        assert!(c.execute(&Op::Upgrade).is_err());
        assert_eq!(f.count(&["upgrade"]), 1);
        // the wait is cut short when the helper is told to stop: an
        // interrupted update, not a failed one
        let d = tempfile::tempdir().unwrap();
        let f = Flaky::new(1, RESET);
        let c = Core::new(f.clone())
            .with_retry_wait(|_| false)
            .with_events(d.path().join("events.jsonl"));
        let e = c.execute(&Op::Upgrade);
        assert!(matches!(e, Err(HelperError::Failed(m)) if m == INTERRUPTED));
        assert_eq!(f.count(&["upgrade"]), 1);
        assert!(names(&d).is_empty());
    }

    #[test]
    fn an_upgrade_that_got_through_on_a_retry_is_still_checked() {
        // the first try's connection drops; the second stages an older build
        let f = Arc::new(Stager {
            calls: Mutex::default(),
            before: PLAIN.into(),
            after: plain_with_staged("44.20260920", "2026-09-20T04:00:00Z", "sha256:old"),
            cleanup_fails: false,
            before_fails: false,
            flaky: 1,
        });
        let c = Core::new(f.clone()).with_retry_wait(|_| true);
        match c.execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => assert!(m.starts_with(DOWNGRADE_REFUSED), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(f.calls().iter().filter(|c| c[0] == "upgrade").count(), 2);
        assert!(ran(&f.calls(), &["rpm-ostree", "cleanup", "-p"]));
    }

    #[test]
    fn a_layered_upgrade_goes_to_rpm_ostree_at_once_and_retries_there() {
        struct FlakyRpm(Arc<Layered>, Mutex<usize>);
        impl BootcRunner for FlakyRpm {
            fn run(&self, args: &[&str]) -> Result<String, String> {
                self.0.run(args)
            }
            fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
                if args == ["upgrade"] {
                    let mut n = self.1.lock().unwrap();
                    *n += 1;
                    if *n == 1 {
                        self.0.record("rpm-ostree", args);
                        return Err(RESET.into());
                    }
                }
                self.0.rpm_ostree(args)
            }
        }
        let l = Layered::new();
        let f = Arc::new(FlakyRpm(l.clone(), Mutex::new(0)));
        Core::new(f)
            .with_retry_wait(|_| true)
            .execute(&Op::Upgrade)
            .unwrap();
        let calls = l.calls();
        // the read before shows the local changes: bootc isn't asked
        assert!(!ran(&calls, &["bootc", "upgrade"]));
        assert_eq!(
            calls
                .iter()
                .filter(|c| *c == &["rpm-ostree", "upgrade"])
                .count(),
            2
        );
    }

    #[test]
    fn a_layered_check_asks_the_registry_again() {
        struct FlakySkopeo(Arc<Layered>, Mutex<usize>);
        impl BootcRunner for FlakySkopeo {
            fn run(&self, args: &[&str]) -> Result<String, String> {
                self.0.run(args)
            }
            fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
                self.0.rpm_ostree(args)
            }
            fn skopeo(&self, args: &[&str]) -> Result<String, String> {
                let mut n = self.1.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    return Err("pinging container registry ghcr.io: i/o timeout".into());
                }
                self.0.skopeo(args)
            }
        }
        let f = Arc::new(FlakySkopeo(Layered::new(), Mutex::new(0)));
        let c = Core::new(f.clone()).with_retry_wait(|_| true);
        c.execute(&Op::CheckForUpdate).unwrap();
        assert_eq!(*f.1.lock().unwrap(), 2);
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
            vec!["status", "--json"], // what the upgrade must not go back behind
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

    /// The `bootc switch` among `calls` (status reads come before and after).
    fn switch_call(calls: &[Vec<String>]) -> Vec<String> {
        calls
            .iter()
            .find(|c| c[0] == "switch")
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn switch_rewrites_only_the_tag_of_the_booted_ref() {
        let f = Fake::new(BOOTED_WITH_UPDATE); // booted :stable
        core(&f)
            .execute(&Op::SwitchChannel("testing".into()))
            .unwrap();
        assert_eq!(
            switch_call(&f.calls()),
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
            switch_call(&f.calls()),
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
        let f = Fake::new(crate::fixtures::NOT_BOOTC);
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
        assert_eq!(
            version_cmp("44.20261008-10", "44.20261008-2"),
            Some(Cmp::Greater)
        );
        assert_eq!(version_cmp("44.20261008", "44.20261008-1"), Some(Cmp::Less));
        assert_eq!(
            version_cmp("44.20261009-1", "44.20261008-5"),
            Some(Cmp::Greater)
        );
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
        assert_eq!(*r.0.lock().unwrap(), 4); // queued check + rollback + its status
        // and shares the status it ended with
        c.execute(&Op::Status).unwrap();
        assert_eq!(*r.0.lock().unwrap(), 4);
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
        assert!(switch_call(&f.calls()).contains(&"--enforce-container-sigpolicy".to_string()));
        assert_eq!(
            switch_call(&f.calls()).last().unwrap(),
            "ghcr.io/eternalcoder454/atlasos:testing"
        );
        let f = Fake::new(&signed("\"insecure\""));
        core(&f)
            .execute(&Op::SwitchChannel("testing".into()))
            .unwrap();
        assert!(!switch_call(&f.calls()).contains(&"--enforce-container-sigpolicy".to_string()));
        let f = Fake::new(&signed("{\"ostreeRemoteSignature\": \"fedora\"}"));
        assert!(matches!(
            core(&f).execute(&Op::SwitchChannel("testing".into())),
            Err(HelperError::Failed(_))
        ));
        assert!(
            f.calls().iter().all(|c| c[0] == "status"),
            "refused before switching"
        );
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

    fn script(d: &tempfile::TempDir, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = d.path().join("fake");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// `run_with`, retried while the just-written script is "Text file busy"
    /// (another test thread forked while it was open for writing).
    fn go(
        prog: &Path,
        args: &[&str],
        timeout: Duration,
        feed: impl Fn() -> Feed,
    ) -> Result<String, String> {
        for _ in 0..40 {
            match run_with(prog, args, timeout, 1024, feed()) {
                Err(e) if e.contains("Text file busy") => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                other => return other,
            }
        }
        Err("Text file busy".into())
    }

    const TEN: Duration = Duration::from_secs(10);

    fn collector() -> (ProgressSink, Arc<Mutex<Vec<crate::progress::Progress>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        (Arc::new(move |p| lock(&s2).push(p)), seen)
    }

    #[test]
    fn bootc_gets_a_progress_pipe_and_its_lines_are_parsed() {
        let d = tempfile::tempdir().unwrap();
        // args: upgrade --progress-fd N
        let prog = script(
            &d,
            r#"[ "$1 $2" = "upgrade --progress-fd" ] || exit 3
eval "echo '{\"type\":\"ProgressBytes\",\"bytes\":5,\"bytesTotal\":10}' >&$3"
eval "echo '{\"type\":\"ProgressSteps\",\"task\":\"staging\",\"steps\":1,\"stepsTotal\":3,\"description\":\"Deploying Image\"}' >&$3"
echo out"#,
        );
        let (sink, seen) = collector();
        let out = go(&prog, &["upgrade"], TEN, || Feed::BootcFd(sink.clone()));
        assert_eq!(out.as_deref(), Ok("out\n"));
        let seen = lock(&seen);
        assert_eq!(seen.len(), 2);
        assert_eq!((seen[0].done, seen[0].total), (5, 10));
        assert_eq!(seen[1].detail, "Deploying Image");
    }

    #[test]
    fn the_flag_follows_the_subcommand_and_the_rest_of_the_argv_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let prog = script(&d, "echo \"$@\"");
        let (sink, _) = collector();
        let out = go(
            &prog,
            &["switch", "--transport", "oci", "img:testing"],
            TEN,
            || Feed::BootcFd(sink.clone()),
        )
        .unwrap();
        let words: Vec<&str> = out.split_whitespace().collect();
        assert_eq!(&words[..2], ["switch", "--progress-fd"]);
        assert!(words[2].parse::<i32>().unwrap() > 2);
        assert_eq!(&words[3..], ["--transport", "oci", "img:testing"]);
        // without a feed nothing is added
        let out = go(&prog, &["status", "--json"], TEN, || Feed::None).unwrap();
        assert_eq!(out.trim(), "status --json");
    }

    #[test]
    fn rpm_ostree_stdout_is_parsed_while_it_is_still_captured() {
        let d = tempfile::tempdir().unwrap();
        let prog = script(
            &d,
            "echo 'custom layers needed: 1 (300.0 MB)'\necho 'Writing rpmdb...done'",
        );
        let (sink, seen) = collector();
        let out = go(&prog, &["upgrade"], TEN, || Feed::Stdout(sink.clone())).unwrap();
        assert!(out.contains("Writing rpmdb"));
        let seen = lock(&seen);
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].total, 300_000_000);
        assert_eq!(seen[1].stage, "installing");
    }

    #[test]
    fn a_child_that_ignores_the_progress_pipe_still_finishes_and_fails_normally() {
        let d = tempfile::tempdir().unwrap();
        let prog = script(&d, "echo oops >&2; exit 1");
        let (sink, seen) = collector();
        let e = go(&prog, &["upgrade"], TEN, || Feed::BootcFd(sink.clone()));
        assert_eq!(e, Err("oops".into()));
        assert!(lock(&seen).is_empty());
    }

    #[test]
    fn a_bootc_without_the_flag_is_run_again_without_it() {
        let d = tempfile::tempdir().unwrap();
        let prog = script(
            &d,
            r#"case "$*" in *--progress-fd*)
echo "error: unexpected argument '--progress-fd' found" >&2; exit 2;; esac
echo "ran: $*""#,
        );
        let (sink, _) = collector();
        let out = go(&prog, &["upgrade"], TEN, || Feed::BootcFd(sink.clone()));
        assert_eq!(out.as_deref(), Ok("ran: upgrade\n"));
    }

    #[test]
    fn a_failure_after_progress_that_names_the_flag_is_not_run_again() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("log");
        let prog = script(
            &d,
            &format!(
                r#"echo run >> {}
eval "echo '{{\"type\":\"ProgressBytes\",\"bytes\":1,\"bytesTotal\":2}}' >&$3"
echo "error: unexpected argument '--progress-fd' found in the pipe" >&2; exit 2"#,
                log.display()
            ),
        );
        let (sink, _) = collector();
        let e = go(&prog, &["upgrade"], TEN, || Feed::BootcFd(sink.clone()));
        assert!(e.unwrap_err().contains("--progress-fd"));
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1);
        // and an unrelated failure naming the fd, with no progress, is not either
        let prog = script(
            &d,
            &format!(
                "echo run >> {}\necho 'cannot write to --progress-fd 5' >&2; exit 1",
                log.display()
            ),
        );
        let _ = std::fs::remove_file(&log);
        let _ = go(&prog, &["upgrade"], TEN, || Feed::BootcFd(sink.clone()));
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1);
    }

    #[test]
    fn a_grandchild_holding_the_progress_pipe_does_not_hang_the_runner() {
        let d = tempfile::tempdir().unwrap();
        let prog = script(&d, "eval \"sleep 5 >&$3 2>&1 &\"\necho done");
        let (sink, _) = collector();
        let t = Instant::now();
        let out = go(&prog, &["upgrade"], TEN, || Feed::BootcFd(sink.clone()));
        assert_eq!(out.as_deref(), Ok("done\n"));
        assert!(
            t.elapsed() < PIPE_GRACE + Duration::from_secs(2),
            "{:?}",
            t.elapsed()
        );
    }

    use crate::progress::Progress as Prog;

    /// Reports one update through `run_progress` and says what a reader of
    /// the property sees at that moment.
    struct Reporting {
        calls: Mutex<Vec<(String, bool)>>,
        seen: Mutex<Vec<String>>,
        rx: Mutex<Option<tokio::sync::watch::Receiver<String>>>,
        fail: bool,
    }

    impl Reporting {
        fn new(fail: bool) -> Arc<Self> {
            Arc::new(Reporting {
                calls: Mutex::new(Vec::new()),
                seen: Mutex::new(Vec::new()),
                rx: Mutex::new(None),
                fail,
            })
        }
        fn core(self: &Arc<Self>) -> Core {
            let c = Core::new(self.clone());
            *lock(&self.rx) = Some(c.progress_watch());
            c
        }
        fn called(&self, with_progress: bool) -> Vec<String> {
            lock(&self.calls)
                .iter()
                .filter(|(_, p)| *p == with_progress)
                .map(|(a, _)| a.clone())
                .collect()
        }
    }

    impl BootcRunner for Reporting {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            lock(&self.calls).push((args.join(" "), false));
            match args {
                ["status", ..] => Ok(BOOTED_WITH_UPDATE.into()),
                _ => Ok(String::new()),
            }
        }
        fn run_progress(&self, args: &[&str], sink: &ProgressSink) -> Result<String, String> {
            lock(&self.calls).push((args.join(" "), true));
            sink(Prog {
                stage: "downloading".into(),
                done: 1,
                total: 2,
                ..Default::default()
            });
            let now = lock(&self.rx).as_ref().unwrap().borrow().clone();
            lock(&self.seen).push(now);
            if self.fail {
                return Err("boom".into());
            }
            Ok(String::new())
        }
    }

    #[test]
    fn upgrade_and_switch_report_progress_and_clear_it_after() {
        let r = Reporting::new(false);
        let c = r.core();
        c.execute(&Op::Upgrade).unwrap();
        assert_eq!(c.progress_json(), "");
        c.execute(&Op::SwitchChannel("testing".into())).unwrap();
        assert_eq!(c.progress_json(), "");
        let seen = lock(&r.seen).clone();
        assert_eq!(seen.len(), 2);
        let ops: Vec<String> = seen
            .iter()
            .map(|j| {
                assert!(!j.is_empty());
                let p: Prog = serde_json::from_str(j).unwrap();
                assert_eq!((p.stage.as_str(), p.done, p.total), ("downloading", 1, 2));
                p.op
            })
            .collect();
        assert_eq!(ops, ["upgrade", "switch"]);
        let with = r.called(true);
        assert_eq!(with[0], "upgrade");
        assert!(with[1].starts_with("switch "), "{with:?}");
    }

    #[test]
    fn status_check_and_rollback_never_get_the_progress_flag() {
        let r = Reporting::new(false);
        let c = r.core();
        for op in [Op::Status, Op::CheckForUpdate, Op::Rollback] {
            let _ = c.execute(&op);
        }
        assert!(r.called(true).is_empty(), "{:?}", r.called(true));
        assert!(lock(&r.seen).is_empty());
        assert_eq!(c.progress_json(), "");
    }

    #[test]
    fn a_failing_operation_clears_the_property() {
        let r = Reporting::new(true);
        let c = r.core();
        assert!(c.execute(&Op::Upgrade).is_err());
        assert_eq!(c.progress_json(), "");
        assert!(c.execute(&Op::SwitchChannel("testing".into())).is_err());
        assert_eq!(c.progress_json(), "");
        assert_eq!(lock(&r.seen).len(), 2);
    }

    #[test]
    fn a_timeout_clears_the_property() {
        struct Slow(PathBuf);
        impl BootcRunner for Slow {
            fn run(&self, _a: &[&str]) -> Result<String, String> {
                Ok(PLAIN.into())
            }
            fn run_progress(&self, args: &[&str], sink: &ProgressSink) -> Result<String, String> {
                go(&self.0, args, Duration::from_millis(500), || {
                    Feed::BootcFd(sink.clone())
                })
            }
        }
        let d = tempfile::tempdir().unwrap();
        let prog = script(
            &d,
            "eval \"echo '{\\\"type\\\":\\\"ProgressBytes\\\",\\\"bytes\\\":1,\\\"bytesTotal\\\":2}' >&$3\"\nexec sleep 30",
        );
        let c = Core::new(Arc::new(Slow(prog)));
        let e = c.execute(&Op::Upgrade).unwrap_err();
        assert!(format!("{e:?}").contains("did not finish"), "{e:?}");
        assert_eq!(c.progress_json(), "");
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
    fn an_upgrade_or_switch_reads_the_status_once_before_and_once_after() {
        let reads = |calls: &[Vec<String>], program: &str| {
            calls
                .iter()
                .filter(|c| c[0] == program && c[1..] == ["status", "--json"])
                .count()
        };
        for op in [Op::Upgrade, Op::SwitchChannel("testing".into())] {
            let f = Layered::new();
            Core::new(f.clone()).execute(&op).unwrap();
            let calls = f.calls();
            assert_eq!(reads(&calls, "bootc"), 2, "{op:?} {calls:?}");
            assert_eq!(reads(&calls, "rpm-ostree"), 2, "{op:?} {calls:?}");
            let f = Arc::new(Stager {
                calls: Mutex::default(),
                before: PLAIN.into(),
                after: PLAIN.into(),
                cleanup_fails: false,
                before_fails: false,
                flaky: 0,
            });
            Core::new(f.clone()).execute(&op).unwrap();
            let calls = f.calls();
            let bootc_reads = calls.iter().filter(|c| *c == &["status", "--json"]).count();
            assert_eq!(bootc_reads, 2, "{op:?} {calls:?}");
        }
    }

    #[test]
    fn without_a_read_before_a_layered_upgrade_still_finds_rpm_ostree() {
        // the read before fails; bootc's refusal sends it to rpm-ostree
        struct FirstReadFails(Arc<Layered>, Mutex<bool>);
        impl BootcRunner for FirstReadFails {
            fn run(&self, args: &[&str]) -> Result<String, String> {
                if args == ["status", "--json"]
                    && std::mem::replace(&mut *self.1.lock().unwrap(), false)
                {
                    return Err("busy".into());
                }
                self.0.run(args)
            }
            fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
                self.0.rpm_ostree(args)
            }
        }
        let l = Layered::new();
        Core::new(Arc::new(FirstReadFails(l.clone(), Mutex::new(true))))
            .execute(&Op::Upgrade)
            .unwrap();
        let calls = l.calls();
        assert!(ran(&calls, &["bootc", "upgrade"]));
        assert!(ran(&calls, &["rpm-ostree", "upgrade"]));
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

    /// The plain fixture (booted 44.20261001 on :testing) with `staged` set
    /// to an image of :testing with this version, build time and digest.
    fn plain_with_staged(version: &str, time: &str, digest: &str) -> String {
        let staged = format!(
            r#""staged": {{"image": {{"image": {{"image": "ghcr.io/eternalcoder454/atlasos:testing",
                "transport": "registry"}}, "version": "{version}", "timestamp": "{time}",
                "imageDigest": "{digest}"}}, "incompatible": false, "pinned": false}},"#
        );
        PLAIN.replacen("\"staged\": null,", &staged, 1)
    }

    /// bootc as `Fake`, but with `rpm-ostree` (for `cleanup -p`) and a status
    /// that changes after `upgrade` or `switch`; with `before_fails`, the
    /// status can't be read before.
    struct Stager {
        calls: Mutex<Vec<Vec<String>>>,
        before: String,
        after: String,
        cleanup_fails: bool,
        before_fails: bool,
        /// the first this many `upgrade`/`switch` calls drop the connection
        flaky: usize,
    }

    impl Stager {
        fn new(after: String, cleanup_fails: bool) -> Arc<Stager> {
            Arc::new(Stager {
                calls: Mutex::default(),
                before: PLAIN.into(),
                after,
                cleanup_fails,
                before_fails: false,
                flaky: 0,
            })
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl BootcRunner for Stager {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(args.iter().map(|s| s.to_string()).collect());
            let fetches = calls
                .iter()
                .filter(|c| c[0] == "upgrade" || c[0] == "switch")
                .count();
            if matches!(args[0], "upgrade" | "switch") && fetches <= self.flaky {
                return Err(RESET.into());
            }
            let upgraded = fetches > self.flaky;
            let cleaned = calls.iter().any(|c| c == &["rpm-ostree", "cleanup", "-p"]);
            Ok(match (args, upgraded && !cleaned) {
                (["status", "--json"], true) => self.after.clone(),
                (["status", "--json"], false) if self.before_fails => {
                    return Err("bootc status timed out".into());
                }
                (["status", "--json"], false) => self.before.clone(),
                _ => String::new(),
            })
        }
        fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
            let mut call = vec!["rpm-ostree".to_string()];
            call.extend(args.iter().map(|s| s.to_string()));
            self.calls.lock().unwrap().push(call);
            if self.cleanup_fails {
                Err("rpm-ostreed is not running".into())
            } else {
                Ok(String::new())
            }
        }
    }

    #[test]
    fn an_older_image_staged_by_upgrade_is_taken_out_and_refused() {
        let d = tempfile::tempdir().unwrap();
        // the tag went back to a build older than the booted 44.20261001
        let f = Stager::new(
            plain_with_staged("44.20260920", "2026-09-20T04:00:00Z", "sha256:old"),
            false,
        );
        let c = Core::new(f.clone()).with_events(d.path().join("events.jsonl"));
        match c.execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => {
                assert!(m.starts_with(DOWNGRADE_REFUSED), "{m}");
                assert!(
                    m.contains("44.20260920") && m.contains("44.20261001"),
                    "{m}"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(ran(&f.calls(), &["rpm-ostree", "cleanup", "-p"]));
        assert_eq!(names(&d), ["update-failed"]);
        assert!(!c.is_busy());
    }

    /// `Stager` plus a registry that answers `skopeo inspect` with `out`.
    struct Registry {
        stager: Arc<Stager>,
        out: Result<String, String>,
    }

    impl BootcRunner for Registry {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            self.stager.run(args)
        }
        fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
            self.stager.rpm_ostree(args)
        }
        fn skopeo(&self, args: &[&str]) -> Result<String, String> {
            let mut call = vec!["skopeo".to_string()];
            call.extend(args.iter().map(|s| s.to_string()));
            self.stager.calls.lock().unwrap().push(call);
            self.out.clone()
        }
    }

    fn skopeo_json(version: &str, created: &str, digest: &str) -> String {
        format!(
            r#"{{"Digest":"{digest}","Created":"{created}","Architecture":"amd64",
                "Labels":{{"org.opencontainers.image.version":"{version}"}}}}"#
        )
    }

    #[test]
    fn an_older_image_is_refused_before_it_is_downloaded() {
        let stager = Stager::new(PLAIN.into(), false);
        let f = Arc::new(Registry {
            stager: stager.clone(),
            out: Ok(skopeo_json(
                "44.20260920",
                "2026-09-20T04:00:00Z",
                "sha256:old",
            )),
        });
        match Core::new(f).execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => {
                assert!(m.starts_with(DOWNGRADE_REFUSED), "{m}");
                assert!(m.contains("44.20260920") && m.contains("Nothing was downloaded"));
            }
            other => panic!("{other:?}"),
        }
        let calls = stager.calls();
        assert!(ran(
            &calls,
            &[
                "skopeo",
                "inspect",
                "--",
                "docker://ghcr.io/eternalcoder454/atlasos:testing"
            ]
        ));
        // nothing pulled, nothing to clean up
        assert!(
            !calls
                .iter()
                .any(|c| c[0] == "upgrade" || c[0] == "rpm-ostree")
        );
    }

    #[test]
    fn a_newer_image_or_an_unreadable_registry_goes_on_to_the_pull() {
        for out in [
            Ok(skopeo_json(
                "44.20261008",
                "2026-10-08T04:00:00Z",
                "sha256:new",
            )),
            Err("connection refused".to_string()),
            Ok("not json".to_string()),
        ] {
            let stager = Stager::new(
                plain_with_staged("44.20261008", "2026-10-08T04:00:00Z", "sha256:new"),
                false,
            );
            let f = Arc::new(Registry {
                stager: stager.clone(),
                out,
            });
            let c = Core::new(f).with_retry_wait(|_| true);
            assert!(c.execute(&Op::Upgrade).is_ok());
            assert!(stager.calls().iter().any(|c| c[0] == "upgrade"));
        }
    }

    #[test]
    fn a_switch_to_the_other_channel_may_be_older_than_the_booted_image() {
        // booted :testing 44.20261001; :stable is another tag, so another image
        let stager = Stager::new(PLAIN.into(), false);
        let f = Arc::new(Registry {
            stager: stager.clone(),
            out: Ok(skopeo_json(
                "44.20260920",
                "2026-09-20T04:00:00Z",
                "sha256:stable",
            )),
        });
        Core::new(f)
            .execute(&Op::SwitchChannel("stable".into()))
            .unwrap();
        assert!(stager.calls().iter().any(|c| c[0] == "switch"));
    }

    #[test]
    fn a_failed_pre_pull_check_lets_a_switch_through() {
        let stager = Stager::new(PLAIN.into(), false);
        let f = Arc::new(Registry {
            stager: stager.clone(),
            out: Err("i/o timeout".into()),
        });
        Core::new(f)
            .execute(&Op::SwitchChannel("stable".into()))
            .unwrap();
        assert!(stager.calls().iter().any(|c| c[0] == "switch"));
    }

    /// The layered fixture, but with a booted image reference (version
    /// 44.20261034 of :stable) and a registry that answers with `out`.
    struct LayeredKnown {
        inner: Arc<Layered>,
        out: String,
    }

    impl LayeredKnown {
        fn new(version: &str) -> Arc<LayeredKnown> {
            Arc::new(LayeredKnown {
                inner: Layered::new(),
                out: skopeo_json(version, "2026-09-20T04:00:00Z", "sha256:old"),
            })
        }
    }

    impl BootcRunner for LayeredKnown {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            if args == ["status", "--json"] {
                self.inner.record("bootc", args);
                let booted = r#""image": {
          "architecture": "amd64",
          "image": {"image": "/var/mnt/atlasreg/registry:stable", "transport": "oci"},
          "imageDigest": "sha256:booted",
          "timestamp": "2026-10-02T18:54:39Z",
          "version": "44.20261034"
        },
        "incompatible": true,"#;
                let json = include_str!("../../tests/fixtures/status-layered.json");
                let old = "\"image\": null,\n      \"incompatible\": true,";
                assert!(json.contains(old));
                return Ok(json.replacen(old, booted, 1));
            }
            self.inner.run(args)
        }
        fn rpm_ostree(&self, args: &[&str]) -> Result<String, String> {
            self.inner.rpm_ostree(args)
        }
        fn skopeo(&self, args: &[&str]) -> Result<String, String> {
            self.inner.record("skopeo", args);
            Ok(self.out.clone())
        }
    }

    #[test]
    fn a_layered_upgrade_to_an_older_image_is_refused_before_the_pull() {
        let f = LayeredKnown::new("44.20260920");
        match Core::new(f.clone()).execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => {
                assert!(m.starts_with(DOWNGRADE_REFUSED), "{m}");
                assert!(m.contains("Nothing was downloaded"), "{m}");
            }
            other => panic!("{other:?}"),
        }
        let calls = f.inner.calls();
        assert!(!ran(&calls, &["rpm-ostree", "upgrade"]));
        assert!(!calls.iter().any(|c| c[..2] == ["bootc", "upgrade"]));
    }

    #[test]
    fn a_layered_rebase_to_an_older_image_is_refused_before_the_pull() {
        let f = LayeredKnown::new("44.20260920");
        match Core::new(f.clone()).execute(&Op::SwitchChannel("stable".into())) {
            Err(HelperError::Failed(m)) => {
                assert!(m.starts_with(DOWNGRADE_REFUSED), "{m}");
                assert!(m.contains("Nothing was downloaded"), "{m}");
            }
            other => panic!("{other:?}"),
        }
        assert!(!f.inner.calls().iter().any(|c| c[..2] == ["rpm-ostree", "rebase"]));
    }

    #[test]
    fn a_failed_removal_is_tried_three_times_then_says_it_is_still_staged() {
        let f = Stager::new(
            plain_with_staged("44.20260920", "2026-09-20T04:00:00Z", "sha256:old"),
            true,
        );
        let c = Core::new(f.clone()).with_retry_wait(|_| true);
        match c.execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => {
                assert!(m.starts_with(DOWNGRADE_REFUSED), "{m}");
                assert!(m.contains("STILL STAGED") && m.contains("sudo rpm-ostree cleanup -p"));
                assert!(!m.contains("Nothing was downloaded"));
            }
            other => panic!("{other:?}"),
        }
        let tries = f.calls().iter().filter(|c| c[0] == "rpm-ostree").count();
        assert_eq!(tries, 3);
    }

    #[test]
    fn a_failed_removal_says_the_old_image_is_still_staged() {
        let f = Stager::new(
            plain_with_staged("44.20260920", "2026-09-20T04:00:00Z", "sha256:old"),
            true,
        );
        match Core::new(f).with_retry_wait(|_| true).execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => {
                assert!(
                    m.starts_with(DOWNGRADE_REFUSED)
                        && m.contains("STILL STAGED")
                        && m.contains("rpm-ostree cleanup -p")
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn without_the_status_before_the_booted_image_is_compared() {
        let f = Arc::new(Stager {
            calls: Mutex::default(),
            before: PLAIN.into(),
            after: plain_with_staged("44.20260920", "2026-09-20T04:00:00Z", "sha256:old"),
            cleanup_fails: false,
            before_fails: true,
            flaky: 0,
        });
        match Core::new(f.clone()).execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => assert!(m.starts_with(DOWNGRADE_REFUSED), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(ran(&f.calls(), &["rpm-ostree", "cleanup", "-p"]));
    }

    #[test]
    fn the_refusal_says_what_was_staged_before_is_gone_too() {
        let f = Arc::new(Stager {
            calls: Mutex::default(),
            before: plain_with_staged("44.20261005", "2026-10-05T04:00:00Z", "sha256:s"),
            after: plain_with_staged("44.20260920", "2026-09-20T04:00:00Z", "sha256:old"),
            cleanup_fails: true,
            before_fails: false,
            flaky: 0,
        });
        match Core::new(f).with_retry_wait(|_| true).execute(&Op::Upgrade) {
            Err(HelperError::Failed(m)) => {
                assert!(m.contains("was removed too") && m.contains("STILL STAGED"));
                // (the fake's cleanup error is short; `tail` keeps it whole)
                assert!(m.ends_with("rpm-ostreed is not running"), "{m}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_switch_to_the_followed_channel_cannot_go_back_either() {
        let d = tempfile::tempdir().unwrap();
        // booted :testing 44.20261001; the testing tag went back
        let f = Stager::new(
            plain_with_staged("44.20260920", "2026-09-20T04:00:00Z", "sha256:old"),
            false,
        );
        let c = Core::new(f.clone()).with_events(d.path().join("events.jsonl"));
        match c.execute(&Op::SwitchChannel("testing".into())) {
            Err(HelperError::Failed(m)) => assert!(m.starts_with(DOWNGRADE_REFUSED), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(ran(&f.calls(), &["rpm-ostree", "cleanup", "-p"]));
        assert_eq!(names(&d), ["channel-switch-failed"]);
        // a newer build stays
        let after = plain_with_staged("44.20261008", "2026-10-08T04:00:00Z", "sha256:new");
        let f = Stager::new(after.clone(), false);
        let got = Core::new(f).execute(&Op::SwitchChannel("testing".into()));
        assert_eq!(got.unwrap(), after);
    }

    #[test]
    fn a_newer_or_incomparable_image_stays_staged() {
        for after in [
            plain_with_staged("44.20261008", "2026-10-08T04:00:00Z", "sha256:new"),
            // a rebuild of the same version
            plain_with_staged("44.20261001", "2026-10-01T04:12:09Z", "sha256:re"),
            // no version label and no build time: nothing to go on
            plain_with_staged("x", "", "sha256:odd"),
        ] {
            let f = Stager::new(after.clone(), false);
            assert_eq!(Core::new(f.clone()).execute(&Op::Upgrade).unwrap(), after);
            assert!(!f.calls().iter().any(|c| c[0] == "rpm-ostree"));
        }
        // without a version label, by build time
        let f = Stager::new(
            plain_with_staged("x", "2026-09-01T00:00:00Z", "sha256:old"),
            false,
        );
        assert!(Core::new(f).execute(&Op::Upgrade).is_err());
    }

    #[test]
    fn a_staged_switch_to_the_other_channel_is_not_a_downgrade() {
        // booted :testing 44.20261001; a stable build from before is staged
        // by a channel switch, then upgraded to a newer stable build
        let stable = |v: &str, t: &str, d: &str| {
            plain_with_staged(v, t, d).replacen(
                "\"image\": \"ghcr.io/eternalcoder454/atlasos:testing\",\n                \"transport\"",
                "\"image\": \"ghcr.io/eternalcoder454/atlasos:stable\",\n                \"transport\"",
                1,
            )
        };
        let after = stable("44.20260925", "2026-09-25T04:00:00Z", "sha256:s2");
        assert!(after.contains("atlasos:stable"));
        let f = Stager::new(after.clone(), false);
        assert_eq!(Core::new(f).execute(&Op::Upgrade).unwrap(), after);
    }

    const POLICY: &str = r#"{"default": [{"type": "reject"}],
        "transports": {"docker": {"": [{"type": "insecureAcceptAnything"}],
            "ghcr.io/eternalcoder454/atlasos": [{"type": "sigstoreSigned",
                "keyPaths": ["/etc/pki/containers/atlasos.pub"],
                "signedIdentity": {"type": "matchRepository"}}]}}}"#;

    fn with_policy(c: Core, d: &tempfile::TempDir, policy: &str) -> Core {
        let p = d.path().join("policy.json");
        std::fs::write(&p, policy).unwrap();
        c.with_policy(p)
    }

    #[test]
    fn a_switch_enforces_the_signature_the_policy_demands() {
        let d = tempfile::tempdir().unwrap();
        // unverified (no signature setting) and "insecure" both get enforced
        for json in [BOOTED_WITH_UPDATE.to_string(), signed("\"insecure\"")] {
            let f = Fake::new(&json);
            with_policy(core(&f), &d, POLICY)
                .execute(&Op::SwitchChannel("testing".into()))
                .unwrap();
            assert_eq!(
                switch_call(&f.calls()),
                [
                    "switch",
                    "--transport",
                    "registry",
                    "--enforce-container-sigpolicy",
                    "ghcr.io/eternalcoder454/atlasos:testing"
                ]
            );
        }
        // a policy that accepts anything by default, or none: as before
        for policy in [
            POLICY.replace(
                r#"[{"type": "reject"}]"#,
                r#"[{"type": "insecureAcceptAnything"}]"#,
            ),
            "not json".to_string(),
        ] {
            let f = Fake::new(BOOTED_WITH_UPDATE);
            with_policy(core(&f), &d, &policy)
                .execute(&Op::SwitchChannel("testing".into()))
                .unwrap();
            assert!(
                !switch_call(&f.calls()).contains(&"--enforce-container-sigpolicy".to_string())
            );
        }
    }

    #[test]
    fn a_layered_switch_records_the_signature_check_too() {
        let d = tempfile::tempdir().unwrap();
        let rpm = include_str!("../../tests/fixtures/rpm-ostree-layered.json").replace(
            "ostree-unverified-image:oci:/var/mnt/atlasreg/registry:stable",
            "ostree-unverified-registry:ghcr.io/eternalcoder454/atlasos:stable",
        );
        let f = Arc::new(Layered {
            calls: Mutex::default(),
            rpm_ostree_status: Ok(rpm),
        });
        with_policy(Core::new(f.clone()), &d, POLICY)
            .execute(&Op::SwitchChannel("testing".into()))
            .unwrap();
        assert!(ran(
            &f.calls(),
            &[
                "rpm-ostree",
                "rebase",
                "ostree-image-signed:docker://ghcr.io/eternalcoder454/atlasos:testing"
            ]
        ));
    }
}
