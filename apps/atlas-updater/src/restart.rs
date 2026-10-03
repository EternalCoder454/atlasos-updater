//! Restart through Plasma, so apps get to save first.

/// The error for a request Plasma did not answer in time. The request may
/// still go through later, so callers must not treat it as a failure to report.
pub const NO_ANSWER: &str = "Plasma hasn't confirmed the restart yet. If a logout prompt is showing, answer it; otherwise restart from the system menu.";

/// How long Plasma gets to start logging out after taking the request.
const START_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

/// How long a logout may run (apps asking to save their work) before the
/// wait gives up; it never waits for good.
const LOGOUT_LIMIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// The logout started, then ended without a restart: an app with unsaved
/// work, or the user, canceled it. Not an error.
pub const CANCELED: &str =
    "The restart was canceled before the session ended. The update is still waiting.";

/// `org.kde.Shutdown /Shutdown logoutAndReboot` on the session bus, then
/// wait while Plasma logs out: it answers the call at once, before apps are
/// closed, so returning then would show the restart as over while it runs.
/// Returns only if the logout did not go through ([`CANCELED`]), could not be
/// seen starting ([`NO_ANSWER`]: a prompt may be waiting for an answer), or
/// failed; on success the session (and this process) ends first.
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
    with_timeout(&rt, std::time::Duration::from_secs(60), call)?;
    let conn = rt.block_on(zbus::Connection::session()).ok();
    let ask = || {
        let conn = conn.as_ref()?;
        rt.block_on(async {
            let call = conn.call_method(
                Some("org.kde.ksmserver"),
                "/KSMServer",
                Some("org.kde.KSMServerInterface"),
                "isShuttingDown",
                &(),
            );
            let reply = tokio::time::timeout(std::time::Duration::from_secs(2), call)
                .await
                .ok()?
                .ok()?;
            reply.body().deserialize::<bool>().ok()
        })
    };
    wait_for_logout(ask, std::thread::sleep, START_LIMIT, LOGOUT_LIMIT)
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

/// Polls `shutting_down` (ksmserver's `isShuttingDown`; `None` when it
/// doesn't answer, as it may not while busy logging out) once a second, for
/// as long as the logout runs. Ends with [`CANCELED`] once a logout that had
/// started is over; with [`NO_ANSWER`] if none was seen starting within
/// `start_limit`, or the logout still runs after `limit`.
fn wait_for_logout(
    mut shutting_down: impl FnMut() -> Option<bool>,
    mut sleep: impl FnMut(std::time::Duration),
    start_limit: std::time::Duration,
    limit: std::time::Duration,
) -> Result<(), String> {
    let step = std::time::Duration::from_secs(1);
    let mut waited = std::time::Duration::ZERO;
    let mut started = false;
    // "no" twice in a row: one alone can fall between steps of the logout
    let mut no = 0;
    loop {
        match shutting_down() {
            Some(true) => {
                started = true;
                no = 0;
            }
            None => no = 0,
            Some(false) => no += 1,
        }
        if started && no >= 2 {
            return Err(CANCELED.to_string());
        }
        if (!started && waited >= start_limit) || waited >= limit {
            return Err(NO_ANSWER.to_string());
        }
        sleep(step);
        waited += step;
    }
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

    /// Runs `wait_for_logout` over scripted answers; the last one repeats.
    fn wait(answers: &[Option<bool>]) -> (Result<(), String>, usize) {
        let mut i = 0;
        let res = wait_for_logout(
            || {
                let a = answers[i.min(answers.len() - 1)];
                i += 1;
                a
            },
            |_| {},
            Duration::from_secs(20),
            Duration::from_secs(600),
        );
        (res, i)
    }

    #[test]
    fn a_canceled_logout_ends_the_wait() {
        let (res, polls) = wait(&[Some(true), Some(true), Some(false), Some(false)]);
        assert_eq!(res, Err(CANCELED.to_string()));
        assert_eq!(polls, 4);
        // one "no" between steps is not a cancel
        let (_, polls) = wait(&[
            Some(true),
            Some(false),
            Some(true),
            Some(false),
            Some(false),
        ]);
        assert_eq!(polls, 5);
    }

    #[test]
    fn a_logout_that_never_starts_ends_after_the_limit() {
        // a prompt may be up: not called canceled
        let (res, polls) = wait(&[Some(false)]);
        assert_eq!(res, Err(NO_ANSWER.to_string()));
        assert_eq!(polls, 21);
        // ksmserver missing, or the bus gone: still ends
        let (res, polls) = wait(&[None]);
        assert_eq!(res, Err(NO_ANSWER.to_string()));
        assert_eq!(polls, 21);
    }

    #[test]
    fn a_logout_that_never_ends_is_given_up_on() {
        let (res, polls) = wait(&[Some(true), None]);
        assert_eq!(res, Err(NO_ANSWER.to_string()));
        assert_eq!(polls, 601);
    }

    #[test]
    fn a_busy_ksmserver_is_waited_for() {
        // no answer while it closes apps, then "no" once it's done canceling
        let (res, polls) = wait(&[Some(true), None, None, None, Some(false), Some(false)]);
        assert_eq!(res, Err(CANCELED.to_string()));
        assert_eq!(polls, 6);
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
