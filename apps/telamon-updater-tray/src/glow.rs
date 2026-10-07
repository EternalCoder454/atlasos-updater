//! The glow's process: `telamon-updater-glow` (Qt, a program of its own so
//! the tray stays small), started and ended on the decisions of
//! `working::Machine`. Which one: `TELAMON_UPDATER_GLOW_BIN` if set (tests
//! and developers), else `/usr/libexec/telamon-updater-glow`.

use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};

use tokio::process::{Child, Command};

pub const DEFAULT_BIN: &str = "/usr/libexec/telamon-updater-glow";

pub fn glow_bin() -> PathBuf {
    glow_bin_from(std::env::var_os("TELAMON_UPDATER_GLOW_BIN").map(PathBuf::from))
}

fn glow_bin_from(env: Option<PathBuf>) -> PathBuf {
    env.filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BIN))
}

/// What a glow that ended says, for the log.
pub fn describe(status: &std::io::Result<ExitStatus>) -> String {
    match status {
        Ok(s) => match (s.code(), s.signal()) {
            (Some(c), _) => format!("exited with status {c}"),
            (None, Some(sig)) => format!("ended by signal {sig}"),
            _ => "ended".into(),
        },
        Err(e) => format!("could not be waited for: {e}"),
    }
}

pub struct Glow {
    program: PathBuf,
    child: Option<Child>,
}

impl Glow {
    pub fn new(program: PathBuf) -> Glow {
        Glow {
            program,
            child: None,
        }
    }

    #[cfg(test)]
    pub fn running(&self) -> bool {
        self.child.is_some()
    }

    /// Starts the glow (not if one is running already). It ends with the
    /// tray: SIGTERM when the tray's thread ends, and a kill if this is
    /// dropped.
    pub fn spawn(&mut self) -> std::io::Result<()> {
        if self.child.is_some() {
            return Ok(());
        }
        let mut cmd = Command::new(&self.program);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .env_remove("XDG_ACTIVATION_TOKEN")
            .kill_on_drop(true);
        // SAFETY: prctl is async-signal-safe; nothing else runs between fork
        // and exec. As for the app update worker: it must not outlive the tray.
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        self.child = Some(cmd.spawn()?);
        Ok(())
    }

    /// Asks it to end (SIGTERM).
    pub fn terminate(&mut self) {
        self.signal(libc::SIGTERM);
    }

    /// Ends it (SIGKILL).
    pub fn kill(&mut self) {
        self.signal(libc::SIGKILL);
    }

    fn signal(&mut self, sig: libc::c_int) {
        // `id()` is None once the child was reaped: never a stale pid.
        if let Some(pid) = self.child.as_ref().and_then(|c| c.id())
            && let Ok(pid) = libc::pid_t::try_from(pid)
        {
            // SAFETY: kill(2) on a child of ours that was not reaped yet.
            unsafe {
                libc::kill(pid, sig);
            }
        }
    }

    /// Resolves when the process ends (and forgets it); never when none
    /// runs. Safe to drop and call again.
    pub async fn exited(&mut self) -> std::io::Result<ExitStatus> {
        let Some(child) = self.child.as_mut() else {
            return std::future::pending().await;
        };
        let status = child.wait().await;
        self.child = None;
        status
    }

    /// For the tray's exit: SIGTERM, a moment to end, then SIGKILL.
    pub async fn shutdown(&mut self) {
        if self.child.is_none() {
            return;
        }
        self.terminate();
        let ended = tokio::time::timeout(crate::working::TERM_GRACE, self.exited()).await;
        if ended.is_err() {
            self.kill();
            let _ = tokio::time::timeout(crate::working::TERM_GRACE, self.exited()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_is_the_override_or_libexec() {
        assert_eq!(
            glow_bin_from(None),
            PathBuf::from("/usr/libexec/telamon-updater-glow")
        );
        assert_eq!(
            glow_bin_from(Some("/x/glow".into())),
            PathBuf::from("/x/glow")
        );
        assert_eq!(glow_bin_from(Some("".into())), PathBuf::from(DEFAULT_BIN));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_missing_program_does_not_start() {
        let mut g = Glow::new("/nonexistent/glow".into());
        assert!(g.spawn().is_err());
        assert!(!g.running());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn it_ends_when_asked_and_is_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("glow");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let mut g = Glow::new(script);
        g.spawn().unwrap();
        assert!(g.running());
        // a second spawn changes nothing
        g.spawn().unwrap();
        g.terminate();
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), g.exited())
            .await
            .expect("it ended")
            .unwrap();
        assert!(!status.success());
        assert!(!g.running());
        // after that, signals go nowhere
        g.terminate();
        g.kill();
    }
}
