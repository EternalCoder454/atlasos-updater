//! Files the Updater kept under its old name, `atlas-updater`, until 0.3.0:
//!
//! | old | new |
//! |---|---|
//! | `~/.config/atlas-updaterrc` | `~/.config/telamon-updaterrc` |
//! | `~/.local/state/atlas-updater/` (`app-updates.jsonl`, ...) | `~/.local/state/telamon-updater/` |
//! | `~/.cache/atlas-updater/` (`releases.json`) | `~/.cache/telamon-updater/` |
//!
//! Each is **moved** once, with one atomic rename that never replaces
//! anything, when the new name is not there yet: one way, so nothing is
//! written to the old name afterwards (an older tray still running keeps
//! its open file, which is then the new one; a new file under the old name
//! is simply not read). Nothing is followed through a symlink, and only what
//! belongs to this user is touched. When both names exist (a partial
//! state: the other process was first, or someone made the new folder), a
//! folder is merged file by file, a file the new side already has stays
//! where it is, and nothing is overwritten or deleted.
//!
//! The window, the worker and the tray all call [`adopt_legacy_files`] when
//! they start, and the settings and the state paths ask for it too, so
//! whichever runs first does it.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Once;

use rustix::fs::{CWD, RenameFlags, renameat_with};

pub const RC: (&str, &str) = ("atlas-updaterrc", "telamon-updaterrc");
pub const DIR: (&str, &str) = ("atlas-updater", "telamon-updater");

/// Where things are looked for: each is `None` without a home folder.
#[derive(Debug, Clone, Default)]
pub struct Dirs {
    pub config: Option<PathBuf>,
    pub state: Option<PathBuf>,
    pub cache: Option<PathBuf>,
}

impl Dirs {
    /// `$XDG_CONFIG_HOME` / `$XDG_STATE_HOME` / `$XDG_CACHE_HOME` (absolute
    /// values only), else under `$HOME`.
    pub fn from_env() -> Dirs {
        let abs = |k: &str| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        let home = abs("HOME");
        let pick =
            |var: &str, under: &str| abs(var).or_else(|| home.as_ref().map(|h| h.join(under)));
        Dirs {
            config: pick("XDG_CONFIG_HOME", ".config"),
            state: pick("XDG_STATE_HOME", ".local/state"),
            cache: pick("XDG_CACHE_HOME", ".cache"),
        }
    }
}

/// What a run did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Old path, new path.
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// What was left alone and why (a link, someone else's file, an error).
    pub left: Vec<(PathBuf, String)>,
}

static ONCE: Once = Once::new();

/// [`adopt`] on the real folders, once per process; what it did goes to
/// stderr. Cheap after the first call.
pub fn adopt_legacy_files() {
    ONCE.call_once(|| {
        let r = adopt(&Dirs::from_env());
        for (old, new) in &r.moved {
            eprintln!(
                "telamon-updater: moved {} to {}",
                old.display(),
                new.display()
            );
        }
        for (path, why) in &r.left {
            eprintln!("telamon-updater: left {} alone: {why}", path.display());
        }
    });
}

/// Moves what is there under the old names. Safe to run again and from
/// several processes at once.
pub fn adopt(dirs: &Dirs) -> Report {
    let mut r = Report::default();
    if let Some(config) = &dirs.config {
        let old = config.join(RC.0);
        let new = config.join(RC.1);
        if adopt_file(&old, &new, &mut r) {
            // the settings writer's lock file belongs to the old name
            let stale = config.join(format!(".{}.lock", RC.0));
            if fs::symlink_metadata(&stale).is_ok_and(|m| m.file_type().is_file()) {
                let _ = fs::remove_file(stale);
            }
        }
    }
    for base in [&dirs.state, &dirs.cache].into_iter().flatten() {
        adopt_dir(&base.join(DIR.0), &base.join(DIR.1), &mut r);
    }
    r
}

fn uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// What is at `path`, without following a link.
fn look(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// `rename(old, new)` that fails with `AlreadyExists` instead of replacing.
fn rename_new(old: &Path, new: &Path) -> io::Result<()> {
    match renameat_with(CWD, old, CWD, new, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(e) if e == rustix::io::Errno::EXIST || e == rustix::io::Errno::NOTEMPTY => {
            Err(io::ErrorKind::AlreadyExists.into())
        }
        // a file system without RENAME_NOREPLACE: look first, then rename
        Err(e) if e == rustix::io::Errno::INVAL || e == rustix::io::Errno::NOSYS => {
            if look(new)?.is_some() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            fs::rename(old, new)
        }
        Err(e) => Err(e.into()),
    }
}

/// Moves the regular file `old` to `new` unless `new` is there. True when it
/// moved.
fn adopt_file(old: &Path, new: &Path, r: &mut Report) -> bool {
    let go = || -> io::Result<Option<bool>> {
        let Some(m) = look(old)? else {
            return Ok(None);
        };
        if !m.file_type().is_file() {
            return Err(io::Error::other(
                "not a regular file (a link, or something else)",
            ));
        }
        if m.uid() != uid() {
            return Err(io::Error::other("not this user's file"));
        }
        if look(new)?.is_some() {
            // the new name is there already: the old one stays, unread
            return Ok(Some(false));
        }
        match rename_new(old, new) {
            Ok(()) => Ok(Some(true)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(Some(false)),
            Err(e) => Err(e),
        }
    };
    match go() {
        Ok(Some(true)) => {
            r.moved.push((old.to_path_buf(), new.to_path_buf()));
            true
        }
        Ok(_) => false,
        Err(e) => {
            r.left.push((old.to_path_buf(), e.to_string()));
            false
        }
    }
}

/// Moves the folder `old` to `new`; when `new` is a folder already, moves
/// what it has no file of that name for, one file at a time, and removes
/// `old` if that emptied it.
fn adopt_dir(old: &Path, new: &Path, r: &mut Report) {
    let mut go = || -> io::Result<()> {
        let Some(m) = look(old)? else {
            return Ok(());
        };
        if !m.file_type().is_dir() {
            return Err(io::Error::other("not a folder (a link, or something else)"));
        }
        if m.uid() != uid() {
            return Err(io::Error::other("not this user's folder"));
        }
        match look(new)? {
            None => match rename_new(old, new) {
                Ok(()) => {
                    r.moved.push((old.to_path_buf(), new.to_path_buf()));
                    Ok(())
                }
                // another process was first: merge into what it made
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => merge(old, new, r),
                Err(e) => Err(e),
            },
            Some(n) if n.file_type().is_dir() && n.uid() == uid() => merge(old, new, r),
            Some(_) => Err(io::Error::other(
                "the new name is a link, or not this user's: nothing moved",
            )),
        }
    };
    if let Err(e) = go() {
        r.left.push((old.to_path_buf(), e.to_string()));
    }
}

fn merge(old: &Path, new: &Path, r: &mut Report) -> io::Result<()> {
    let entries = match fs::read_dir(old) {
        Ok(e) => e,
        // another process moved it all while we looked
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let (from, to) = (entry.path(), new.join(entry.file_name()));
        // files only: a link or a folder inside is not ours to move
        adopt_file(&from, &to, r);
    }
    // gone if empty; kept (with what could not move) if not
    let _ = fs::remove_dir(old);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct Home {
        _t: tempfile::TempDir,
        dirs: Dirs,
    }

    fn home() -> Home {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        let dirs = Dirs {
            config: Some(p.join("config")),
            state: Some(p.join("state")),
            cache: Some(p.join("cache")),
        };
        for d in [&dirs.config, &dirs.state, &dirs.cache] {
            fs::create_dir(d.as_ref().unwrap()).unwrap();
        }
        Home { _t: t, dirs }
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn gone(path: &Path) -> bool {
        fs::symlink_metadata(path).is_err()
    }

    #[test]
    fn everything_under_the_old_names_moves_once() {
        let h = home();
        let (c, s, k) = (
            h.dirs.config.clone().unwrap(),
            h.dirs.state.clone().unwrap(),
            h.dirs.cache.clone().unwrap(),
        );
        put(&c.join("atlas-updaterrc"), "[Restart]\nScheduledAt=123\n");
        put(&c.join(".atlas-updaterrc.lock"), "");
        put(&s.join("atlas-updater/app-updates.jsonl"), "{\"at\":1}\n");
        put(&s.join("atlas-updater/other"), "x");
        put(&k.join("atlas-updater/releases.json"), "[]");
        fs::set_permissions(s.join("atlas-updater"), fs::Permissions::from_mode(0o700)).unwrap();

        let r = adopt(&h.dirs);
        assert_eq!(r.moved.len(), 3, "{r:?}");
        assert!(r.left.is_empty(), "{r:?}");
        assert_eq!(
            read(&c.join("telamon-updaterrc")),
            "[Restart]\nScheduledAt=123\n"
        );
        assert_eq!(
            read(&s.join("telamon-updater/app-updates.jsonl")),
            "{\"at\":1}\n"
        );
        assert_eq!(read(&s.join("telamon-updater/other")), "x");
        assert_eq!(read(&k.join("telamon-updater/releases.json")), "[]");
        // moved, not copied: nothing is left under the old names, not even
        // the settings writer's lock file
        for old in [
            c.join("atlas-updaterrc"),
            c.join(".atlas-updaterrc.lock"),
            s.join("atlas-updater"),
            k.join("atlas-updater"),
        ] {
            assert!(gone(&old), "{}", old.display());
        }
        // the folder keeps its mode
        let mode = fs::metadata(s.join("telamon-updater")).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // a second run, from another process say, finds nothing to do
        assert_eq!(adopt(&h.dirs), Report::default());
    }

    #[test]
    fn nothing_there_is_nothing_done() {
        let h = home();
        assert_eq!(adopt(&h.dirs), Report::default());
        assert!(
            fs::read_dir(h.dirs.config.as_ref().unwrap())
                .unwrap()
                .next()
                .is_none()
        );
        // and without a home folder at all
        assert_eq!(adopt(&Dirs::default()), Report::default());
    }

    #[test]
    fn a_settings_file_that_exists_under_the_new_name_is_never_replaced() {
        let h = home();
        let c = h.dirs.config.clone().unwrap();
        put(&c.join("atlas-updaterrc"), "old");
        put(&c.join("telamon-updaterrc"), "new");
        let r = adopt(&h.dirs);
        assert!(r.moved.is_empty());
        assert_eq!(read(&c.join("telamon-updaterrc")), "new");
        assert_eq!(read(&c.join("atlas-updaterrc")), "old");
    }

    #[test]
    fn a_half_done_state_is_merged_file_by_file_and_nothing_is_lost() {
        let h = home();
        let s = h.dirs.state.clone().unwrap();
        // a new tray wrote its history before the old folder was adopted
        put(&s.join("atlas-updater/app-updates.jsonl"), "old history\n");
        put(&s.join("atlas-updater/extra"), "extra");
        put(
            &s.join("telamon-updater/app-updates.jsonl"),
            "new history\n",
        );
        let r = adopt(&h.dirs);
        assert_eq!(r.moved.len(), 1, "{r:?}");
        assert_eq!(read(&s.join("telamon-updater/extra")), "extra");
        assert_eq!(
            read(&s.join("telamon-updater/app-updates.jsonl")),
            "new history\n"
        );
        // the one both had stays where it was: the old folder stays with it
        assert_eq!(
            read(&s.join("atlas-updater/app-updates.jsonl")),
            "old history\n"
        );
        assert!(gone(&s.join("atlas-updater/extra")));
        // nothing more to do the next time
        assert!(adopt(&h.dirs).moved.is_empty());
        // and with nothing in the way the leftover folder goes
        let h = home();
        let s = h.dirs.state.clone().unwrap();
        put(&s.join("atlas-updater/a"), "1");
        fs::create_dir(s.join("telamon-updater")).unwrap();
        adopt(&h.dirs);
        assert_eq!(read(&s.join("telamon-updater/a")), "1");
        assert!(gone(&s.join("atlas-updater")));
    }

    #[test]
    fn links_are_never_followed_or_moved() {
        let h = home();
        let (c, s, k) = (
            h.dirs.config.clone().unwrap(),
            h.dirs.state.clone().unwrap(),
            h.dirs.cache.clone().unwrap(),
        );
        let secret = h._t.path().join("secret");
        put(&secret, "private");
        let elsewhere = h._t.path().join("elsewhere");
        put(&elsewhere.join("keep"), "k");
        // a settings file that is a link
        symlink(&secret, c.join("atlas-updaterrc")).unwrap();
        // a state folder that is a link to a folder
        symlink(&elsewhere, s.join("atlas-updater")).unwrap();
        // a cache folder with a link inside, beside a file
        put(&k.join("atlas-updater/releases.json"), "[]");
        symlink(&secret, k.join("atlas-updater/leak")).unwrap();

        let r = adopt(&h.dirs);
        assert!(gone(&c.join("telamon-updaterrc")));
        assert!(
            fs::symlink_metadata(c.join("atlas-updaterrc"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(gone(&s.join("telamon-updater")));
        assert!(
            fs::symlink_metadata(s.join("atlas-updater"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(read(&elsewhere.join("keep")), "k");
        // the cache: a real folder moves as it is, the link inside with it,
        // still a link and not followed
        assert_eq!(read(&k.join("telamon-updater/releases.json")), "[]");
        assert!(
            fs::symlink_metadata(k.join("telamon-updater/leak"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(gone(&k.join("atlas-updater")));
        assert_eq!(read(&secret), "private");
        assert!(r.left.len() >= 2, "{r:?}");
    }

    #[test]
    fn a_new_name_that_is_a_link_is_left_alone() {
        let h = home();
        let s = h.dirs.state.clone().unwrap();
        let target = h._t.path().join("target");
        fs::create_dir(&target).unwrap();
        put(&s.join("atlas-updater/f"), "1");
        symlink(&target, s.join("telamon-updater")).unwrap();
        let r = adopt(&h.dirs);
        assert!(r.moved.is_empty(), "{r:?}");
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        assert_eq!(read(&s.join("atlas-updater/f")), "1");
        // same for a settings file
        let c = h.dirs.config.clone().unwrap();
        put(&c.join("atlas-updaterrc"), "old");
        symlink(h._t.path().join("nowhere"), c.join("telamon-updaterrc")).unwrap();
        assert!(adopt(&h.dirs).moved.is_empty());
        assert_eq!(read(&c.join("atlas-updaterrc")), "old");
    }

    #[test]
    fn a_folder_where_a_file_belongs_is_refused() {
        let h = home();
        let c = h.dirs.config.clone().unwrap();
        fs::create_dir(c.join("atlas-updaterrc")).unwrap();
        let r = adopt(&h.dirs);
        assert!(r.moved.is_empty());
        assert!(!r.left.is_empty());
        assert!(gone(&c.join("telamon-updaterrc")));
    }

    #[test]
    fn two_processes_at_once_move_each_thing_exactly_once() {
        let h = home();
        let s = h.dirs.state.clone().unwrap();
        let c = h.dirs.config.clone().unwrap();
        for i in 0..20 {
            put(&s.join(format!("atlas-updater/f{i}")), &i.to_string());
        }
        put(&c.join("atlas-updaterrc"), "rc");
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let d = h.dirs.clone();
                std::thread::spawn(move || adopt(&d))
            })
            .collect();
        let reports: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        // the rc file and each state file were moved by one run only (the
        // folder itself counts as one move when a run was first)
        let rc_moves = reports
            .iter()
            .flat_map(|r| &r.moved)
            .filter(|(o, _)| o.ends_with("atlas-updaterrc"))
            .count();
        assert_eq!(rc_moves, 1);
        for i in 0..20 {
            assert_eq!(
                read(&s.join(format!("telamon-updater/f{i}"))),
                i.to_string()
            );
        }
        assert_eq!(read(&c.join("telamon-updaterrc")), "rc");
        assert!(gone(&s.join("atlas-updater")));
    }
}
