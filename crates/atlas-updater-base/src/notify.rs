//! Desktop notifications over `org.freedesktop.Notifications`, sent the way
//! KNotification sends the events in `atlas-updater.notifyrc`: Plasma groups
//! them under Atlas Updater and its notification settings apply.

use std::collections::HashMap;
use std::path::Path;

use atlas_framework_core::settings::{Settings, config_dir};
use zbus::zvariant::Value;

/// The notifyrc's name (KNotification's component name).
pub const COMPONENT: &str = "atlas-updater";
pub const DESKTOP_ENTRY: &str = "net.eterneon.atlas.updater";
pub const APP_NAME: &str = "Atlas Updater";
pub const APP_ICON: &str = "net.eterneon.atlas.updater";

pub const SERVICE: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";

/// The key the server sends when the notification itself is clicked.
pub const DEFAULT_ACTION: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Normal,
    /// KNotification's HighUrgency: the spec has no such level, so it goes
    /// out as normal, as KNotification sends it.
    High,
    Critical,
}

#[derive(Debug, Clone)]
pub struct Note {
    /// The notifyrc event (`updateStaged`, `restartSoon`, …).
    pub event: &'static str,
    pub title: String,
    /// Body markup: escape anything that came from outside with [`escape`].
    pub text: String,
    pub icon: &'static str,
    /// (key, label) pairs. [`DEFAULT_ACTION`] is the click on the
    /// notification itself.
    pub actions: Vec<(&'static str, String)>,
    pub urgency: Option<Urgency>,
    /// Stays until the user acts (no timeout).
    pub persistent: bool,
}

/// Escapes text for the body, which the server reads as markup.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Whether `event` pops up. Plasma's notification settings write the user's
/// choice to `~/.config/atlas-updater.notifyrc` (`[Event/<id>] Action=`, a
/// `|`-separated list); without one, the shipped file's `Popup` applies.
pub fn popup_enabled(event: &str) -> bool {
    popup_in(&config_dir().join(format!("{COMPONENT}.notifyrc")), event)
}

fn popup_in(path: &Path, event: &str) -> bool {
    // Only a regular file: anything else would block the reader.
    if !path.is_file() {
        return true;
    }
    match Settings::at(path).get(&format!("Event/{event}"), "Action") {
        Some(actions) => actions.split('|').any(|a| a.trim() == "Popup"),
        None => true,
    }
}

/// A notification the server showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub id: u32,
    /// The server's unique bus name: only its signals are about `id`.
    pub server: Option<String>,
}

/// Sends `n`. `Ok(None)`: the user turned this event's popup off.
pub async fn send(conn: &zbus::Connection, n: &Note) -> zbus::Result<Option<Sent>> {
    if !popup_enabled(n.event) {
        return Ok(None);
    }
    let mut actions: Vec<&str> = Vec::with_capacity(n.actions.len() * 2);
    for (key, label) in &n.actions {
        actions.push(key);
        actions.push(label);
    }
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("desktop-entry", Value::from(DESKTOP_ENTRY));
    hints.insert("x-kde-appname", Value::from(COMPONENT));
    hints.insert("x-kde-eventId", Value::from(n.event));
    if let Some(u) = n.urgency {
        let level: u8 = match u {
            Urgency::Normal | Urgency::High => 1,
            Urgency::Critical => 2,
        };
        hints.insert("urgency", Value::from(level));
    }
    let timeout: i32 = if n.persistent { 0 } else { -1 };
    let reply = conn
        .call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "Notify",
            &(
                APP_NAME,
                0u32,
                n.icon,
                n.title.as_str(),
                n.text.as_str(),
                actions,
                hints,
                timeout,
            ),
        )
        .await?;
    let id = reply.body().deserialize::<u32>()?;
    let server = reply.header().sender().map(|s| s.to_string());
    Ok(Some(Sent { id, server }))
}

pub async fn close(conn: &zbus::Connection, id: u32) -> zbus::Result<()> {
    conn.call_method(
        Some(SERVICE),
        PATH,
        Some(INTERFACE),
        "CloseNotification",
        &id,
    )
    .await?;
    Ok(())
}

/// [`send`] from a worker thread, for a notification nobody acts on (its
/// actions are not followed). Blocking.
pub fn send_blocking(n: &Note) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let go = async {
            let conn = zbus::Connection::session().await?;
            send(&conn, n).await
        };
        match tokio::time::timeout(std::time::Duration::from_secs(10), go).await {
            Ok(r) => r.map(|_| ()).map_err(|e| e.to_string()),
            Err(_) => Err("the notification server did not answer".into()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup() {
        assert_eq!(
            escape("<b>Tom & \"Jerry's\"</b>"),
            "&lt;b&gt;Tom &amp; &quot;Jerry&#39;s&quot;&lt;/b&gt;"
        );
    }

    #[test]
    fn popup_follows_the_users_notifyrc() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("atlas-updater.notifyrc");
        // no file, no group, no key: the shipped default (Popup)
        assert!(popup_in(&p, "updateStaged"));
        std::fs::write(
            &p,
            "[Event/updateStaged]\nAction=Sound\n\n[Event/restartSoon]\nAction=Popup|Sound\n\n[Event/crashReport]\nAction=\n",
        )
        .unwrap();
        assert!(!popup_in(&p, "updateStaged"));
        assert!(popup_in(&p, "restartSoon"));
        assert!(!popup_in(&p, "crashReport"));
        assert!(popup_in(&p, "appUpdatesReady"));
    }
}
