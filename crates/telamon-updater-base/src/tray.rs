//! How the window reaches the tray (`telamon-updater-tray`): one method,
//! `Reload`, after it changed a setting the tray acts on (the scheduled
//! restart, background app updates, crash reports). The call starts the
//! tray through D-Bus activation if it isn't running.

use std::time::Duration;

/// The tray's bus name. Owning it also keeps the tray single-instance.
pub const BUS_NAME: &str = "net.eterneon.telamon.updater.Tray";
pub const PATH: &str = "/net/eterneon/telamon/updater/Tray";
pub const INTERFACE: &str = "net.eterneon.telamon.updater.Tray";
/// The names the tray had until 0.3.0. The tray still owns them (same
/// method, same implementation) for programs that have not moved, and the
/// window calls them when only an older tray, still running from before the
/// upgrade, answers. Remove in the release after the next.
pub const LEGACY_BUS_NAME: &str = "net.eterneon.atlas.updater.Tray";
pub const LEGACY_PATH: &str = "/net/eterneon/atlas/updater/Tray";
pub const LEGACY_INTERFACE: &str = "net.eterneon.atlas.updater.Tray";
/// The window's single-instance name (KDBusService, from the app ID).
pub const WINDOW_BUS_NAME: &str = "net.eterneon.telamon.updater";

/// Long enough for D-Bus activation to start the tray.
const LIMIT: Duration = Duration::from_secs(25);

/// Asks the tray to read the settings again. Blocking: call from a worker
/// thread. The tray answers on its new name; an older one, still running
/// from before the upgrade, on the old: that one is tried when the new name
/// has no owner and cannot be started.
pub fn reload() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let call = async {
            let conn = zbus::Connection::session().await?;
            call_reload(&conn).await
        };
        match tokio::time::timeout(LIMIT, call).await {
            Ok(r) => r.map_err(|e| e.to_string()),
            Err(_) => Err("the tray did not answer".into()),
        }
    })
}

/// `Reload` on the tray `conn` can reach: the new name; the old one only if
/// the new has no owner and no activation file (an older tray runs).
pub async fn call_reload(conn: &zbus::Connection) -> zbus::Result<()> {
    let new = reload_call(conn, BUS_NAME, PATH, INTERFACE).await;
    let Err(zbus::Error::MethodError(name, ..)) = &new else {
        return new;
    };
    if !is_bus_error(name.as_str()) {
        return new;
    }
    match reload_call(conn, LEGACY_BUS_NAME, LEGACY_PATH, LEGACY_INTERFACE).await {
        Ok(()) => Ok(()),
        // the old name answers no better: report the first failure
        Err(_) => new,
    }
}

/// An error of the bus itself (nobody owns the name and no file starts one,
/// or what started did not answer), as opposed to one the tray returned.
fn is_bus_error(name: &str) -> bool {
    name.starts_with("org.freedesktop.DBus.Error.")
}

async fn reload_call(
    conn: &zbus::Connection,
    bus_name: &str,
    path: &str,
    interface: &str,
) -> zbus::Result<()> {
    conn.call_method(Some(bus_name), path, Some(interface), "Reload", &())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_names_are_the_new_ones_with_atlas_for_telamon() {
        for (old, new) in [
            (LEGACY_BUS_NAME, BUS_NAME),
            (LEGACY_INTERFACE, INTERFACE),
            (LEGACY_PATH, PATH),
        ] {
            assert_eq!(old, new.replace("telamon", "atlas"));
        }
        assert_eq!(WINDOW_BUS_NAME, "net.eterneon.telamon.updater");
        assert_eq!(BUS_NAME, format!("{WINDOW_BUS_NAME}.Tray"));
    }

    #[test]
    fn only_the_buss_own_errors_make_the_old_name_worth_a_try() {
        for name in [
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.NameHasNoOwner",
            "org.freedesktop.DBus.Error.Spawn.ChildExited",
            "org.freedesktop.DBus.Error.NoReply",
        ] {
            assert!(is_bus_error(name), "{name}");
        }
        assert!(!is_bus_error("net.eterneon.telamon.Error.Failed"));
        assert!(!is_bus_error("org.freedesktop.zbus.Error"));
    }
}
