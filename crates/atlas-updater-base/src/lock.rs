//! Locks shared by the window, the tray and the workers it starts, in
//! `$XDG_RUNTIME_DIR` (private to the user): one Flatpak operation at a time,
//! one crash collector at a time.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

pub const APPS: &str = "atlas-updater-apps.lock";
pub const CRASH: &str = "atlas-updater-crash.lock";
pub const FIRMWARE: &str = "atlas-updater-firmware.lock";

/// Held until dropped (closing the file releases the lock; so does the
/// process ending, however it ends).
#[derive(Debug)]
pub struct Held(#[allow(dead_code)] File);

fn dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Takes the lock `name`. `wait`: block until it is free; otherwise
/// `Ok(None)` when someone else holds it. Without a runtime directory there
/// is nothing to share a lock in: an error, and the caller decides whether
/// to go ahead (the user asked) or not (a background round).
pub fn take(name: &str, wait: bool) -> io::Result<Option<Held>> {
    let Some(dir) = dir() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no runtime directory (XDG_RUNTIME_DIR is not set)",
        ));
    };
    take_in(&dir.join(name), wait)
}

fn take_in(path: &std::path::Path, wait: bool) -> io::Result<Option<Held>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
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
            return Ok(Some(Held(file)));
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

    #[test]
    fn a_second_taker_is_told_it_is_busy() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.lock");
        let first = take_in(&p, false).unwrap();
        assert!(first.is_some());
        assert!(take_in(&p, false).unwrap().is_none());
        drop(first);
        assert!(take_in(&p, false).unwrap().is_some());
    }

    #[test]
    fn a_symlink_is_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        std::fs::write(&target, b"keep").unwrap();
        let p = d.path().join("x.lock");
        std::os::unix::fs::symlink(&target, &p).unwrap();
        assert!(take_in(&p, false).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }
}
