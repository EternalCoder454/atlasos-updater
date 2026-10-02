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
    with_timeout(&rt, std::time::Duration::from_secs(60), call)
}

/// Runs `fut` on `rt`, giving up after `limit`. The timer must be created
/// inside the runtime: `tokio::time::timeout` built outside `block_on` panics
/// with "there is no reactor running".
fn with_timeout<F>(
    rt: &tokio::runtime::Runtime,
    limit: std::time::Duration,
    fut: F,
) -> Result<(), String>
where
    F: std::future::Future<Output = Result<(), String>>,
{
    rt.block_on(async { tokio::time::timeout(limit, fut).await })
        .unwrap_or_else(|_| Err(NO_ANSWER.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn answer_in_time_is_passed_through() {
        let rt = runtime();
        assert_eq!(
            with_timeout(&rt, Duration::from_secs(5), async { Ok(()) }),
            Ok(())
        );
        assert_eq!(
            with_timeout(&rt, Duration::from_secs(5), async {
                Err("refused".to_string())
            }),
            Err("refused".to_string())
        );
    }

    #[test]
    fn no_answer_times_out_without_panicking() {
        let rt = runtime();
        let never = std::future::pending::<Result<(), String>>();
        assert_eq!(
            with_timeout(&rt, Duration::from_millis(50), never),
            Err(NO_ANSWER.to_string())
        );
    }
}
