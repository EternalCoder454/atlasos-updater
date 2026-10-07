//! The app updates this user's Telamon Updater installed, by hand or in the
//! background: `~/.local/state/telamon-updater/app-updates.jsonl`, one JSON
//! object per line, oldest first. Shown on the History page.

use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config;

/// Past this size the file is cut down to the newest `KEEP` lines.
const MAX_BYTES: u64 = 256 * 1024;
const KEEP: usize = 500;
/// Never read more than this much of the file: it lives where any app with
/// access to the home folder could have grown it.
const READ_MAX: u64 = 512 * 1024;
/// The History page shows at most this many.
pub const SHOW: usize = 200;
/// How long a writer waits for the other one.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// A longer line isn't one we wrote: it is skipped.
const LINE_MAX: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix seconds.
    pub at: i64,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub runtime: bool,
    #[serde(default)]
    pub system: bool,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    /// Installed by the background updater rather than "Update Apps".
    #[serde(default)]
    pub auto: bool,
}

fn state_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
        && d.is_absolute()
    {
        return Some(d);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    home.is_absolute().then(|| home.join(".local/state"))
}

pub fn path() -> Option<PathBuf> {
    // the old folder, `atlas-updater`, moves to this name first
    telamon_updater_base::migrate::adopt_legacy_files();
    state_dir().map(|d| d.join("telamon-updater").join("app-updates.jsonl"))
}

/// Adds `entries`. Fixture mode writes nothing. Errors are returned for the
/// log; the update itself already happened.
pub fn record(entries: &[Entry], fixtures: Option<&Path>) -> Result<(), String> {
    if fixtures.is_some() || entries.is_empty() {
        return Ok(());
    }
    let path = path().ok_or("no home folder")?;
    record_at(&path, entries).map_err(|e| format!("{}: {e}", path.display()))
}

fn record_at(path: &Path, entries: &[Entry]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        own_dir(dir)?;
    }
    // One write for the batch.
    let lines = entries
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(io::Error::other)?;
    // The tray and the window both write: one at a time, and on the file
    // that is there now (a trim by the other may have replaced it).
    // One deadline for all of it: whoever holds or keeps replacing the
    // file doesn't keep us here.
    let give_up = std::time::Instant::now() + LOCK_WAIT;
    let mut tries = 0;
    let f = loop {
        tries += 1;
        if tries > 5 {
            return Err(io::Error::other("the file keeps being replaced"));
        }
        let f = open(path, true)?;
        lock(&f, give_up)?;
        let m = f.metadata()?;
        match std::fs::symlink_metadata(path) {
            Ok(now) if (now.dev(), now.ino()) == (m.dev(), m.ino()) => break f,
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    };
    let len = f.metadata()?.len();
    let mut data = String::new();
    if len > 0 {
        // A torn earlier write: start on a line of our own.
        let mut last = [0u8; 1];
        f.read_exact_at(&mut last, len - 1)?;
        if last[0] != b'\n' {
            data.push('\n');
        }
    }
    data.push_str(&lines.join("\n"));
    data.push('\n');
    (&f).write_all(data.as_bytes())?;
    f.sync_all()?;
    if f.metadata()?.len() > MAX_BYTES {
        trim(&f, path)?;
    }
    Ok(())
}

/// An exclusive lock on `f`, released when it is closed. Gives up at
/// `give_up`: any app with access to the home folder could hold it.
fn lock(f: &File, give_up: std::time::Instant) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    loop {
        // SAFETY: a valid open file descriptor.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        match e.kind() {
            io::ErrorKind::Interrupted => {}
            io::ErrorKind::WouldBlock if std::time::Instant::now() < give_up => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            io::ErrorKind::WouldBlock => {
                return Err(io::Error::other("another program holds the file"));
            }
            _ => return Err(e),
        }
    }
}

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Makes `dir` (0700) or checks the one there: a real folder of this user,
/// not a symlink; group and other access is taken away.
fn own_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let m = std::fs::symlink_metadata(dir)?;
    if !m.file_type().is_dir() || m.uid() != uid() {
        return Err(io::Error::other("not a folder of this user"));
    }
    if m.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Opens the history file: never through a symlink, never a FIFO or a
/// device (O_NONBLOCK keeps a FIFO from hanging the open), and only a
/// regular file this user owns. `append` creates it (0600) for writing.
fn open(path: &Path, append: bool) -> io::Result<File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    if append {
        o.append(true).create(true).mode(0o600);
    }
    let f = o
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file() || m.uid() != uid() {
        return Err(io::Error::other("not a regular file of this user"));
    }
    Ok(f)
}

/// The last `READ_MAX` bytes at most, starting at a whole line.
fn read_tail(f: &File) -> io::Result<Vec<u8>> {
    let len = f.metadata()?.len();
    let start = len.saturating_sub(READ_MAX);
    let mut buf = vec![0u8; (len - start) as usize];
    let mut got = 0;
    while got < buf.len() {
        match f.read_at(&mut buf[got..], start + got as u64)? {
            0 => break, // Cut short meanwhile.
            n => got += n,
        }
    }
    buf.truncate(got);
    if start > 0 {
        match buf.iter().position(|b| *b == b'\n') {
            Some(i) => {
                buf.drain(..=i);
            }
            None => buf.clear(),
        }
    }
    Ok(buf)
}

/// Keeps the newest `KEEP` lines: written to a temporary file, then renamed
/// over the old one, so a crash leaves one or the other.
fn trim(f: &File, path: &Path) -> io::Result<()> {
    let bytes = read_tail(f)?;
    let lines: Vec<_> = telamon_framework_core::fsutil::lossy_lines(&bytes)
        .into_iter()
        .filter(|l| l.len() <= LINE_MAX)
        .collect();
    let start = lines.len().saturating_sub(KEEP);
    // A name nobody else picks: create_new never opens a file planted there.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("jsonl.{}-{nanos}.tmp", std::process::id()));
    let mut t = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    let done = (|| {
        for l in &lines[start..] {
            writeln!(t, "{l}")?;
        }
        t.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if done.is_err() {
        // A full disk, say: leave nothing behind for the next try.
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

/// Newest first, at most `SHOW`. A missing file is an empty history; a
/// damaged line is skipped. Fixture mode reads `app-history.jsonl`.
pub fn read(fixtures: Option<&Path>) -> Vec<Entry> {
    let text = match fixtures {
        Some(dir) => config::read_fixture(dir, "app-history.jsonl").unwrap_or_default(),
        None => match path().map(|p| open(&p, false).and_then(|f| read_tail(&f))) {
            Some(Ok(b)) => String::from_utf8_lossy(&b).into_owned(),
            Some(Err(e)) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Some(Err(e)) => {
                eprintln!("telamon-updater: could not read the app update history: {e}");
                String::new()
            }
            None => String::new(),
        },
    };
    parse(&text)
}

/// Any app with access to the home folder can write the file: entries are
/// cleaned like text from a remote, and impossible ones are dropped.
fn sane(mut e: Entry, now: i64) -> Option<Entry> {
    use telamon_framework_flatpak::{clean, clean_to};
    // A day of clock difference is fine; a date in the far future isn't.
    if e.at < 0 || e.at > now + 86_400 {
        return None;
    }
    e.id = clean_to(&e.id, 255);
    e.name = clean(&e.name);
    e.branch = clean(&e.branch);
    e.from = e.from.map(|v| clean(&v)).filter(|v| !v.is_empty());
    e.to = e.to.map(|v| clean(&v)).filter(|v| !v.is_empty());
    Some(e)
}

fn parse(text: &str) -> Vec<Entry> {
    let now = crate::schedule::unix_now();
    let mut v: Vec<Entry> = text
        .lines()
        .filter(|l| l.len() <= LINE_MAX)
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter_map(|e| sane(e, now))
        .collect();
    v.reverse();
    v.truncate(SHOW);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: i64) -> Entry {
        Entry {
            at: n,
            id: format!("org.example.App{n}"),
            name: format!("App {n}"),
            branch: "stable".into(),
            runtime: false,
            system: true,
            from: Some("1.0".into()),
            to: Some("1.1".into()),
            auto: n % 2 == 0,
        }
    }

    #[test]
    fn records_and_reads_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/app-updates.jsonl");
        record_at(&p, &[entry(1), entry(2)]).unwrap();
        record_at(&p, &[entry(3)]).unwrap();
        let got = parse(&std::fs::read_to_string(&p).unwrap());
        assert_eq!(got.iter().map(|e| e.at).collect::<Vec<_>>(), vec![3, 2, 1]);
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = std::fs::metadata(p.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dmode, 0o700);
    }

    #[test]
    fn damaged_lines_are_skipped() {
        let good = serde_json::to_string(&entry(1)).unwrap();
        let got = parse(&format!("{good}\n{{not json\n\n{good}"));
        assert_eq!(got.len(), 2);
        // Older files without the optional fields still read.
        let got = parse(r#"{"at":5,"id":"a","name":"A"}"#);
        assert_eq!(got[0].from, None);
        assert!(!got[0].auto);
    }

    #[test]
    fn grows_no_further_than_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app-updates.jsonl");
        let batch: Vec<Entry> = (0..200).map(entry).collect();
        for _ in 0..15 {
            record_at(&p, &batch).unwrap();
        }
        assert!(std::fs::metadata(&p).unwrap().len() <= MAX_BYTES);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.lines().count() >= KEEP);
        // The newest line is still the last one written.
        assert_eq!(parse(&text)[0].at, 199);
        let left = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(left, 1, "no temporary file stays behind");
    }

    #[test]
    fn entries_are_cleaned_and_impossible_ones_dropped() {
        let mut e = entry(1);
        e.name = "Evil\u{202E}gpj.exe\nApp".into();
        e.to = Some("2\u{200B}.0".into());
        let mut future = entry(2);
        future.at = i64::MAX;
        let long = format!(r#"{{"at":3,"id":"a","name":"{}"}}"#, "x".repeat(LINE_MAX));
        let text = [
            serde_json::to_string(&e).unwrap(),
            serde_json::to_string(&future).unwrap(),
            long,
        ]
        .join("\n");
        let got = parse(&text);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "Evilgpj.exe App");
        assert_eq!(got[0].to.as_deref(), Some("2.0"));
    }

    #[test]
    fn shows_at_most_show() {
        let text: String = (0..(SHOW as i64 + 50))
            .map(|n| serde_json::to_string(&entry(n)).unwrap() + "\n")
            .collect();
        let got = parse(&text);
        assert_eq!(got.len(), SHOW);
        assert_eq!(got[0].at, SHOW as i64 + 49);
    }

    #[test]
    fn a_held_lock_is_given_up_on() {
        use std::os::fd::AsRawFd;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app-updates.jsonl");
        record_at(&p, &[entry(1)]).unwrap();
        let other = File::open(&p).unwrap();
        // SAFETY: a valid open file descriptor.
        assert_eq!(unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX) }, 0);
        let t = std::time::Instant::now();
        assert!(record_at(&p, &[entry(2)]).is_err());
        assert!(t.elapsed() < LOCK_WAIT + std::time::Duration::from_secs(2));
        drop(other);
        record_at(&p, &[entry(3)]).unwrap();
    }

    #[test]
    fn refuses_a_symlinked_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        std::fs::write(&target, "").unwrap();
        let p = dir.path().join("app-updates.jsonl");
        std::os::unix::fs::symlink(&target, &p).unwrap();
        assert!(record_at(&p, &[entry(1)]).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "");
        assert!(open(&p, false).is_err());
    }

    #[test]
    fn refuses_a_fifo_without_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app-updates.jsonl");
        let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(open(&p, false).is_err());
        assert!(record_at(&p, &[entry(1)]).is_err());
    }

    #[test]
    fn reads_only_the_tail_of_a_huge_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app-updates.jsonl");
        let line = serde_json::to_string(&entry(7)).unwrap();
        let mut text = "x".repeat(2 * READ_MAX as usize);
        text.push('\n');
        text.push_str(&line);
        text.push('\n');
        std::fs::write(&p, &text).unwrap();
        let tail = read_tail(&open(&p, false).unwrap()).unwrap();
        assert_eq!(parse(&String::from_utf8_lossy(&tail))[0].at, 7);
        assert!(tail.len() as u64 <= READ_MAX);
    }

    #[test]
    fn tightens_an_open_folder() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("telamon-updater");
        std::fs::create_dir(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        record_at(&d.join("app-updates.jsonl"), &[entry(1)]).unwrap();
        let mode = std::fs::metadata(&d).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let link = dir.path().join("linked");
        std::os::unix::fs::symlink(&d, &link).unwrap();
        assert!(record_at(&link.join("app-updates.jsonl"), &[entry(2)]).is_err());
    }
}
