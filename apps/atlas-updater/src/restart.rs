//! Restart through Plasma, so apps get to save first.

/// The error for a request Plasma did not answer in time. The request may
/// still go through later, so callers must not treat it as a failure to report.
pub const NO_ANSWER: &str = "Plasma hasn't confirmed the restart yet. If a logout prompt is showing, answer it; otherwise restart from the system menu.";

/// `org.kde.Shutdown /Shutdown logoutAndReboot` on the session bus.
/// Blocking; call from a worker thread.
pub fn logout_and_reboot() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    // Plasma may never answer; the caller's "restarting" state must not stick.
    let call = async {
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
    };
    rt.block_on(tokio::time::timeout(
        std::time::Duration::from_secs(60),
        call,
    ))
    .unwrap_or_else(|_| Err(NO_ANSWER.to_string()))
}
