//! One background thread that sleeps until the next thing is due: the 6 h
//! status poll, the restart warning, the scheduled restart. It wakes at no
//! other time, so the idle tray process costs nothing.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const POLL_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
pub const WARN_BEFORE_SECS: i64 = 5 * 60;
/// While a restart is scheduled, re-check the wall clock at least this often
/// (the monotonic sleep does not count time spent suspended).
const WALL_RECHECK: Duration = Duration::from_secs(30);

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Default)]
struct State {
    restart_at: Option<i64>,
    warned: bool,
    quit: bool,
}

#[derive(Clone, Default)]
pub struct Schedule {
    inner: Arc<(Mutex<State>, Condvar)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Poll,
    RestartWarning,
    RestartDue,
}

impl Schedule {
    pub fn set_restart(&self, at: Option<i64>) {
        let (m, cv) = &*self.inner;
        let mut s = m.lock().unwrap();
        s.restart_at = at;
        // No warning when the user picks a time that is already close.
        s.warned = at.is_some_and(|t| t - unix_now() <= WARN_BEFORE_SECS);
        cv.notify_all();
    }

    pub fn stop(&self) {
        let (m, cv) = &*self.inner;
        m.lock().unwrap().quit = true;
        cv.notify_all();
    }

    /// Runs `fire` for each due event until `stop()`. Blocks: use a thread.
    pub fn run(&self, mut fire: impl FnMut(Event)) {
        let (m, cv) = &*self.inner;
        let mut next_poll = Instant::now() + POLL_EVERY;
        let mut guard = m.lock().unwrap();
        loop {
            if guard.quit {
                return;
            }
            let now = unix_now();
            let mut wait = next_poll.saturating_duration_since(Instant::now());
            let mut fired = None;
            if let Some(t) = guard.restart_at {
                if now >= t {
                    guard.restart_at = None;
                    fired = Some(Event::RestartDue);
                } else if !guard.warned && now >= t - WARN_BEFORE_SECS {
                    guard.warned = true;
                    fired = Some(Event::RestartWarning);
                } else {
                    let until = if guard.warned {
                        t - now
                    } else {
                        t - WARN_BEFORE_SECS - now
                    };
                    wait = wait
                        .min(Duration::from_secs(until.max(1) as u64))
                        .min(WALL_RECHECK);
                }
            }
            if fired.is_none() && Instant::now() >= next_poll {
                next_poll += POLL_EVERY;
                fired = Some(Event::Poll);
            }
            if let Some(ev) = fired {
                drop(guard);
                fire(ev);
                guard = m.lock().unwrap();
                continue;
            }
            guard = cv.wait_timeout(guard, wait).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn due_restart_fires_and_clears() {
        let s = Schedule::default();
        let (tx, rx) = mpsc::channel();
        let s2 = s.clone();
        let h = std::thread::spawn(move || s2.run(move |e| tx.send(e).unwrap()));
        s.set_restart(Some(unix_now() + 1));
        // Within the warning window already: no warning, just the restart.
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::RestartDue
        );
        s.stop();
        h.join().unwrap();
    }

    #[test]
    fn warning_comes_first() {
        let s = Schedule::default();
        let (tx, rx) = mpsc::channel();
        let s2 = s.clone();
        let h = std::thread::spawn(move || s2.run(move |e| tx.send(e).unwrap()));
        // 5 minutes and 2 seconds out: the warning is 2 s away.
        s.set_restart(Some(unix_now() + WARN_BEFORE_SECS + 2));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(6)).unwrap(),
            Event::RestartWarning
        );
        s.set_restart(None);
        s.stop();
        h.join().unwrap();
    }
}
