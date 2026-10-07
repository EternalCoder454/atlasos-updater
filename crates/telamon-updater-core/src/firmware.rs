//! Firmware updates, window side. `telamon_updater_base::fwupd` talks to fwupd;
//! this module fetches the file for an install (https only, into a sealed
//! in-memory file, checked against the release's checksum before fwupd sees
//! it) and turns a listing into the JSON the Updates page shows.
//!
//! Nothing here touches the Qt thread: `backend.rs` calls it from workers.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::errors::OpError;
use telamon_updater_base::fwupd::{
    self, Checksum, Done, FirmwareUpdate, Listing, Pending, PendingState, Progress, Urgency,
};

/// The most a firmware file may be, whatever the release says.
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const BUS_TIMEOUT: Duration = Duration::from_secs(25);
/// Metadata older than this gets a note on the page.
const STALE_AFTER: Duration = Duration::from_secs(30 * 24 * 3600);

fn msg(s: impl Into<String>) -> OpError {
    OpError::Message(s.into())
}

// ------------------------------------------------------------ URL policy

/// The first location that is acceptable (absolute `https://`, or relative to
/// an `https://` `FirmwareBaseUri`), as the URL to fetch.
pub fn pick_url(locations: &[String], base_uri: Option<&str>) -> Option<String> {
    locations
        .iter()
        .find_map(|l| fwupd::resolve_location(l, base_uri))
}

/// The checksum (hex) the page shows for a release and the install pins:
/// the one [`fwupd::pick_checksum`] chooses, or "".
pub fn picked_checksum(checksums: &[String]) -> String {
    fwupd::pick_checksum(checksums)
        .map(|c| c.hex().to_string())
        .unwrap_or_default()
}

const UNTRUSTED: &str =
    "This update is not signed by a trusted source, so Telamon Updater won't install it.";
const CHANGED: &str = "This update changed since it was shown. Check again.";

/// A file name in the developer folder: a plain name, no path.
fn plain_file_name(location: &str) -> Option<&str> {
    let name = location.strip_prefix("./").unwrap_or(location);
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && !name.contains("..");
    ok.then_some(name)
}

// ------------------------------------------------------------- the memfd

/// A new in-memory file that can be sealed.
fn new_memfd() -> io::Result<File> {
    // SAFETY: a valid NUL-terminated name; the result is checked.
    let fd = unsafe {
        libc::memfd_create(
            c"telamon-firmware".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh descriptor nobody else owns.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// Rewinds and seals: nobody can change, grow or shrink it afterwards.
fn seal(mut file: File) -> io::Result<OwnedFd> {
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    let seals = libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;
    // SAFETY: fcntl on a descriptor we own; no memory is passed.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(OwnedFd::from(file))
}

#[derive(Debug, PartialEq, Eq)]
enum CopyError {
    TooBig,
    Io(String),
}

/// Copies at most `cap` bytes; one more byte than that is `TooBig`.
fn copy_capped(r: &mut impl Read, w: &mut impl Write, cap: u64) -> Result<u64, CopyError> {
    let mut limited = r.take(cap.saturating_add(1));
    let n = io::copy(&mut limited, w).map_err(|e| CopyError::Io(e.to_string()))?;
    if n > cap {
        Err(CopyError::TooBig)
    } else {
        Ok(n)
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Hashes the whole file from its start (positional reads: the file's
/// offset is not touched) and compares with `want`.
fn verify(file: &File, want: &Checksum) -> Result<(), String> {
    use ring::digest;
    let algo = match want {
        Checksum::Sha256(_) => &digest::SHA256,
        Checksum::Sha1(_) => &digest::SHA1_FOR_LEGACY_USE_ONLY,
    };
    let mut ctx = digest::Context::new(algo);
    let mut buf = vec![0u8; 64 * 1024];
    let mut at = 0u64;
    loop {
        let n = match file.read_at(&mut buf, at) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        };
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
        at += n as u64;
    }
    if hex(ctx.finish().as_ref()) == want.hex() {
        Ok(())
    } else {
        Err(
            "The downloaded firmware file does not match its checksum. Nothing was installed."
                .into(),
        )
    }
}

/// Seals first, then checks the sealed file: what was hashed is what fwupd
/// gets, whoever else still holds a descriptor to it.
fn seal_verified(file: File, want: &Checksum) -> Result<OwnedFd, String> {
    let fd = seal(file).map_err(|e| format!("Could not prepare the firmware file: {e}"))?;
    let f = File::from(fd);
    verify(&f, want)?;
    Ok(OwnedFd::from(f))
}

// -------------------------------------------------------------- fetching

fn download_error(e: &ureq::Error) -> String {
    // The kind only: a redirect target's URL can carry a token.
    let kind = match e {
        ureq::Error::Timeout(_) => "timeout".to_string(),
        ureq::Error::StatusCode(c) => format!("status {c}"),
        ureq::Error::Io(_) => "i/o error".to_string(),
        ureq::Error::HostNotFound => "host not found".to_string(),
        ureq::Error::ConnectionFailed => "connection failed".to_string(),
        _ => "other error".to_string(),
    };
    eprintln!("telamon-updater: firmware download failed: {kind}");
    match e {
        ureq::Error::Timeout(_) => "The firmware download took too long. Try again.".into(),
        ureq::Error::StatusCode(c) => format!("The firmware server answered with an error ({c})."),
        ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
            "Could not reach the firmware server. Check your internet connection.".into()
        }
        _ => "Could not download the firmware file.".into(),
    }
}

/// Streams `url` into `sink`, at most `cap` bytes. https only, also across
/// redirects.
fn download(url: &str, cap: u64, sink: &mut File) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(TOTAL_TIMEOUT))
        .user_agent(concat!("telamon-updater/", env!("CARGO_PKG_VERSION")))
        .max_redirects(3)
        .https_only(true)
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| download_error(&e))?;
    if let Some(len) = resp.body().content_length()
        && len > cap
    {
        return Err("The firmware file is too large. Nothing was installed.".into());
    }
    let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
    copy_capped(&mut reader, sink, cap).map_err(|e| match e {
        CopyError::TooBig => "The firmware file is too large. Nothing was installed.".into(),
        CopyError::Io(e) => {
            eprintln!("telamon-updater: firmware download failed: {e}");
            "The firmware download was interrupted. Try again.".to_string()
        }
    })?;
    Ok(())
}

/// Debug builds only: a relative location read from the developer folder.
#[cfg(any(debug_assertions, test))]
fn read_local(dir: &Path, location: &str, cap: u64, sink: &mut File) -> Result<(), String> {
    let name = plain_file_name(location)
        .ok_or_else(|| "That firmware location is not a plain file name.".to_string())?;
    use std::os::unix::fs::OpenOptionsExt;
    // No symlink, no waiting on a FIFO; the kind is checked on the open fd.
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(dir.join(name))
        .map_err(|e| format!("Cannot read {name}: {e}"))?;
    let meta = f
        .metadata()
        .map_err(|e| format!("Cannot read {name}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{name} is not a regular file."));
    }
    copy_capped(&mut f, sink, cap).map_err(|e| match e {
        CopyError::TooBig => "The firmware file is too large.".to_string(),
        CopyError::Io(e) => e,
    })?;
    Ok(())
}

/// Blocking: the verified, sealed file for `update`. `base_uri` is its
/// remote's `FirmwareBaseUri`.
pub fn fetch(
    update: &FirmwareUpdate,
    base_uri: Option<&str>,
    local_dir: Option<&Path>,
) -> Result<OwnedFd, String> {
    if !update.trusted {
        return Err(UNTRUSTED.into());
    }
    let want = fwupd::pick_checksum(&update.checksums).ok_or_else(|| {
        "This update has no checksum to check the file against, so it was not installed."
            .to_string()
    })?;
    // fwupd's release Size is not the size of the cabinet, so only the
    // hard limit applies.
    let cap = MAX_BYTES;
    let mut file =
        new_memfd().map_err(|e| format!("Could not make room for the firmware file: {e}"))?;

    let local = local_dir.and_then(|d| {
        update
            .locations
            .iter()
            .find(|l| {
                fwupd::resolve_location(l, base_uri).is_none() && plain_file_name(l).is_some()
            })
            .map(|l| (d, l.as_str()))
    });
    match local {
        #[cfg(any(debug_assertions, test))]
        Some((dir, loc)) => read_local(dir, loc, cap, &mut file)?,
        #[cfg(not(any(debug_assertions, test)))]
        Some(_) => return Err("That firmware location is not allowed.".into()),
        None => {
            let url = pick_url(&update.locations, base_uri).ok_or_else(|| {
                "This update has no secure (https) download address, so it was not installed."
                    .to_string()
            })?;
            download(&url, cap, &mut file)?;
        }
    }
    seal_verified(file, &want)
}

/// The developer folder, debug builds only.
pub fn local_dir() -> Option<PathBuf> {
    #[cfg(any(debug_assertions, test))]
    {
        crate::config::env("FIRMWARE_FILES")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
    #[cfg(not(any(debug_assertions, test)))]
    {
        None
    }
}

// ------------------------------------------------------------ the D-Bus side

async fn connect() -> Result<zbus::Connection, OpError> {
    match tokio::time::timeout(BUS_TIMEOUT, zbus::Connection::system()).await {
        Ok(Ok(c)) => Ok(c),
        Ok(Err(e)) => {
            eprintln!("telamon-updater: cannot reach the system bus: {e}");
            Err(msg("Can't reach the system bus."))
        }
        Err(_) => Err(msg("The system bus did not answer in time.")),
    }
}

/// What a check found. `None`: fwupd is not on this system.
pub async fn list() -> Result<Option<Listing>, OpError> {
    let conn = match connect().await {
        Ok(c) => c,
        // No bus at all: nothing to offer, and nothing to complain about.
        Err(_) => return Ok(None),
    };
    if !fwupd::available(&conn).await.unwrap_or(false) {
        return Ok(None);
    }
    fwupd::list(&conn).await.map(Some)
}

/// `FirmwareBaseUri` of the remote `remote_id`, when it has one.
async fn base_uri_of(conn: &zbus::Connection, remote_id: &str) -> Option<String> {
    use zbus::zvariant::{OwnedValue, Value};
    let why = |what: &str, e: &dyn std::fmt::Display| -> Option<String> {
        eprintln!("telamon-updater: cannot read the firmware remotes ({what}): {e}");
        None
    };
    let built = zbus::proxy::Builder::<zbus::Proxy>::new(conn)
        .destination("org.freedesktop.fwupd")
        .and_then(|b| b.path("/"))
        .and_then(|b| b.interface("org.freedesktop.fwupd"))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No));
    let proxy = match built {
        Ok(b) => match b.build().await {
            Ok(p) => p,
            Err(e) => return why("proxy", &e),
        },
        Err(e) => return why("proxy", &e),
    };
    let remotes: Vec<HashMap<String, OwnedValue>> =
        match tokio::time::timeout(BUS_TIMEOUT, proxy.call("GetRemotes", &())).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return why("call", &e),
            Err(_) => return why("call", &"timed out"),
        };
    let r = remotes.iter().find(|r| {
        matches!(r.get("RemoteId").map(|v| &**v), Some(Value::Str(s)) if s.as_str() == remote_id)
    })?;
    match r.get("FirmwareBaseUri").map(|v| &**v) {
        Some(Value::Str(s)) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

/// Installs the update the user chose. Reads the release again and refuses if
/// it is no longer `shown_version` with `shown_checksum`, or not trusted; downloads and checks the file; hands it to
/// fwupd. Runs inside a tokio runtime on a worker thread.
pub async fn install(
    device_id: &str,
    shown_version: &str,
    shown_checksum: &str,
    mut progress: impl FnMut(Progress),
) -> Result<Done, OpError> {
    let conn = connect().await?;
    let update = fwupd::upgrade_for(&conn, device_id)
        .await?
        .ok_or_else(|| msg("That update is no longer available. Check for updates again."))?;
    if update.version != shown_version {
        return Err(msg(format!(
            "The update for {} changed to version {} while you were looking at it. Check the new one and press Install again.",
            update.device, update.version
        )));
    }
    if picked_checksum(&update.checksums) != shown_checksum {
        return Err(msg(CHANGED));
    }
    if !update.trusted {
        return Err(msg(UNTRUSTED));
    }
    let base = base_uri_of(&conn, &update.remote_id).await;
    progress(Progress {
        status: "Downloading",
        percent: None,
        request: None,
    });
    let local = local_dir();
    let for_fetch = update.clone();
    let fd =
        tokio::task::spawn_blocking(move || fetch(&for_fetch, base.as_deref(), local.as_deref()))
            .await
            .map_err(|_| msg("The firmware download stopped unexpectedly."))?
            .map_err(OpError::Message)?;
    fwupd::install(&conn, device_id, fd, progress).await
}

// ----------------------------------------------------------------- the page

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct UpdateRow {
    pub id: String,
    pub device: String,
    pub vendor: String,
    pub current: String,
    pub version: String,
    pub summary: String,
    pub description: String,
    pub important: bool,
    pub reboot: bool,
    pub shutdown: bool,
    /// fwupd marks the release as signed by a trusted source.
    pub trusted: bool,
    /// The checksum the install is pinned to (hex), or "".
    pub checksum: String,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct PendingRow {
    pub id: String,
    pub device: String,
    pub version: String,
    /// "reboot", "shutdown" or "failed".
    pub state: &'static str,
    /// fwupd's words, for "failed".
    pub text: String,
}

#[derive(Serialize, Debug, PartialEq, Eq, Default)]
pub struct View {
    pub updates: Vec<UpdateRow>,
    pub pending: Vec<PendingRow>,
    /// The metadata-age note, or "".
    pub note: String,
}

/// "…last updated 45 days ago" when the metadata is old or was never fetched.
pub fn age_note(age: Option<Duration>) -> String {
    match age {
        None => "The firmware list has never been downloaded.".into(),
        Some(a) if a > STALE_AFTER => {
            let days = a.as_secs() / 86_400;
            format!("The firmware list was last updated {days} days ago.")
        }
        Some(_) => String::new(),
    }
}

/// `shutdown`: devices a finished install this session said need a shut down
/// (fwupd lists them as plain pending).
pub fn view(l: &Listing, shutdown: &HashSet<String>) -> View {
    View {
        updates: l
            .updates
            .iter()
            .map(|u| UpdateRow {
                id: u.device_id.clone(),
                device: u.device.clone(),
                vendor: u.vendor.clone(),
                current: u.current.clone(),
                version: u.version.clone(),
                summary: u.summary.clone(),
                description: u.description.clone(),
                important: u.urgency >= Urgency::High,
                reboot: u.needs_reboot,
                shutdown: u.needs_shutdown,
                trusted: u.trusted,
                checksum: picked_checksum(&u.checksums),
            })
            .collect(),
        pending: l
            .pending
            .iter()
            .map(|p| {
                let (state, text) = match &p.state {
                    PendingState::Reboot if shutdown.contains(&p.device_id) => {
                        ("shutdown", String::new())
                    }
                    PendingState::Reboot => ("reboot", String::new()),
                    PendingState::Failed(why) => ("failed", why.clone()),
                };
                PendingRow {
                    id: p.device_id.clone(),
                    device: p.device.clone(),
                    version: p.version.clone(),
                    state,
                    text,
                }
            })
            .collect(),
        note: age_note(l.metadata_age),
    }
}

// ---------------------------------------------------------------- fixtures

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixUpdate {
    device_id: String,
    device: String,
    #[serde(default)]
    vendor: String,
    current: String,
    version: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    /// "low", "medium", "high" or "critical".
    #[serde(default)]
    urgency: String,
    #[serde(default)]
    needs_reboot: bool,
    #[serde(default)]
    needs_shutdown: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixPending {
    device_id: String,
    device: String,
    version: String,
    /// "reboot", or "failed" with `error`.
    state: String,
    #[serde(default)]
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixFile {
    #[serde(default)]
    updates: Vec<FixUpdate>,
    #[serde(default)]
    pending: Vec<FixPending>,
    /// Days since the metadata was fetched; absent: never.
    #[serde(default)]
    metadata_age_days: Option<u64>,
}

/// The listing a developer fixture file describes.
pub fn parse_fixture(text: &str) -> Result<Listing, String> {
    let f: FixFile = serde_json::from_str(text).map_err(|e| format!("firmware.json: {e}"))?;
    let urgency = |s: &str| match s {
        "low" => Urgency::Low,
        "medium" => Urgency::Medium,
        "high" => Urgency::High,
        "critical" => Urgency::Critical,
        _ => Urgency::Unknown,
    };
    let mut l = Listing {
        metadata_age: f
            .metadata_age_days
            .map(|d| Duration::from_secs(d.min(36_500) * 86_400)),
        ..Listing::default()
    };
    for u in f.updates {
        l.updates.push(FirmwareUpdate {
            urgency: urgency(&u.urgency),
            device_id: u.device_id,
            device: u.device,
            vendor: u.vendor,
            current: u.current,
            version: u.version,
            summary: u.summary,
            description: u.description,
            size: 0,
            checksums: Vec::new(),
            locations: Vec::new(),
            remote_id: String::new(),
            trusted: true,
            needs_reboot: u.needs_reboot,
            needs_shutdown: u.needs_shutdown,
            internal: false,
        });
    }
    for p in f.pending {
        let state = match p.state.as_str() {
            "reboot" => PendingState::Reboot,
            "failed" => PendingState::Failed(p.error),
            other => return Err(format!("firmware.json: unknown state {other:?}")),
        };
        l.pending.push(Pending {
            device_id: p.device_id,
            device: p.device,
            version: p.version,
            state,
        });
    }
    Ok(l)
}

/// Fixture mode: the listing from `firmware.json`, or `None` (no fwupd) when
/// the file is not there.
pub fn fixture_listing(dir: &Path) -> Result<Option<Listing>, String> {
    match telamon_updater_base::config::read_fixture(dir, "firmware.json") {
        Some(t) => parse_fixture(&t).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_https_locations_are_used() {
        let locs = strs(&[
            "http://x.test/a.cab",
            "file:///etc/passwd",
            "https://cdn.test/a.cab",
        ]);
        assert_eq!(
            pick_url(&locs, None).as_deref(),
            Some("https://cdn.test/a.cab")
        );
        assert_eq!(pick_url(&strs(&["http://x.test/a.cab"]), None), None);
        assert_eq!(pick_url(&[], None), None);
    }

    #[test]
    fn a_relative_location_needs_an_https_base() {
        let locs = strs(&["./a.cab"]);
        assert_eq!(pick_url(&locs, None), None);
        assert_eq!(pick_url(&locs, Some("http://x.test/fw")), None);
        assert_eq!(
            pick_url(&locs, Some("https://x.test/fw")).as_deref(),
            Some("https://x.test/fw/a.cab")
        );
    }

    #[test]
    fn a_wrong_release_size_does_not_stop_a_good_file() {
        // fwupd's Size is not the cabinet size: only MAX_BYTES caps a file.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.cab"), b"abc").unwrap();
        for size in [0, 1, 17, u64::MAX] {
            let u = update(&[ABC_SHA256], &["./a.cab"], size);
            assert!(fetch(&u, None, Some(d.path())).is_ok(), "{size}");
        }
    }

    #[test]
    fn untrusted_releases_are_refused() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.cab"), b"abc").unwrap();
        let mut u = update(&[ABC_SHA256], &["./a.cab"], 3);
        u.trusted = false;
        assert_eq!(fetch(&u, None, Some(d.path())).unwrap_err(), UNTRUSTED);
    }

    #[test]
    fn the_pinned_checksum_is_the_preferred_one() {
        let c = strs(&[ABC_SHA1, ABC_SHA256]);
        assert_eq!(picked_checksum(&c), ABC_SHA256);
        assert_eq!(picked_checksum(&c[..1]), ABC_SHA1);
        assert_eq!(picked_checksum(&[]), "");
        let l = Listing {
            updates: vec![update(&[ABC_SHA256], &[], 0)],
            ..Listing::default()
        };
        let v = view(&l, &HashSet::new());
        assert_eq!(v.updates[0].checksum, ABC_SHA256);
        assert!(v.updates[0].trusted);
    }

    #[test]
    fn a_descriptor_opened_before_sealing_cannot_change_the_file() {
        let f = memfd_with(b"abc");
        // a second, writable descriptor to the same file
        let other = std::fs::OpenOptions::new()
            .write(true)
            .open(format!("/proc/self/fd/{}", f.as_raw_fd()))
            .unwrap();
        let fd = seal_verified(f, &Checksum::Sha256(ABC_SHA256.into())).unwrap();
        let mut other = other;
        let err = other.write_all(b"evil").unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EPERM));
        assert!(other.set_len(0).is_err());
        let mut s = String::new();
        File::from(fd).read_to_string(&mut s).unwrap();
        assert_eq!(s, "abc");
        // content that does not match is refused after sealing too
        let g = memfd_with(b"abd");
        assert!(seal_verified(g, &Checksum::Sha256(ABC_SHA256.into())).is_err());
    }

    #[test]
    fn a_symlink_in_the_developer_folder_is_refused() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("real.cab"), b"abc").unwrap();
        std::os::unix::fs::symlink(d.path().join("real.cab"), d.path().join("a.cab")).unwrap();
        let u = update(&[ABC_SHA256], &["./a.cab"], 3);
        assert!(fetch(&u, None, Some(d.path())).is_err());
        std::fs::create_dir(d.path().join("dir.cab")).unwrap();
        let u = update(&[ABC_SHA256], &["./dir.cab"], 3);
        assert!(fetch(&u, None, Some(d.path())).is_err());
    }

    #[test]
    fn copying_stops_one_byte_over_the_cap() {
        let data = [7u8; 100];
        let mut out = Vec::new();
        assert_eq!(copy_capped(&mut &data[..], &mut out, 100), Ok(100));
        assert_eq!(out.len(), 100);
        let mut out = Vec::new();
        assert_eq!(
            copy_capped(&mut &data[..], &mut out, 99),
            Err(CopyError::TooBig)
        );
        assert!(out.len() <= 100);
    }

    fn memfd_with(data: &[u8]) -> File {
        let mut f = new_memfd().unwrap();
        f.write_all(data).unwrap();
        f
    }

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const ABC_SHA1: &str = "a9993e364706816aba3e25717850c26c9cd0d89d";

    #[test]
    fn checksums_are_checked_on_the_memfd() {
        let f = memfd_with(b"abc");
        assert!(verify(&f, &Checksum::Sha256(ABC_SHA256.into())).is_ok());
        assert!(verify(&f, &Checksum::Sha1(ABC_SHA1.into())).is_ok());
        assert!(verify(&f, &Checksum::Sha256(ABC_SHA256.replace('b', "c"))).is_err());
        let g = memfd_with(b"abd");
        assert!(verify(&g, &Checksum::Sha256(ABC_SHA256.into())).is_err());
        assert!(verify(&g, &Checksum::Sha1(ABC_SHA1.into())).is_err());
    }

    #[test]
    fn a_sealed_file_cannot_change_and_reads_from_the_start() {
        let f = memfd_with(b"abc");
        let fd = seal(f).unwrap();
        let mut f = File::from(fd);
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        assert_eq!(s, "abc");
        assert!(f.write_all(b"x").is_err());
        // SAFETY: ftruncate on a descriptor we own.
        assert!(unsafe { libc::ftruncate(f.as_raw_fd(), 0) } < 0);
    }

    fn update(checksums: &[&str], locations: &[&str], size: u64) -> FirmwareUpdate {
        FirmwareUpdate {
            device_id: "abc".into(),
            device: "Dev".into(),
            vendor: String::new(),
            current: "1".into(),
            version: "2".into(),
            summary: String::new(),
            description: String::new(),
            urgency: Urgency::Unknown,
            size,
            checksums: strs(checksums),
            locations: strs(locations),
            remote_id: "r".into(),
            trusted: true,
            needs_reboot: false,
            needs_shutdown: false,
            internal: false,
        }
    }

    #[test]
    fn nothing_without_a_checksum_or_a_secure_address() {
        assert!(fetch(&update(&[], &["https://x.test/a.cab"], 3), None, None).is_err());
        assert!(
            fetch(
                &update(&[ABC_SHA256], &["http://x.test/a.cab"], 3),
                None,
                None
            )
            .is_err()
        );
        assert!(
            fetch(
                &update(&["not-hex"], &["https://x.test/a.cab"], 3),
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn the_developer_folder_serves_plain_names_only() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.cab"), b"abc").unwrap();
        let u = update(&[ABC_SHA256], &["./a.cab"], 3);
        let fd = fetch(&u, None, Some(d.path())).unwrap();
        let mut s = String::new();
        File::from(fd).read_to_string(&mut s).unwrap();
        assert_eq!(s, "abc");
        // wrong content and names that leave the folder
        std::fs::write(d.path().join("b.cab"), b"abd").unwrap();
        assert!(
            fetch(
                &update(&[ABC_SHA256], &["./b.cab"], 3),
                None,
                Some(d.path())
            )
            .is_err()
        );
        for bad in [
            "../a.cab",
            "./sub/a.cab",
            "/etc/passwd",
            "..",
            ".hidden",
            "a\\b",
        ] {
            assert!(
                fetch(&update(&[ABC_SHA256], &[bad], 3), None, Some(d.path())).is_err(),
                "{bad}"
            );
        }
        assert!(plain_file_name("a.cab").is_some());
    }

    #[test]
    fn the_folder_never_replaces_a_secure_address() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.cab"), b"abc").unwrap();
        // https://x.test/a.cab is resolvable, so it would be downloaded, not read locally:
        // no network here, so it must fail rather than read the folder.
        let u = update(&[ABC_SHA256], &["https://127.0.0.1:9/a.cab"], 3);
        assert!(fetch(&u, None, Some(d.path())).is_err());
    }

    #[test]
    fn age_notes() {
        assert!(age_note(None).contains("never"));
        assert_eq!(age_note(Some(Duration::from_secs(29 * 86_400))), "");
        assert!(age_note(Some(Duration::from_secs(45 * 86_400))).contains("45 days"));
    }

    #[test]
    fn the_shipped_fixture_parses() {
        let text = include_str!("../fixtures/firmware.json");
        let l = parse_fixture(text).unwrap();
        assert_eq!(l.updates.len(), 2);
        assert_eq!(
            l.updates
                .iter()
                .filter(|u| u.urgency >= Urgency::High)
                .count(),
            1
        );
        assert_eq!(l.pending.len(), 1);
        let v = view(&l, &HashSet::new());
        assert_eq!(v.updates.iter().filter(|u| u.important).count(), 1);
        assert_eq!(v.pending[0].state, "reboot");
        let shut: HashSet<String> = [v.pending[0].id.clone()].into();
        assert_eq!(view(&l, &shut).pending[0].state, "shutdown");
    }

    #[test]
    fn bad_fixtures_are_errors() {
        assert!(parse_fixture("nope").is_err());
        assert!(parse_fixture(r#"{"surprise":1}"#).is_err());
        assert!(
            parse_fixture(
                r#"{"pending":[{"device_id":"a","device":"d","version":"1","state":"x"}]}"#
            )
            .is_err()
        );
        let empty = parse_fixture("{}").unwrap();
        assert!(empty.updates.is_empty() && empty.metadata_age.is_none());
    }
}
