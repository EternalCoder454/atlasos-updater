//! What the glow follows on the buses: callers of `SetWorking` leaving the
//! session bus, and the system helper's `Progress` on the system bus. Both
//! are tasks that send their findings to the tray as messages.
//!
//! The helper is D-Bus activated and exits when idle: nothing here ever calls
//! one of its methods or makes a proxy for it. `Progress` is read (once, and
//! only when the helper is on the bus) with `Properties.Get` sent to its
//! unique name, which never starts a service, and followed with a match rule
//! for `PropertiesChanged`.

use std::collections::HashMap;
use std::time::Duration;

use telamon_update_engine::identity::Identity;
use telamon_update_engine::progress::is_image_operation;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use zbus::message::Type as MsgType;
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, MessageStream};

use crate::Msg;
use crate::working::Helper;

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The property that says an upgrade or a switch runs.
const PROGRESS: &str = "Progress";
/// An answer from the bus or the helper that takes longer is none.
const CALL_LIMIT: Duration = Duration::from_secs(10);
/// No system bus yet: try again after this.
const SYSTEM_RETRY: Duration = Duration::from_secs(60);

fn identity(which: Helper) -> Identity {
    match which {
        Helper::Telamon => Identity::Telamon,
        Helper::Legacy => Identity::Legacy,
    }
}

/// Tells the tray (`Msg::SenderGone`) when `sender` (a unique name) is gone
/// from the session bus: at once if it already is. Abort the task when the
/// claim is released.
pub fn watch_sender(
    conn: zbus::Connection,
    sender: String,
    tx: UnboundedSender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let gone = |tx: &UnboundedSender<Msg>, sender: String| {
            let _ = tx.send(Msg::SenderGone(sender));
        };
        let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
            eprintln!("telamon-updater-tray: cannot watch {sender} leave the bus");
            return;
        };
        // The stream first, so a departure between the two is not missed.
        let Ok(mut changes) = dbus
            .receive_name_owner_changed_with_args(&[(0, sender.as_str())])
            .await
        else {
            eprintln!("telamon-updater-tray: cannot watch {sender} leave the bus");
            return;
        };
        if let Ok(name) = zbus::names::BusName::try_from(sender.as_str())
            && matches!(dbus.name_has_owner(name).await, Ok(false))
        {
            return gone(&tx, sender);
        }
        while let Some(signal) = next(&mut changes).await {
            if let Ok(args) = signal.args()
                && args.new_owner().is_none()
            {
                return gone(&tx, sender);
            }
        }
    })
}

async fn next<S: zbus::export::futures_core::Stream + Unpin>(s: &mut S) -> Option<S::Item> {
    std::future::poll_fn(|cx| std::pin::Pin::new(&mut *s).poll_next(cx)).await
}

/// `Progress` of the helper under both its names, as `Msg::Helper`. The
/// system bus is asked for when it is there; without it, again every minute
/// (logged once).
pub fn follow_helper(tx: UnboundedSender<Msg>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut logged = false;
        let conn = loop {
            match zbus::Connection::system().await {
                Ok(c) => break c,
                Err(e) => {
                    if !std::mem::replace(&mut logged, true) {
                        eprintln!(
                            "telamon-updater-tray: no system bus: the update glow follows only what Settings says ({e})"
                        );
                    }
                    tokio::time::sleep(SYSTEM_RETRY).await;
                }
            }
        };
        let names = [Helper::Telamon, Helper::Legacy].map(|w| {
            let conn = conn.clone();
            let tx = tx.clone();
            tokio::spawn(follow_one(conn, w, tx))
        });
        for t in names {
            let _ = t.await;
        }
    })
}

fn props_rule(id: Identity) -> zbus::Result<MatchRule<'static>> {
    // Not restricted to the helper's sender here (that would be a name that
    // may have no owner yet): the sender is checked against the name's
    // owner when a signal arrives.
    Ok(MatchRule::builder()
        .msg_type(MsgType::Signal)
        .interface(PROPERTIES)?
        .member("PropertiesChanged")?
        .path(id.object_path())?
        .arg(0, id.interface())?
        .build())
}

fn owner_rule(id: Identity) -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder()
        .msg_type(MsgType::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, id.bus_name())?
        .build())
}

async fn stream_of(
    conn: &zbus::Connection,
    rule: zbus::Result<MatchRule<'static>>,
) -> Option<MessageStream> {
    MessageStream::for_match_rule(rule.ok()?, conn, Some(64))
        .await
        .ok()
}

/// The unique name that owns `name`, if anyone does. Never starts a service.
async fn owner_of(conn: &zbus::Connection, name: &str) -> Option<String> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(name).ok()?;
    let call = dbus.get_name_owner(name);
    tokio::time::timeout(CALL_LIMIT, call)
        .await
        .ok()?
        .ok()
        .map(|o| o.to_string())
}

/// Whether `Progress` says the OS image is being changed, asked of `owner`
/// (a unique name: the call cannot start anything).
async fn read_progress(conn: &zbus::Connection, id: Identity, owner: &str) -> Option<bool> {
    let args = (id.interface(), PROGRESS);
    let call = conn.call_method(
        Some(owner),
        id.object_path(),
        Some(PROPERTIES),
        "Get",
        &args,
    );
    let reply = tokio::time::timeout(CALL_LIMIT, call).await.ok()?.ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    image_op_running(&value)
}

/// `Progress` names an operation that changes the OS image (an upgrade or a
/// switch, `telamon_update_engine::progress::is_image_operation`): the only
/// thing the helper's word turns the glow on for. An empty value, any other
/// `op` and a value that is not JSON do not. `None` for anything that is not a
/// string.
fn image_op_running(v: &OwnedValue) -> Option<bool> {
    let s: &str = v.downcast_ref().ok()?;
    Some(is_image_operation(s))
}

/// What a `PropertiesChanged` body says about `Progress`: `Some(Some(set))`
/// it came with a value, `Some(None)` it was invalidated (read it), `None`
/// it is not about it.
fn progress_change(body: &zbus::message::Body) -> Option<Option<bool>> {
    let (_, changed, invalidated): (String, HashMap<String, OwnedValue>, Vec<String>) =
        body.deserialize().ok()?;
    if let Some(v) = changed.get(PROGRESS) {
        return Some(image_op_running(v));
    }
    invalidated.iter().any(|p| p == PROGRESS).then_some(None)
}

async fn follow_one(conn: zbus::Connection, which: Helper, tx: UnboundedSender<Msg>) {
    let id = identity(which);
    let mut props = stream_of(&conn, props_rule(id)).await;
    let mut owners = stream_of(&conn, owner_rule(id)).await;
    if props.is_none() || owners.is_none() {
        eprintln!(
            "telamon-updater-tray: cannot follow the helper's progress ({})",
            id.bus_name()
        );
        return;
    }
    let send = |set: bool| {
        let _ = tx.send(Msg::Helper(which, set));
    };
    // Someone may be upgrading already.
    let mut owner = owner_of(&conn, id.bus_name()).await;
    if let Some(o) = &owner
        && let Some(set) = read_progress(&conn, id, o).await
    {
        send(set);
    }
    loop {
        tokio::select! {
            m = crate::next_message(&mut props) => {
                let Some(m) = m else { continue };
                let Some(sender) = m.header().sender().map(|s| s.to_string()) else {
                    continue;
                };
                // Only the helper's own word counts, not any program's
                // that sends the same signal.
                if owner.as_deref() != Some(sender.as_str()) {
                    owner = owner_of(&conn, id.bus_name()).await;
                    if owner.as_deref() != Some(sender.as_str()) {
                        continue;
                    }
                }
                match progress_change(&m.body()) {
                    Some(Some(set)) => send(set),
                    Some(None) => {
                        if let Some(set) = read_progress(&conn, id, &sender).await {
                            send(set);
                        }
                    }
                    None => {}
                }
            }
            m = crate::next_message(&mut owners) => {
                let Some(m) = m else { continue };
                if let Ok((_, _, new)) = m.body().deserialize::<(String, String, String)>() {
                    if new.is_empty() {
                        // The helper is gone: whatever it was doing is over.
                        owner = None;
                        send(false);
                    } else {
                        owner = Some(new);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    #[test]
    fn the_glow_follows_only_image_operations() {
        let v = |s: &str| OwnedValue::try_from(Value::from(s.to_string())).unwrap();
        assert_eq!(image_op_running(&v("")), Some(false));
        assert_eq!(image_op_running(&v(r#"{"op":"upgrade"}"#)), Some(true));
        assert_eq!(image_op_running(&v(r#"{"op":"switch"}"#)), Some(true));
        // anything else the helper (or a newer one) might report: no glow
        for other in [
            r#"{"op":"apps"}"#,
            r#"{"op":"firmware"}"#,
            r#"{"op":"check"}"#,
            r#"{"stage":"downloading"}"#,
            "downloading",
        ] {
            assert_eq!(image_op_running(&v(other)), Some(false), "{other}");
        }
        let n = OwnedValue::try_from(Value::from(7u32)).unwrap();
        assert_eq!(image_op_running(&n), None);
    }

    #[test]
    fn the_rules_name_the_helpers_two_identities() {
        let a = props_rule(Identity::Telamon).unwrap().to_string();
        assert!(
            a.contains("path='/net/eterneon/telamon/SystemHelper'"),
            "{a}"
        );
        assert!(
            a.contains("arg0='net.eterneon.telamon.SystemHelper1'"),
            "{a}"
        );
        let b = props_rule(Identity::Legacy).unwrap().to_string();
        assert!(b.contains("path='/net/eterneon/atlas/SystemHelper'"), "{b}");
        let o = owner_rule(Identity::Legacy).unwrap().to_string();
        assert!(o.contains("arg0='net.eterneon.atlas.SystemHelper'"), "{o}");
    }
}
