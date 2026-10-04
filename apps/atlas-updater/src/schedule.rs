//! One background thread that sleeps until the next thing is due: the 6 h
//! status poll, the app update round (10 minutes after start, then every
//! 6 h), the restart warning, the scheduled restart. It wakes at no
//! other time, so the idle tray process costs nothing.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const POLL_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The first app update round waits this long, so it stays out of the way
/// while the desktop starts.
pub const APPS_FIRST: Duration = Duration::from_secs(10 * 60);
pub const WARN_BEFORE_SECS: i64 = 5 * 60;
/// A restart that is more overdue than this (the computer slept, the clock
/// jumped) is missed, not run: a user who was away gets no surprise reboot.
pub const GRACE_SECS: i64 = 2 * 60;
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
    /// An app round asked for sooner than the regular one.
    apps_at: Option<Instant>,
}

#[derive(Clone)]
pub struct Schedule {
    inner: Arc<(Mutex<State>, Condvar)>,
    apps_first: Duration,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule {
            inner: Arc::default(),
            apps_first: APPS_FIRST,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Poll,
    /// Time to look for app updates (and install them, if the user said so).
    Apps,
    /// The restart scheduled for this Unix time is 5 minutes away.
    RestartWarning(i64),
    /// The restart scheduled for this Unix time is due.
    RestartDue(i64),
    /// The time passed while we could not run (sleep, clock jump).
    RestartMissed(i64),
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Missed,
    Due,
    Warn,
    /// Seconds until the next thing for this restart.
    Wait(i64),
}

/// What to do about a restart at `t` when it is `now`. The warning always
/// comes before the restart: if we reach `t` without having warned (we slept
/// through the warning window), the restart is missed.
fn step(now: i64, t: i64, warned: bool) -> Step {
    if now >= t {
        if warned && now - t <= GRACE_SECS {
            Step::Due
        } else {
            Step::Missed
        }
    } else if !warned && now >= t - WARN_BEFORE_SECS {
        Step::Warn
    } else if warned {
        Step::Wait(t - now)
    } else {
        Step::Wait(t - WARN_BEFORE_SECS - now)
    }
}

impl Schedule {
    /// The user picked `at` (or cleared it). No warning when the time is
    /// already close: the user just chose it.
    pub fn set_restart(&self, at: Option<i64>) {
        self.set(at, at.is_some_and(|t| t - unix_now() <= WARN_BEFORE_SECS));
    }

    /// A time saved by an earlier run: nobody just chose it, so a time that is
    /// already inside the warning window is warned about at once.
    pub fn restore_restart(&self, at: i64) {
        self.set(Some(at), false);
    }

    fn set(&self, at: Option<i64>, warned: bool) {
        let (m, cv) = &*self.inner;
        let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
        s.restart_at = at;
        s.warned = warned;
        cv.notify_all();
    }

    /// The next app round in `d` (the switch was just turned on, or the last
    /// round had to wait). Replaces the regular time.
    pub fn apps_in(&self, d: Duration) {
        let (m, cv) = &*self.inner;
        m.lock().unwrap_or_else(|e| e.into_inner()).apps_at = Some(Instant::now() + d);
        cv.notify_all();
    }

    pub fn stop(&self) {
        let (m, cv) = &*self.inner;
        m.lock().unwrap_or_else(|e| e.into_inner()).quit = true;
        cv.notify_all();
    }

    /// Runs `fire` for each due event until `stop()`. Blocks: use a thread.
    pub fn run(&self, mut fire: impl FnMut(Event)) {
        let (m, cv) = &*self.inner;
        let mut next_poll = Instant::now() + POLL_EVERY;
        let mut next_apps = Instant::now() + self.apps_first;
        let mut guard = m.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if guard.quit {
                return;
            }
            if let Some(t) = guard.apps_at.take() {
                next_apps = t;
            }
            let now = unix_now();
            let mut wait = next_poll
                .min(next_apps)
                .saturating_duration_since(Instant::now());
            let mut fired = None;
            if let Some(t) = guard.restart_at {
                match step(now, t, guard.warned) {
                    Step::Due => {
                        guard.restart_at = None;
                        fired = Some(Event::RestartDue(t));
                    }
                    Step::Missed => {
                        guard.restart_at = None;
                        fired = Some(Event::RestartMissed(t));
                    }
                    Step::Warn => {
                        guard.warned = true;
                        fired = Some(Event::RestartWarning(t));
                    }
                    Step::Wait(until) => {
                        wait = wait
                            .min(Duration::from_secs(until.max(1) as u64))
                            .min(WALL_RECHECK);
                    }
                }
            }
            if fired.is_none() && Instant::now() >= next_poll {
                next_poll += POLL_EVERY;
                fired = Some(Event::Poll);
            }
            if fired.is_none() && Instant::now() >= next_apps {
                // From now, not from the missed time: no burst after a suspend.
                next_apps = Instant::now() + POLL_EVERY;
                fired = Some(Event::Apps);
            }
            if let Some(ev) = fired {
                drop(guard);
                fire(ev);
                guard = m.lock().unwrap_or_else(|e| e.into_inner());
                continue;
            }
            guard = cv
                .wait_timeout(guard, wait)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn steps() {
        let t = 10_000;
        assert_eq!(step(t - 1000, t, false), Step::Wait(700));
        assert_eq!(step(t - 299, t, false), Step::Warn);
        assert_eq!(step(t - 299, t, true), Step::Wait(299));
        assert_eq!(step(t, t, true), Step::Due);
        assert_eq!(step(t + GRACE_SECS, t, true), Step::Due);
        // slept through: never a surprise reboot
        assert_eq!(step(t + GRACE_SECS + 1, t, true), Step::Missed);
        assert_eq!(step(t + 5, t, false), Step::Missed);
        assert_eq!(step(t + 3 * 3600, t, true), Step::Missed);
    }

    #[test]
    fn first_app_round_comes_after_the_delay() {
        let s = Schedule {
            apps_first: Duration::from_millis(200),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let s2 = s.clone();
        let started = Instant::now();
        let h = std::thread::spawn(move || s2.run(move |e| tx.send(e).unwrap()));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Apps
        );
        assert!(started.elapsed() >= Duration::from_millis(200));
        // The next one is hours away.
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
        s.stop();
        h.join().unwrap();
    }

    #[test]
    fn an_early_app_round_replaces_the_regular_one() {
        let s = Schedule::default();
        let (tx, rx) = mpsc::channel();
        let s2 = s.clone();
        let h = std::thread::spawn(move || s2.run(move |e| tx.send(e).unwrap()));
        s.apps_in(Duration::from_millis(100));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Apps
        );
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        s.stop();
        h.join().unwrap();
    }

    #[test]
    fn due_restart_fires_and_clears() {
        let s = Schedule::default();
        let (tx, rx) = mpsc::channel();
        let s2 = s.clone();
        let h = std::thread::spawn(move || s2.run(move |e| tx.send(e).unwrap()));
        let at = unix_now() + 1;
        s.set_restart(Some(at));
        // Within the warning window already: no warning, just the restart.
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::RestartDue(at)
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
        let at = unix_now() + WARN_BEFORE_SECS + 2;
        s.set_restart(Some(at));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(6)).unwrap(),
            Event::RestartWarning(at)
        );
        s.set_restart(None);
        s.stop();
        h.join().unwrap();
    }

    #[test]
    fn a_restored_time_inside_the_window_still_warns() {
        let s = Schedule::default();
        let (tx, rx) = mpsc::channel();
        let s2 = s.clone();
        let h = std::thread::spawn(move || s2.run(move |e| tx.send(e).unwrap()));
        let at = unix_now() + 120;
        s.restore_restart(at);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::RestartWarning(at)
        );
        s.set_restart(None);
        s.stop();
        h.join().unwrap();
    }
}
