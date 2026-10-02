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

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

fn config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(v) => Some(PathBuf::from(v)),
        None => {
            Some(PathBuf::from(std::env::var_os("HOME").filter(|v| !v.is_empty())?).join(".config"))
        }
    }
}

fn state_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(v) => Some(PathBuf::from(v)),
        None => Some(
            PathBuf::from(std::env::var_os("HOME").filter(|v| !v.is_empty())?).join(".local/state"),
        ),
    }
}

/// `key = "value"` / `key = true` lookup in a tiny TOML subset.
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

    pub fn save(&self) -> io::Result<()> {
        let p = Self::path().ok_or_else(|| io::Error::other("no config directory"))?;
        self.save_to(&p)
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

impl Endpoint {
    /// The configured endpoint: `/etc/atlas/...` first, then the shipped
    /// default. `None` when no DSN is set (the default).
    pub fn load() -> Option<Endpoint> {
        [SYSTEM_CONFIG, DEFAULT_CONFIG]
            .iter()
            .find_map(|p| Self::load_from(Path::new(p)))
    }

    pub fn load_from(path: &Path) -> Option<Endpoint> {
        Self::parse(&toml_value(&fs::read_to_string(path).ok()?, "dsn")?)
    }

    pub fn parse(dsn: &str) -> Option<Endpoint> {
        let (scheme, rest) = dsn.trim().split_once("://")?;
        if scheme != "https" && scheme != "http" {
            return None;
        }
        let (key, rest) = rest.split_once('@')?;
        let (host, path) = rest.split_once('/')?;
        let (prefix, project) = path
            .trim_matches('/')
            .rsplit_once('/')
            .unwrap_or(("", path.trim_matches('/')));
        let ok = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c))
        };
        if !ok(key) || !ok(host) || !ok(project) {
            return None;
        }
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("/{prefix}")
        };
        Some(Endpoint {
            key: key.into(),
            store_url: format!("{scheme}://{host}{prefix}/api/{project}/store/"),
        })
    }
}

// ---------------------------------------------------------------- scrubbing

/// Replaces home directories, the username, the hostname, MAC and IP
/// addresses in strings.
#[derive(Debug, Clone, Default)]
pub struct Scrubber {
    user: Option<String>,
    host: Option<String>,
}

impl Scrubber {
    pub fn new(user: Option<&str>, host: Option<&str>) -> Self {
        let keep = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_string);
        Scrubber {
            user: keep(user),
            host: keep(host),
        }
    }

    /// From the process environment (only to know what to remove).
    pub fn from_env() -> Self {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .ok()
            .or_else(|| {
                let h = std::env::var("HOME").ok()?;
                Path::new(&h)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            });
        let host = fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .map(|h| h.trim().to_string());
        Scrubber::new(user.as_deref(), host.as_deref())
    }

    /// `/home/<name>` and `/var/home/<name>` become `.../USER`, the bare
    /// username `USER`, the hostname `HOST`; MAC and IP addresses go.
    pub fn scrub(&self, s: &str) -> String {
        let mut out = scrub_homes(s);
        if let Some(u) = &self.user {
            out = replace_token(&out, u, "USER");
        }
        if let Some(h) = &self.host {
            out = replace_token(&out, h, "HOST");
        }
        scrub_addresses(&out)
    }

    /// [`scrub`](Self::scrub), then hide paths that can name a file the user
    /// had open. For panic messages, which can quote anything.
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

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replace `needle` where it is not part of a longer word.
fn replace_token(s: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(needle) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + needle.len()..].chars().next();
        let (head, tail) = rest.split_at(i + needle.len());
        out.push_str(&head[..i]);
        if before.is_some_and(is_word) || after.is_some_and(is_word) {
            out.push_str(needle);
        } else {
            out.push_str(with);
        }
        rest = tail;
    }
    out.push_str(rest);
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

/// Replace MAC and IP address words with `<mac>` / `<ip>`.
fn scrub_addresses(s: &str) -> String {
    let delim = |c: char| c.is_whitespace() || "'\"`(),;[]{}<>=/".contains(c);
    let mut out = String::with_capacity(s.len());
    let mut tok = String::new();
    let flush = |tok: &mut String, out: &mut String| {
        // an address may carry a trailing sentence dot/colon or a :port
        let mut core = tok.trim_end_matches(['.', ':']);
        if let Some((h, p)) = core.rsplit_once(':')
            && is_ipv4(h)
            && !p.is_empty()
            && p.chars().all(|c| c.is_ascii_digit())
        {
            core = h;
        }
        if is_mac(core) || is_ipv4(core) || is_ipv6(core) {
            out.push_str(if is_mac(core) { "<mac>" } else { "<ip>" });
            out.push_str(&tok[core.len()..]);
        } else {
            out.push_str(tok);
        }
        tok.clear();
    };
    for c in s.chars() {
        if delim(c) {
            flush(&mut tok, &mut out);
            out.push(c);
        } else {
            tok.push(c);
        }
    }
    flush(&mut tok, &mut out);
    out
}

const PRIVATE_PREFIXES: &[&str] = &[
    "/home/",
    "/var/home/",
    "/media/",
    "/mnt/",
    "/run/media/",
    "/tmp/",
    "/var/tmp/",
];

/// Replace path-like words that point into user data with `<path>`.
fn redact_paths(s: &str) -> String {
    let is_delim = |c: char| c.is_whitespace() || "'\"`(),;[]{}".contains(c);
    let mut out = String::with_capacity(s.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        out.push_str(if PRIVATE_PREFIXES.iter().any(|p| token.starts_with(p)) {
            "<path>"
        } else {
            token
        });
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
pub fn os_info(
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
pub struct Crash<'a> {
    pub report_type: &'a str,
    pub app_name: &'a str,
    pub app_version: Option<&'a str>,
    pub message: &'a str,
    pub stacktrace: &'a str,
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = io::Read::read_exact(&mut f, &mut buf);
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Build a report for `crash` now. All strings are scrubbed. Does not check
/// [`Settings`]; callers do.
pub fn build_report(crash: &Crash, scrubber: &Scrubber, time: Option<&str>) -> Report {
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
    Report {
        schema: 2,
        event_id: random_hex(16),
        report_type: crash.report_type.to_string(),
        time: time.map_or_else(now_rfc3339, str::to_string),
        crash_id: crash_id(),
        atlasos_version,
        channel,
        previous_version,
        app_name: s(crash.app_name),
        app_version: crash.app_version.map(s),
        category: category_of(crash.app_name).to_string(),
        message: scrubber.scrub_message(crash.message),
        stacktrace: s(crash.stacktrace),
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
    }
}

// ----------------------------------------------------------------- storage

/// `$XDG_STATE_HOME/atlas`.
fn state_dir() -> Option<PathBuf> {
    Some(state_home()?.join("atlas"))
}

fn reports_dir() -> Option<PathBuf> {
    Some(state_dir()?.join("crash-reports"))
}

/// The rotating anonymous ID: random, replaced when 30 days old. There is no
/// permanent ID anywhere.
pub fn crash_id() -> String {
    state_dir().map_or_else(|| random_hex(16), |d| crash_id_in(&d, SystemTime::now()))
}

fn crash_id_in(dir: &Path, now: SystemTime) -> String {
    let path = dir.join("crash-id");
    let secs = |t: SystemTime| t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(text) = fs::read_to_string(&path) {
        let mut l = text.lines();
        if let (Some(id), Some(created)) = (l.next(), l.next().and_then(|c| c.parse::<u64>().ok()))
            && id.len() == 32
            && id.chars().all(|c| c.is_ascii_hexdigit())
            && secs(now).saturating_sub(created) < ID_MAX_AGE.as_secs()
        {
            return id.to_string();
        }
    }
    let id = random_hex(16);
    let _ = write_private(&path, format!("{id}\n{}\n", secs(now)).as_bytes(), true);
    id
}

fn write_private(path: &Path, data: &[u8], overwrite: bool) -> io::Result<()> {
    if let Some(d) = path.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(d)?;
    }
    let mut o = fs::OpenOptions::new();
    o.write(true).mode(0o600);
    if overwrite {
        o.create(true).truncate(true);
    } else {
        o.create_new(true);
    }
    o.open(path)?.write_all(data)
}

/// Save `report` in `dir` as `<time>[-n].json` (0600).
pub fn write_report(dir: &Path, report: &Report) -> io::Result<PathBuf> {
    let json = serde_json::to_string_pretty(report).map_err(io::Error::other)?;
    for n in 0..100 {
        let name = if n == 0 {
            format!("{}.json", report.time)
        } else {
            format!("{}-{n}.json", report.time)
        };
        let path = dir.join(name);
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

fn prune_sent() {
    if let Some(d) = reports_dir() {
        prune_older_than(&d.join("sent"), SENT_KEEP, SystemTime::now());
    }
}

fn prune_older_than(dir: &Path, keep: Duration, now: SystemTime) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > keep);
        if old {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Move a pending report to `sent/`, recording the server's event ID.
fn mark_sent(report: &Report, server_id: Option<String>) -> io::Result<()> {
    let from = report
        .path
        .as_ref()
        .ok_or_else(|| io::Error::other("report has no path"))?;
    let mut r = report.clone();
    r.sent_event_id = server_id;
    let sent = from
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("bad path"))?
        .join("sent");
    write_report(&sent, &r)?;
    fs::remove_file(from)
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

/// Install the panic hook for `app`. Call once, early in `main`. At a panic
/// it queues a report only when crash reporting is enabled, then runs the
/// previous hook (the default one prints the panic).
pub fn install(app: AppInfo) {
    if APP.set(app).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
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
        previous(info);
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
    let trace = std::backtrace::Backtrace::force_capture().to_string();
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
    queue(&build_report(&crash, &Scrubber::from_env(), None))
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

/// The "Stack trace of thread" blocks of a coredump MESSAGE.
fn trace_lines(message: &str) -> String {
    message
        .lines()
        .skip_while(|l| !l.contains("Stack trace of thread"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One coredump journal entry (`journalctl -o json`) as a report plus its
/// timestamp in microseconds. Reads only COREDUMP_EXE, COMM, SIGNAL_NAME,
/// TIMESTAMP, PACKAGE_NAME/VERSION and MESSAGE.
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
    let trace = trace_lines(&field(entry, "MESSAGE").unwrap_or_default());
    let time = history::rfc3339_from_unix(ts / 1_000_000);
    let crash = Crash {
        report_type: "coredump",
        app_name: &scrubber.scrub(&name),
        app_version: version.as_deref(),
        message: &format!("{} crashed with {signal}", scrubber.scrub(&comm)),
        stacktrace: &trace,
    };
    Some((ts, build_report(&crash, scrubber, Some(&time))))
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

fn journal(args: &[&str]) -> Vec<Value> {
    let out = Command::new("journalctl")
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

fn rpm_version(exe: &str) -> Option<String> {
    let o = Command::new("rpm")
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
/// Tries the user journal, then the system journal filtered to our UID
/// (readable for members of `wheel`/`systemd-journal`). Empty when disabled.
pub fn collect_coredumps(since_micros: Option<u64>) -> Vec<Report> {
    if !Settings::load().enabled {
        return Vec::new();
    }
    let marker = last_seen_path("coredump-last");
    let since = since_micros
        .or_else(|| {
            marker
                .as_ref()
                .and_then(|p| fs::read_to_string(p).ok())
                .and_then(|t| t.trim().parse().ok())
        })
        .unwrap_or(0);
    let id = format!("MESSAGE_ID={COREDUMP_MESSAGE_ID}");
    let mut entries = journal(&["--user", "-o", "json", "--no-pager", &id]);
    if entries.is_empty() {
        entries = journal(&[
            "-o",
            "json",
            "--no-pager",
            &id,
            &format!("COREDUMP_UID={}", own_uid()),
        ]);
    }
    let scrubber = Scrubber::from_env();
    let mut newest = since;
    let mut out = Vec::new();
    for e in &entries {
        let Some((ts, report)) = coredump_report(e, &scrubber, rpm_version) else {
            continue;
        };
        if ts <= since {
            continue;
        }
        newest = newest.max(ts);
        if let Some(d) = reports_dir() {
            let mut r = report;
            r.path = write_report(&d.join("pending"), &r).ok();
            out.push(r);
        }
    }
    if let Some(p) = marker.filter(|_| newest > since) {
        let _ = write_private(&p, newest.to_string().as_bytes(), true);
    }
    out
}

// --------------------------------------------------------------- events

/// Reports for helper events (update and rollback results) newer than the
/// last call (or `since`, an RFC 3339 time), queued as pending. Empty when
/// disabled.
pub fn collect_events(since: Option<&str>) -> Vec<Report> {
    if !Settings::load().enabled {
        return Vec::new();
    }
    let marker = last_seen_path("events-last");
    let since = since
        .map(str::to_string)
        .or_else(|| {
            marker
                .as_ref()
                .and_then(|p| fs::read_to_string(p).ok())
                .map(|t| t.trim().to_string())
        })
        .unwrap_or_default();
    let events = crate::helper::events::read(Path::new(crate::helper::events::DEFAULT_PATH));
    let scrubber = Scrubber::from_env();
    let mut newest = since.clone();
    let mut out = Vec::new();
    for e in events.iter().filter(|e| e.time > since) {
        let mut msg = e.event.clone();
        if let Some(v) = &e.version {
            msg.push_str(&format!(" (version {v})"));
        }
        if let Some(err) = &e.error {
            msg.push_str(&format!(": {err}"));
        }
        let crash = Crash {
            report_type: &e.event,
            app_name: "atlas-system-helper",
            app_version: Some(env!("CARGO_PKG_VERSION")),
            message: &msg,
            stacktrace: "",
        };
        let mut r = build_report(&crash, &scrubber, Some(&e.time));
        if r.atlasos_version.is_none() {
            r.atlasos_version = e.version.clone();
        }
        if let Some(d) = reports_dir() {
            r.path = write_report(&d.join("pending"), &r).ok();
        }
        if e.time > newest {
            newest = e.time.clone();
        }
        out.push(r);
    }
    if let Some(p) = marker.filter(|_| newest != since) {
        let _ = write_private(&p, newest.as_bytes(), true);
    }
    out
}

// ------------------------------------------------------------- reporting

/// POST the report's [`payload`](Report::payload) to GlitchTip (Sentry store
/// API) and move it to `sent/`. The caller must have shown the user the
/// payload and got a yes. Fails with "no endpoint configured" when the DSN is
/// empty. Uses `curl`.
pub fn send(report: &Report) -> io::Result<()> {
    let ep = Endpoint::load()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no endpoint configured"))?;
    let id = post(report, &ep)?;
    mark_sent(report, id)
}

fn post(report: &Report, ep: &Endpoint) -> io::Result<Option<String>> {
    let body = serde_json::to_vec(&report.payload()).map_err(io::Error::other)?;
    let auth = format!(
        "X-Sentry-Auth: Sentry sentry_version=7, sentry_key={}, sentry_client=atlas-core/{}",
        ep.key,
        env!("CARGO_PKG_VERSION")
    );
    let mut child = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "30",
            "--request",
            "POST",
        ])
        .args([
            "--header",
            "Content-Type: application/json",
            "--header",
            &auth,
            "--data-binary",
            "@-",
            "--url",
            &ep.store_url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("no stdin"))?
        .write_all(&body)?;
    let out = child.wait_with_output()?;
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
    let lines: Vec<&str> = r.stacktrace.lines().collect();
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
    use std::os::unix::fs::PermissionsExt;

    fn sc() -> Scrubber {
        Scrubber::new(Some("zach"), Some("atlas-box"))
    }

    fn report(trace: &str) -> Report {
        let c = Crash {
            report_type: "panic",
            app_name: "net.eterneon.atlas.updater",
            app_version: Some("0.1.0"),
            message: "boom",
            stacktrace: trace,
        };
        build_report(&c, &sc(), Some("2026-10-02T10:00:00Z"))
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
        assert_eq!(s.scrub("zachary zach_x xzach"), "zachary zach_x xzach");
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
            "ip <ip> and <ip>:8080."
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
        let r = build_report(&c, &sc(), None);
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
        assert!(json.contains("/var/home/USER/.cargo"));
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
        ] {
            assert!(
                Endpoint::parse(bad).is_none() || bad == "https://-x@h/1;rm" && false,
                "{bad:?}"
            );
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
        assert!(r.stacktrace.contains("/home/USER/lib.so"));
        let all = serde_json::to_string(&r).unwrap();
        assert!(!all.contains("secret") && !all.contains("zach") && !all.contains("Process 1"));
        assert_eq!(r.time, "2026-09-21T14:13:20Z");
    }

    #[test]
    fn rotating_id_is_stable_then_replaced_after_30_days() {
        let d = tempfile::tempdir().unwrap();
        let t0 = SystemTime::now();
        let a = crash_id_in(d.path(), t0);
        assert_eq!(a.len(), 32);
        assert_eq!(
            crash_id_in(d.path(), t0 + Duration::from_secs(29 * 86_400)),
            a
        );
        let b = crash_id_in(d.path(), t0 + Duration::from_secs(31 * 86_400));
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
        mark_sent(&got[1], Some("srv1".into())).unwrap();
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
        // the packaged default has an empty dsn, and tests have no /etc config
        if Endpoint::load().is_none() {
            let e = send(&report("")).unwrap_err();
            assert_eq!(e.to_string(), "no endpoint configured");
        }
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
}
