//! inotify on the few paths the tray reacts to: `/run/ostree` (an update
//! was staged or went away) and, only while crash reports are on, the crash
//! sources (systemd-coredump's directory and the helper's event log, which is
//! also watched for driver notices, crash reports or not). A
//! source that does not exist yet is waited for on its nearest parent.

use std::collections::HashMap;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::path::Path;

use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;
use tokio::io::unix::AsyncFd;

pub const OSTREE: &str = "/run/ostree";
const RUN: &str = "/run";
const COREDUMP: &str = "/var/lib/systemd/coredump";
const COREDUMP_PARENTS: [&str; 2] = ["/var/lib/systemd", "/var/lib"];
// The helper's state directory keeps the name from before the package was
// telamon-system-helper (telamon_framework_system::events::DEFAULT_PATH).
const EVENTS: &str = "/var/lib/atlas-core/events.jsonl";
const EVENTS_PARENTS: [&str; 2] = ["/var/lib/atlas-core", "/var/lib"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Something changed in `/run/ostree`: read the status again.
    Ostree,
    /// A crash source changed: collect reports.
    Crash,
    /// The helper's event log changed (always watched): look for driver events.
    Events,
}

fn dir_mask() -> WatchFlags {
    WatchFlags::ATTRIB
        | WatchFlags::MOVE
        | WatchFlags::CREATE
        | WatchFlags::DELETE
        | WatchFlags::DELETE_SELF
        | WatchFlags::MOVE_SELF
}

fn file_mask() -> WatchFlags {
    WatchFlags::MODIFY
        | WatchFlags::CLOSE_WRITE
        | WatchFlags::ATTRIB
        | WatchFlags::DELETE_SELF
        | WatchFlags::MOVE_SELF
}

pub struct Watcher {
    fd: AsyncFd<OwnedFd>,
    /// Watch descriptor → the path it watches.
    wds: HashMap<i32, String>,
    crash: bool,
}

impl Watcher {
    /// Must be called inside the tokio runtime.
    pub fn new() -> io::Result<Watcher> {
        let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
        let mut w = Watcher {
            fd: AsyncFd::new(fd)?,
            wds: HashMap::new(),
            crash: false,
        };
        w.sync();
        Ok(w)
    }

    fn watching(&self, path: &str) -> bool {
        self.wds.values().any(|p| p == path)
    }

    /// True if `path` is newly watched.
    fn add(&mut self, path: &str, mask: WatchFlags) -> bool {
        if self.watching(path) {
            return false;
        }
        match inotify::add_watch(self.fd.get_ref(), path, mask) {
            Ok(wd) => {
                self.wds.insert(wd, path.to_string());
                true
            }
            Err(e) => {
                eprintln!("telamon-updater-tray: cannot watch {path}: {e}");
                false
            }
        }
    }

    fn remove(&mut self, path: &str) {
        let wds: Vec<i32> = self
            .wds
            .iter()
            .filter(|(_, p)| p.as_str() == path)
            .map(|(wd, _)| *wd)
            .collect();
        for wd in wds {
            self.wds.remove(&wd);
            let _ = inotify::remove_watch(self.fd.get_ref(), wd);
        }
    }

    /// Watches what should be watched now. Returns (`/run/ostree` newly
    /// watched, a crash source newly watched, the event log newly watched).
    fn sync(&mut self) -> (bool, bool, bool) {
        let ostree = if Path::new(OSTREE).is_dir() {
            let added = self.add(OSTREE, dir_mask());
            self.remove(RUN);
            added
        } else {
            // Not there yet: wait for it to appear (only in unusual setups).
            self.add(RUN, WatchFlags::CREATE | WatchFlags::MOVED_TO);
            false
        };
        let mut crash_added = false;
        let mut events_added = false;
        let mut needed: Vec<&str> = Vec::new();
        let sources: [(&str, &[&str], WatchFlags); 2] = [
            (COREDUMP, &COREDUMP_PARENTS, dir_mask()),
            (EVENTS, &EVENTS_PARENTS, file_mask()),
        ];
        for (source, parents, mask) in sources {
            // the event log is watched for driver notices as well
            if !self.crash && source != EVENTS {
                self.remove(source);
                continue;
            }
            if Path::new(source).exists() {
                let added = self.add(source, mask);
                if source == EVENTS {
                    events_added |= added;
                    crash_added |= added && self.crash;
                } else {
                    crash_added |= added;
                }
                continue;
            }
            self.remove(source);
            if let Some(p) = parents.iter().find(|p| Path::new(p).is_dir()) {
                needed.push(p);
                self.add(p, WatchFlags::CREATE | WatchFlags::MOVED_TO);
            }
        }
        // Parents only stand in for a missing source.
        for p in COREDUMP_PARENTS.iter().chain(EVENTS_PARENTS.iter()) {
            if !needed.contains(p) {
                self.remove(p);
            }
        }
        (ostree, crash_added, events_added)
    }

    /// Crash reports switched on or off: watch the crash sources, or stop.
    pub fn set_crash(&mut self, on: bool) {
        self.crash = on;
        self.sync();
    }

    /// Waits for changes that matter and says what they were.
    pub async fn next(&mut self) -> io::Result<Vec<Hit>> {
        loop {
            let mut events: Vec<(i32, ReadFlags)> = Vec::new();
            {
                let mut guard = self.fd.readable().await?;
                let mut buf = [MaybeUninit::<u8>::uninit(); 4096];
                let mut reader = inotify::Reader::new(guard.get_inner(), &mut buf);
                let drained = loop {
                    match reader.next() {
                        Ok(e) => events.push((e.wd(), e.events())),
                        Err(Errno::AGAIN) => break true,
                        Err(Errno::INTR) => continue,
                        Err(e) => return Err(e.into()),
                    }
                };
                if drained {
                    guard.clear_ready();
                }
            }
            let mut hits = Vec::new();
            let mut resync = false;
            for (wd, flags) in events {
                if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                    // Events were lost: assume everything changed.
                    hits.push(Hit::Ostree);
                    hits.push(Hit::Events);
                    if self.crash {
                        hits.push(Hit::Crash);
                    }
                    resync = true;
                    continue;
                }
                let Some(path) = self.wds.get(&wd).cloned() else {
                    continue;
                };
                if flags.contains(ReadFlags::IGNORED) {
                    // The watched file or directory is gone (or replaced).
                    self.wds.remove(&wd);
                    resync = true;
                }
                match path.as_str() {
                    OSTREE => hits.push(Hit::Ostree),
                    COREDUMP | EVENTS => {
                        if path == EVENTS {
                            hits.push(Hit::Events);
                        }
                        if path == COREDUMP || self.crash {
                            hits.push(Hit::Crash);
                        }
                        if flags.intersects(ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF) {
                            resync = true;
                        }
                    }
                    // /run or a crash source's parent: maybe it appeared.
                    _ => resync = true,
                }
            }
            if resync {
                let (ostree, crash, events) = self.sync();
                if ostree {
                    hits.push(Hit::Ostree);
                }
                if events {
                    hits.push(Hit::Events);
                }
                if crash {
                    hits.push(Hit::Crash);
                }
            }
            hits.dedup();
            if !hits.is_empty() {
                return Ok(hits);
            }
        }
    }
}
