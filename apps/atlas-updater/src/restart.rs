//! Restart through Plasma, so apps get to save first.

/// `org.kde.Shutdown /Shutdown logoutAndReboot` on the session bus.
/// Blocking; call from a worker thread.
pub fn logout_and_reboot() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let conn = zbus::Connection::session()
            .await
            .map_err(|e| e.to_string())?;
        conn.call_method(
            Some("org.kde.Shutdown"),
            "/Shutdown",
            Some("org.kde.Shutdown"),
            "logoutAndReboot",
            &(),
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}
