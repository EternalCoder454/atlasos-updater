//! Locks shared by the window, the tray and the workers it starts, in
//! `$XDG_RUNTIME_DIR` (private to the user): one Flatpak operation at a time,
//! one crash collector at a time, one firmware operation at a time.
//!
//! Each lock has two file names, the current one and the one it had until
//! 0.3.0 (`atlas-updater-*.lock`), which other programs (the Store) still
//! use. Taking a lock takes both, the current one first and then the old
//! one, always in that order, and giving it up gives up both, so two
//! programs exclude each other whichever name each of them knows. Remove the
//! old names in the release after the next.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// A lock and the name it had before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lock {
    pub name: &'static str,
    pub legacy: &'static str,
}

pub const APPS: Lock = Lock {
    name: "telamon-updater-apps.lock",
    legacy: "atlas-updater-apps.lock",
};
pub const CRASH: Lock = Lock {
    name: "telamon-updater-crash.lock",
    legacy: "atlas-updater-crash.lock",
};
pub const FIRMWARE: Lock = Lock {
    name: "telamon-updater-firmware.lock",
    legacy: "atlas-updater-firmware.lock",
};

/// Held until dropped (closing the files releases the lock; so does the
/// process ending, however it ends).
#[derive(Debug)]
pub struct Held(#[allow(dead_code)] Vec<File>);

fn dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Takes `lock` (both of its names). `wait`: block until it is free;
/// otherwise `Ok(None)` when someone else holds it. Without a runtime
/// directory there is nothing to share a lock in: an error, and the caller
/// decides whether to go ahead (the user asked) or not (a background round).
pub fn take(lock: Lock, wait: bool) -> io::Result<Option<Held>> {
    let Some(dir) = dir() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no runtime directory (XDG_RUNTIME_DIR is not set)",
        ));
    };
    take_in(&dir, lock, wait)
}

fn take_in(dir: &Path, lock: Lock, wait: bool) -> io::Result<Option<Held>> {
    let mut held = Vec::with_capacity(2);
    for name in [lock.name, lock.legacy] {
        match take_file(&dir.join(name), wait)? {
            Some(f) => held.push(f),
            // the files taken so far close with `held`: nothing stays locked
            None => return Ok(None),
        }
    }
    Ok(Some(Held(held)))
}

fn take_file(path: &Path, wait: bool) -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let op = if wait {
        libc::LOCK_EX
    } else {
        libc::LOCK_EX | libc::LOCK_NB
    };
    loop {
        // SAFETY: flock on a file descriptor we own; no memory is passed.
        if unsafe { libc::flock(file.as_raw_fd(), op) } == 0 {
            return Ok(Some(file));
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EWOULDBLOCK) => return Ok(None),
            _ => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Lock = Lock {
        name: "new.lock",
        legacy: "old.lock",
    };

    /// Whether `name` in `dir` can be taken now (tries for a moment: a test
    /// that forks leaves the files open in the child until it execs).
    fn free(dir: &Path, name: &str) -> bool {
        for _ in 0..100 {
            if take_file(&dir.join(name), false).unwrap().is_some() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn a_second_taker_is_told_it_is_busy() {
        let d = tempfile::tempdir().unwrap();
        let first = take_in(d.path(), L, false).unwrap();
        assert!(first.is_some());
        assert!(take_in(d.path(), L, false).unwrap().is_none());
        drop(first);
        assert!(take_in(d.path(), L, false).unwrap().is_some());
    }

    #[test]
    fn both_names_are_held_and_given_up_together() {
        let d = tempfile::tempdir().unwrap();
        let held = take_in(d.path(), L, false).unwrap().unwrap();
        // a program that knows only one name is kept out, whichever it is
        // (the Store takes only the old one)
        assert!(
            take_file(&d.path().join("new.lock"), false)
                .unwrap()
                .is_none()
        );
        assert!(
            take_file(&d.path().join("old.lock"), false)
                .unwrap()
                .is_none()
        );
        drop(held);
        assert!(free(d.path(), "new.lock"));
        assert!(free(d.path(), "old.lock"));
    }

    #[test]
    fn an_old_style_holder_keeps_us_out_and_nothing_stays_locked() {
        let d = tempfile::tempdir().unwrap();
        let theirs = take_file(&d.path().join("old.lock"), false)
            .unwrap()
            .unwrap();
        assert!(take_in(d.path(), L, false).unwrap().is_none());
        // the current name was given back when the old one was busy
        assert!(free(d.path(), "new.lock"));
        drop(theirs);
        assert!(take_in(d.path(), L, false).unwrap().is_some());
    }

    #[test]
    fn a_new_style_holder_keeps_an_old_style_one_out_too() {
        // (the other direction: us first, then a program of the old kind)
        let d = tempfile::tempdir().unwrap();
        let ours = take_in(d.path(), L, false).unwrap().unwrap();
        assert!(
            take_file(&d.path().join("old.lock"), false)
                .unwrap()
                .is_none()
        );
        drop(ours);
        assert!(free(d.path(), "old.lock"));
    }

    #[test]
    fn waiting_gets_the_lock_once_the_old_style_holder_lets_go() {
        let d = tempfile::tempdir().unwrap();
        let theirs = take_file(&d.path().join("old.lock"), false)
            .unwrap()
            .unwrap();
        let dir = d.path().to_path_buf();
        let waiter = std::thread::spawn(move || take_in(&dir, L, true).unwrap().is_some());
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(!waiter.is_finished(), "must wait for the old name too");
        drop(theirs);
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn the_real_names_are_the_ones_the_store_and_older_updaters_use() {
        assert_eq!(APPS.name, "telamon-updater-apps.lock");
        assert_eq!(APPS.legacy, "atlas-updater-apps.lock");
        assert_eq!(FIRMWARE.name, "telamon-updater-firmware.lock");
        assert_eq!(FIRMWARE.legacy, "atlas-updater-firmware.lock");
        assert_eq!(CRASH.legacy, "atlas-updater-crash.lock");
    }

    #[test]
    fn a_symlink_is_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, d.path().join("new.lock")).unwrap();
        assert!(take_in(d.path(), L, false).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        // and a link on the old name stops it too, with the new one given back
        let d = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&target, d.path().join("old.lock")).unwrap();
        assert!(take_in(d.path(), L, false).is_err());
        assert!(free(d.path(), "new.lock"));
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }
}
