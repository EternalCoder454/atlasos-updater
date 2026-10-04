//! Flatpak app updates (through atlas-framework-flatpak).

use std::path::Path;

use atlas_framework_flatpak::{self as flatpak, AppUpdate, InstallationKind};
use serde::{Deserialize, Serialize};

use crate::{apphistory, config, power};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub name: String,
    pub id: String,
    pub branch: String,
    pub system: bool,
    pub runtime: bool,
    pub size: u64,
    #[serde(default)]
    pub size_text: String,
    /// What it asks for that the installed version doesn't have, in plain
    /// words; empty unless a background round held it back.
    #[serde(default)]
    pub asks: String,
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    if bytes == 0 {
        return String::new();
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn row(u: AppUpdate) -> Row {
    Row {
        size_text: format_size(u.download_size),
        name: u.name,
        id: u.id,
        branch: u.branch,
        system: u.installation == InstallationKind::System,
        runtime: u.is_runtime,
        size: u.download_size,
        asks: String::new(),
    }
}

/// One installed app or runtime: ID, branch and installation.
pub fn row_key(r: &Row) -> String {
    format!("{}/{}/{}", r.id, r.branch, if r.system { 's' } else { 'u' })
}

/// Blocking. `refresh` updates the appstream and summary caches first.
/// `quiet`: nobody is watching, so nothing may ask for a password.
pub fn list(refresh: bool, quiet: bool, fixtures: Option<&Path>) -> Result<Vec<Row>, String> {
    if let Some(dir) = fixtures {
        let text = config::read_fixture(dir, "flatpak.json").unwrap_or_else(|| "[]".into());
        let mut rows: Vec<Row> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for r in &mut rows {
            r.size_text = format_size(r.size);
        }
        return Ok(rows);
    }
    let mut rows: Vec<Row> = flatpak::list_updates_with(refresh, quiet)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(row)
        .collect();
    // Apps first, then runtimes; alphabetical within each.
    rows.sort_by(|a, b| {
        a.runtime
            .cmp(&b.runtime)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(rows)
}

/// An app a background update left for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldApp {
    /// As [`row_key`].
    pub key: String,
    pub name: String,
    /// What it asks for, in plain words ([`describe`]).
    pub asks: Vec<String>,
    /// The same as flatpak metadata, sorted: what [`unseen`] compares, as
    /// the words leave out detail (a bus name's level, all past the fifth).
    pub raw: Vec<String>,
}

/// The weightiest permissions first, so the few a notice has room for are
/// the ones that matter: files, devices, sockets, services, the rest.
fn by_weight(mut p: Vec<String>) -> Vec<String> {
    let weight = |p: &String| {
        let item = p.split_once('=').map_or("", |(_, i)| i);
        match p.split_once('=').map_or(p.as_str(), |(k, _)| k) {
            _ if [flatpak::UNREADABLE, flatpak::NEW_APP, flatpak::UNMATCHED]
                .contains(&p.as_str()) =>
            {
                0
            }
            "Context: filesystems" if ["host", "home", "~"].iter().any(|w| item.starts_with(w)) => {
                1
            }
            "Context: filesystems" => 3,
            "Context: sockets" if ["session-bus", "system-bus", "ssh-auth"].contains(&item) => 1,
            "Context: devices" => 2,
            "Context: sockets" => 2,
            k if k.starts_with("Application: ") => 2,
            // Ways out of the sandbox, and owning a name.
            k if k.contains("Bus Policy: ")
                && (item == "own"
                    || [".Flatpak", ".systemd1", ".secrets", ".login1"]
                        .iter()
                        .any(|n| k.ends_with(n))) =>
            {
                1
            }
            k if k.contains("Bus Policy: ") => 4,
            "Context: shared" => 4,
            _ => 5,
        }
    };
    p.sort_by_key(weight);
    p
}

/// A permission from `flatpak::new_permissions` in plain words. The rest
/// of the text came from the app's remote, so it is cleaned for showing.
pub fn describe(p: &str) -> String {
    flatpak::clean(&plain_words(p))
}

fn plain_words(p: &str) -> String {
    if p == flatpak::UNREADABLE {
        return "permissions that couldn't be checked".into();
    }
    if p == flatpak::NEW_APP {
        return "a new app to install".into();
    }
    if p == flatpak::UNMATCHED {
        return "new permissions somewhere in this update".into();
    }
    let (group, rest) = p.split_once(": ").unwrap_or(("", p));
    let (key, item) = rest.split_once('=').unwrap_or((rest, ""));
    let plain = match (group, key, item) {
        ("Context", "filesystems", "home" | "~" | "home:rw") => "your home folder",
        ("Context", "filesystems", "host" | "host:rw") => "all your files",
        ("Context", "filesystems", "host-os" | "host-etc") => "the system's files",
        ("Context", "devices", "all") => "all devices, such as cameras and USB",
        ("Context", "shared", "network") => "the network",
        ("Context", "sockets", "x11" | "fallback-x11") => "the X11 display",
        ("Context", "sockets", "pulseaudio") => "sound and the microphone",
        ("Context", "sockets", "session-bus") => "every app's services on your desktop",
        ("Context", "sockets", "system-bus") => "every system service",
        ("Application", _, _) => return format!("a different runtime ({item})"),
        ("System Bus Policy", _, _) => return format!("the system service {key}"),
        ("Session Bus Policy", _, _) => return format!("the service {key}"),
        _ => return rest.to_string(),
    };
    plain.to_string()
}

/// What a row shows for a held app: its first 5 new permissions.
pub fn asks_text(asks: &[String]) -> String {
    let mut shown: Vec<&str> = asks.iter().map(String::as_str).take(5).collect();
    if asks.len() > 5 {
        shown.push("more");
    }
    shown.join(", ")
}

/// What an update run did.
#[derive(Debug, Default)]
pub struct Done {
    /// For the history.
    pub updated: Vec<apphistory::Entry>,
    /// Apps left for the user because they ask for new permissions.
    pub held_back: Vec<HeldApp>,
    pub error: Option<String>,
}

/// Blocking. `progress` gets a short status line. `background`: nobody asked
/// for this run, so it never asks for a password. `hold`: leave out apps
/// that want new permissions (for a run whose check the user may not have
/// seen).
pub fn update(
    fixtures: Option<&Path>,
    background: bool,
    hold: bool,
    mut progress: impl FnMut(String) + 'static,
) -> Done {
    let now = crate::schedule::unix_now();
    if let Some(dir) = fixtures {
        progress("Updating…".into());
        std::thread::sleep(std::time::Duration::from_millis(300));
        let rows = list(false, false, Some(dir)).unwrap_or_default();
        return Done {
            updated: rows
                .into_iter()
                .map(|r| apphistory::Entry {
                    at: now,
                    id: r.id,
                    name: r.name,
                    branch: r.branch,
                    runtime: r.runtime,
                    system: r.system,
                    from: None,
                    to: None,
                    auto: background,
                })
                .collect(),
            ..Default::default()
        };
    }
    let opts = flatpak::UpdateOptions {
        no_interaction: background,
        hold_new_permissions: hold,
        check_only: false,
    };
    let out = flatpak::update(&opts, move |p| {
        let what = p
            .reference
            .split('/')
            .nth(1)
            .unwrap_or(&p.reference)
            .to_string();
        progress(format!("{} {what}… {}%", p.status, p.percent));
    });
    Done {
        updated: out
            .updated
            .into_iter()
            .map(|u| apphistory::Entry {
                at: now,
                system: u.installation == InstallationKind::System,
                id: u.id,
                name: u.name,
                branch: u.branch,
                runtime: u.is_runtime,
                from: u.old_version,
                to: u.new_version,
                auto: background,
            })
            .collect(),
        held_back: held_apps(out.held_back),
        error: out.error.map(|e| e.to_string()),
    }
}

fn held_apps(held: Vec<flatpak::Held>) -> Vec<HeldApp> {
    held.into_iter()
        .map(|h| {
            let mut raw = h.permissions.clone();
            raw.sort();
            raw.dedup();
            HeldApp {
                key: row_key(&row(h.app.clone())),
                name: h.app.name,
                asks: by_weight(h.permissions)
                    .iter()
                    .map(|p| describe(p))
                    .collect(),
                raw,
            }
        })
        .collect()
}

/// Blocking. Which waiting updates ask for new permissions, without
/// downloading or installing anything, so the user sees it before pressing
/// "Update Apps". Never asks for a password. Fixture runs find none.
pub fn check(fixtures: Option<&Path>) -> Done {
    if fixtures.is_some() {
        return Done::default();
    }
    let opts = flatpak::UpdateOptions {
        no_interaction: true,
        hold_new_permissions: true,
        check_only: true,
    };
    let out = flatpak::update(&opts, |_| {});
    Done {
        updated: Vec::new(),
        held_back: held_apps(out.held_back),
        error: out.error.map(|e| e.to_string()),
    }
}

/// One background round's result.
#[derive(Debug, Default)]
pub struct Round {
    /// The updates still waiting afterwards; `None` when nothing was listed
    /// (a metered or offline connection).
    pub rows: Option<Vec<Row>>,
    pub done: Done,
    /// Why nothing was installed although background updates are on.
    pub waited: Option<&'static str>,
    /// An update or a check ran to the end (so `done.held_back` is the
    /// whole held set).
    pub ran: bool,
    /// Background updates are off and the check of what the waiting
    /// updates ask for failed: the notice offers no "Update Apps".
    pub unchecked: Option<String>,
    /// Updates were installed by themselves (background updates on).
    pub auto: bool,
}

/// Blocking. Looks for app updates, unless the connection is metered or
/// offline, or its state unknown (then `rows` is `None`); when `auto()` says so (asked again just
/// before installing, so turning the switch off mid-round counts), installs
/// them unless the battery is low. Nothing asks for a password.
pub fn background(auto: impl Fn() -> bool, fixtures: Option<&Path>) -> Result<Round, String> {
    let state = if fixtures.is_some() {
        power::State::default()
    } else {
        power::read()
    };
    if let Some(why) = power::network_why_not(&state) {
        return Ok(Round {
            waited: Some(why),
            ..Default::default()
        });
    }
    let rows = list(true, true, fixtures)?;
    if rows.is_empty() {
        return Ok(Round {
            rows: Some(rows),
            ..Default::default()
        });
    }
    if !auto() {
        return Ok(only_check(rows, fixtures));
    }
    if let Some(why) = power::why_not(&state) {
        return Ok(Round {
            rows: Some(rows),
            waited: Some(why),
            ..Default::default()
        });
    }
    if !auto() {
        return Ok(only_check(rows, fixtures));
    }
    let done = update(fixtures, true, true, |_| {});
    let rows = if fixtures.is_some() {
        Vec::new()
    } else {
        // Can't list again: what waited before, less what was installed.
        list(false, true, None).unwrap_or_else(|_| left_over(rows, &done.updated))
    };
    Ok(Round {
        rows: Some(rows),
        done,
        waited: None,
        ran: true,
        unchecked: None,
        auto: true,
    })
}

/// What stops an update the user started from the Updates page: `None`
/// when the check found only what the page showed (`shown`, by row key);
/// otherwise the check's result, with nothing updated, so the rows get the
/// new notes and the user presses again.
pub fn unseen(checked: Done, shown: &std::collections::HashMap<String, HeldApp>) -> Option<Done> {
    let new = checked
        .held_back
        .iter()
        .any(|h| shown.get(&h.key).map(|s| &s.raw) != Some(&h.raw));
    if checked.error.is_none() && !new {
        return None;
    }
    let error = checked
        .error
        .map(|e| format!("couldn't check what the updates ask for: {e}"));
    Some(Done {
        updated: Vec::new(),
        held_back: checked.held_back,
        error,
    })
}

/// A round with background updates off: the waiting updates, and which of
/// them ask for new permissions.
fn only_check(rows: Vec<Row>, fixtures: Option<&Path>) -> Round {
    let mut checked = check(fixtures);
    let unchecked = checked.error.take();
    Round {
        rows: Some(rows),
        ran: unchecked.is_none(),
        done: checked,
        waited: None,
        unchecked,
        auto: false,
    }
}

fn left_over(rows: Vec<Row>, updated: &[apphistory::Entry]) -> Vec<Row> {
    rows.into_iter()
        .filter(|r| {
            !updated
                .iter()
                .any(|u| u.id == r.id && u.branch == r.branch && u.system == r.system)
        })
        .collect()
}

/// Names a notice, so the same one is given only once: the waiting
/// updates, which apps were held back and whether the update failed.
pub fn notice_key(rows: &[Row], held_back: &[HeldApp], failed: bool) -> String {
    let mut ids: Vec<String> = rows.iter().map(row_key).collect();
    ids.sort();
    let mut held: Vec<String> = held_back
        .iter()
        .map(|h| format!("held {}", h.key))
        .collect();
    held.sort();
    ids.extend(held);
    if failed {
        ids.push("failed".into());
    }
    // FNV-1a: short, stable across runs and versions.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in ids.join("\n").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The notification text for updates waiting on the user. `failed`: the
/// background update ran and failed.
pub fn ready_text(rows: &[Row], held_back: &[HeldApp], failed: bool, auto: bool) -> String {
    if !auto {
        // Nothing installed by itself: what is ready, and what to look at
        // before pressing "Update Apps".
        let ready = ready_list(rows);
        return match held_back {
            [] => ready,
            [one] => format!(
                "{ready} {} asks for new permissions ({}): open Atlas Updater to review it.",
                one.name,
                first_asks(&one.asks)
            ),
            more => format!(
                "{ready} {} of them ask for new permissions: open Atlas Updater to review them.",
                more.len()
            ),
        };
    }
    // The rows are what is still waiting after the round: those not held
    // are the ones that failed (others may have updated).
    let failed_rows = rows
        .iter()
        .filter(|r| !held_back.iter().any(|h| h.key == row_key(r)))
        .count();
    let also = match (failed, failed_rows) {
        (false, _) | (true, 0) => String::new(),
        (true, 1) => " 1 other app couldn't update either.".to_string(),
        (true, n) => format!(" {n} other apps couldn't update either."),
    };
    if let [one] = held_back {
        return format!(
            "{} asks for new permissions ({}), so it wasn't updated on its own. Open Atlas Updater to review it.{also}",
            one.name,
            first_asks(&one.asks)
        );
    }
    if !held_back.is_empty() {
        return format!(
            "{} apps ask for new permissions, so they weren't updated on their own. Open Atlas Updater to review them.{also}",
            held_back.len()
        );
    }
    if failed {
        return if rows.len() == 1 {
            "1 app couldn't update on its own. Open Atlas Updater to see why, or press Update Apps to try again.".to_string()
        } else {
            format!(
                "{} apps couldn't update on their own. Open Atlas Updater to see why, or press Update Apps to try again.",
                rows.len()
            )
        };
    }
    ready_list(rows)
}

/// A notice's room for what one app asks for: the first three.
fn first_asks(asks: &[String]) -> String {
    let mut shown: Vec<&str> = asks.iter().map(String::as_str).take(3).collect();
    if asks.len() > 3 {
        shown.push("more");
    }
    shown.join(", ")
}

/// "3 app updates are ready: Kate, Okular and 1 more."
fn ready_list(rows: &[Row]) -> String {
    let apps: Vec<&str> = rows
        .iter()
        .filter(|r| !r.runtime)
        .map(|r| r.name.as_str())
        .collect();
    let n = rows.len();
    let head = if n == 1 {
        "1 app update is ready".to_string()
    } else {
        format!("{n} app updates are ready")
    };
    match apps.as_slice() {
        [] => format!("{head}."),
        [a] => format!("{head}: {a}."),
        [a, b] => format!("{head}: {a} and {b}."),
        [a, b, rest @ ..] => format!("{head}: {a}, {b} and {} more.", rest.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, runtime: bool) -> Row {
        Row {
            name: name.into(),
            id: format!("org.example.{name}"),
            branch: "stable".into(),
            system: true,
            runtime,
            size: 0,
            size_text: String::new(),
            asks: String::new(),
        }
    }

    fn held(name: &str, asks: &[&str]) -> HeldApp {
        HeldApp {
            key: format!("org.example.{name}/stable/s"),
            name: name.into(),
            asks: asks.iter().map(|a| a.to_string()).collect(),
            raw: asks.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn ready_texts() {
        assert_eq!(
            ready_text(&[row("Kate", false)], &[], false, true),
            "1 app update is ready: Kate."
        );
        let rows = [
            row("Kate", false),
            row("Platform", true),
            row("Okular", false),
            row("Gwenview", false),
        ];
        assert_eq!(
            ready_text(&rows, &[], false, true),
            "4 app updates are ready: Kate, Okular and 1 more."
        );
        assert_eq!(
            ready_text(&rows[..3], &[], false, true),
            "3 app updates are ready: Kate and Okular."
        );
        assert_eq!(
            ready_text(&[row("Platform", true)], &[], false, true),
            "1 app update is ready."
        );
        assert_eq!(
            ready_text(&rows, &[held("Kate", &["your home folder"])], false, true),
            "Kate asks for new permissions (your home folder), so it wasn't updated on its own. Open Atlas Updater to review it."
        );
        assert_eq!(
            ready_text(
                &rows,
                &[held("Kate", &["a", "b", "c", "d"]), held("Okular", &[])],
                true,
                true
            ),
            "2 apps ask for new permissions, so they weren't updated on their own. Open Atlas Updater to review them. 2 other apps couldn't update either."
        );
        // One held, one failed (others updated and are no longer listed).
        assert_eq!(
            ready_text(
                &rows[..2],
                &[held("Kate", &["your home folder"])],
                true,
                true
            ),
            "Kate asks for new permissions (your home folder), so it wasn't updated on its own. Open Atlas Updater to review it. 1 other app couldn't update either."
        );
        // Failed, but all that is left is held: nothing more to say.
        assert_eq!(
            ready_text(&rows[..1], &[held("Kate", &["x"])], true, true),
            "Kate asks for new permissions (x), so it wasn't updated on its own. Open Atlas Updater to review it."
        );
        assert_eq!(
            ready_text(&rows, &[held("Kate", &["a", "b", "c", "d"])], false, true),
            "Kate asks for new permissions (a, b, c, more), so it wasn't updated on its own. Open Atlas Updater to review it."
        );
        assert_eq!(
            ready_text(&rows[..1], &[], true, true),
            "1 app couldn't update on its own. Open Atlas Updater to see why, or press Update Apps to try again."
        );
        assert_eq!(
            ready_text(&rows[..2], &[], true, true),
            "2 apps couldn't update on their own. Open Atlas Updater to see why, or press Update Apps to try again."
        );
        // Background updates off: nothing installed by itself.
        assert_eq!(
            ready_text(&rows, &[], false, false),
            "4 app updates are ready: Kate, Okular and 1 more."
        );
        assert_eq!(
            ready_text(&rows, &[held("Kate", &["your home folder"])], false, false),
            "4 app updates are ready: Kate, Okular and 1 more. Kate asks for new permissions (your home folder): open Atlas Updater to review it."
        );
        assert_eq!(
            ready_text(
                &rows,
                &[held("Kate", &[]), held("Okular", &[])],
                false,
                false
            ),
            "4 app updates are ready: Kate, Okular and 1 more. 2 of them ask for new permissions: open Atlas Updater to review them."
        );
    }

    #[test]
    fn page_updates_stop_for_what_the_page_did_not_show() {
        let kate = HeldApp {
            key: "k".into(),
            name: "Kate".into(),
            asks: vec!["the service org.x".into()],
            raw: vec!["Session Bus Policy: org.x=talk".into()],
        };
        let checked = |held: Vec<HeldApp>, error: Option<&str>| Done {
            held_back: held,
            error: error.map(String::from),
            ..Default::default()
        };
        let mut shown = std::collections::HashMap::new();
        // Nothing asks: go on.
        assert!(unseen(checked(vec![], None), &shown).is_none());
        // Asks for something the page didn't show: stop.
        let stop = unseen(checked(vec![kate.clone()], None), &shown).unwrap();
        assert!(stop.updated.is_empty() && stop.error.is_none());
        assert_eq!(stop.held_back, vec![kate.clone()]);
        // Shown just so: go on.
        shown.insert("k".to_string(), kate.clone());
        assert!(unseen(checked(vec![kate.clone()], None), &shown).is_none());
        // The same words for more (talk → own): stop.
        let mut owns = kate.clone();
        owns.raw = vec!["Session Bus Policy: org.x=own".into()];
        assert!(unseen(checked(vec![owns], None), &shown).is_some());
        // More than the words have room for: stop.
        let mut more = kate.clone();
        more.raw.push("Context: devices=all".into());
        assert!(unseen(checked(vec![more], None), &shown).is_some());
        // Couldn't check: stop, and say why.
        let failed = unseen(checked(vec![], Some("offline")), &shown).unwrap();
        assert_eq!(
            failed.error.as_deref(),
            Some("couldn't check what the updates ask for: offline")
        );
    }

    #[test]
    fn rounds_say_whether_they_installed() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        // Off: listed and checked, nothing installed by itself.
        let off = background(|| false, Some(&dir)).unwrap();
        assert!(off.rows.is_some_and(|r| !r.is_empty()));
        assert!(off.ran && !off.auto && off.unchecked.is_none());
        let on = background(|| true, Some(&dir)).unwrap();
        assert!(on.ran && on.auto);
    }

    #[test]
    fn set_keys_ignore_order_but_not_content() {
        let a = [row("Kate", false), row("Okular", false)];
        let b = [row("Okular", false), row("Kate", false)];
        assert_eq!(notice_key(&a, &[], false), notice_key(&b, &[], false));
        assert_ne!(notice_key(&a, &[], false), notice_key(&a[..1], &[], false));
        let mut user = row("Kate", false);
        user.system = false;
        assert_ne!(
            notice_key(&a[..1], &[], false),
            notice_key(&[user], &[], false)
        );
    }

    #[test]
    fn permissions_in_plain_words() {
        assert_eq!(describe("Context: filesystems=home"), "your home folder");
        assert_eq!(
            describe("Context: filesystems=xdg-music"),
            "filesystems=xdg-music"
        );
        assert_eq!(
            describe("System Bus Policy: org.freedesktop.login1=talk"),
            "the system service org.freedesktop.login1"
        );
        assert_eq!(
            describe("Application: runtime=com.example.Platform"),
            "a different runtime (com.example.Platform)"
        );
        assert_eq!(
            describe(flatpak::UNREADABLE),
            "permissions that couldn't be checked"
        );
    }

    #[test]
    fn session_and_system_bus_read_differently() {
        assert_eq!(
            describe("Context: sockets=session-bus"),
            "every app's services on your desktop"
        );
        assert_eq!(
            describe("Context: sockets=system-bus"),
            "every system service"
        );
        assert_eq!(describe(flatpak::NEW_APP), "a new app to install");
        // Text from the remote is cleaned.
        assert_eq!(describe("Context: x=a\u{202E}b\nc"), "x=ab c");
    }

    #[test]
    fn weightiest_permissions_come_first() {
        let p = vec![
            "Context: devices=dri".to_string(),
            "Context: allow=bluetooth".to_string(),
            "Context: filesystems=xdg-music".to_string(),
            "Context: filesystems=host".to_string(),
        ];
        assert_eq!(by_weight(p)[0], "Context: filesystems=host");
        let p = vec![
            "Context: devices=dri".to_string(),
            "Session Bus Policy: org.freedesktop.Flatpak=talk".to_string(),
        ];
        assert_eq!(
            by_weight(p)[0],
            "Session Bus Policy: org.freedesktop.Flatpak=talk"
        );
    }

    #[test]
    fn rows_show_five_asks_at_most() {
        let asks: Vec<String> = (1..=7).map(|n| format!("p{n}")).collect();
        assert_eq!(asks_text(&asks), "p1, p2, p3, p4, p5, more");
        assert_eq!(asks_text(&asks[..2]), "p1, p2");
    }

    fn app(id: &str) -> Row {
        Row {
            name: id.into(),
            id: id.into(),
            branch: "stable".into(),
            system: true,
            runtime: false,
            size: 0,
            size_text: String::new(),
            asks: String::new(),
        }
    }

    #[test]
    fn a_notice_differs_when_what_it_says_differs() {
        let rows = vec![app("a"), app("b")];
        let held = vec![HeldApp {
            key: row_key(&rows[0]),
            name: "a".into(),
            asks: vec![],
            raw: vec![],
        }];
        let plain = notice_key(&rows, &[], false);
        assert_ne!(plain, notice_key(&rows, &held, false));
        assert_ne!(plain, notice_key(&rows, &[], true));
        assert_eq!(notice_key(&rows, &[], true), notice_key(&rows, &[], true));
    }

    #[test]
    fn left_over_drops_what_was_installed() {
        let rows = vec![app("a"), app("b")];
        let installed = apphistory::Entry {
            at: 1,
            id: "a".into(),
            name: "a".into(),
            branch: "stable".into(),
            runtime: false,
            system: true,
            from: None,
            to: None,
            auto: true,
        };
        let left = left_over(rows, &[installed]);
        assert_eq!(
            left.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["b"]
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(12_300_000), "12.3 MB");
    }
}
