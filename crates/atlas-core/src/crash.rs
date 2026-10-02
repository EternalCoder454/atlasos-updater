//! Crash reports: the only telemetry an Atlas app may have.
//!
//! [`install`] puts a panic hook in place that saves a [`Report`] under
//! `$XDG_STATE_HOME/atlas/<app-id>/crashes/` (mode 0600) and then lets the
//! previous hook run. For fatal errors that are not panics (a Qt fatal message
//! handler, say) call [`record_fatal`].
//!
//! Nothing here sends anything by itself. The app shows the user what would be
//! sent ([`Report::to_json_pretty`]) and, only if the user agrees, opens
//! [`github_issue_url`] or calls [`send`] with a configured endpoint.
//!
//! A report holds: app name, ID and version; OS name, version, image version
//! and variant; kernel, CPU model and thread count, RAM and GPU IDs; the
//! process's memory and CPU time, system memory and load at crash time; and the
//! crash message, location and backtrace. It never holds a username, hostname,
//! machine-id, network address, serial number, environment variable or the
//! name of a file the user opened. `$HOME` and the username are replaced by
//! `~` and `<user>` in every string.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::history::now_rfc3339;

/// Identifies the app a report is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    /// Reverse-DNS app ID, e.g. `net.eterneon.atlas.updater`.
    pub id: String,
    pub version: String,
    /// Repository name under `github.com/EternalCoder454/` that gets issues.
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    /// RFC 3339 UTC time of the crash.
    pub time: String,
    pub app: AppInfo,
    pub os: OsInfo,
    pub system: SystemInfo,
    pub usage: Usage,
    pub crash: CrashInfo,
    /// Where this report is stored; not part of the report itself.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OsInfo {
    pub name: Option<String>,
    pub version: Option<String>,
    pub image_version: Option<String>,
    pub variant_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SystemInfo {
    pub kernel: Option<String>,
    pub cpu_model: Option<String>,
    pub cpu_threads: u32,
    pub ram_total_kb: u64,
    /// `AMD [1002:744c]` style: vendor name and PCI IDs, nothing else.
    pub gpus: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Usage {
    pub rss_kb: u64,
    pub cpu_time_secs: f64,
    pub mem_used_kb: u64,
    pub mem_available_kb: u64,
    pub load_avg: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashInfo {
    /// `panic` or `fatal`.
    pub kind: String,
    pub message: String,
    pub location: Option<String>,
    pub backtrace: String,
}

impl Report {
    /// The report as pretty JSON: exactly what [`send`] would post.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- scrubbing

/// Replaces `$HOME`, the username and the hostname in strings.
#[derive(Debug, Clone, Default)]
pub struct Scrubber {
    home: Option<String>,
    user: Option<String>,
    host: Option<String>,
}

impl Scrubber {
    pub fn new(home: Option<&str>, user: Option<&str>, host: Option<&str>) -> Self {
        let keep = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_string);
        Scrubber {
            home: keep(home)
                .map(|h| h.trim_end_matches('/').to_string())
                .filter(|h| !h.is_empty()),
            user: keep(user),
            host: keep(host),
        }
    }

    /// From the process environment (only to know what to remove).
    pub fn from_env() -> Self {
        let home = std::env::var("HOME").ok();
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .ok()
            .or_else(|| {
                home.as_deref()
                    .and_then(|h| Path::new(h).file_name())
                    .map(|n| n.to_string_lossy().into_owned())
            });
        let host = fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .map(|h| h.trim().to_string());
        Scrubber::new(home.as_deref(), user.as_deref(), host.as_deref())
    }

    /// `$HOME` becomes `~`, the username `<user>`, the hostname `<host>`.
    pub fn scrub(&self, s: &str) -> String {
        let mut out = s.to_string();
        if let Some(h) = self.home.as_deref().filter(|h| *h != "/") {
            // on ostree systems /home is /var/home
            out = replace_path(&out, &format!("/var{h}"), "~");
            out = replace_path(&out, h, "~");
        }
        if let Some(u) = &self.user {
            out = replace_token(&out, u, "<user>");
        }
        if let Some(h) = &self.host {
            out = replace_token(&out, h, "<host>");
        }
        out
    }

    /// [`scrub`](Self::scrub), then hide paths that can name a file the user
    /// had open. Used for the panic message, which can quote anything.
    pub fn scrub_message(&self, s: &str) -> String {
        redact_paths(&self.scrub(s))
    }
}

/// Replace the path `needle` where it starts a path (not in the middle of a
/// longer one) and ends at a `/` or a non-word character.
fn replace_path(s: &str, needle: &str, with: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(needle) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + needle.len()..].chars().next();
        let (head, tail) = rest.split_at(i + needle.len());
        out.push_str(&head[..i]);
        let starts_path = !before.is_some_and(|c| is_word(c) || "/.-~".contains(c));
        let ends_path = !after.is_some_and(|c| is_word(c) || c == '-');
        out.push_str(if starts_path && ends_path {
            with
        } else {
            needle
        });
        rest = tail;
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

const PRIVATE_PREFIXES: &[&str] = &[
    "~/",
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
        if PRIVATE_PREFIXES.iter().any(|p| token.starts_with(p)) || token == "~" {
            out.push_str("<path>");
        } else {
            out.push_str(token);
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

// ------------------------------------------------------------- collecting

/// `KEY=value` lines of an os-release file; values may be quoted.
fn parse_os_release(text: &str) -> OsInfo {
    let get = |key: &str| {
        text.lines().find_map(|l| {
            let v = l.strip_prefix(key)?.strip_prefix('=')?;
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            (!v.is_empty()).then(|| v.to_string())
        })
    };
    OsInfo {
        name: get("NAME"),
        version: get("VERSION"),
        image_version: get("IMAGE_VERSION"),
        variant_id: get("VARIANT_ID"),
    }
}

fn kb_field(text: &str, key: &str) -> Option<u64> {
    let rest = text.lines().find_map(|l| l.strip_prefix(key))?;
    rest.trim_start_matches(':')
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// `(model name, logical cores)` from /proc/cpuinfo text.
fn parse_cpuinfo(text: &str) -> (Option<String>, u32) {
    let model = text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "model name").then(|| v.trim().to_string())
    });
    let threads = text.lines().filter(|l| l.starts_with("processor")).count() as u32;
    (model, threads)
}

/// utime + stime in seconds from /proc/self/stat (assumes 100 ticks/s, which
/// holds on every Linux target we build for).
fn parse_stat_cpu_secs(stat: &str) -> f64 {
    // The command name is in parentheses and may contain spaces.
    let Some(after) = stat.rfind(')').map(|i| &stat[i + 1..]) else {
        return 0.0;
    };
    let f: Vec<&str> = after.split_whitespace().collect();
    // After ")": state is f[0]; utime is field 14 overall, f[11]; stime f[12].
    let tick = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    (tick(11) + tick(12)) / 100.0
}

fn parse_loadavg(text: &str) -> [f64; 3] {
    let mut it = text.split_whitespace().map(|v| v.parse().unwrap_or(0.0));
    [
        it.next().unwrap_or(0.0),
        it.next().unwrap_or(0.0),
        it.next().unwrap_or(0.0),
    ]
}

fn vendor_name(id: &str) -> &'static str {
    match id {
        "0x1002" => "AMD",
        "0x10de" => "NVIDIA",
        "0x8086" => "Intel",
        "0x1af4" => "virtio",
        "0x15ad" => "VMware",
        _ => "GPU",
    }
}

/// GPUs from sysfs: `<vendor> [<vendor id>:<device id>]` per `cardN`.
fn read_gpus(drm: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut gpus: Vec<String> = entries
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_prefix("card")
                .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
        })
        .filter_map(|e| {
            let dev = e.path().join("device");
            let vendor = fs::read_to_string(dev.join("vendor")).ok()?;
            let device = fs::read_to_string(dev.join("device")).ok()?;
            let (v, d) = (vendor.trim(), device.trim());
            Some(format!(
                "{} [{}:{}]",
                vendor_name(v),
                v.trim_start_matches("0x"),
                d.trim_start_matches("0x")
            ))
        })
        .collect();
    gpus.sort();
    gpus.dedup();
    gpus
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn collect_system() -> SystemInfo {
    let (cpu_model, cpu_threads) = parse_cpuinfo(&read("/proc/cpuinfo"));
    SystemInfo {
        kernel: Some(read("/proc/sys/kernel/osrelease").trim().to_string())
            .filter(|s| !s.is_empty()),
        cpu_model,
        cpu_threads,
        ram_total_kb: kb_field(&read("/proc/meminfo"), "MemTotal").unwrap_or(0),
        gpus: read_gpus(Path::new("/sys/class/drm")),
    }
}

fn collect_usage() -> Usage {
    let meminfo = read("/proc/meminfo");
    let total = kb_field(&meminfo, "MemTotal").unwrap_or(0);
    let avail = kb_field(&meminfo, "MemAvailable").unwrap_or(0);
    Usage {
        rss_kb: kb_field(&read("/proc/self/status"), "VmRSS").unwrap_or(0),
        cpu_time_secs: parse_stat_cpu_secs(&read("/proc/self/stat")),
        mem_used_kb: total.saturating_sub(avail),
        mem_available_kb: avail,
        load_avg: parse_loadavg(&read("/proc/loadavg")),
    }
}

/// Build a report for a crash now. All strings are scrubbed.
pub fn build_report(
    app: &AppInfo,
    kind: &str,
    message: &str,
    location: Option<&str>,
    backtrace: &str,
    scrubber: &Scrubber,
) -> Report {
    let os_text = {
        let t = read("/etc/os-release");
        if t.is_empty() {
            read("/usr/lib/os-release")
        } else {
            t
        }
    };
    let os = parse_os_release(&os_text);
    let s = |x: Option<String>| x.map(|v| scrubber.scrub(&v));
    Report {
        schema: 1,
        time: now_rfc3339(),
        app: app.clone(),
        os: OsInfo {
            name: s(os.name),
            version: s(os.version),
            image_version: s(os.image_version),
            variant_id: s(os.variant_id),
        },
        system: collect_system(),
        usage: collect_usage(),
        crash: CrashInfo {
            kind: kind.to_string(),
            message: scrubber.scrub_message(message),
            location: location.map(|l| scrubber.scrub(l)),
            backtrace: scrubber.scrub(backtrace),
        },
        path: None,
    }
}

// ----------------------------------------------------------------- storage

/// `$XDG_STATE_HOME/atlas/<app-id>/crashes` (default `~/.local/state/...`).
pub fn crash_dir(app_id: &str) -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => {
            PathBuf::from(std::env::var_os("HOME").filter(|v| !v.is_empty())?).join(".local/state")
        }
    };
    // an app ID is a plain name; refuse anything that could leave the dir
    if app_id.is_empty() || app_id.contains('/') || app_id.starts_with('.') {
        return None;
    }
    Some(base.join("atlas").join(app_id).join("crashes"))
}

/// Write `report` into `dir` as `<time>.json` (0600). Returns the path.
pub fn write_report(dir: &Path, report: &Report) -> io::Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let json = report.to_json_pretty();
    for n in 0..100 {
        let name = if n == 0 {
            format!("{}.json", report.time)
        } else {
            format!("{}-{n}.json", report.time)
        };
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(json.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("too many reports in the same second"))
}

/// Unsent reports in `dir`, oldest first.
pub fn pending_in(dir: &Path) -> Vec<Report> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd
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

/// Reports saved for `app_id` that were neither sent nor discarded.
pub fn pending(app_id: &str) -> Vec<Report> {
    crash_dir(app_id)
        .map(|d| pending_in(&d))
        .unwrap_or_default()
}

/// Keep the report on disk but stop listing it as pending (`.sent` suffix).
pub fn mark_sent(report: &Report) -> io::Result<()> {
    let path = report
        .path
        .as_ref()
        .ok_or_else(|| io::Error::other("report has no path"))?;
    let mut to = path.clone().into_os_string();
    to.push(".sent");
    fs::rename(path, to)
}

/// Delete the report's file.
pub fn discard(report: &Report) -> io::Result<()> {
    let path = report
        .path
        .as_ref()
        .ok_or_else(|| io::Error::other("report has no path"))?;
    fs::remove_file(path)
}

// -------------------------------------------------------------------- hooks

static APP: OnceLock<AppInfo> = OnceLock::new();

/// Install the panic hook for `app`. Call once, early in `main`. The previous
/// hook (the default one prints the panic) still runs afterwards.
pub fn install(app: AppInfo) {
    if APP.set(app).is_err() {
        return; // already installed
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = match info.payload().downcast_ref::<&str>() {
            Some(s) => (*s).to_string(),
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => s.clone(),
                None => "Box<dyn Any>".to_string(),
            },
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let _ = save("panic", &message, location.as_deref());
        previous(info);
    }));
}

/// Save a report for a fatal error that is not a Rust panic, for example from
/// a Qt message handler. Needs [`install`] to have been called. Returns the
/// path of the saved report.
pub fn record_fatal(message: &str) -> Option<PathBuf> {
    save("fatal", message, None)
}

fn save(kind: &str, message: &str, location: Option<&str>) -> Option<PathBuf> {
    let app = APP.get()?;
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let report = build_report(
        app,
        kind,
        message,
        location,
        &backtrace,
        &Scrubber::from_env(),
    );
    write_report(&crash_dir(&app.id)?, &report).ok()
}

// ------------------------------------------------------------- reporting

/// Longest issue URL we produce.
const MAX_URL: usize = 7000;

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn issue_body(r: &Report, backtrace: &str, truncated: bool) -> String {
    let mut b = String::new();
    let na = |o: &Option<String>| o.clone().unwrap_or_else(|| "unknown".into());
    b.push_str(&format!(
        "**App:** {} {} ({})\n",
        r.app.name, r.app.version, r.app.id
    ));
    b.push_str(&format!(
        "**OS:** {} {} (image {}, {})\n",
        na(&r.os.name),
        na(&r.os.version),
        na(&r.os.image_version),
        na(&r.os.variant_id)
    ));
    b.push_str(&format!(
        "**System:** kernel {}, {} ({} threads), {} MiB RAM, GPU {}\n",
        na(&r.system.kernel),
        na(&r.system.cpu_model),
        r.system.cpu_threads,
        r.system.ram_total_kb / 1024,
        if r.system.gpus.is_empty() {
            "unknown".to_string()
        } else {
            r.system.gpus.join(", ")
        }
    ));
    b.push_str(&format!(
        "**At crash:** RSS {} MiB, CPU time {:.1} s, memory used {} MiB, available {} MiB, load {:.2} {:.2} {:.2}\n\n",
        r.usage.rss_kb / 1024,
        r.usage.cpu_time_secs,
        r.usage.mem_used_kb / 1024,
        r.usage.mem_available_kb / 1024,
        r.usage.load_avg[0],
        r.usage.load_avg[1],
        r.usage.load_avg[2]
    ));
    b.push_str(&format!(
        "**Crash ({}):** {}\n",
        r.crash.kind, r.crash.message
    ));
    if let Some(l) = &r.crash.location {
        b.push_str(&format!("**Location:** {l}\n"));
    }
    b.push_str("\n```\n");
    b.push_str(backtrace);
    if truncated {
        b.push_str("\n... (backtrace truncated)");
    }
    b.push_str("\n```\n");
    b
}

/// A prefilled `https://github.com/EternalCoder454/<repo>/issues/new?...` URL,
/// at most about 7 KB: the backtrace is cut line by line to fit.
pub fn github_issue_url(r: &Report) -> String {
    let first_line = r.crash.message.lines().next().unwrap_or("");
    let mut short: String = first_line.chars().take(80).collect();
    if first_line.chars().count() > 80 {
        short.push_str("...");
    }
    let title = format!("Crash in {} {}: {}", r.app.name, r.app.version, short);
    let base = format!(
        "https://github.com/EternalCoder454/{}/issues/new?title={}&body=",
        percent_encode(&r.app.repo),
        percent_encode(&title)
    );
    let lines: Vec<&str> = r.crash.backtrace.lines().collect();
    let mut keep = lines.len();
    loop {
        let bt = lines[..keep].join("\n");
        let body = percent_encode(&issue_body(r, &bt, keep < lines.len()));
        if base.len() + body.len() <= MAX_URL || keep == 0 {
            return format!("{base}{body}");
        }
        // drop a tenth of the remaining lines at a time, at least one
        keep -= (keep / 10).max(1);
    }
}

/// POST the report's JSON to `endpoint` (an `http://` or `https://` URL from
/// the app's config; empty by default, which is an error here). The caller
/// must have the user's consent. Uses `curl`, which AtlasOS ships.
pub fn send(report: &Report, endpoint: &str) -> io::Result<()> {
    let endpoint = endpoint.trim();
    if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no crash report endpoint configured",
        ));
    }
    let mut child = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--max-time", "30"])
        .args([
            "--request",
            "POST",
            "--header",
            "Content-Type: application/json",
        ])
        .args(["--data-binary", "@-", "--url", endpoint])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("no stdin"))?
        .write_all(report.to_json_pretty().as_bytes())?;
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scrubber() -> Scrubber {
        Scrubber::new(Some("/home/zach"), Some("zach"), Some("atlas-box"))
    }

    fn app() -> AppInfo {
        AppInfo {
            name: "Atlas Updater".into(),
            id: "net.eterneon.atlas.updater".into(),
            version: "0.1.0".into(),
            repo: "atlasos-updater".into(),
        }
    }

    #[test]
    fn scrubs_home_user_and_host() {
        let s = scrubber();
        assert_eq!(s.scrub("/home/zach/.cargo/x.rs"), "~/.cargo/x.rs");
        assert_eq!(s.scrub("/home/zach"), "~");
        assert_eq!(s.scrub("user zach on atlas-box"), "user <user> on <host>");
        // no HOME match, still no username
        assert_eq!(s.scrub("/var/home/zach/x"), "~/x");
        assert_eq!(s.scrub("/srv/home/zach/x"), "/srv/home/<user>/x");
        assert_eq!(s.scrub("/run/user/1000/zach/a"), "/run/user/1000/<user>/a");
    }

    #[test]
    fn username_inside_other_words_is_left_alone() {
        let s = scrubber();
        assert_eq!(
            s.scrub("zachary and zach_x and xzach"),
            "zachary and zach_x and xzach"
        );
        assert_eq!(s.scrub("zach."), "<user>.");
    }

    #[test]
    fn short_home_and_empty_values_do_not_mangle_text() {
        let s = Scrubber::new(Some("/"), Some(""), None);
        assert_eq!(s.scrub("a/b zach"), "a/b zach");
        let s = Scrubber::new(Some("/home/zach/"), Some("zach"), None);
        assert_eq!(s.scrub("/home/zach/a"), "~/a");
    }

    #[test]
    fn message_paths_into_user_data_are_hidden() {
        let s = scrubber();
        let m = s.scrub_message("No such file: '/home/zach/Documents/tax 2025.pdf' (os error 2)");
        assert_eq!(m, "No such file: '<path> 2025.pdf' (os error 2)");
        assert!(!m.contains("Documents"));
        assert_eq!(
            s.scrub_message("open /run/media/zach/USB/a.txt failed"),
            "open <path> failed"
        );
        assert_eq!(s.scrub_message("read /mnt/data/x"), "read <path>");
        assert_eq!(
            s.scrub_message("see /usr/lib/foo.so"),
            "see /usr/lib/foo.so"
        );
    }

    #[test]
    fn report_has_no_identifying_strings() {
        let s = scrubber();
        let r = build_report(
            &app(),
            "panic",
            "failed at /home/zach/Documents/notes.md for zach",
            Some("/home/zach/src/main.rs:10:5"),
            "  0: f\n     at /home/zach/.cargo/registry/foo.rs:1\n  1: zach::main\n",
            &s,
        );
        let json = r.to_json_pretty();
        for needle in ["zach", "atlas-box", "Documents", "notes.md"] {
            assert!(!json.contains(needle), "{needle} in {json}");
        }
        assert!(r.crash.location.as_deref().unwrap().starts_with("~/src"));
        assert!(r.crash.backtrace.contains("~/.cargo/registry"));
        assert_eq!(r.app.id, "net.eterneon.atlas.updater");
        assert!(!json.contains("machine-id") && !json.contains("hostname"));
    }

    #[test]
    fn parses_os_release() {
        let o = parse_os_release(
            "NAME=\"Fedora Linux\"\nVERSION=\"44 (Kinoite)\"\nVARIANT_ID=kinoite\nIMAGE_VERSION=44.20261001\nID=fedora\nHOME_URL=\"x\"\n",
        );
        assert_eq!(o.name.as_deref(), Some("Fedora Linux"));
        assert_eq!(o.version.as_deref(), Some("44 (Kinoite)"));
        assert_eq!(o.variant_id.as_deref(), Some("kinoite"));
        assert_eq!(o.image_version.as_deref(), Some("44.20261001"));
        assert_eq!(parse_os_release("NAME=x\n").image_version, None);
    }

    #[test]
    fn parses_proc_files() {
        let meminfo = "MemTotal:       32768000 kB\nMemFree: 100 kB\nMemAvailable:   20000000 kB\n";
        assert_eq!(kb_field(meminfo, "MemTotal"), Some(32768000));
        assert_eq!(kb_field(meminfo, "MemAvailable"), Some(20000000));
        assert_eq!(kb_field("VmRSS:\t   1234 kB\n", "VmRSS"), Some(1234));
        let stat = "123 (my (app)) S 1 2 3 4 5 6 7 8 9 10 250 50 0 0 20 0 1 0 5 6 7";
        assert_eq!(parse_stat_cpu_secs(stat), 3.0);
        assert_eq!(
            parse_loadavg("0.50 1.25 2.00 1/200 999\n"),
            [0.5, 1.25, 2.0]
        );
        let (m, n) = parse_cpuinfo(
            "processor\t: 0\nmodel name\t: Intel(R) Core(TM) i9\nprocessor\t: 1\nmodel name\t: Intel(R) Core(TM) i9\n",
        );
        assert_eq!(m.as_deref(), Some("Intel(R) Core(TM) i9"));
        assert_eq!(n, 2);
    }

    #[test]
    fn reads_gpus_from_sysfs_layout() {
        let d = tempfile::tempdir().unwrap();
        for (card, v, dev) in [("card0", "0x1002", "0x744c"), ("card1", "0x8086", "0xa780")] {
            let p = d.path().join(card).join("device");
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("vendor"), format!("{v}\n")).unwrap();
            fs::write(p.join("device"), format!("{dev}\n")).unwrap();
        }
        // connectors are not GPUs and carry no device dir
        fs::create_dir_all(d.path().join("card0-DP-1")).unwrap();
        assert_eq!(
            read_gpus(d.path()),
            ["AMD [1002:744c]", "Intel [8086:a780]"]
        );
    }

    #[test]
    fn write_pending_mark_sent_and_discard() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("atlas/app/crashes");
        let mut r = build_report(&app(), "panic", "boom", None, "bt", &scrubber());
        r.time = "2026-10-02T10:00:00Z".into();
        let p = write_report(&dir, &r).unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // same second again does not overwrite
        let p2 = write_report(&dir, &r).unwrap();
        assert_ne!(p, p2);
        let got = pending_in(&dir);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].crash.message, "boom");
        mark_sent(&got[0]).unwrap();
        assert_eq!(pending_in(&dir).len(), 1);
        discard(&pending_in(&dir)[0]).unwrap();
        assert!(pending_in(&dir).is_empty());
        assert!(pending_in(&d.path().join("none")).is_empty());
    }

    #[test]
    fn crash_dir_rejects_odd_app_ids() {
        assert!(crash_dir("../x").is_none());
        assert!(crash_dir("").is_none());
        assert!(crash_dir("a/b").is_none());
    }

    fn big_report(lines: usize) -> Report {
        let bt: String = (0..lines)
            .map(|i| format!("  {i}: some::module::function_{i}\n"))
            .collect();
        build_report(
            &app(),
            "panic",
            "it broke & stuff",
            Some("a.rs:1:1"),
            &bt,
            &scrubber(),
        )
    }

    #[test]
    fn issue_url_is_prefilled_and_bounded() {
        let small = github_issue_url(&big_report(3));
        assert!(
            small.starts_with(
                "https://github.com/EternalCoder454/atlasos-updater/issues/new?title="
            )
        );
        assert!(small.contains("&body="));
        assert!(small.contains("it%20broke%20%26%20stuff"));
        assert!(!small.contains("truncated"));
        let big = github_issue_url(&big_report(2000));
        assert!(big.len() <= MAX_URL, "{}", big.len());
        assert!(big.contains("truncated"));
        assert!(!big.contains(' ') && !big.contains('\n'));
    }

    #[test]
    fn send_refuses_empty_or_odd_endpoints() {
        let r = big_report(1);
        for bad in ["", "  ", "ftp://x", "--url=x", "file:///etc/passwd"] {
            let e = send(&r, bad).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
    }
}
