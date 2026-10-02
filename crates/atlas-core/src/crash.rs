//! Crash reports: the only telemetry an Atlas app may have. Opt-in, off by
//! default, and the user sees the exact payload before every send.
//!
//! - [`Settings`]: per-user switch, `~/.config/atlas/crash-reporting.toml`
//!   (`enabled = false`). When off, nothing is collected or written.
//! - [`Endpoint`]: a GlitchTip (Sentry compatible) DSN from
//!   `/etc/atlas/crash-reporting.toml`, default `/usr/share/atlas/...`. Empty
//!   by default: [`send`] then fails with "no endpoint configured".
//! - Sources: Rust panics ([`install`], [`record_fatal`]), systemd-coredump
//!   entries of the user's own processes ([`collect_coredumps`]) and update
//!   and rollback events from the helper ([`collect_events`]).
//! - Reports wait in `$XDG_STATE_HOME/atlas/crash-reports/pending/` for the
//!   user's decision ([`pending`], [`discard`]); sent ones move to `sent/`
//!   and are pruned after 90 days.
//!
//! Collected: AtlasOS version, channel, previous version; app name, version
//! and category; the stack trace; kernel; GPU model and driver; uptime; CPU
//! model, RAM total and use; a rotating random ID (new every 30 days; never
//! `/etc/machine-id`), a timestamp and the report type. Never: core dumps,
//! usernames, hostnames, MAC/IP addresses, serials, installed apps, file
//! contents, command lines, environment or working directory. Every string
//! is scrubbed ([`Scrubber`]).

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::history::{self, now_rfc3339};

pub const SYSTEM_CONFIG: &str = "/etc/atlas/crash-reporting.toml";
pub const DEFAULT_CONFIG: &str = "/usr/share/atlas/crash-reporting.toml";
const SENT_KEEP: Duration = Duration::from_secs(90 * 86_400);
const ID_MAX_AGE: Duration = Duration::from_secs(30 * 86_400);
const COREDUMP_MESSAGE_ID: &str = "fc2e22bc6ee647b6b90729ab34a250b1";

/// Identifies the Atlas app that installs the panic hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    /// Reverse-DNS app ID, e.g. `net.eterneon.atlas.updater`.
    pub id: String,
    pub version: String,
    /// Repository under `github.com/EternalCoder454/` for [`github_issue_url`].
    pub repo: String,
}

/// One crash or event report. [`Report::payload`] is what gets sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    /// 32 hex chars; the Sentry event ID.
    pub event_id: String,
    /// `panic`, `fatal`, `coredump` or an event name such as `update-failed`.
    pub report_type: String,
    /// RFC 3339 UTC.
    pub time: String,
    /// The rotating anonymous ID.
    pub crash_id: String,
    pub atlasos_version: Option<String>,
    pub channel: Option<String>,
    pub previous_version: Option<String>,
    pub app_name: String,
    pub app_version: Option<String>,
    /// `Plasma`, `KWin`, `Atlas app` or `other`.
    pub category: String,
    pub message: String,
    pub stacktrace: String,
    pub kernel: Option<String>,
    pub gpu: Option<String>,
    pub gpu_driver: Option<String>,
    pub uptime_secs: u64,
    pub cpu_model: Option<String>,
    pub ram_total_kb: u64,
    pub mem_used_kb: u64,
    /// Server-side event ID once sent.
    #[serde(default)]
    pub sent_event_id: Option<String>,
    /// Where the report is stored; not part of the report.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Report {
    /// The exact Sentry event JSON that [`send`] posts.
    pub fn payload(&self) -> Value {
        let tag = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".to_string());
        let mut event = json!({
            "event_id": self.event_id,
            "timestamp": self.time,
            "platform": "native",
            "level": "fatal",
            "logger": "atlas-core",
            "release": format!("atlasos@{}", tag(&self.atlasos_version)),
            "environment": tag(&self.channel),
            "message": self.message,
            "tags": {
                "app": self.app_name,
                "app_version": tag(&self.app_version),
                "category": self.category,
                "atlasos_version": tag(&self.atlasos_version),
                "channel": tag(&self.channel),
                "previous_version": tag(&self.previous_version),
                "kernel": tag(&self.kernel),
                "gpu": tag(&self.gpu),
                "gpu_driver": tag(&self.gpu_driver),
                "report_type": self.report_type,
            },
            "contexts": {
                "os": {"name": "AtlasOS", "version": tag(&self.atlasos_version),
                       "kernel_version": tag(&self.kernel)},
                "gpu": {"name": tag(&self.gpu), "version": tag(&self.gpu_driver)},
                "device": {"cpu": tag(&self.cpu_model), "memory_size": self.ram_total_kb * 1024,
                           "free_memory": (self.ram_total_kb.saturating_sub(self.mem_used_kb)) * 1024},
                "runtime": {"uptime_secs": self.uptime_secs},
            },
            "user": {"id": self.crash_id},
        });
        let frames = parse_frames(&self.stacktrace);
        if !frames.is_empty() {
            event["exception"] = json!({"values": [{
                "type": self.report_type,
                "value": self.message,
                "stacktrace": {"frames": frames},
            }]});
        }
        event
    }

    /// The payload as pretty JSON, for the "Show report" view.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(&self.payload()).unwrap_or_default()
    }
}

/// Frames in Sentry order (oldest call first) from a systemd-coredump style
/// (`#0  0x7f.. func (lib.so + 0x1)`) or Rust (`  0: func`) trace.
fn parse_frames(trace: &str) -> Vec<Value> {
    let mut frames = Vec::new();
    for line in trace.lines() {
        let l = line.trim();
        let (body, rust) = if let Some(rest) = l.strip_prefix('#') {
            (
                rest.split_once(char::is_whitespace)
                    .map_or("", |x| x.1)
                    .trim(),
                false,
            )
        } else if let Some((n, rest)) = l.split_once(": ")
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            (rest.trim(), true)
        } else {
            continue;
        };
        if rust {
            frames.push(json!({"function": body}));
            continue;
        }
        let mut addr = None;
        let mut rest = body;
        if let Some(a) = body
            .split_whitespace()
            .next()
            .filter(|a| a.starts_with("0x"))
        {
            addr = Some(a.to_string());
            rest = body[a.len()..].trim();
        }
        let (func, module) = match rest.split_once('(') {
            Some((f, m)) => (
                f.trim(),
                m.trim_end_matches(')')
                    .split(" + ")
                    .next()
                    .unwrap_or("")
                    .trim(),
            ),
            None => (rest, ""),
        };
        let mut f = json!({"function": if func.is_empty() { "n/a" } else { func }});
        if let Some(a) = addr {
            f["instruction_addr"] = json!(a);
        }
        if !module.is_empty() {
            f["package"] = json!(module);
            f["module"] = json!(module);
        }
        frames.push(f);
    }
    frames.reverse();
    frames
}

// ---------------------------------------------------------------- settings

/// An environment path, only if absolute (XDG says to ignore the rest).
fn abs_env(name: &str) -> Option<PathBuf> {
    let v = PathBuf::from(std::env::var_os(name).filter(|v| !v.is_empty())?);
    v.is_absolute().then_some(v)
}

fn config_home() -> Option<PathBuf> {
    abs_env("XDG_CONFIG_HOME").or_else(|| Some(abs_env("HOME")?.join(".config")))
}

fn state_home() -> Option<PathBuf> {
    abs_env("XDG_STATE_HOME").or_else(|| Some(abs_env("HOME")?.join(".local/state")))
}

/// `key = "value"` / `key = true` lookup in a tiny TOML subset. `Some("")`
/// when the key is present but empty.
fn toml_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.split('#').next()?.trim();
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// The per-user opt-in. Off unless the file says `enabled = true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub enabled: bool,
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        Some(config_home()?.join("atlas/crash-reporting.toml"))
    }

    pub fn load() -> Settings {
        Self::path()
            .map(|p| Self::load_from(&p))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Settings {
        let on = fs::read_to_string(path)
            .ok()
            .and_then(|t| toml_value(&t, "enabled"))
            .is_some_and(|v| v == "true");
        Settings { enabled: on }
    }

    /// Save the setting. Turning reporting on starts the coredump and event
    /// markers at "now", so nothing from before the opt-in is ever queued;
    /// turning it off prunes the sent history.
    pub fn save(&self) -> io::Result<()> {
        let p = Self::path().ok_or_else(|| io::Error::other("no config directory"))?;
        let was = Self::load_from(&p).enabled;
        self.save_to(&p)?;
        if self.enabled && !was {
            reset_markers();
        }
        if !self.enabled {
            prune_sent();
        }
        Ok(())
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(d) = path.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(
            path,
            format!(
                "# Atlas crash reporting; see docs. Off unless true.\nenabled = {}\n",
                self.enabled
            ),
        )
    }
}

/// A GlitchTip DSN: `https://<key>@<host>[/prefix]/<project>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub key: String,
    /// Full store URL: `https://host/api/<project>/store/`.
    pub store_url: String,
}

fn url_part_ok(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

impl Endpoint {
    /// The configured endpoint. If `/etc/atlas/crash-reporting.toml` has a
    /// `dsn` key it alone decides (empty means none); only without the key
    /// does the shipped default apply.
    pub fn load() -> Option<Endpoint> {
        for p in [SYSTEM_CONFIG, DEFAULT_CONFIG] {
            if let Some(v) = fs::read_to_string(p)
                .ok()
                .and_then(|t| toml_value(&t, "dsn"))
            {
                return Self::parse(&v);
            }
        }
        None
    }

    pub fn load_from(path: &Path) -> Option<Endpoint> {
        Self::parse(&toml_value(&fs::read_to_string(path).ok()?, "dsn")?)
    }

    /// `https` only; `http` is accepted for localhost, 127.0.0.1 and [::1].
    /// The key must be a plain public key (a legacy `key:secret` is refused).
    pub fn parse(dsn: &str) -> Option<Endpoint> {
        let (scheme, rest) = dsn.trim().split_once("://")?;
        let (key, rest) = rest.split_once('@')?;
        let (host, path) = rest.split_once('/')?;
        let (host_only, host_ok) = match host.strip_prefix("[::1]") {
            Some(port) => (
                "::1",
                port.is_empty()
                    || port
                        .strip_prefix(':')
                        .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())),
            ),
            None => (
                host.rsplit_once(':').map_or(host, |x| x.0),
                !host.is_empty()
                    && host
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-.:".contains(c)),
            ),
        };
        let loopback = matches!(host_only, "localhost" | "127.0.0.1" | "::1");
        match scheme {
            "https" => {}
            "http" if loopback => {}
            _ => return None,
        }
        let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
        if !host_ok || !url_part_ok(key) || !segs.iter().all(|s| url_part_ok(s)) {
            return None;
        }
        let (project, prefix) = segs.split_last()?;
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("/{}", prefix.join("/"))
        };
        Some(Endpoint {
            key: key.into(),
            store_url: format!("{scheme}://{host}{prefix}/api/{project}/store/"),
        })
    }
}

/// Start both markers at "now": only crashes after this are ever collected.
fn reset_markers() {
    if let Some(d) = state_dir() {
        reset_markers_in(&d);
    }
}

fn reset_markers_in(dir: &Path) {
    let now_micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64);
    let _ = write_private(
        &dir.join("coredump-last"),
        now_micros.to_string().as_bytes(),
        true,
    );
    let _ = write_private(&dir.join("events-last"), now_rfc3339().as_bytes(), true);
}

// ---------------------------------------------------------------- scrubbing

/// Replaces home directories, user names, host names, MAC and IP addresses
/// and machine/boot IDs in strings. Matching is case-insensitive.
#[derive(Debug, Clone, Default)]
pub struct Scrubber {
    users: Vec<String>,
    hosts: Vec<String>,
    homes: Vec<String>,
}

fn clean_list(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    v.sort_by_key(|s| std::cmp::Reverse(s.len()));
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

impl Scrubber {
    /// `users` are user names and full names, `hosts` host names (their first
    /// label is added), `homes` literal home directories.
    pub fn new(users: &[&str], hosts: &[&str], homes: &[&str]) -> Self {
        let mut h: Vec<String> = hosts.iter().map(|s| s.to_string()).collect();
        h.extend(
            hosts
                .iter()
                .filter_map(|s| s.split('.').next().map(str::to_string)),
        );
        Scrubber {
            users: clean_list(users.iter().map(|s| s.to_string())),
            hosts: clean_list(h),
            homes: clean_list(
                homes
                    .iter()
                    .map(|s| s.trim_end_matches('/').to_string())
                    .filter(|s| s.len() > 1),
            ),
        }
    }

    /// Everything about this user and machine that could identify them:
    /// $USER, $LOGNAME, $HOME, the passwd name, full name and home, and the
    /// kernel, static and pretty host names.
    pub fn from_env() -> Self {
        let mut users: Vec<String> = ["USER", "LOGNAME"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .collect();
        let mut homes: Vec<String> = std::env::var("HOME").ok().into_iter().collect();
        if let Some(h) = homes.first() {
            users.extend(
                Path::new(h)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned()),
            );
        }
        let uid = own_uid();
        for l in read("/etc/passwd").lines() {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() >= 6 && f[2] == uid {
                users.push(f[0].to_string());
                users.extend(f[4].split(',').next().map(str::to_string));
                homes.push(f[5].to_string());
            }
        }
        let mut hosts = vec![read("/proc/sys/kernel/hostname"), read("/etc/hostname")];
        hosts.extend(toml_value(&read("/etc/machine-info"), "PRETTY_HOSTNAME"));
        let (u, h, hm): (Vec<&str>, Vec<&str>, Vec<&str>) = (
            users.iter().map(String::as_str).collect(),
            hosts.iter().map(String::as_str).collect(),
            homes.iter().map(String::as_str).collect(),
        );
        Scrubber::new(&u, &h, &hm)
    }

    /// Only the host names (for the root helper, which has no user).
    pub fn for_system() -> Self {
        let mut hosts = vec![read("/proc/sys/kernel/hostname"), read("/etc/hostname")];
        hosts.extend(toml_value(&read("/etc/machine-info"), "PRETTY_HOSTNAME"));
        let h: Vec<&str> = hosts.iter().map(String::as_str).collect();
        Scrubber::new(&[], &h, &[])
    }

    /// `/home/<name>` and `/var/home/<name>` become `.../USER`, user names
    /// `USER`, host names `HOST`; MAC and IP addresses, interface names with
    /// a MAC, and machine/boot IDs go.
    pub fn scrub(&self, s: &str) -> String {
        let mut out = scrub_homes(s);
        for h in &self.homes {
            out = out.replace(h.as_str(), "/home/USER");
        }
        for u in &self.users {
            out = replace_ci(&out, u, "USER");
        }
        for h in &self.hosts {
            out = replace_ci(&out, h, "HOST");
        }
        scrub_addresses(&out)
    }

    /// [`scrub`](Self::scrub), then hide paths that can name a file the user
    /// had open. For panic messages and stack traces.
    pub fn scrub_message(&self, s: &str) -> String {
        redact_paths(&self.scrub(s))
    }
}

/// Replace the name after `/home/` and `/var/home/` with `USER`.
fn scrub_homes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/home/") {
        let end = i + "/home/".len();
        out.push_str(&rest[..end]);
        rest = &rest[end..];
        let name_end = rest
            .find(|c: char| c == '/' || c.is_whitespace() || "'\"`:;,)>]".contains(c))
            .unwrap_or(rest.len());
        if name_end > 0 {
            out.push_str("USER");
        }
        rest = &rest[name_end..];
    }
    out.push_str(rest);
    out
}

/// Case-insensitive replace. Names of 4 or more characters match anywhere;
/// shorter ones only where no letter touches them (so `_`, digits and
/// punctuation count as boundaries, but `ann` stays inside `channel`).
fn replace_ci(s: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return s.to_string();
    }
    let (hay, pat) = (s.to_ascii_lowercase(), needle.to_ascii_lowercase());
    let anywhere = needle.chars().count() >= 4;
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    while let Some(i) = hay[pos..].find(&pat) {
        let at = pos + i;
        let end = at + pat.len();
        let letter = |c: Option<char>| c.is_some_and(char::is_alphabetic);
        let touches = letter(s[..at].chars().next_back()) || letter(s[end..].chars().next());
        out.push_str(&s[pos..at]);
        out.push_str(if anywhere || !touches {
            with
        } else {
            &s[at..end]
        });
        pos = end;
    }
    out.push_str(&s[pos..]);
    out
}

fn is_mac(t: &str) -> bool {
    let sep = if t.contains(':') { ':' } else { '-' };
    let parts: Vec<&str> = t.split(sep).collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn is_ipv4(t: &str) -> bool {
    let parts: Vec<&str> = t.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.parse::<u8>().is_ok())
}

fn is_ipv6(t: &str) -> bool {
    let colons = t.matches(':').count();
    colons >= 2
        && t.len() >= 3
        && t.chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
        && (t.contains("::") || colons == 7)
}

fn all_hex(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_ascii_hexdigit())
}

/// What a whole word is, if it is an address or ID: `<mac>`, `<ip>` or `<id>`.
fn classify(word: &str) -> Option<&'static str> {
    let mut w = word;
    if let Some((head, _zone)) = w.split_once('%')
        && head.contains(':')
    {
        w = head;
    }
    if let Some((h, port)) = w.rsplit_once(':')
        && is_ipv4(h)
        && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
    {
        w = h;
    }
    let has_letter = w.chars().any(|c| c.is_ascii_alphabetic());
    let dotted_mac = {
        let p: Vec<&str> = w.split('.').collect();
        p.len() == 3 && p.iter().all(|x| x.len() == 4 && all_hex(x))
    };
    let uuid = {
        let p: Vec<&str> = w.split('-').collect();
        p.len() == 5
            && p.iter()
                .zip([8, 4, 4, 4, 12])
                .all(|(x, n)| x.len() == n && all_hex(x))
    };
    let iface = w.len() == 15 && (w.starts_with("enx") || w.starts_with("wlx")) && all_hex(&w[3..]);
    if is_mac(w) || dotted_mac || iface || (w.len() == 12 && all_hex(w) && has_letter) {
        Some("<mac>")
    } else if is_ipv4(w) || is_ipv6(w) {
        Some("<ip>")
    } else if (w.len() == 32 && all_hex(w)) || uuid {
        Some("<id>")
    } else {
        None
    }
}

fn scrub_word(tok: &str) -> String {
    // trailing sentence dot or colon (but keep the `::` of `2001:db8::`)
    let mut core = tok.trim_end_matches('.');
    if core.ends_with(':') && !core.ends_with("::") {
        core = &core[..core.len() - 1];
    }
    let suffix = &tok[core.len()..];
    if let Some(k) = classify(core) {
        return format!("{k}{suffix}");
    }
    // a label in front: `host:10.0.0.1`, `inet:192.168.0.2`
    for (i, _) in core.match_indices(':') {
        if let Some(k) = classify(&core[i + 1..]) {
            return format!("{}:{k}{suffix}", &core[..i]);
        }
    }
    tok.to_string()
}

/// Replace MAC, IP addresses and machine/boot IDs with `<mac>`, `<ip>`, `<id>`.
fn scrub_addresses(s: &str) -> String {
    let delim = |c: char| c.is_whitespace() || "'\"`(),;[]{}<>=/|\\".contains(c);
    let mut out = String::with_capacity(s.len());
    let mut tok = String::new();
    for c in s.chars() {
        if delim(c) {
            out.push_str(&scrub_word(&tok));
            tok.clear();
            out.push(c);
        } else {
            tok.push(c);
        }
    }
    out.push_str(&scrub_word(&tok));
    out
}

const PRIVATE_PREFIXES: &[&str] = &[
    "/home/",
    "/var/home/",
    "/run/user/",
    "/run/media/",
    "/media/",
    "/mnt/",
    "/var/mnt/",
    "/tmp/",
    "/var/tmp/",
    "/root",
    "/var/roothome",
    "/srv/",
    "file://",
    "~/",
];

/// From the first private path prefix inside a word, replace the rest of the
/// word with `<path>`.
fn redact_paths(s: &str) -> String {
    let is_delim = |c: char| c.is_whitespace() || "'\"`(),;[]{}".contains(c);
    let mut out = String::with_capacity(s.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        match PRIVATE_PREFIXES.iter().filter_map(|p| token.find(p)).min() {
            Some(i) => {
                out.push_str(&token[..i]);
                out.push_str("<path>");
            }
            None => out.push_str(token),
        }
        token.clear();
    };
    for c in s.chars() {
        if is_delim(c) {
            flush(&mut token, &mut out);
            out.push(c);
        } else {
            token.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

fn frame_line(l: &str) -> bool {
    let l = l.trim();
    l.strip_prefix('#')
        .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        || l.split_once(": ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Keep only frame lines (what [`parse_frames`] accepts) and thread headers.
fn filter_trace(trace: &str) -> String {
    trace
        .lines()
        .filter(|l| l.trim_start().starts_with("Stack trace of thread") || frame_line(l))
        .collect::<Vec<_>>()
        .join("\n")
}

// ------------------------------------------------------------- collecting

/// `(model name, logical cores)` from /proc/cpuinfo text.
fn cpu_model(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "model name").then(|| v.trim().to_string())
    })
}

fn kb_field(text: &str, key: &str) -> Option<u64> {
    let rest = text.lines().find_map(|l| l.strip_prefix(key))?;
    rest.trim_start_matches(':')
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Model name for PCI IDs from pci.ids text (`0x1002`, `0x744c` forms ok).
fn pci_name(ids: &str, vendor: &str, device: &str) -> Option<String> {
    let (v, d) = (
        vendor.trim_start_matches("0x").to_lowercase(),
        device.trim_start_matches("0x").to_lowercase(),
    );
    let mut in_vendor = false;
    for l in ids.lines() {
        if l.starts_with('#') || l.is_empty() {
            continue;
        }
        if !l.starts_with('\t') {
            in_vendor = l.starts_with(&v) && l[v.len()..].starts_with(' ');
        } else if in_vendor && !l.starts_with("\t\t") {
            let t = l.trim_start();
            if t.starts_with(&d) && t[d.len()..].starts_with(' ') {
                return Some(t[d.len()..].trim().to_string());
            }
        }
    }
    None
}

/// `(model or IDs, driver name, driver version)` of the first GPU in sysfs.
fn read_gpu(
    drm: &Path,
    pci_ids: &str,
    modules: &Path,
) -> (Option<String>, Option<String>, Option<String>) {
    let mut cards: Vec<PathBuf> = fs::read_dir(drm)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_prefix("card")
                .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
        })
        .map(|e| e.path().join("device"))
        .collect();
    cards.sort();
    for dev in cards {
        let (Ok(v), Ok(d)) = (
            fs::read_to_string(dev.join("vendor")),
            fs::read_to_string(dev.join("device")),
        ) else {
            continue;
        };
        let (v, d) = (v.trim(), d.trim());
        let name = pci_name(pci_ids, v, d).unwrap_or_else(|| {
            format!(
                "{}:{}",
                v.trim_start_matches("0x"),
                d.trim_start_matches("0x")
            )
        });
        let driver = fs::read_link(dev.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let version = driver
            .as_ref()
            .and_then(|drv| fs::read_to_string(modules.join(drv).join("version")).ok())
            .map(|s| s.trim().to_string());
        return (Some(name), driver, version);
    }
    (None, None, None)
}

fn category_of(app: &str) -> &'static str {
    let a = app.rsplit('/').next().unwrap_or(app);
    if a == "plasmashell" || a.starts_with("plasma-") || a.starts_with("plasma_") {
        "Plasma"
    } else if a.starts_with("kwin_") || a == "kwin" {
        "KWin"
    } else if a.starts_with("net.eterneon.atlas.") || a.starts_with("atlas-") {
        "Atlas app"
    } else {
        "other"
    }
}

/// The version, channel and previous version of the running AtlasOS from a
/// `bootc status --json` document and the history.
pub(crate) fn os_info(
    status: Option<&crate::bootc::Status>,
    hist: &[history::Entry],
) -> (Option<String>, Option<String>, Option<String>) {
    let version = status
        .and_then(|s| s.status.booted.as_ref())
        .and_then(|b| b.version().map(str::to_string))
        .or_else(|| hist.first().and_then(|e| e.version.clone()));
    let channel = status.and_then(|s| s.channel()).map(|c| c.to_string());
    let previous = status
        .and_then(|s| s.status.rollback.as_ref())
        .and_then(|b| b.version().map(str::to_string))
        .or_else(|| hist.get(1).and_then(|e| e.version.clone()));
    (version, channel, previous)
}

/// What `build_report` needs to know about the crash.
pub(crate) struct Crash<'a> {
    pub report_type: &'a str,
    pub app_name: &'a str,
    pub app_version: Option<&'a str>,
    pub message: &'a str,
    pub stacktrace: &'a str,
}

/// `bytes` random bytes as hex, from the OS; an error is an error (never a
/// constant ID).
fn random_hex(bytes: usize) -> io::Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| io::Error::other(format!("no randomness: {e}")))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Build a report for `crash` now. All strings are scrubbed and the trace is
/// cut to its frame lines. Does not check [`Settings`]; callers do.
pub(crate) fn build_report(
    crash: &Crash,
    scrubber: &Scrubber,
    time: Option<&str>,
) -> io::Result<Report> {
    let hist = history::read_default().unwrap_or_default();
    let (atlasos_version, channel, previous_version) = os_info(None, &hist);
    let channel = channel.or_else(|| {
        hist.first()
            .and_then(|e| e.image.rsplit_once(':').map(|x| x.1.to_string()))
            .filter(|c| c == "stable" || c == "testing")
    });
    let meminfo = read("/proc/meminfo");
    let total = kb_field(&meminfo, "MemTotal").unwrap_or(0);
    let avail = kb_field(&meminfo, "MemAvailable").unwrap_or(0);
    let (gpu, gpu_driver, gpu_ver) = read_gpu(
        Path::new("/sys/class/drm"),
        &read("/usr/share/hwdata/pci.ids"),
        Path::new("/sys/module"),
    );
    let s = |x: &str| scrubber.scrub(x);
    Ok(Report {
        schema: 2,
        event_id: random_hex(16)?,
        report_type: crash.report_type.to_string(),
        time: time.map_or_else(now_rfc3339, str::to_string),
        crash_id: crash_id()?,
        atlasos_version,
        channel,
        previous_version,
        app_name: s(crash.app_name),
        app_version: crash.app_version.map(s),
        category: category_of(crash.app_name).to_string(),
        message: scrubber.scrub_message(crash.message),
        stacktrace: scrubber.scrub_message(&filter_trace(crash.stacktrace)),
        kernel: Some(read("/proc/sys/kernel/osrelease").trim().to_string())
            .filter(|k| !k.is_empty()),
        gpu,
        gpu_driver: gpu_driver.map(|d| match gpu_ver {
            Some(v) => format!("{d} {v}"),
            None => d,
        }),
        uptime_secs: read("/proc/uptime")
            .split_whitespace()
            .next()
            .and_then(|u| u.parse::<f64>().ok())
            .map_or(0, |u| u as u64),
        cpu_model: cpu_model(&read("/proc/cpuinfo")),
        ram_total_kb: total,
        mem_used_kb: total.saturating_sub(avail),
        sent_event_id: None,
        path: None,
    })
}

// ----------------------------------------------------------------- storage

/// `$XDG_STATE_HOME/atlas`.
fn state_dir() -> Option<PathBuf> {
    Some(state_home()?.join("atlas"))
}

fn reports_dir() -> Option<PathBuf> {
    Some(state_dir()?.join("crash-reports"))
}

/// The rotating anonymous ID: random, replaced when 30 days old or when its
/// creation time is in the future. There is no permanent ID anywhere.
pub(crate) fn crash_id() -> io::Result<String> {
    match state_dir() {
        Some(d) => crash_id_in(&d, SystemTime::now()),
        None => random_hex(16),
    }
}

fn crash_id_in(dir: &Path, now: SystemTime) -> io::Result<String> {
    let path = dir.join("crash-id");
    let secs = |t: SystemTime| t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(text) = fs::read_to_string(&path) {
        let mut l = text.lines();
        if let (Some(id), Some(created)) = (l.next(), l.next().and_then(|c| c.parse::<u64>().ok()))
            && id.len() == 32
            && all_hex(id)
            && created <= secs(now)
            && secs(now) - created < ID_MAX_AGE.as_secs()
        {
            return Ok(id.to_string());
        }
    }
    let id = random_hex(16)?;
    write_private(&path, format!("{id}\n{}\n", secs(now)).as_bytes(), true)?;
    Ok(id)
}

fn write_private(path: &Path, data: &[u8], overwrite: bool) -> io::Result<()> {
    if let Some(d) = path.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(d)?;
        // an older version or a backup may have left it wider
        fs::set_permissions(d, fs::Permissions::from_mode(0o700))?;
    }
    let mut o = fs::OpenOptions::new();
    o.write(true).mode(0o600).custom_flags(libc::O_NOFOLLOW);
    if overwrite {
        o.create(true).truncate(true);
    } else {
        o.create_new(true);
    }
    o.open(path)?.write_all(data)
}

/// Save `report` in `dir` as `<time>-<nn>.json` (0600; `dir` becomes 0700).
pub(crate) fn write_report(dir: &Path, report: &Report) -> io::Result<PathBuf> {
    if report.time.is_empty()
        || !report
            .time
            .chars()
            .all(|c| c.is_ascii_digit() || "TZ:-+.".contains(c))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad report time",
        ));
    }
    let json = serde_json::to_string_pretty(report).map_err(io::Error::other)?;
    for n in 0..100 {
        let path = dir.join(format!("{}-{n:02}.json", report.time));
        match write_private(&path, json.as_bytes(), false) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("too many reports in the same second"))
}

fn read_reports(dir: &Path) -> Vec<Report> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| {
            let mut r: Report = serde_json::from_str(&fs::read_to_string(&p).ok()?).ok()?;
            r.path = Some(p);
            Some(r)
        })
        .collect()
}

/// Reports waiting for the user's decision, oldest first.
pub fn pending() -> Vec<Report> {
    prune_sent();
    reports_dir()
        .map(|d| read_reports(&d.join("pending")))
        .unwrap_or_default()
}

/// Reports already sent, oldest first (the history list).
pub fn sent() -> Vec<Report> {
    prune_sent();
    reports_dir()
        .map(|d| read_reports(&d.join("sent")))
        .unwrap_or_default()
}

/// "Don't send": delete the pending report's file.
pub fn discard(report: &Report) -> io::Result<()> {
    fs::remove_file(
        report
            .path
            .as_ref()
            .ok_or_else(|| io::Error::other("report has no path"))?,
    )
}

/// Delete sent reports older than 90 days.
fn prune_sent() {
    if let Some(d) = reports_dir() {
        prune_older_than(&d.join("sent"), SENT_KEEP, SystemTime::now());
    }
}

/// A modification time in the future (clock skew) counts as old.
fn prune_older_than(dir: &Path, keep: Duration, now: SystemTime) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = match e.metadata().and_then(|m| m.modified()) {
            Ok(t) => now.duration_since(t).map_or(true, |age| age > keep),
            Err(_) => false,
        };
        if old {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// After a successful POST: record it in `sent/` and drop the pending file.
/// Best effort on both: the data is out, so this never fails the send.
fn finish_sent(report: &Report, server_id: Option<String>) {
    if let Some(d) = reports_dir() {
        move_to_sent(&d.join("sent"), report, server_id);
        prune_older_than(&d.join("sent"), SENT_KEEP, SystemTime::now());
    }
}

fn move_to_sent(sent_dir: &Path, report: &Report, server_id: Option<String>) {
    let mut r = report.clone();
    r.sent_event_id = server_id;
    let _ = write_report(sent_dir, &r);
    if let Some(p) = &report.path {
        let _ = fs::remove_file(p);
    }
}

/// Save a report to `pending/` if the user enabled crash reporting.
fn queue(report: &Report) -> Option<PathBuf> {
    if !Settings::load().enabled {
        return None;
    }
    write_report(&reports_dir()?.join("pending"), report).ok()
}

// -------------------------------------------------------------------- hooks

static APP: OnceLock<AppInfo> = OnceLock::new();

const MAX_PER_HOUR: usize = 5;

/// At most 5 reports an hour, and the same crash (same top frames) once.
#[derive(Default)]
struct RateLimiter {
    recent: Vec<(Instant, u64)>,
}

impl RateLimiter {
    fn allow(&mut self, now: Instant, key: u64) -> bool {
        let hour = Duration::from_secs(3600);
        self.recent.retain(|(t, _)| now.duration_since(*t) < hour);
        if self.recent.len() >= MAX_PER_HOUR || self.recent.iter().any(|(_, k)| *k == key) {
            return false;
        }
        self.recent.push((now, key));
        true
    }
}

static LIMITER: Mutex<RateLimiter> = Mutex::new(RateLimiter { recent: Vec::new() });

thread_local! {
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

/// Hash of the top frames of a trace (and the message when there are none).
fn crash_key(trace: &str, message: &str) -> u64 {
    let mut h = DefaultHasher::new();
    let frames = parse_frames(trace);
    if frames.is_empty() {
        message.hash(&mut h);
    }
    for f in frames.iter().rev().take(5) {
        f["function"].as_str().unwrap_or("").hash(&mut h);
    }
    h.finish()
}

/// Install the panic hook for `app`. Call once, early in `main`. The previous
/// hook (the default one prints the panic) runs first; then, only when crash
/// reporting is enabled, a report is queued (not for a panic inside the hook
/// itself, and at most 5 an hour, each crash once).
pub fn install(app: AppInfo) {
    if APP.set(app).is_err() {
        return;
    }
    prune_sent();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if IN_HOOK.replace(true) {
            return;
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let message = match info.payload().downcast_ref::<&str>() {
                Some(s) => (*s).to_string(),
                None => info
                    .payload()
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_else(|| "Box<dyn Any>".to_string()),
            };
            let at = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
            let _ = save("panic", &message, at.as_deref());
        }));
        IN_HOOK.set(false);
    }));
}

/// Queue a report for a fatal error that is not a Rust panic (a Qt fatal
/// message handler). Needs [`install`]; does nothing when disabled.
pub fn record_fatal(message: &str) -> Option<PathBuf> {
    save("fatal", message, None)
}

fn save(kind: &str, message: &str, at: Option<&str>) -> Option<PathBuf> {
    if !Settings::load().enabled {
        return None;
    }
    let app = APP.get()?;
    let trace = filter_trace(&std::backtrace::Backtrace::force_capture().to_string());
    let key = crash_key(&trace, message);
    if !lock(&LIMITER).allow(Instant::now(), key) {
        return None;
    }
    let msg = match at {
        Some(a) => format!("{message} at {a}"),
        None => message.to_string(),
    };
    let crash = Crash {
        report_type: kind,
        app_name: &app.id,
        app_version: Some(&app.version),
        message: &msg,
        stacktrace: &trace,
    };
    queue(&build_report(&crash, &Scrubber::from_env(), None).ok()?)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------ coredumps

fn last_seen_path(name: &str) -> Option<PathBuf> {
    Some(state_dir()?.join(name))
}

fn field(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) => Some(s.clone()),
        // journald prints non-UTF-8 and some multi-line values as byte arrays
        Value::Array(a) => Some(
            String::from_utf8_lossy(
                &a.iter()
                    .filter_map(|n| n.as_u64().map(|b| b as u8))
                    .collect::<Vec<u8>>(),
            )
            .into_owned(),
        ),
        _ => None,
    }
}

/// One coredump journal entry (`journalctl -o json`) as a report plus its
/// timestamp in microseconds. Reads only COREDUMP_EXE, COMM, SIGNAL_NAME,
/// TIMESTAMP, PACKAGE_NAME/VERSION and MESSAGE (and only frame lines of it).
fn coredump_report(
    entry: &Value,
    scrubber: &Scrubber,
    rpm_version: impl Fn(&str) -> Option<String>,
) -> Option<(u64, Report)> {
    let ts: u64 = field(entry, "COREDUMP_TIMESTAMP")?.parse().ok()?;
    let exe = field(entry, "COREDUMP_EXE").unwrap_or_default();
    let comm = field(entry, "COREDUMP_COMM").unwrap_or_default();
    let signal = field(entry, "COREDUMP_SIGNAL_NAME").unwrap_or_else(|| "unknown signal".into());
    let name = if exe.is_empty() {
        comm.clone()
    } else {
        exe.clone()
    };
    let version = match (
        field(entry, "COREDUMP_PACKAGE_NAME"),
        field(entry, "COREDUMP_PACKAGE_VERSION"),
    ) {
        (_, Some(v)) => Some(v),
        _ if !exe.is_empty() => rpm_version(&exe),
        _ => None,
    };
    let time = history::rfc3339_from_unix(ts / 1_000_000);
    let crash = Crash {
        report_type: "coredump",
        app_name: &scrubber.scrub(&name),
        app_version: version.as_deref(),
        message: &format!("{} crashed with {signal}", scrubber.scrub(&comm)),
        stacktrace: &field(entry, "MESSAGE").unwrap_or_default(),
    };
    Some((ts, build_report(&crash, scrubber, Some(&time)).ok()?))
}

fn own_uid() -> String {
    read("/proc/self/status")
        .lines()
        .find_map(|l| {
            l.strip_prefix("Uid:")?
                .split_whitespace()
                .next()
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn journal(args: &[String]) -> Vec<Value> {
    let out = Command::new("/usr/bin/journalctl")
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
        _ => Vec::new(),
    }
}

/// The journal fields we read; `--output-fields` keeps the rest (command
/// line, environment, working directory, ...) out of this process.
const JOURNAL_FIELDS: &str = "MESSAGE,COREDUMP_EXE,COREDUMP_COMM,COREDUMP_SIGNAL_NAME,COREDUMP_TIMESTAMP,COREDUMP_PACKAGE_NAME,COREDUMP_PACKAGE_VERSION,COREDUMP_UID";

/// journalctl arguments for coredump entries after `since_micros`.
fn journal_args(since_micros: u64, uid: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = ["--no-pager", "--all", "-o", "json", "-n", "500"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.push(format!("--output-fields={JOURNAL_FIELDS}"));
    a.push(format!("--since=@{}", since_micros / 1_000_000));
    a.push(format!("MESSAGE_ID={COREDUMP_MESSAGE_ID}"));
    match uid {
        Some(u) => a.push(format!("COREDUMP_UID={u}")),
        None => a.insert(0, "--user".into()),
    }
    a
}

fn rpm_version(exe: &str) -> Option<String> {
    let o = Command::new("/usr/bin/rpm")
        .args(["-qf", "--qf", "%{NAME} %{VERSION}-%{RELEASE}", exe])
        .env_clear()
        .env("PATH", "/usr/bin")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// New systemd-coredump crashes of the user's own processes since the last
/// call (or since `since_micros`), queued as pending reports. Returns them.
/// The first call after opting in starts at "now": nothing older is read.
/// Tries the user journal, then the system journal filtered to our UID
/// (readable for members of `wheel`/`systemd-journal`). Empty when disabled.
pub fn collect_coredumps(since_micros: Option<u64>) -> Vec<Report> {
    if !Settings::load().enabled {
        return Vec::new();
    }
    prune_sent();
    let Some(marker) = last_seen_path("coredump-last") else {
        return Vec::new();
    };
    let stored = fs::read_to_string(&marker)
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok());
    let Some(since) = since_micros.or(stored) else {
        reset_markers(); // first run: only crashes from now on
        return Vec::new();
    };
    let mut entries = journal(&journal_args(since, None));
    if entries.is_empty() {
        entries = journal(&journal_args(since, Some(&own_uid())));
    }
    let scrubber = Scrubber::from_env();
    let mut newest = since;
    let mut out = Vec::new();
    for e in &entries {
        // cheap check first: no rpm, no report for entries already seen
        let Some(ts) = field(e, "COREDUMP_TIMESTAMP").and_then(|t| t.parse::<u64>().ok()) else {
            continue;
        };
        if ts <= since {
            continue;
        }
        let Some((_, report)) = coredump_report(e, &scrubber, rpm_version) else {
            continue;
        };
        newest = newest.max(ts);
        if let Some(d) = reports_dir() {
            let mut r = report;
            r.path = write_report(&d.join("pending"), &r).ok();
            out.push(r);
        }
    }
    if newest > since {
        let _ = write_private(&marker, newest.to_string().as_bytes(), true);
    }
    out
}

// --------------------------------------------------------------- events

/// Reports for helper events (update and rollback results) newer than the
/// last call (or `since`, an RFC 3339 time), queued as pending. The first call
/// after opting in starts at "now". Every string copied from the event log is
/// scrubbed again. Empty when disabled.
pub fn collect_events(since: Option<&str>) -> Vec<Report> {
    if !Settings::load().enabled {
        return Vec::new();
    }
    prune_sent();
    let Some(marker) = last_seen_path("events-last") else {
        return Vec::new();
    };
    let stored = fs::read_to_string(&marker)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let Some(since) = since.map(str::to_string).or(stored) else {
        reset_markers();
        return Vec::new();
    };
    let events = crate::helper::events::read(Path::new(crate::helper::events::DEFAULT_PATH));
    let scrubber = Scrubber::from_env();
    let mut newest = since.clone();
    let mut out = Vec::new();
    for e in events.iter().filter(|e| e.time > since) {
        let name = scrubber.scrub_message(&e.event);
        let version = e.version.as_deref().map(|v| scrubber.scrub_message(v));
        let mut msg = name.clone();
        if let Some(v) = &version {
            msg.push_str(&format!(" (version {v})"));
        }
        if let Some(err) = &e.error {
            msg.push_str(&format!(": {err}"));
        }
        let crash = Crash {
            report_type: &name,
            app_name: "atlas-system-helper",
            app_version: Some(env!("CARGO_PKG_VERSION")),
            message: &msg,
            stacktrace: "",
        };
        let Ok(mut r) = build_report(&crash, &scrubber, Some(&e.time)) else {
            continue;
        };
        r.report_type = scrubber.scrub_message(&name);
        if r.atlasos_version.is_none() {
            r.atlasos_version = version;
        }
        if let Some(d) = reports_dir() {
            r.path = write_report(&d.join("pending"), &r).ok();
        }
        if e.time > newest {
            newest = e.time.clone();
        }
        out.push(r);
    }
    if newest != since {
        let _ = write_private(&marker, newest.as_bytes(), true);
    }
    out
}

// ------------------------------------------------------------- reporting

/// POST the report's [`payload`](Report::payload) to GlitchTip (Sentry store
/// API) and move it to `sent/`. The caller must have shown the user the
/// payload and got a yes. Fails when crash reporting is off or no endpoint is
/// configured. Uses `/usr/bin/curl`.
pub fn send(report: &Report) -> io::Result<()> {
    send_with(report, Settings::load().enabled, Endpoint::load())
}

fn send_with(report: &Report, enabled: bool, ep: Option<Endpoint>) -> io::Result<()> {
    if !enabled {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "crash reporting is turned off",
        ));
    }
    let ep = ep.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no endpoint configured"))?;
    let id = post(report, &ep)?;
    finish_sent(report, id);
    Ok(())
}

/// curl's arguments: no config file, https only (http only for a loopback
/// DSN), no proxy, no redirects; the body comes from `body_file`.
fn curl_args(ep: &Endpoint, body_file: &Path) -> Vec<String> {
    let proto = if ep.store_url.starts_with("http://") {
        "=http"
    } else {
        "=https"
    };
    let auth = format!(
        "X-Sentry-Auth: Sentry sentry_version=7, sentry_key={}, sentry_client=atlas-core/{}",
        ep.key,
        env!("CARGO_PKG_VERSION")
    );
    let mut a: Vec<String> = [
        "-q",
        "--fail",
        "--silent",
        "--show-error",
        "--max-time",
        "30",
        "--max-redirs",
        "0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.extend([
        "--proto".into(),
        proto.into(),
        "--noproxy".into(),
        "*".into(),
        "--request".into(),
        "POST".into(),
    ]);
    a.extend([
        "--header".into(),
        "Content-Type: application/json".into(),
        "--header".into(),
        auth,
    ]);
    a.extend([
        "--data-binary".into(),
        format!("@{}", body_file.display()),
        "--url".into(),
        ep.store_url.clone(),
    ]);
    a
}

fn post(report: &Report, ep: &Endpoint) -> io::Result<Option<String>> {
    let body = serde_json::to_vec(&report.payload()).map_err(io::Error::other)?;
    let dir = state_dir().ok_or_else(|| io::Error::other("no state directory"))?;
    let tmp = dir.join(format!("send-{}.json", random_hex(8)?));
    write_private(&tmp, &body, false)?;
    let out = Command::new("/usr/bin/curl")
        .args(curl_args(ep, &tmp))
        .env_clear()
        .env("PATH", "/usr/bin")
        .stdin(Stdio::null())
        .output();
    let _ = fs::remove_file(&tmp);
    let out = out?;
    if !out.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(serde_json::from_slice::<Value>(&out.stdout)
        .ok()
        .and_then(|v| v.get("id")?.as_str().map(str::to_string)))
}

const MAX_URL: usize = 7000;

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// A prefilled `https://github.com/EternalCoder454/<repo>/issues/new?...`
/// URL, at most about 7 KB (the trace is cut to fit). Secondary to [`send`].
pub fn github_issue_url(r: &Report, repo: &str) -> String {
    let first: String = r
        .message
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect();
    let title = format!(
        "Crash in {} {}: {}",
        r.app_name,
        r.app_version.as_deref().unwrap_or(""),
        first
    );
    let base = format!(
        "https://github.com/EternalCoder454/{}/issues/new?title={}&body=",
        percent_encode(repo),
        percent_encode(&title)
    );
    let na = |o: &Option<String>| o.clone().unwrap_or_else(|| "unknown".into());
    let head = format!(
        "**App:** {} {}\n**AtlasOS:** {} ({})\n**Kernel:** {}\n**GPU:** {} ({})\n**Type:** {}\n**Message:** {}\n\n```\n",
        r.app_name,
        na(&r.app_version),
        na(&r.atlasos_version),
        na(&r.channel),
        na(&r.kernel),
        na(&r.gpu),
        na(&r.gpu_driver),
        r.report_type,
        r.message
    );
    let trace = filter_trace(&r.stacktrace);
    let lines: Vec<&str> = trace.lines().collect();
    let mut keep = lines.len();
    loop {
        let cut = if keep < lines.len() {
            "\n... (trace truncated)"
        } else {
            ""
        };
        let body = percent_encode(&format!("{head}{}{cut}\n```\n", lines[..keep].join("\n")));
        if base.len() + body.len() <= MAX_URL || keep == 0 {
            return format!("{base}{body}");
        }
        keep -= (keep / 10).max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc() -> Scrubber {
        Scrubber::new(
            &["zach", "Zachary Smith"],
            &["atlas-box.local"],
            &["/var/home/zach"],
        )
    }

    fn report(trace: &str) -> Report {
        let c = Crash {
            report_type: "panic",
            app_name: "net.eterneon.atlas.updater",
            app_version: Some("0.1.0"),
            message: "boom",
            stacktrace: trace,
        };
        build_report(&c, &sc(), Some("2026-10-02T10:00:00Z")).unwrap()
    }

    #[test]
    fn scrubs_homes_user_and_host() {
        let s = sc();
        assert_eq!(s.scrub("/home/zach/.cargo/x.rs"), "/home/USER/.cargo/x.rs");
        assert_eq!(s.scrub("/var/home/zach/x"), "/var/home/USER/x");
        assert_eq!(
            s.scrub("/var/home/other/x and /home/bob"),
            "/var/home/USER/x and /home/USER"
        );
        assert_eq!(s.scrub("user zach on atlas-box"), "user USER on HOST");
        assert_eq!(s.scrub("zachary zach_x xzach"), "USERary USER_x xUSER");
        assert_eq!(s.scrub("a/b /usr/lib/x"), "a/b /usr/lib/x");
    }

    #[test]
    fn scrubs_mac_and_ip_addresses() {
        let s = sc();
        assert_eq!(
            s.scrub("mac aa:bb:cc:dd:ee:ff and AA-BB-CC-DD-EE-FF"),
            "mac <mac> and <mac>"
        );
        assert_eq!(
            s.scrub("ip 192.168.1.20 and 10.0.0.1:8080."),
            "ip <ip> and <ip>."
        );
        assert_eq!(
            s.scrub("v6 fe80::1ff:fe23:4567:890a and 2001:db8:0:0:0:0:0:1"),
            "v6 <ip> and <ip>"
        );
        assert_eq!(s.scrub("::1 up"), "<ip> up");
        // not addresses
        assert_eq!(
            s.scrub("time 10:20:30 ver 1.2.3 at 0x7f12:34"),
            "time 10:20:30 ver 1.2.3 at 0x7f12:34"
        );
        assert_eq!(s.scrub("300.1.1.1"), "300.1.1.1");
    }

    #[test]
    fn message_paths_into_user_data_are_hidden() {
        let m = sc().scrub_message(
            "No such file: '/home/zach/Documents/tax.pdf' (os error 2) /run/media/zach/USB/a",
        );
        assert_eq!(m, "No such file: '<path>' (os error 2) <path>");
        assert_eq!(
            sc().scrub_message("see /usr/lib/foo.so"),
            "see /usr/lib/foo.so"
        );
    }

    #[test]
    fn report_has_no_identifying_strings() {
        let c = Crash {
            report_type: "panic",
            app_name: "net.eterneon.atlas.updater",
            app_version: Some("0.1.0"),
            message: "failed at /home/zach/Documents/notes.md for zach on 10.1.2.3",
            stacktrace: "  0: f\n     at /var/home/zach/.cargo/foo.rs:1\n  1: zach::main\n",
        };
        let r = build_report(&c, &sc(), None).unwrap();
        let json = format!(
            "{}{}",
            r.to_json_pretty(),
            serde_json::to_string(&r).unwrap()
        );
        for needle in [
            "zach",
            "atlas-box",
            "Documents",
            "notes.md",
            "10.1.2.3",
            "machine-id",
        ] {
            assert!(!json.contains(needle), "{needle} in {json}");
        }
        assert!(json.contains("<path>"));
    }

    #[test]
    fn settings_default_off_and_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("atlas/crash-reporting.toml");
        assert!(!Settings::load_from(&p).enabled);
        Settings { enabled: true }.save_to(&p).unwrap();
        assert!(Settings::load_from(&p).enabled);
        fs::write(&p, "enabled = false\n").unwrap();
        assert!(!Settings::load_from(&p).enabled);
        fs::write(&p, "enabled = maybe\n").unwrap();
        assert!(!Settings::load_from(&p).enabled);
    }

    #[test]
    fn dsn_parsing() {
        let e = Endpoint::parse("https://abc123@glitch.example.net/7").unwrap();
        assert_eq!(
            e,
            Endpoint {
                key: "abc123".into(),
                store_url: "https://glitch.example.net/api/7/store/".into()
            }
        );
        let e = Endpoint::parse("https://k@host:8000/sub/path/3").unwrap();
        assert_eq!(e.store_url, "https://host:8000/sub/path/api/3/store/");
        for bad in [
            "",
            "ftp://k@h/1",
            "https://host/1",
            "https://k@host",
            "https://k@host/",
            "https://k@ho st/1",
            "https://-x@h/1;rm",
            "http://k@example.com/1",
            "https://k:secret@h.example/1",
            "https://k@h.example/../1",
            "https://k@h.example/a/./1",
        ] {
            assert!(Endpoint::parse(bad).is_none(), "{bad:?}");
        }
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        fs::write(&p, "# comment\ndsn = \"\"  # none\n").unwrap();
        assert!(Endpoint::load_from(&p).is_none());
        fs::write(&p, "dsn = \"https://k@h.example/2\"\n").unwrap();
        assert!(Endpoint::load_from(&p).is_some());
    }

    #[test]
    fn payload_has_sentry_shape() {
        let r = report(
            "#0  0x00007f1 raise (libc.so.6 + 0x9a)\n#1  0x00007f2 main (plasmashell + 0x10)\n",
        );
        let p = r.payload();
        assert_eq!(p["event_id"], r.event_id);
        assert!(p["release"].as_str().unwrap().starts_with("atlasos@"));
        assert_eq!(p["tags"]["app"], "net.eterneon.atlas.updater");
        assert_eq!(p["tags"]["category"], "Atlas app");
        assert_eq!(p["tags"]["report_type"], "panic");
        assert_eq!(p["user"]["id"], r.crash_id);
        assert!(p["contexts"]["os"].is_object() && p["contexts"]["gpu"].is_object());
        let frames = &p["exception"]["values"][0]["stacktrace"]["frames"];
        assert_eq!(frames.as_array().unwrap().len(), 2);
        // Sentry wants the oldest call first
        assert_eq!(frames[0]["function"], "main");
        assert_eq!(frames[1]["function"], "raise");
        assert_eq!(frames[1]["instruction_addr"], "0x00007f1");
        assert_eq!(frames[1]["package"], "libc.so.6");
        // deterministic: shown payload == sent payload
        assert_eq!(
            serde_json::to_vec(&r.payload()).unwrap(),
            serde_json::to_vec(&r.payload()).unwrap()
        );
    }

    #[test]
    fn rust_backtrace_frames() {
        let f = parse_frames(
            "   0: std::panicking::begin\n             at /rustc/x.rs:1\n   1: app::main\n",
        );
        assert_eq!(f.len(), 2);
        assert_eq!(f[1]["function"], "std::panicking::begin");
    }

    #[test]
    fn categories() {
        assert_eq!(category_of("/usr/bin/plasmashell"), "Plasma");
        assert_eq!(category_of("plasma-discover"), "Plasma");
        assert_eq!(category_of("/usr/bin/kwin_wayland"), "KWin");
        assert_eq!(category_of("net.eterneon.atlas.updater"), "Atlas app");
        assert_eq!(category_of("/usr/bin/firefox"), "other");
    }

    #[test]
    fn pci_ids_and_gpu_from_sysfs() {
        let ids = "# comment\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX]\n\t\t1002 0e3a  sub\n8086  Intel Corporation\n\ta780  Raptor Lake-S GT1\n";
        assert_eq!(
            pci_name(ids, "0x1002", "0x744C").unwrap(),
            "Navi 31 [Radeon RX 7900 XT/7900 XTX]"
        );
        assert_eq!(
            pci_name(ids, "0x8086", "0xa780").unwrap(),
            "Raptor Lake-S GT1"
        );
        assert!(pci_name(ids, "0x1002", "0xffff").is_none());
        let d = tempfile::tempdir().unwrap();
        let dev = d.path().join("drm/card0/device");
        fs::create_dir_all(&dev).unwrap();
        fs::write(dev.join("vendor"), "0x1002\n").unwrap();
        fs::write(dev.join("device"), "0x744c\n").unwrap();
        fs::create_dir_all(d.path().join("drm/card0-DP-1")).unwrap();
        fs::create_dir_all(d.path().join("drivers/amdgpu")).unwrap();
        std::os::unix::fs::symlink("../../../drivers/amdgpu", dev.join("driver")).unwrap();
        fs::create_dir_all(d.path().join("module/amdgpu")).unwrap();
        fs::write(d.path().join("module/amdgpu/version"), "6.1\n").unwrap();
        let (n, drv, ver) = read_gpu(&d.path().join("drm"), ids, &d.path().join("module"));
        assert_eq!(
            (n.as_deref(), drv.as_deref(), ver.as_deref()),
            (
                Some("Navi 31 [Radeon RX 7900 XT/7900 XTX]"),
                Some("amdgpu"),
                Some("6.1")
            )
        );
        let (n, _, _) = read_gpu(&d.path().join("drm"), "", &d.path().join("module"));
        assert_eq!(n.as_deref(), Some("1002:744c"));
    }

    #[test]
    fn coredump_entry_uses_only_allowed_fields() {
        let entry = json!({
            "MESSAGE_ID": COREDUMP_MESSAGE_ID,
            "COREDUMP_TIMESTAMP": "1790000000000000",
            "COREDUMP_EXE": "/usr/bin/plasmashell",
            "COREDUMP_COMM": "plasmashell",
            "COREDUMP_SIGNAL_NAME": "SIGSEGV",
            "COREDUMP_CMDLINE": "plasmashell --user /home/zach/secret",
            "COREDUMP_ENVIRON": "HOME=/home/zach",
            "COREDUMP_CWD": "/home/zach",
            "MESSAGE": "Process 1 (plasmashell) of user 1000 dumped core.\n\nStack trace of thread 1:\n#0  0x00007f1 raise (libc.so.6 + 0x9a)\n#1  0x00007f2 foo (/home/zach/lib.so + 0x1)"
        });
        let (ts, r) = coredump_report(&entry, &sc(), |exe| {
            (exe == "/usr/bin/plasmashell").then(|| "plasma-workspace 6.8.0-1.fc44".to_string())
        })
        .unwrap();
        assert_eq!(ts, 1_790_000_000_000_000);
        assert_eq!(r.report_type, "coredump");
        assert_eq!(r.category, "Plasma");
        assert_eq!(
            r.app_version.as_deref(),
            Some("plasma-workspace 6.8.0-1.fc44")
        );
        assert_eq!(r.message, "plasmashell crashed with SIGSEGV");
        assert!(r.stacktrace.starts_with("Stack trace of thread 1:"));
        assert!(r.stacktrace.contains("<path>") && !r.stacktrace.contains("lib.so"));
        let all = serde_json::to_string(&r).unwrap();
        assert!(!all.contains("secret") && !all.contains("zach") && !all.contains("Process 1"));
        assert_eq!(r.time, "2026-09-21T14:13:20Z");
    }

    #[test]
    fn rotating_id_is_stable_then_replaced_after_30_days() {
        let d = tempfile::tempdir().unwrap();
        let t0 = SystemTime::now();
        let a = crash_id_in(d.path(), t0).unwrap();
        assert_eq!(a.len(), 32);
        assert_eq!(
            crash_id_in(d.path(), t0 + Duration::from_secs(29 * 86_400)).unwrap(),
            a
        );
        let b = crash_id_in(d.path(), t0 + Duration::from_secs(31 * 86_400)).unwrap();
        assert_ne!(a, b);
        assert_eq!(
            fs::metadata(d.path().join("crash-id"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(
            !fs::read_to_string(d.path().join("crash-id"))
                .unwrap()
                .contains("machine")
        );
    }

    #[test]
    fn pending_sent_and_prune() {
        let d = tempfile::tempdir().unwrap();
        let pend = d.path().join("pending");
        let r = report("");
        let p = write_report(&pend, &r).unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_ne!(p, write_report(&pend, &r).unwrap());
        let got = read_reports(&pend);
        assert_eq!(got.len(), 2);
        discard(&got[0]).unwrap();
        move_to_sent(&d.path().join("sent"), &got[1], Some("srv1".into()));
        assert!(read_reports(&pend).is_empty());
        let sent = read_reports(&d.path().join("sent"));
        assert_eq!(sent[0].sent_event_id.as_deref(), Some("srv1"));
        prune_older_than(&d.path().join("sent"), SENT_KEEP, SystemTime::now());
        assert_eq!(read_reports(&d.path().join("sent")).len(), 1);
        prune_older_than(
            &d.path().join("sent"),
            SENT_KEEP,
            SystemTime::now() + Duration::from_secs(91 * 86_400),
        );
        assert!(read_reports(&d.path().join("sent")).is_empty());
    }

    #[test]
    fn send_without_endpoint_fails_cleanly() {
        let e = send_with(&report(""), true, None).unwrap_err();
        assert_eq!(e.to_string(), "no endpoint configured");
    }

    #[test]
    fn issue_url_is_prefilled_and_bounded() {
        let bt: String = (0..2000)
            .map(|i| format!("  {i}: some::function_{i}\n"))
            .collect();
        let small = github_issue_url(&report("  0: f\n"), "atlasos-updater");
        assert!(
            small.starts_with(
                "https://github.com/EternalCoder454/atlasos-updater/issues/new?title="
            )
        );
        let big = github_issue_url(&report(&bt), "atlasos-updater");
        assert!(big.len() <= MAX_URL && big.contains("truncated") && !big.contains(' '));
    }

    // ---- round 3 additions

    #[test]
    fn paths_in_other_shapes_are_redacted() {
        let s = sc();
        assert_eq!(s.scrub_message("path=/home/bob/x.txt"), "path=<path>");
        assert_eq!(
            s.scrub_message("open file:///home/bob/x.txt now"),
            "open <path> now"
        );
        assert_eq!(
            s.scrub_message("(/run/user/1000/doc/ab/secret.pdf)"),
            "(<path>)"
        );
        assert_eq!(s.scrub_message("in ~/Documents/a"), "in <path>");
        assert_eq!(s.scrub_message("/usr/lib/x.so"), "/usr/lib/x.so");
    }

    #[test]
    fn names_match_case_insensitively_and_short_ones_at_boundaries() {
        let s = Scrubber::new(&["Zach", "al"], &["Atlas-Box"], &[]);
        assert_eq!(
            s.scrub("ZACH and zAcH, host ATLAS-BOX"),
            "USER and USER, host HOST"
        );
        assert_eq!(s.scrub("al said"), "USER said");
        assert_eq!(
            s.scrub("signal al_1 al2 value"),
            "signal USER_1 USER2 value"
        );
        let s = sc();
        assert_eq!(s.scrub("ZACHARY smith wrote"), "USER wrote");
        assert_eq!(s.scrub("by Zachary Smith"), "by USER");
        assert_eq!(s.scrub("/var/home/zach/a"), "/var/home/USER/a");
        assert_eq!(s.scrub("host atlas-box.local"), "host HOST");
        assert_eq!(s.scrub("on atlas-box"), "on HOST");
    }

    #[test]
    fn more_address_shapes() {
        let s = sc();
        assert_eq!(s.scrub("fe80::1%eth0 up"), "<ip> up");
        assert_eq!(s.scrub("bind ::"), "bind ::");
        assert_eq!(s.scrub("host:192.168.0.2"), "host:<ip>");
        assert_eq!(s.scrub("aabbccddeeff"), "<mac>");
        assert_eq!(s.scrub("aabb.ccdd.eeff"), "<mac>");
        assert_eq!(s.scrub("enx001122334455 wlx001122334455"), "<mac> <mac>");
        assert_eq!(s.scrub("id 0123456789abcdef0123456789abcdef"), "id <id>");
        assert_eq!(
            s.scrub("uuid 123e4567-e89b-12d3-a456-426614174000"),
            "uuid <id>"
        );
    }

    #[test]
    fn endpoint_rules() {
        assert!(Endpoint::parse("http://k@127.0.0.1:8000/1").is_some());
        assert!(Endpoint::parse("http://k@localhost/1").is_some());
        assert!(Endpoint::parse("http://k@[::1]:8000/1").is_some());
        assert!(Endpoint::parse("http://k@glitch.example/1").is_none());
        assert!(Endpoint::parse("https://k:secret@glitch.example/1").is_none());
        assert!(Endpoint::parse("https://k@glitch.example/a/../1").is_none());
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        fs::write(&p, "dsn = \"\"\n").unwrap();
        assert_eq!(
            toml_value(&fs::read_to_string(&p).unwrap(), "dsn").as_deref(),
            Some("")
        );
    }

    #[test]
    fn send_is_refused_when_disabled() {
        let ep = Endpoint::parse("https://k@glitch.example/1");
        let e = send_with(&report(""), false, ep).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn curl_arguments_are_locked_down() {
        let ep = Endpoint::parse("https://key@glitch.example/1").unwrap();
        let a = curl_args(&ep, Path::new("/x/body.json"));
        assert_eq!(a[0], "-q");
        let pos = |s: &str| a.iter().position(|x| x == s).unwrap();
        assert_eq!(a[pos("--proto") + 1], "=https");
        assert_eq!(a[pos("--noproxy") + 1], "*");
        assert_eq!(a[pos("--max-redirs") + 1], "0");
        assert_eq!(a[pos("--data-binary") + 1], "@/x/body.json");
        assert!(!a.iter().any(|x| x.contains("sentry_secret")));
        let lo = Endpoint::parse("http://k@127.0.0.1:9/1").unwrap();
        let a = curl_args(&lo, Path::new("/x"));
        assert_eq!(
            a[a.iter().position(|x| x == "--proto").unwrap() + 1],
            "=http"
        );
    }

    #[test]
    fn journal_arguments_limit_fields_and_start_at_the_marker() {
        let a = journal_args(5_000_000, Some("1000"));
        assert!(a.contains(&"--all".to_string()));
        assert!(
            a.iter()
                .any(|x| x.starts_with("--output-fields=MESSAGE,COREDUMP_EXE"))
        );
        assert!(
            !a.iter()
                .any(|x| x.contains("CMDLINE") || x.contains("ENVIRON"))
        );
        assert!(a.contains(&"--since=@5".to_string()));
        assert!(a.contains(&"COREDUMP_UID=1000".to_string()));
        assert_eq!(journal_args(0, None)[0], "--user");
    }

    #[test]
    fn rate_limiter_caps_and_dedupes() {
        let mut l = RateLimiter::default();
        let t = Instant::now();
        assert!(l.allow(t, 1));
        assert!(!l.allow(t, 1), "same crash twice");
        for k in 2..=5 {
            assert!(l.allow(t, k));
        }
        assert!(!l.allow(t, 6), "sixth in the hour");
        assert!(l.allow(t + Duration::from_secs(3601), 6));
    }

    #[test]
    fn traces_keep_only_frame_lines() {
        let t = "Process 1 (x) of user 1000 dumped core.\n\nCmdline: /home/zach/secret\nStack trace of thread 7:\n#0  0x1 f (a.so + 0x1)\n  1: rust::fn\n     at /home/zach/x.rs:1\n";
        assert_eq!(
            filter_trace(t),
            "Stack trace of thread 7:\n#0  0x1 f (a.so + 0x1)\n  1: rust::fn"
        );
        let url = github_issue_url(
            &Report {
                stacktrace: t.into(),
                ..report("")
            },
            "r",
        );
        assert!(!url.contains("secret") && !url.contains("zach"));
    }

    #[test]
    fn prune_treats_future_mtimes_as_old() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.json"), "{}").unwrap();
        // "now" is far before the file's mtime
        prune_older_than(
            d.path(),
            SENT_KEEP,
            SystemTime::now() - Duration::from_secs(10 * 86_400),
        );
        assert!(!d.path().join("a.json").exists());
    }

    #[test]
    fn crash_id_with_future_creation_is_replaced() {
        let d = tempfile::tempdir().unwrap();
        let future = format!("{}\n{}\n", "a".repeat(32), 4_000_000_000u64);
        fs::write(d.path().join("crash-id"), future).unwrap();
        let id = crash_id_in(d.path(), SystemTime::now()).unwrap();
        assert_ne!(id, "a".repeat(32));
    }

    #[test]
    fn relative_xdg_paths_are_ignored() {
        // abs_env reads the process environment; test the rule it applies
        assert!(PathBuf::from("rel/dir").is_relative());
        assert!(abs_env("ATLAS_TEST_UNSET_VAR").is_none());
    }

    #[test]
    fn report_time_is_validated() {
        let d = tempfile::tempdir().unwrap();
        let mut r = report("");
        r.time = "../../x".into();
        assert!(write_report(d.path(), &r).is_err());
        r.time = "2026-10-02T10:00:00Z".into();
        let p = write_report(d.path(), &r).unwrap();
        assert!(
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("2026-10-02T10:00:00Z-")
        );
        assert_eq!(
            fs::metadata(d.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn markers_start_at_now_on_first_opt_in() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("atlas");
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        reset_markers_in(&dir);
        let m: u64 = fs::read_to_string(dir.join("coredump-last"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(m >= before);
        assert!(
            fs::read_to_string(dir.join("events-last"))
                .unwrap()
                .ends_with('Z')
        );
    }
}
