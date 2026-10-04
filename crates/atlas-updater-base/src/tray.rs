//! How the window reaches the tray (`atlas-updater-tray`): one method,
//! `Reload`, after it changed a setting the tray acts on (the scheduled
//! restart, background app updates, crash reports). The call starts the
//! tray through D-Bus activation if it isn't running.

use std::time::Duration;

/// The tray's bus name. Owning it also keeps the tray single-instance.
pub const BUS_NAME: &str = "net.eterneon.atlas.updater.Tray";
pub const PATH: &str = "/net/eterneon/atlas/updater/Tray";
pub const INTERFACE: &str = "net.eterneon.atlas.updater.Tray";
/// The window's single-instance name (KDBusService, from the app ID).
pub const WINDOW_BUS_NAME: &str = "net.eterneon.atlas.updater";

/// Long enough for D-Bus activation to start the tray.
const LIMIT: Duration = Duration::from_secs(25);

/// Asks the tray to read the settings again. Blocking: call from a worker
/// thread.
pub fn reload() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let call = async {
            let conn = zbus::Connection::session().await?;
            conn.call_method(Some(BUS_NAME), PATH, Some(INTERFACE), "Reload", &())
                .await?;
            Ok::<(), zbus::Error>(())
        };
        match tokio::time::timeout(LIMIT, call).await {
            Ok(r) => r.map_err(|e| e.to_string()),
            Err(_) => Err("the tray did not answer".into()),
        }
    })
}
