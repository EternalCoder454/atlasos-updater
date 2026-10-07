//! The fwupd client the window and the tray share (zbus only, no Qt).
//!
//! fwupd (`org.freedesktop.fwupd` on the system bus) asks polkit itself, so
//! nothing here is privileged. Everything it sends is untrusted text: it is
//! cleaned, capped and never trusted to have the type or the keys expected.
//! Firmware never installs by itself: [`install`] runs only when the user
//! asked for it, with a file the caller fetched and checked.
//!
//! The connection passed in must have no `method_timeout` (zbus's default):
//! an install can take minutes. The list calls bound themselves.

use std::collections::HashMap;
use std::future::Future;
use std::os::fd::OwnedFd;
use std::pin::Pin;
use std::task::Poll;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zbus::proxy::{CacheProperties, MethodFlags};
use zbus::zvariant::{Fd, OwnedValue, Value};
use zbus::{Connection, Proxy, fdo};

pub use crate::errors::OpError as Error;
pub type Result<T> = std::result::Result<T, Error>;

const DEST: &str = "org.freedesktop.fwupd";
const IFACE: &str = "org.freedesktop.fwupd";
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest an Install call is waited for.
const INSTALL_LIMIT: Duration = Duration::from_secs(60 * 60);
const INSTALL_UNKNOWN: &str = "The firmware service has not finished after an hour, so the result is unknown. Check the device's firmware version before trying again.";

/// Most devices read, and so most releases (one per device).
pub const MAX_DEVICES: usize = 256;
const MAX_LOCATIONS: usize = 8;
const MAX_CHECKSUMS: usize = 8;
const MAX_URL: usize = 2048;
const NAME_MAX: usize = 120;
const VERSION_MAX: usize = 64;
const SUMMARY_MAX: usize = 300;
/// Longest description, in characters.
pub const DESCRIPTION_MAX: usize = 2000;
const MARKUP_MAX: usize = 64 * 1024;
/// A tag that does not close within this many bytes is literal text.
const TAG_MAX: usize = 256;
/// An entity's `;` must come within this many bytes of the `&`.
const ENTITY_MAX: usize = 10;
const ERROR_MAX: usize = 300;

// Feature flags (fwupd-enums-struct.h): detach-action 1, update-action 2,
// requests 4, allow-authentication 8, requests-non-generic 9 (bit numbers).
const FEATURES: u64 = (1 << 1) | (1 << 2) | (1 << 4) | (1 << 8) | (1 << 9);

// Device flags (bit numbers).
const DEV_INTERNAL: u32 = 0;
const DEV_UPDATABLE: u32 = 1;
const DEV_LOCKED: u32 = 4;
const DEV_NEEDS_REBOOT: u32 = 8;
const DEV_NEEDS_SHUTDOWN: u32 = 17;
const DEV_UPDATABLE_HIDDEN: u32 = 37;
// Release flags (bit numbers).
const REL_TRUSTED_PAYLOAD: u32 = 0;
const REL_TRUSTED_METADATA: u32 = 1;
const REL_IS_UPGRADE: u32 = 2;
const REL_BLOCKED_VERSION: u32 = 4;
const REL_BLOCKED_APPROVAL: u32 = 5;
// Update states.
const STATE_PENDING: u64 = 1;
const STATE_FAILED: u64 = 3;
const STATE_NEEDS_REBOOT: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

impl Urgency {
    fn from_u64(n: u64) -> Self {
        match n {
            1 => Self::Low,
            2 => Self::Medium,
            3 => Self::High,
            4 => Self::Critical,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareUpdate {
    pub device_id: String,
    /// The device's name.
    pub device: String,
    pub vendor: String,
    pub current: String,
    pub version: String,
    pub summary: String,
    /// Plain text (see [`description_text`]).
    pub description: String,
    pub urgency: Urgency,
    pub size: u64,
    /// Valid lower-case hex SHA-1 / SHA-256 only.
    pub checksums: Vec<String>,
    /// Not yet resolved: see [`resolve_location`].
    pub locations: Vec<String>,
    pub remote_id: String,
    /// fwupd trusts the release: its payload or its metadata is signed by a
    /// trusted key. fwupd checks the payload itself again when it installs.
    pub trusted: bool,
    pub needs_reboot: bool,
    pub needs_shutdown: bool,
    pub internal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingState {
    /// Installed, finishes at the next restart.
    Reboot,
    /// The last attempt failed, with fwupd's reason.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub device_id: String,
    pub device: String,
    pub version: String,
    pub state: PendingState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    pub updates: Vec<FirmwareUpdate>,
    pub pending: Vec<Pending>,
    /// Age of the newest metadata of an enabled download remote.
    pub metadata_age: Option<Duration>,
}

/// One step of a running install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// A short word or phrase for fwupd's status ("Writing").
    pub status: &'static str,
    /// 0 to 100 once fwupd reports one.
    pub percent: Option<u8>,
    /// Something the user has to do now, in words.
    pub request: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Done {
    pub needs_reboot: bool,
    pub needs_shutdown: bool,
}

/// The checksum to verify a download against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checksum {
    Sha256(String),
    Sha1(String),
}

impl Checksum {
    pub fn hex(&self) -> &str {
        match self {
            Self::Sha256(h) | Self::Sha1(h) => h,
        }
    }
}

// ---------------------------------------------------------------- text

fn hidden(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{0890}'..='\u{0891}' | '\u{08E2}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{110BD}' | '\u{110CD}'
        | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0001}' | '\u{E0020}'..='\u{E007F}'
        | '\u{034F}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{2800}'
        | '\u{3164}' | '\u{FFA0}')
}

fn combining(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}' | '\u{0483}'..='\u{0489}' | '\u{0591}'..='\u{05C7}'
        | '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}'
        | '\u{06D6}'..='\u{06ED}' | '\u{0900}'..='\u{0903}' | '\u{093A}'..='\u{094F}'
        | '\u{0E31}' | '\u{0E34}'..='\u{0E3A}' | '\u{0E47}'..='\u{0E4E}'
        | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// Remote text made safe to show: control characters become spaces,
/// invisible and direction-changing ones go, at most `max` characters
/// (same rules as telamon_framework_flatpak's `clean_to`, which this crate
/// cannot link: it would pull libflatpak into the tray).
pub fn clean_to(s: &str, max: usize) -> String {
    clean_impl(s, max, false)
}

fn clean_impl(s: &str, max: usize, keep_newlines: bool) -> String {
    let mut marks = 0;
    let s: String = s
        .chars()
        .filter(|c| !hidden(*c))
        .filter(|c| {
            marks = if combining(*c) { marks + 1 } else { 0 };
            marks <= 3
        })
        .map(|c| {
            if c == '\n' && keep_newlines {
                c
            } else if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let s = s.trim();
    if s.chars().count() > max {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        s.to_string()
    }
}

/// AppStream markup (`<p>`, `<ul>`, `<ol>`, `<li>`, `<em>`, `<code>`) as
/// plain text: paragraphs separated by a blank line, list items on their
/// own lines ("• " or "1. "), every other tag dropped, entities decoded.
/// Malformed input gives its text, never a panic. At most
/// [`DESCRIPTION_MAX`] characters.
pub fn description_text(markup: &str) -> String {
    let markup: String = markup.chars().take(MARKUP_MAX).collect();
    let mut out = String::new();
    let mut word_gap = false; // whitespace seen since the last text
    let mut lists: Vec<Option<u32>> = Vec::new(); // None: bullets, Some(n): numbered
    // Breaks pending before the next text: 0 none, 1 newline, 2 blank line.
    let mut brk = 0u8;
    let mut item_start: Option<String> = None;
    let mut chars = markup.char_indices().peekable();
    let rest = |i: usize| &markup[i..];
    let text = |t: &str,
                out: &mut String,
                word_gap: &mut bool,
                brk: &mut u8,
                item: &mut Option<String>| {
        for c in t.chars() {
            if c.is_whitespace() || c.is_control() {
                *word_gap = true;
                continue;
            }
            if !out.is_empty() && *brk > 0 {
                for _ in 0..*brk {
                    out.push('\n');
                }
                *word_gap = false;
            } else if *word_gap && !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            *brk = 0;
            *word_gap = false;
            if let Some(m) = item.take() {
                out.push_str(&m);
            }
            out.push(c);
        }
    };
    while let Some((i, c)) = chars.next() {
        if c == '<' {
            // A tag runs to the next '>'; none (or a '<' first) means a
            // literal '<'.
            let tail = rest(i + 1);
            // Bounded: no tag is longer than TAG_MAX bytes.
            let window = &tail.as_bytes()[..tail.len().min(TAG_MAX)];
            let close = window.iter().position(|b| *b == b'>');
            let reopen = window.iter().position(|b| *b == b'<');
            let starts_tag = tail.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/');
            if let Some(end) = close.filter(|e| starts_tag && reopen.is_none_or(|r| *e < r)) {
                let tag = &tail[..end];
                let tag = tag.trim_start_matches('/').trim();
                let closing = tail.starts_with('/');
                let name: String = tag
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                match (name.as_str(), closing) {
                    ("p", _) => brk = brk.max(2),
                    ("ul", false) => {
                        lists.push(None);
                        brk = brk.max(if lists.len() > 1 { 1 } else { 2 });
                    }
                    ("ol", false) => {
                        lists.push(Some(0));
                        brk = brk.max(if lists.len() > 1 { 1 } else { 2 });
                    }
                    ("ul" | "ol", true) => {
                        lists.pop();
                        brk = brk.max(2);
                    }
                    ("li", false) => {
                        brk = brk.max(1);
                        let marker = match lists.last_mut() {
                            Some(Some(n)) => {
                                *n = n.saturating_add(1);
                                format!("{n}. ")
                            }
                            _ => "• ".to_string(),
                        };
                        item_start = Some(marker);
                    }
                    ("li", true) => brk = brk.max(1),
                    ("br", _) => brk = brk.max(1),
                    _ => {}
                }
                // skip past the '>'
                let skip_to = i + 1 + end + 1;
                while chars.peek().is_some_and(|(j, _)| *j < skip_to) {
                    chars.next();
                }
                continue;
            }
            text("<", &mut out, &mut word_gap, &mut brk, &mut item_start);
        } else if c == '&' {
            let tail = rest(i + 1);
            if let Some((ch, len)) = entity(tail) {
                let mut b = [0u8; 4];
                text(
                    ch.encode_utf8(&mut b),
                    &mut out,
                    &mut word_gap,
                    &mut brk,
                    &mut item_start,
                );
                let skip_to = i + 1 + len;
                while chars.peek().is_some_and(|(j, _)| *j < skip_to) {
                    chars.next();
                }
            } else {
                text("&", &mut out, &mut word_gap, &mut brk, &mut item_start);
            }
        } else {
            let mut b = [0u8; 4];
            text(
                c.encode_utf8(&mut b),
                &mut out,
                &mut word_gap,
                &mut brk,
                &mut item_start,
            );
        }
    }
    clean_impl(&out, DESCRIPTION_MAX, true)
}

/// `amp;`, `#38;`, `#x26;` at the start of `s`: the character and how many
/// bytes of `s` it used (up to the `;`).
fn entity(s: &str) -> Option<(char, usize)> {
    let window = &s.as_bytes()[..s.len().min(ENTITY_MAX + 1)];
    let end = window
        .iter()
        .position(|b| *b == b';')
        .filter(|e| *e <= 10)?;
    let body = &s[..end];
    let ch = match body {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            let num = body.strip_prefix('#')?;
            let n = if let Some(h) = num.strip_prefix(['x', 'X']) {
                u32::from_str_radix(h, 16).ok()?
            } else {
                num.parse::<u32>().ok()?
            };
            char::from_u32(n)?
        }
    };
    Some((ch, end + 1))
}

// ------------------------------------------------- checksums and URLs

fn is_hex(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The checksum to verify against: SHA-256 if given, else SHA-1, else none.
/// Entries that are not hex of those lengths are ignored.
pub fn pick_checksum(checksums: &[String]) -> Option<Checksum> {
    let find = |len: usize| {
        checksums
            .iter()
            .map(|c| c.trim())
            .find(|c| c.len() == len && is_hex(c))
            .map(|c| c.to_ascii_lowercase())
    };
    find(64)
        .map(Checksum::Sha256)
        .or_else(|| find(40).map(Checksum::Sha1))
}

/// The SHA-256 (hex) among `checksums`, if any.
pub fn sha256_of(checksums: &[String]) -> Option<String> {
    match pick_checksum(checksums) {
        Some(Checksum::Sha256(h)) => Some(h),
        _ => None,
    }
}

/// A host[:port] authority that is safe to fetch from.
fn valid_authority(a: &str) -> bool {
    let (host, port) = match a.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (a, None),
    };
    if let Some(p) = port
        && (p.is_empty()
            || p.len() > 5
            || !p.bytes().all(|b| b.is_ascii_digit())
            || p.parse::<u32>().is_ok_and(|n| n > 65535))
    {
        return false;
    }
    if host.is_empty()
        || host.len() > 253
        || host.starts_with(['.', '-'])
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return false;
    }
    let lower = host.to_ascii_lowercase();
    let lower = lower.trim_end_matches('.');
    if lower == "localhost" || lower.ends_with(".localhost") {
        return false;
    }
    // A numeric last label is an address in some spelling (127.1, a bare
    // decimal, 0x7f...): only a plain public dotted quad is allowed.
    let last = lower.rsplit('.').next().unwrap_or("");
    if last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x") {
        return match lower.parse::<std::net::Ipv4Addr>() {
            Ok(ip) => public_v4(ip),
            Err(_) => false,
        };
    }
    true
}

fn public_v4(ip: std::net::Ipv4Addr) -> bool {
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.octets()[0] == 0
        // 100.64.0.0/10 (carrier-grade NAT)
        || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xC0) == 0x40))
}

/// A path (or relative reference) with no way out of its folder.
fn safe_path(p: &str) -> bool {
    let lower = p.to_ascii_lowercase();
    !lower.contains("%2e")
        && !lower.contains("%2f")
        && !lower.contains("%5c")
        && !lower.contains("%00")
        && !p.contains('#')
        && p.split(['/', '?']).all(|seg| seg != "..")
}

fn plain_ascii(s: &str) -> bool {
    s.len() <= MAX_URL && s.bytes().all(|b| (0x21..0x7f).contains(&b)) && !s.contains('\\')
}

/// Splits `https://authority/path` after checking the scheme.
fn https_parts(u: &str) -> Option<(&str, &str)> {
    u.get(..8).filter(|s| s.eq_ignore_ascii_case("https://"))?;
    let rest = &u[8..];
    let at = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (auth, path) = rest.split_at(at);
    valid_authority(auth).then_some((auth, path))
}

/// The URL a release's `location` means, or `None`: absolute `https://`
/// only. A relative location (`./name.cab`) is joined to
/// `firmware_base_uri`, which must itself be `https://`. Rejected:
/// `http`, `file`, `data` and any other scheme, user info (`@`), `..`
/// segments (also percent-encoded), fragments, white space, non-ASCII.
pub fn resolve_location(location: &str, firmware_base_uri: Option<&str>) -> Option<String> {
    let loc = location.trim();
    if loc.is_empty() || !plain_ascii(loc) || loc.starts_with("//") || loc.starts_with('/') {
        return None;
    }
    let colon = loc.find(':');
    let slash = loc.find(['/', '?']);
    if colon.is_some_and(|c| slash.is_none_or(|s| c < s)) {
        // a scheme: only https
        let (auth, path) = https_parts(loc)?;
        if !safe_path(path) {
            return None;
        }
        return Some(format!("https://{auth}{path}"));
    }
    let base = firmware_base_uri?.trim();
    if !plain_ascii(base) || base.contains(['?', '#']) {
        return None;
    }
    let (auth, bpath) = https_parts(base)?;
    if !safe_path(bpath) {
        return None;
    }
    let rel = loc.strip_prefix("./").unwrap_or(loc);
    if rel.is_empty() || rel.starts_with('/') {
        return None;
    }
    if !safe_path(rel) || rel.split('/').any(|s| s == ".") {
        return None;
    }
    Some(format!(
        "https://{auth}{}/{rel}",
        bpath.trim_end_matches('/')
    ))
}

// ---------------------------------------------------------- notice key

/// A key for a set of updates that is the same for the same
/// (device, version) pairs in any order, and changes when one changes.
/// Empty for no updates. FNV-1a over the sorted pairs: stable across runs
/// and Rust versions, so it can be kept in a settings file.
pub fn notice_key(updates: &[FirmwareUpdate]) -> String {
    if updates.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(&str, &str)> = updates
        .iter()
        .map(|u| (u.device_id.as_str(), u.version.as_str()))
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for (id, v) in pairs {
        id.bytes().for_each(&mut eat);
        eat(0);
        v.bytes().for_each(&mut eat);
        eat(1);
    }
    format!("{h:016x}")
}

// ------------------------------------------------------------- errors

/// fwupd's D-Bus error `name` and `message`, in plain words.
fn fwupd_error(name: &str, message: &str) -> Error {
    let short = name.strip_prefix("org.freedesktop.fwupd.").unwrap_or(name);
    let say = |s: &str| Error::Message(s.to_string());
    match short {
        "AuthFailed" | "PermissionDenied" => Error::Cancelled,
        "AuthExpired" => say("The permission to update firmware ran out. Try again."),
        "BatteryLevelTooLow" => say(
            "The battery is too low to update firmware safely. Plug in the charger and try again.",
        ),
        "AcPowerRequired" => say("Plug in the charger to update this device, then try again."),
        "NeedsUserAction" => say(
            "The device needs something from you first (such as unplugging it or pressing a button). Follow its instructions and try again.",
        ),
        "NothingToDo" => say("There is nothing to update for this device."),
        "NotSupported" => say("This device can't be updated from here."),
        "AlreadyPending" => {
            say("An update for this device is already waiting for a restart. Restart to finish it.")
        }
        "Busy" => say("The device or the firmware service is busy. Try again in a moment."),
        "SignatureInvalid" => say(
            "The firmware's signature couldn't be verified, so it wasn't installed. Nothing was changed.",
        ),
        "VersionSame" => say("This firmware version is already installed."),
        "VersionNewer" => say("A newer firmware version is already installed."),
        "TimedOut" => say("The device did not answer in time. Try again."),
        // fwupd was told to stop (memory pressure, an upgrade of fwupd, a
        // shutdown) during the install; it finishes the install first, then
        // answers with this instead of the install's own result.
        "Internal" if message == "daemon was stopped" => say(
            "The firmware service was stopped during the update, so its result is unknown. Check the device's firmware version before trying again.",
        ),
        _ => {
            let m = clean_to(message, ERROR_MAX);
            if m.is_empty() {
                Error::Message(format!(
                    "The firmware update failed ({}).",
                    clean_to(short, 40)
                ))
            } else {
                Error::Message(m)
            }
        }
    }
}

const NO_SERVICE: &str = "Can't reach the firmware service (fwupd). It may be missing or stopped.";
const TOO_SLOW: &str = "The firmware service did not answer in time. Try again.";

fn map_error(e: &zbus::Error) -> Error {
    match e {
        zbus::Error::MethodError(name, desc, _) => {
            fwupd_error(name.as_str(), desc.as_deref().unwrap_or(""))
        }
        zbus::Error::FDO(f) => match **f {
            fdo::Error::ServiceUnknown(_) | fdo::Error::NameHasNoOwner(_) => {
                Error::Message(NO_SERVICE.into())
            }
            fdo::Error::NoReply(_) | fdo::Error::Timeout(_) | fdo::Error::TimedOut(_) => {
                Error::Message(TOO_SLOW.into())
            }
            fdo::Error::AccessDenied(_) => Error::Message(
                "The system did not allow this account to talk to the firmware service.".into(),
            ),
            _ => Error::Message(NO_SERVICE.into()),
        },
        zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            Error::Message(TOO_SLOW.into())
        }
        _ => Error::Message(NO_SERVICE.into()),
    }
}

/// True for fwupd's "no update for this device" answers.
fn no_update(e: &zbus::Error) -> bool {
    matches!(e, zbus::Error::MethodError(n, _, _)
        if matches!(n.as_str(), "org.freedesktop.fwupd.NothingToDo" | "org.freedesktop.fwupd.NotSupported"))
}

/// A fwupd-level refusal for one device (not a broken connection).
fn is_method_error(e: &zbus::Error) -> bool {
    matches!(e, zbus::Error::MethodError(..))
}

// ---------------------------------------------------- reading answers

type Dict = HashMap<String, OwnedValue>;

fn val<'a>(d: &'a Dict, k: &str) -> Option<&'a Value<'static>> {
    let mut v: &Value<'static> = d.get(k)?;
    for _ in 0..2 {
        if let Value::Value(inner) = v {
            v = inner;
        }
    }
    Some(v)
}

fn get_u64(d: &Dict, k: &str) -> Option<u64> {
    match val(d, k)? {
        Value::U8(n) => Some(u64::from(*n)),
        Value::U16(n) => Some(u64::from(*n)),
        Value::U32(n) => Some(u64::from(*n)),
        Value::U64(n) => Some(*n),
        Value::I16(n) => u64::try_from(*n).ok(),
        Value::I32(n) => u64::try_from(*n).ok(),
        Value::I64(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

fn get_str<'a>(d: &'a Dict, k: &str) -> Option<&'a str> {
    match val(d, k)? {
        Value::Str(s) => Some(s.as_str()),
        _ => None,
    }
}

fn text(d: &Dict, k: &str, max: usize) -> String {
    get_str(d, k).map(|s| clean_to(s, max)).unwrap_or_default()
}

fn get_strs(d: &Dict, k: &str, cap: usize) -> Vec<String> {
    match val(d, k) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::Str(s) => Some(s.as_str().to_string()),
                _ => None,
            })
            .take(cap)
            .collect(),
        _ => Vec::new(),
    }
}

fn bit(flags: u64, n: u32) -> bool {
    flags >> n & 1 == 1
}

/// A valid fwupd device id (40 hex characters in practice).
fn valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Debug, Clone)]
struct Device {
    id: String,
    name: String,
    vendor: String,
    version: String,
    flags: u64,
    state: u64,
    error: String,
}

impl Device {
    fn parse(d: &Dict) -> Option<Device> {
        let id = get_str(d, "DeviceId")?;
        if !valid_device_id(id) {
            return None;
        }
        let name = text(d, "Name", NAME_MAX);
        Some(Device {
            id: id.to_string(),
            name: if name.is_empty() {
                "Unnamed device".into()
            } else {
                name
            },
            vendor: text(d, "Vendor", NAME_MAX),
            version: text(d, "Version", VERSION_MAX),
            flags: get_u64(d, "Flags").unwrap_or(0),
            state: get_u64(d, "UpdateState").unwrap_or(0),
            error: text(d, "UpdateError", ERROR_MAX),
        })
    }

    /// Worth asking `GetUpgrades` about.
    fn updatable(&self) -> bool {
        bit(self.flags, DEV_UPDATABLE)
            && !bit(self.flags, DEV_UPDATABLE_HIDDEN)
            && !bit(self.flags, DEV_LOCKED)
    }

    fn waiting_for_reboot(&self) -> bool {
        self.state == STATE_PENDING || self.state == STATE_NEEDS_REBOOT
    }

    fn pending(&self) -> Option<Pending> {
        let state = if self.waiting_for_reboot() {
            PendingState::Reboot
        } else if self.state == STATE_FAILED {
            PendingState::Failed(if self.error.is_empty() {
                "The last update did not work.".into()
            } else {
                self.error.clone()
            })
        } else {
            return None;
        };
        Some(Pending {
            device_id: self.id.clone(),
            device: self.name.clone(),
            version: self.version.clone(),
            state,
        })
    }
}

/// The newest release for `dev` from one `GetUpgrades` row, if it is a
/// usable upgrade.
fn parse_release(dev: &Device, r: &Dict) -> Option<FirmwareUpdate> {
    let flags = get_u64(r, "TrustFlags")
        .or_else(|| get_u64(r, "Flags"))
        .unwrap_or(0);
    if !bit(flags, REL_IS_UPGRADE)
        || bit(flags, REL_BLOCKED_VERSION)
        || bit(flags, REL_BLOCKED_APPROVAL)
    {
        return None;
    }
    let version = text(r, "Version", VERSION_MAX);
    if version.is_empty() {
        return None;
    }
    let checksums: Vec<String> = get_str(r, "Checksum")
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|c| matches!(c.len(), 40 | 64) && is_hex(c))
        .map(str::to_ascii_lowercase)
        .take(MAX_CHECKSUMS)
        .collect();
    let locations = get_strs(r, "Locations", MAX_LOCATIONS)
        .into_iter()
        .filter(|l| l.len() <= MAX_URL)
        .collect();
    let vendor = {
        let v = text(r, "Vendor", NAME_MAX);
        if dev.vendor.is_empty() {
            v
        } else {
            dev.vendor.clone()
        }
    };
    Some(FirmwareUpdate {
        device_id: dev.id.clone(),
        device: dev.name.clone(),
        vendor,
        current: dev.version.clone(),
        version,
        summary: text(r, "Summary", SUMMARY_MAX),
        description: description_text(get_str(r, "Description").unwrap_or("")),
        urgency: Urgency::from_u64(get_u64(r, "Urgency").unwrap_or(0)),
        size: get_u64(r, "Size").unwrap_or(0),
        checksums,
        locations,
        remote_id: text(r, "RemoteId", NAME_MAX),
        trusted: bit(flags, REL_TRUSTED_PAYLOAD) || bit(flags, REL_TRUSTED_METADATA),
        needs_reboot: bit(dev.flags, DEV_NEEDS_REBOOT),
        needs_shutdown: bit(dev.flags, DEV_NEEDS_SHUTDOWN),
        internal: bit(dev.flags, DEV_INTERNAL),
    })
}

/// Age of the newest metadata among enabled download remotes.
fn metadata_age(remotes: &[Dict], now: SystemTime) -> Option<Duration> {
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    remotes
        .iter()
        .filter(|r| matches!(val(r, "Enabled"), Some(Value::Bool(true))))
        .filter(|r| get_u64(r, "Type") == Some(1))
        .filter_map(|r| get_u64(r, "ModificationTime"))
        .filter(|t| *t != u64::MAX && *t > 0)
        .max()
        .map(|t| Duration::from_secs(now.saturating_sub(t)))
}

// ------------------------------------------------------------- D-Bus

async fn fwupd_proxy(conn: &Connection) -> Result<Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(DEST)
        .and_then(|b| b.path("/"))
        .and_then(|b| b.interface(IFACE))
        .map(|b| b.cache_properties(CacheProperties::No))
        .map_err(|e| map_error(&e))?
        .build()
        .await
        .map_err(|e| map_error(&e))
}

/// A call bounded by [`LIST_TIMEOUT`]; the raw zbus error stays visible.
async fn bounded<T>(
    f: impl Future<Output = zbus::Result<T>>,
) -> std::result::Result<T, zbus::Error> {
    match tokio::time::timeout(LIST_TIMEOUT, f).await {
        Ok(r) => r,
        Err(_) => Err(zbus::Error::InputOutput(
            std::io::Error::from(std::io::ErrorKind::TimedOut).into(),
        )),
    }
}

async fn get_devices(p: &Proxy<'_>) -> Result<Vec<Device>> {
    let rows: Vec<Dict> = bounded(p.call("GetDevices", &()))
        .await
        .map_err(|e| map_error(&e))?;
    Ok(rows
        .iter()
        .take(MAX_DEVICES)
        .filter_map(Device::parse)
        .collect())
}

/// The newest release for `dev`: `Ok(None)` for no update.
async fn newest(
    p: &Proxy<'_>,
    dev: &Device,
) -> std::result::Result<Option<FirmwareUpdate>, zbus::Error> {
    let rows: Vec<Dict> = match bounded(p.call("GetUpgrades", &(dev.id.as_str(),))).await {
        Ok(r) => r,
        Err(e) if no_update(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(rows.first().and_then(|r| parse_release(dev, r)))
}

/// Whether fwupd is running or can be started by D-Bus activation.
pub async fn available(conn: &Connection) -> Result<bool> {
    let bus = fdo::DBusProxy::new(conn).await.map_err(|e| map_error(&e))?;
    let name: zbus::names::BusName<'_> = DEST
        .try_into()
        .map_err(|e| map_error(&zbus::Error::from(e)))?;
    if bounded(async { bus.name_has_owner(name).await.map_err(zbus::Error::from) })
        .await
        .map_err(|e| map_error(&e))?
    {
        return Ok(true);
    }
    let names = bounded(async {
        bus.list_activatable_names()
            .await
            .map_err(zbus::Error::from)
    })
    .await
    .map_err(|e| map_error(&e))?;
    Ok(names.iter().any(|n| n.as_str() == DEST))
}

/// Firmware updates, updates waiting for a restart, and how old the
/// metadata is. Nothing is installed or changed.
pub async fn list(conn: &Connection) -> Result<Listing> {
    let p = fwupd_proxy(conn).await?;
    let devices = get_devices(&p).await?;
    let mut out = Listing::default();
    for dev in &devices {
        if let Some(pending) = dev.pending() {
            out.pending.push(pending);
        }
        if dev.waiting_for_reboot() || !dev.updatable() {
            continue;
        }
        match newest(&p, dev).await {
            Ok(Some(u)) => out.updates.push(u),
            Ok(None) => {}
            // fwupd refusing for one device must not hide the others
            Err(e) if is_method_error(&e) => {}
            Err(e) => return Err(map_error(&e)),
        }
    }
    // Extra information: a failure here is "unknown", not a failed check.
    if let Ok(remotes) = bounded(p.call::<_, _, Vec<Dict>>("GetRemotes", &())).await {
        out.metadata_age = metadata_age(&remotes, SystemTime::now());
    }
    Ok(out)
}

/// The newest release for one device, read again (at install time).
pub async fn upgrade_for(conn: &Connection, device_id: &str) -> Result<Option<FirmwareUpdate>> {
    if !valid_device_id(device_id) {
        return Err(Error::Message("That is not a valid device.".into()));
    }
    let p = fwupd_proxy(conn).await?;
    let devices = get_devices(&p).await?;
    let Some(dev) = devices.iter().find(|d| d.id == device_id) else {
        return Ok(None);
    };
    if dev.waiting_for_reboot() || !dev.updatable() {
        return Ok(None);
    }
    newest(&p, dev).await.map_err(|e| map_error(&e))
}

/// fwupd's `Status` number as a short phrase.
pub fn status_word(n: u32) -> &'static str {
    match n {
        1 => "Working",
        2 => "Reading the firmware",
        3 => "Unpacking",
        4 => "Restarting the device",
        5 => "Writing",
        6 => "Verifying",
        7 => "Scheduling the update",
        8 => "Downloading",
        9 => "Reading the device",
        10 => "Erasing",
        11 => "Waiting for your password",
        12 => "Device busy",
        13 => "Shutting down",
        14 => "Waiting for you",
        _ => "Working",
    }
}

/// A `DeviceRequest` said plainly: its message, else its id in words.
fn request_text(d: &Dict, device_id: &str) -> Option<String> {
    if let Some(dev) = get_str(d, "DeviceId")
        && dev != device_id
    {
        return None;
    }
    let m = text(d, "Message", 300);
    if !m.is_empty() {
        return Some(m);
    }
    let id = get_str(d, "Id")?;
    Some(
        match id.strip_prefix("org.freedesktop.fwupd.").unwrap_or(id) {
            "request.remove-replug" => "Unplug the device and plug it in again.".to_string(),
            "request.press-unlock" => "Press the unlock button on the device.".to_string(),
            "request.remove-usb-cable" => "Unplug the USB cable.".to_string(),
            "request.insert-usb-cable" => "Plug in the USB cable.".to_string(),
            "request.do-not-power-off" => "Do not turn off the computer.".to_string(),
            "replug-install" => "Unplug the device and plug it in again.".to_string(),
            "replug-power" => "Unplug the power cable and plug it in again.".to_string(),
            "restart-daemon" => "The firmware service needs to restart.".to_string(),
            other => {
                let w = clean_to(other.rsplit('.').next().unwrap_or(other), 60)
                    .replace(['-', '_'], " ");
                if w.is_empty() {
                    return None;
                }
                format!("The device asks you to: {w}.")
            }
        },
    )
}

/// Installs the firmware cabinet behind `fd` on `device_id`, which the user
/// asked for. `fd` must be the already verified file. `progress` is called
/// from this task for each status, percentage and request.
///
/// No method timeout applies (an install can take minutes); it returns an
/// error if fwupd leaves the bus or the connection closes. The call allows
/// interactive authorization, so polkit may prompt.
pub async fn install(
    conn: &Connection,
    device_id: &str,
    fd: OwnedFd,
    mut progress: impl FnMut(Progress),
) -> Result<Done> {
    if !valid_device_id(device_id) {
        return Err(Error::Message("That is not a valid device.".into()));
    }
    let p = fwupd_proxy(conn).await?;
    bounded(p.call::<_, _, ()>("SetFeatureFlags", &(FEATURES,)))
        .await
        .map_err(|e| map_error(&e))?;

    // Subscribed before the call so no early event is missed. Progress is
    // extra: failing to follow it does not stop the install.
    let build = async {
        let props = fdo::PropertiesProxy::builder(conn)
            .destination(DEST)?
            .path("/")?
            .build()
            .await?;
        let props_s = props.receive_properties_changed().await?;
        zbus::Result::Ok(props_s)
    };
    let mut props_s = build.await.ok();
    let mut req_s = p.receive_signal("DeviceRequest").await.ok();
    let bus = fdo::DBusProxy::new(conn).await.map_err(|e| map_error(&e))?;
    let mut owner_s = bus
        .receive_name_owner_changed_with_args(&[(0, DEST)])
        .await
        .map_err(|e| map_error(&e))?;

    let opts: HashMap<&str, Value<'_>> = HashMap::new();
    let call_fd = Fd::from(&fd);
    let body = (device_id, call_fd, opts);
    let mut call = Box::pin(p.call_with_flags::<_, _, ()>(
        "Install",
        MethodFlags::AllowInteractiveAuth.into(),
        &body,
    ));

    enum Ev {
        Props(Option<(u32, bool)>, Option<u8>),
        Request(Option<String>),
        Gone,
        TimedOut,
        Ended(zbus::Result<Option<()>>),
    }
    let deadline = tokio::time::sleep(INSTALL_LIMIT);
    tokio::pin!(deadline);
    let mut status = 0u32;
    let mut percent: Option<u8> = None;
    let result = loop {
        let ev = std::future::poll_fn(|cx| {
            use zbus::export::futures_core::Stream;
            if let Some(s) = props_s.as_mut() {
                match Pin::new(s).poll_next(cx) {
                    Poll::Ready(Some(sig)) => {
                        let mut st = None;
                        let mut pc = None;
                        if let Ok(args) = sig.args()
                            && args.interface_name().as_str() == IFACE
                        {
                            for (k, v) in args.changed_properties() {
                                match (*k, v) {
                                    ("Status", Value::U32(n)) => st = Some((*n, true)),
                                    ("Percentage", Value::U32(n)) => {
                                        pc = Some(
                                            u8::try_from(*n)
                                                .ok()
                                                .filter(|n| *n <= 100)
                                                .unwrap_or(255),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                        return Poll::Ready(Ev::Props(st, pc));
                    }
                    Poll::Ready(None) => props_s = None,
                    Poll::Pending => {}
                }
            }
            if let Some(s) = req_s.as_mut() {
                match Pin::new(s).poll_next(cx) {
                    Poll::Ready(Some(msg)) => {
                        let body = msg.body();
                        let t = body
                            .deserialize::<Dict>()
                            .ok()
                            .and_then(|d| request_text(&d, device_id));
                        return Poll::Ready(Ev::Request(t));
                    }
                    Poll::Ready(None) => req_s = None,
                    Poll::Pending => {}
                }
            }
            // Drain the owner stream until it is Pending, so its waker is
            // registered and no change is left unread.
            let mut gone = false;
            loop {
                match Pin::new(&mut owner_s).poll_next(cx) {
                    Poll::Ready(Some(sig)) => {
                        if sig.args().is_ok_and(|a| a.new_owner().is_none()) {
                            gone = true;
                            break;
                        }
                    }
                    Poll::Ready(None) => {
                        gone = true;
                        break;
                    }
                    Poll::Pending => break,
                }
            }
            // A reply that is already queued wins over "service stopped".
            match call.as_mut().poll(cx) {
                Poll::Ready(r) => Poll::Ready(Ev::Ended(r)),
                Poll::Pending if gone => Poll::Ready(Ev::Gone),
                Poll::Pending if deadline.as_mut().poll(cx).is_ready() => Poll::Ready(Ev::TimedOut),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
        match ev {
            Ev::Props(st, pc) => {
                let mut changed = false;
                if let Some((n, _)) = st {
                    changed |= n != status;
                    status = n;
                }
                if let Some(pc) = pc {
                    let new = (pc <= 100).then_some(pc);
                    changed |= new != percent;
                    percent = new;
                }
                if changed {
                    progress(Progress {
                        status: status_word(status),
                        percent,
                        request: None,
                    });
                }
            }
            Ev::Request(Some(text)) => progress(Progress {
                status: status_word(status),
                percent,
                request: Some(text),
            }),
            Ev::Request(None) => {}
            Ev::Gone => {
                break Err(Error::Message(
                    "The firmware service stopped while the update was running. Check the device before trying again."
                        .into(),
                ))
            }
            Ev::TimedOut => {
                eprintln!("atlas-updater: fwupd had not answered Install after an hour");
                break Err(Error::Message(INSTALL_UNKNOWN.into()));
            }
            Ev::Ended(r) => break r.map_err(|e| map_error(&e)),
        }
    };
    drop(call);
    drop(fd);
    result?;

    // What the update asks next. Failing to read it is not a failed update.
    let mut done = Done::default();
    if let Ok(devs) = get_devices(&p).await
        && let Some(d) = devs.iter().find(|d| d.id == device_id)
    {
        done.needs_reboot = bit(d.flags, DEV_NEEDS_REBOOT) || d.waiting_for_reboot();
        done.needs_shutdown = bit(d.flags, DEV_NEEDS_SHUTDOWN);
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: Vec<(&str, Value<'static>)>) -> Dict {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), OwnedValue::try_from(v).unwrap()))
            .collect()
    }

    const ID: &str = "08d460be0f1f9f128413f816022a6439e0078018";

    fn webcam() -> Dict {
        dict(vec![
            ("DeviceId", Value::from(ID)),
            ("Name", Value::from("Integrated Webcam™")),
            ("Vendor", Value::from("ACME Corp.")),
            ("Flags", Value::U64(4785074704744482)),
            ("Version", Value::from("1.2.2")),
        ])
    }

    fn release() -> Dict {
        dict(vec![
            ("RemoteId", Value::from("fwupd-tests")),
            ("Size", Value::U64(17)),
            (
                "Summary",
                Value::from("Firmware for the ACME Corp Integrated Webcam"),
            ),
            (
                "Description",
                Value::from("<p>Fixes another bug with the flux capacitor.</p>"),
            ),
            (
                "Checksum",
                Value::from(
                    "db63711564cacef1c7c4e028a1470e214fe0496a,a92d4f433e925ea8e4a10d25dfa58e64ba1e68d07ee963605a2ccbaa2e3185aa",
                ),
            ),
            ("Locations", Value::from(vec!["./fakedevice124.cab"])),
            ("Version", Value::from("1.2.4")),
            ("TrustFlags", Value::U64(7)),
            ("Urgency", Value::U32(2)),
        ])
    }

    #[test]
    fn the_real_webcam_and_release_parse() {
        let dev = Device::parse(&webcam()).unwrap();
        assert!(dev.updatable());
        assert!(!dev.waiting_for_reboot() && dev.pending().is_none());
        let u = parse_release(&dev, &release()).unwrap();
        assert_eq!(u.device, "Integrated Webcam™");
        assert_eq!((u.current.as_str(), u.version.as_str()), ("1.2.2", "1.2.4"));
        assert_eq!(u.urgency, Urgency::Medium);
        assert_eq!(u.size, 17);
        assert_eq!(u.checksums.len(), 2);
        assert_eq!(u.locations, vec!["./fakedevice124.cab"]);
        assert_eq!(u.description, "Fixes another bug with the flux capacitor.");
        assert!(u.trusted && !u.internal && !u.needs_reboot);
        assert_eq!(u.remote_id, "fwupd-tests");
    }

    #[test]
    fn device_flags_filter() {
        let mk = |flags: u64| {
            let mut d = webcam();
            d.insert(
                "Flags".into(),
                OwnedValue::try_from(Value::U64(flags)).unwrap(),
            );
            Device::parse(&d).unwrap()
        };
        assert!(mk(1 << 1).updatable());
        assert!(!mk(1).updatable());
        assert!(!mk((1 << 1) | (1 << 4)).updatable());
        assert!(!mk((1 << 1) | (1 << 37)).updatable());
        let d = mk((1 << 1) | 1 | (1 << 8) | (1 << 17));
        let u = parse_release(&d, &release()).unwrap();
        assert!(u.internal && u.needs_reboot && u.needs_shutdown);
    }

    #[test]
    fn release_flags_filter() {
        let dev = Device::parse(&webcam()).unwrap();
        let with = |flags: u64| {
            let mut r = release();
            r.insert(
                "TrustFlags".into(),
                OwnedValue::try_from(Value::U64(flags)).unwrap(),
            );
            parse_release(&dev, &r)
        };
        assert!(with(4).is_some());
        assert!(!with(4).unwrap().trusted);
        assert!(with(4 | 1).unwrap().trusted);
        assert!(with(4 | 2).unwrap().trusted);
        assert!(with(0).is_none());
        assert!(with(4 | 16).is_none());
        assert!(with(4 | 32).is_none());
        let mut r = release();
        r.remove("Version");
        assert!(parse_release(&dev, &r).is_none());
    }

    #[test]
    fn odd_types_and_missing_keys_do_not_panic() {
        let weird = dict(vec![("DeviceId", Value::U32(7)), ("Name", Value::U64(1))]);
        assert!(Device::parse(&weird).is_none());
        let bad_id = dict(vec![("DeviceId", Value::from("a b/../c"))]);
        assert!(Device::parse(&bad_id).is_none());
        let dev = Device::parse(&dict(vec![
            ("DeviceId", Value::from(ID)),
            ("Name", Value::from(vec![1u8])),
            ("Flags", Value::from("x")),
        ]))
        .unwrap();
        assert_eq!(dev.name, "Unnamed device");
        assert!(!dev.updatable());
        let r = dict(vec![
            ("Version", Value::from("2")),
            ("TrustFlags", Value::I32(4)),
            ("Locations", Value::U8(1)),
            ("Checksum", Value::from("zz,12,")),
            ("Size", Value::from("big")),
            ("Urgency", Value::U32(99)),
        ]);
        let u = parse_release(&dev, &r).unwrap();
        assert!(u.checksums.is_empty() && u.locations.is_empty());
        assert_eq!((u.size, u.urgency), (0, Urgency::Unknown));
    }

    #[test]
    fn pending_and_failed_devices() {
        let mk = |state: u32, err: Option<&str>| {
            let mut d = webcam();
            d.insert(
                "UpdateState".into(),
                OwnedValue::try_from(Value::U32(state)).unwrap(),
            );
            if let Some(e) = err {
                d.insert(
                    "UpdateError".into(),
                    OwnedValue::try_from(Value::from(e)).unwrap(),
                );
            }
            Device::parse(&d).unwrap()
        };
        assert_eq!(mk(1, None).pending().unwrap().state, PendingState::Reboot);
        assert_eq!(mk(4, None).pending().unwrap().state, PendingState::Reboot);
        assert!(mk(4, None).waiting_for_reboot());
        assert_eq!(
            mk(3, Some("flash\u{202e}ed badly"))
                .pending()
                .unwrap()
                .state,
            PendingState::Failed("flashed badly".into())
        );
        assert!(matches!(
            mk(3, None).pending().unwrap().state,
            PendingState::Failed(m) if !m.is_empty()
        ));
        assert!(mk(2, None).pending().is_none());
        assert!(mk(0, None).pending().is_none());
    }

    #[test]
    fn metadata_age_uses_newest_enabled_download_remote() {
        let r = |en: bool, ty: u32, t: u64| {
            dict(vec![
                ("Enabled", Value::Bool(en)),
                ("Type", Value::U32(ty)),
                ("ModificationTime", Value::U64(t)),
            ])
        };
        let now = UNIX_EPOCH + Duration::from_secs(10_000);
        let remotes = vec![
            r(true, 1, 9_000),
            r(true, 1, 9_500),
            r(false, 1, 9_900),
            r(true, 2, 9_990),
            r(true, 1, u64::MAX),
        ];
        assert_eq!(metadata_age(&remotes, now), Some(Duration::from_secs(500)));
        assert_eq!(metadata_age(&[r(true, 1, u64::MAX)], now), None);
        assert_eq!(metadata_age(&[], now), None);
        assert_eq!(
            metadata_age(&[r(true, 1, 20_000)], now),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn description_markup() {
        assert_eq!(
            description_text("<p>One</p><p>Two  &amp; <em>three</em></p>"),
            "One\n\nTwo & three"
        );
        assert_eq!(
            description_text("<p>Fixes:</p><ul><li>a</li><li>b <code>c</code></li></ul><p>End</p>"),
            "Fixes:\n\n• a\n• b c\n\nEnd"
        );
        assert_eq!(
            description_text("<ol><li>x</li><li>y</li></ol>"),
            "1. x\n2. y"
        );
        assert_eq!(
            description_text("plain &lt;text&gt; &#65;&#x42;"),
            "plain <text> AB"
        );
        assert_eq!(description_text(""), "");
    }

    #[test]
    fn malformed_markup_never_panics() {
        for s in [
            "<p>unclosed",
            "</p></ul></li>",
            "<<<>>>",
            "<",
            ">",
            "&",
            "&;",
            "&#;",
            "&#xZZ;",
            "&#99999999999;",
            "a < b and c > d",
            "<p",
            "<p attr='>x",
            "&amp",
            "<li>x",
            "é<é>é&é;é",
            "\u{0}\u{1}<p>\u{202e}x</p>",
            "<a href=\"javascript:x\">link</a>",
            "&#0;",
            "&#xD800;",
        ] {
            let out = description_text(s);
            assert!(out.chars().count() <= DESCRIPTION_MAX);
            assert!(!out.chars().any(|c| c.is_control() && c != '\n'));
        }
        assert_eq!(description_text("a < b and c > d"), "a < b and c > d");
        assert_eq!(description_text("<a href=x>link</a>"), "link");
        let long = format!("<p>{}</p>", "x".repeat(10_000));
        assert_eq!(description_text(&long).chars().count(), DESCRIPTION_MAX);
        assert!(description_text(&"<".repeat(100_000)).chars().count() <= DESCRIPTION_MAX);
    }

    #[test]
    fn text_is_cleaned() {
        assert_eq!(clean_to("a\u{202e}b\tc\nd\u{0}", 50), "ab c d");
        assert_eq!(clean_to("abcdef", 4), "abc…");
        assert_eq!(clean_to("  x  ", 4), "x");
    }

    #[test]
    fn checksum_choice() {
        let s1 = "db63711564cacef1c7c4e028a1470e214fe0496a".to_string();
        let s256 = "A92D4F433E925EA8E4A10D25DFA58E64BA1E68D07EE963605A2CCBAA2E3185AA".to_string();
        assert_eq!(
            pick_checksum(&[s1.clone(), s256.clone()]),
            Some(Checksum::Sha256(s256.to_lowercase()))
        );
        assert_eq!(
            sha256_of(&[s1.clone(), s256.clone()]),
            Some(s256.to_lowercase())
        );
        assert_eq!(
            pick_checksum(std::slice::from_ref(&s1)),
            Some(Checksum::Sha1(s1.clone()))
        );
        assert_eq!(sha256_of(std::slice::from_ref(&s1)), None);
        assert_eq!(pick_checksum(&[]), None);
        assert_eq!(
            pick_checksum(&["nothex".into(), "d41d8cd98f00b204e9800998ecf8427e".into()]),
            None
        );
        assert_eq!(pick_checksum(&["g".repeat(64)]), None);
    }

    #[test]
    fn local_and_private_hosts_are_refused() {
        let url = |h: &str| resolve_location(&format!("https://{h}/a.cab"), None);
        for bad in [
            "localhost",
            "LocalHost:8443",
            "localhost.",
            "a.localhost",
            "127.0.0.1",
            "127.1",
            "2130706433",
            "0x7f.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1:8080",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "[::1]",
            "[::1]:443",
            "[fd00::1]",
            "[fe80::1]",
            "[::]",
            "::1",
        ] {
            assert_eq!(url(bad), None, "{bad}");
        }
        assert!(url("8.8.8.8").is_some());
        assert!(url("fwupd.org").is_some());
        assert!(url("cdn1.example.com:8443").is_some());
        assert!(url("1password.example.com").is_some());
    }

    #[test]
    fn hostile_markup_is_linear() {
        for ch in ['<', '&'] {
            let start = std::time::Instant::now();
            let text = ch.to_string().repeat(64 * 1024);
            let out = description_text(&text);
            assert!(!out.is_empty() || ch == '<');
            assert!(
                start.elapsed() < Duration::from_millis(500),
                "{ch}: {:?}",
                start.elapsed()
            );
        }
        // a tag that never closes within the bound is literal text
        let far = format!("<b{}>x", " ".repeat(400));
        assert!(description_text(&far).starts_with("<b"));
        assert_eq!(description_text("a &amp; b"), "a & b");
    }

    #[test]
    fn locations() {
        let base = Some("https://fwupd.org/downloads");
        let ok = |l: &str, b: Option<&str>| resolve_location(l, b);
        assert_eq!(
            ok("https://fwupd.org/downloads/a-b.cab", None).as_deref(),
            Some("https://fwupd.org/downloads/a-b.cab")
        );
        assert_eq!(
            ok("HTTPS://example.com:8443/x.cab", None).as_deref(),
            Some("https://example.com:8443/x.cab")
        );
        assert_eq!(
            ok("./name.cab", base).as_deref(),
            Some("https://fwupd.org/downloads/name.cab")
        );
        assert_eq!(
            ok("name.cab", Some("https://h.example/dl/")).as_deref(),
            Some("https://h.example/dl/name.cab")
        );
        assert_eq!(
            ok("sub/name.cab?x=1", Some("https://h.example")).as_deref(),
            Some("https://h.example/sub/name.cab?x=1")
        );
        for bad in [
            "http://fwupd.org/x.cab",
            "file:///etc/passwd",
            "data:text/plain,hi",
            "ftp://h/x",
            "javascript:alert(1)",
            "https://user@evil.example/x.cab",
            "https://user:pw@evil.example/x.cab",
            "https://h.example@evil/x",
            "https://fwupd.org/a/../b.cab",
            "https://fwupd.org/a/%2e%2e/b.cab",
            "https://fwupd.org/a%2fb",
            "https://",
            "https:///x",
            "https://h.example:/x",
            "https://h.example:99999/x",
            "https://h ost/x",
            "https://h.example/x#frag",
            "//evil.example/x.cab",
            "/abs/path.cab",
            "../x.cab",
            "./../x.cab",
            "a/../../x.cab",
            "a/./b.cab",
            "",
            "   ",
            "https://h.example/x y",
            "https://h.example/\u{e9}",
            "https://h.example\\x",
            "https://[::1]/x",
            "https://-bad.example/x",
        ] {
            assert_eq!(ok(bad, base), None, "{bad}");
        }
        // a relative location needs an https base
        assert_eq!(ok("./x.cab", None), None);
        assert_eq!(ok("./x.cab", Some("http://fwupd.org/downloads")), None);
        assert_eq!(ok("./x.cab", Some("https://u@fwupd.org/d")), None);
        assert_eq!(ok("./x.cab", Some("https://fwupd.org/d/../e")), None);
        assert_eq!(ok("./x.cab", Some("https://fwupd.org/d?q")), None);
        assert_eq!(
            ok(&format!("https://h.example/{}", "a".repeat(3000)), None),
            None
        );
    }

    #[test]
    fn notice_key_is_stable_and_order_free() {
        let dev = Device::parse(&webcam()).unwrap();
        let a = parse_release(&dev, &release()).unwrap();
        let mut b = a.clone();
        b.device_id = "other".into();
        let mut c = a.clone();
        c.version = "1.2.5".into();
        assert_eq!(notice_key(&[]), "");
        assert_eq!(
            notice_key(&[a.clone(), b.clone()]),
            notice_key(&[b.clone(), a.clone()])
        );
        assert_ne!(
            notice_key(std::slice::from_ref(&a)),
            notice_key(std::slice::from_ref(&b))
        );
        assert_ne!(notice_key(std::slice::from_ref(&a)), notice_key(&[c]));
        assert_ne!(
            notice_key(std::slice::from_ref(&a)),
            notice_key(&[a.clone(), b])
        );
        // pinned: the key is kept in settings, so it must not drift
        assert_eq!(notice_key(&[a]), "6a4a6a8fe7966d82");
    }

    #[test]
    fn fwupd_errors_in_words() {
        let n = |s: &str| format!("org.freedesktop.fwupd.{s}");
        assert_eq!(fwupd_error(&n("AuthFailed"), "x"), Error::Cancelled);
        assert_eq!(fwupd_error(&n("PermissionDenied"), "x"), Error::Cancelled);
        for (name, word) in [
            ("BatteryLevelTooLow", "battery"),
            ("NeedsUserAction", "unplugging"),
            ("NothingToDo", "nothing to update"),
            ("NotSupported", "can't be updated"),
            ("AlreadyPending", "waiting for a restart"),
        ] {
            match fwupd_error(&n(name), "raw") {
                Error::Message(m) => assert!(m.contains(word), "{name}: {m}"),
                Error::Cancelled => panic!("{name}"),
            }
        }
        match fwupd_error(&n("Internal"), "daemon was stopped") {
            Error::Message(m) => assert!(m.contains("result is unknown"), "{m}"),
            Error::Cancelled => panic!("Internal"),
        }
        assert_eq!(
            fwupd_error(&n("Internal"), "other"),
            Error::Message("other".into())
        );
        assert_eq!(
            fwupd_error(&n("Write"), "disk\u{202e} full"),
            Error::Message("disk full".into())
        );
        assert!(matches!(fwupd_error(&n("Read"), ""), Error::Message(m) if m.contains("Read")));
        let call = zbus::Message::method_call("/", "X")
            .unwrap()
            .build(&())
            .unwrap();
        let e = zbus::Error::MethodError(n("AuthFailed").try_into().unwrap(), None, call.clone());
        assert_eq!(map_error(&e), Error::Cancelled);
        assert!(!no_update(&e));
        let e = zbus::Error::MethodError(n("NothingToDo").try_into().unwrap(), None, call);
        assert!(no_update(&e) && is_method_error(&e));
        assert!(
            matches!(map_error(&zbus::Error::Unsupported), Error::Message(m) if m.contains("fwupd"))
        );
    }

    #[test]
    fn statuses_and_requests() {
        assert_eq!(status_word(5), "Writing");
        assert_eq!(status_word(11), "Waiting for your password");
        assert_eq!(status_word(9999), "Working");
        let r = dict(vec![
            ("Message", Value::from("Press the button")),
            ("DeviceId", Value::from(ID)),
        ]);
        assert_eq!(request_text(&r, ID).as_deref(), Some("Press the button"));
        assert_eq!(request_text(&r, "someone-else"), None);
        let r = dict(vec![(
            "Id",
            Value::from("org.freedesktop.fwupd.request.remove-replug"),
        )]);
        assert!(request_text(&r, ID).unwrap().contains("Unplug"));
        let r = dict(vec![(
            "Id",
            Value::from("org.freedesktop.fwupd.request.do-something-odd"),
        )]);
        assert!(request_text(&r, ID).unwrap().contains("do something odd"));
        assert_eq!(request_text(&dict(vec![]), ID), None);
    }

    // ------------------------------------------------ rig (real fwupd)

    /// Against a real fwupd with only the test plugin (`tools/fwupd-rig.sh`
    /// in the container): `ATLAS_FWUPD_RIG=1 cargo test fwupd -- --ignored`.
    #[test]
    #[ignore = "needs the fwupd rig (ATLAS_FWUPD_RIG=1)"]
    fn rig_lists_and_installs_the_test_device() {
        if std::env::var_os("ATLAS_FWUPD_RIG").is_none() {
            return;
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let conn = Connection::system().await.unwrap();
            assert!(available(&conn).await.unwrap());
            let l = list(&conn).await.unwrap();
            println!("before: {l:#?}");
            assert_eq!(l.updates.len(), 1, "{l:?}");
            let u = &l.updates[0];
            assert_eq!(u.device, "Integrated Webcam™");
            assert_eq!(u.version, "1.2.4");
            assert_eq!(u.remote_id, "fwupd-tests");
            assert_eq!(
                upgrade_for(&conn, &u.device_id).await.unwrap().as_ref(),
                Some(u)
            );
            assert_eq!(upgrade_for(&conn, "0000").await.unwrap(), None);

            let f =
                std::fs::File::open("/usr/share/installed-tests/fwupd/fakedevice124.cab").unwrap();
            let mut seen: Vec<Progress> = Vec::new();
            let done = install(&conn, &u.device_id, OwnedFd::from(f), |p| {
                println!("progress: {p:?}");
                seen.push(p);
            })
            .await;
            println!("install: {done:?}");
            let done = done.unwrap();
            println!("done: {done:?}, {} progress events", seen.len());
            assert!(!seen.is_empty(), "no progress arrived");
            let after = list(&conn).await.unwrap();
            println!("after: {after:#?}");
        });
    }
}
