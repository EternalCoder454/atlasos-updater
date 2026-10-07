//! When the screen-edge glow is wanted, and what to do about its process.
//! Plain state, no I/O: time comes in as arguments, what to do comes out as
//! an [`Action`] for the caller (`glow.rs`) to carry out.
//!
//! The system is "being changed" (so the glow shows) only while the OS image
//! is: an update being downloaded and staged, a channel switch, a go back.
//! Not app (Flatpak) updates, not firmware, not checks. It is so while
//!
//! - any caller holds a claim: Settings says `SetWorking(true)` on the
//!   tray's session interface when it stages an update, switches or goes
//!   back (and only then: that is Settings' rule), and `SetWorking(false)`
//!   when that is over. A claim is the caller's (its D-Bus unique name); it ends
//!   when the caller says so, when the caller leaves the bus (a crashed
//!   Settings leaves no glow) and [`CLAIM_LIFETIME`] after the caller last
//!   said `true` (a safety net, a claim that is renewed lives on); or
//! - the system helper reports an upgrade or a switch running (its
//!   `Progress` property names one of those two operations), whether
//!   anybody's window is open.
//!   The helper has two identities for this release (new and legacy
//!   names); either one counts, and saying so twice changes nothing.
//!
//! The glow is one process at a time. Started when wanted, asked to end
//! (SIGTERM) when no longer wanted and killed [`TERM_GRACE`] later if it is
//! still there. If it ends while still wanted it is started again after a
//! pause that doubles (1 s, 2 s, 4 s); a fourth end within a minute makes
//! the machine give up until the system has been idle once.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// A claim older than this is dropped (unless renewed).
pub const CLAIM_LIFETIME: Duration = Duration::from_secs(3 * 3600);
/// From SIGTERM to SIGKILL.
pub const TERM_GRACE: Duration = Duration::from_secs(3);
/// Restarts of a glow that ended on its own are counted over this long …
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// … and at most this many are made.
pub const MAX_RESTARTS: usize = 3;
/// The first pause before a restart; it doubles each time.
pub const FIRST_BACKOFF: Duration = Duration::from_secs(1);
/// Most callers that may hold a claim at once (an application has one
/// connection; this only stops a runaway).
pub const MAX_CLAIMS: usize = 16;

/// What the caller must do to the glow process now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Start it. Report the outcome: [`Machine::start_failed`] if it cannot
    /// be started; [`Machine::exited`] when it ends.
    Start,
    /// Send SIGTERM.
    Terminate,
    /// Send SIGKILL (it did not end after SIGTERM).
    Kill,
}

/// The helper's two names: what it announces `Progress` under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Helper {
    Telamon,
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proc {
    Off,
    Running,
    /// SIGTERM was sent at this time.
    Stopping(Instant),
    /// SIGKILL was sent.
    Killing,
    /// It ended on its own: start again at this time.
    Waiting(Instant),
}

/// A claim was refused: [`MAX_CLAIMS`] callers hold one already.
#[derive(Debug, PartialEq, Eq)]
pub struct TooMany;

#[derive(Debug)]
pub struct Machine {
    claims: HashMap<String, Instant>,
    helper: [bool; 2],
    proc: Proc,
    restarts: VecDeque<Instant>,
    gave_up: bool,
}

impl Default for Machine {
    fn default() -> Self {
        Machine {
            claims: HashMap::new(),
            helper: [false; 2],
            proc: Proc::Off,
            restarts: VecDeque::new(),
            gave_up: false,
        }
    }
}

impl Machine {
    /// Whether the system is being changed.
    pub fn wanted(&self) -> bool {
        !self.claims.is_empty() || self.helper.iter().any(|h| *h)
    }

    #[cfg(test)]
    /// A glow process is running, or ending (not yet reported ended).
    pub fn has_process(&self) -> bool {
        matches!(self.proc, Proc::Running | Proc::Stopping(_) | Proc::Killing)
    }

    pub fn has_claim(&self, sender: &str) -> bool {
        self.claims.contains_key(sender)
    }

    #[cfg(test)]
    pub fn gave_up(&self) -> bool {
        self.gave_up
    }

    /// `sender` says `on` (`SetWorking`). Saying `true` again renews it.
    pub fn claim(
        &mut self,
        sender: &str,
        on: bool,
        now: Instant,
    ) -> Result<Option<Action>, TooMany> {
        self.expire(now);
        if on {
            if !self.claims.contains_key(sender) && self.claims.len() >= MAX_CLAIMS {
                return Err(TooMany);
            }
            self.claims.insert(sender.to_string(), now);
        } else {
            self.claims.remove(sender);
        }
        Ok(self.reconcile(now))
    }

    /// `sender` left the bus: its claim goes with it.
    pub fn sender_gone(&mut self, sender: &str, now: Instant) -> Option<Action> {
        self.expire(now);
        self.claims.remove(sender);
        self.reconcile(now)
    }

    /// The helper's `Progress` under one of its names is (not) empty; or it
    /// left the bus (`false`).
    pub fn helper_progress(&mut self, which: Helper, active: bool, now: Instant) -> Option<Action> {
        self.expire(now);
        self.helper[which as usize] = active;
        self.reconcile(now)
    }

    /// The glow could not be started (after [`Action::Start`]): not tried
    /// again until the system has been idle once.
    pub fn start_failed(&mut self) {
        if self.proc == Proc::Running {
            self.proc = Proc::Off;
            self.gave_up = true;
        }
    }

    /// The glow process ended (any way).
    pub fn exited(&mut self, now: Instant) -> Option<Action> {
        self.expire(now);
        match self.proc {
            Proc::Running => {
                // Not asked to end: start it again, but not forever.
                self.proc = Proc::Off;
                if self.wanted() {
                    while self
                        .restarts
                        .front()
                        .is_some_and(|t| now.saturating_duration_since(*t) >= RESTART_WINDOW)
                    {
                        self.restarts.pop_front();
                    }
                    if self.restarts.len() >= MAX_RESTARTS {
                        self.gave_up = true;
                    } else {
                        let pause = FIRST_BACKOFF * (1 << self.restarts.len());
                        self.restarts.push_back(now);
                        self.proc = Proc::Waiting(now + pause);
                    }
                }
            }
            Proc::Stopping(_) | Proc::Killing => self.proc = Proc::Off,
            // Not a process of ours.
            Proc::Off | Proc::Waiting(_) => {}
        }
        self.reconcile(now)
    }

    /// Call at [`next_deadline`](Self::next_deadline), or any time.
    pub fn tick(&mut self, now: Instant) -> Option<Action> {
        self.expire(now);
        if let Some(a) = self.reconcile(now) {
            return Some(a);
        }
        match self.proc {
            Proc::Stopping(since) if now >= since + TERM_GRACE => {
                self.proc = Proc::Killing;
                Some(Action::Kill)
            }
            Proc::Waiting(at) if now >= at && self.wanted() => {
                self.proc = Proc::Running;
                Some(Action::Start)
            }
            _ => None,
        }
    }

    /// When [`tick`](Self::tick) has something to do next.
    pub fn next_deadline(&self) -> Option<Instant> {
        let claims = self.claims.values().map(|t| *t + CLAIM_LIFETIME);
        let proc = match self.proc {
            Proc::Stopping(since) => Some(since + TERM_GRACE),
            Proc::Waiting(at) => Some(at),
            _ => None,
        };
        claims.chain(proc).min()
    }

    fn expire(&mut self, now: Instant) {
        self.claims
            .retain(|_, at| now.saturating_duration_since(*at) < CLAIM_LIFETIME);
    }

    /// What follows from the current inputs.
    fn reconcile(&mut self, now: Instant) -> Option<Action> {
        let wanted = self.wanted();
        let action = match (self.proc, wanted) {
            (Proc::Off, true) if !self.gave_up => {
                self.proc = Proc::Running;
                Some(Action::Start)
            }
            (Proc::Running, false) => {
                self.proc = Proc::Stopping(now);
                Some(Action::Terminate)
            }
            // A start that was waiting is not needed any more.
            (Proc::Waiting(_), false) => {
                self.proc = Proc::Off;
                None
            }
            _ => None,
        };
        if !wanted && self.proc == Proc::Off {
            // Idle: the next time starts afresh.
            self.gave_up = false;
            self.restarts.clear();
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn t0() -> Instant {
        Instant::now()
    }

    fn claim(m: &mut Machine, who: &str, on: bool, now: Instant) -> Option<Action> {
        m.claim(who, on, now).unwrap()
    }

    #[test]
    fn a_claim_starts_the_glow_and_releasing_it_stops_it() {
        let t = t0();
        let mut m = Machine::default();
        assert!(!m.wanted());
        assert_eq!(claim(&mut m, ":1.5", true, t), Some(Action::Start));
        assert!(m.wanted() && m.has_process());
        // saying so again changes nothing
        assert_eq!(claim(&mut m, ":1.5", true, t + S), None);
        assert_eq!(
            claim(&mut m, ":1.5", false, t + 2 * S),
            Some(Action::Terminate)
        );
        assert!(!m.wanted());
        // releasing what was not claimed changes nothing
        assert_eq!(claim(&mut m, ":1.9", false, t + 3 * S), None);
        assert_eq!(m.exited(t + 3 * S), None);
        assert!(!m.has_process());
    }

    #[test]
    fn it_is_wanted_while_anyone_claims() {
        let t = t0();
        let mut m = Machine::default();
        assert_eq!(claim(&mut m, ":1.1", true, t), Some(Action::Start));
        // a second caller: still one process
        assert_eq!(claim(&mut m, ":1.2", true, t), None);
        assert_eq!(claim(&mut m, ":1.1", false, t), None);
        assert!(m.wanted());
        assert_eq!(claim(&mut m, ":1.2", false, t), Some(Action::Terminate));
    }

    #[test]
    fn a_caller_that_leaves_the_bus_loses_its_claim() {
        let t = t0();
        let mut m = Machine::default();
        claim(&mut m, ":1.7", true, t);
        assert_eq!(m.sender_gone(":1.8", t), None);
        assert!(m.has_claim(":1.7"));
        assert_eq!(m.sender_gone(":1.7", t + S), Some(Action::Terminate));
        assert!(!m.has_claim(":1.7"));
    }

    #[test]
    fn a_claim_older_than_three_hours_is_dropped_unless_renewed() {
        let t = t0();
        let mut m = Machine::default();
        claim(&mut m, ":1.1", true, t);
        claim(&mut m, ":1.2", true, t);
        assert_eq!(m.next_deadline(), Some(t + CLAIM_LIFETIME));
        // :1.2 says it again later: that one lives on
        let later = t + CLAIM_LIFETIME - S;
        assert_eq!(claim(&mut m, ":1.2", true, later), None);
        assert_eq!(m.tick(t + CLAIM_LIFETIME - S), None);
        assert_eq!(m.tick(t + CLAIM_LIFETIME), None);
        assert!(!m.has_claim(":1.1") && m.has_claim(":1.2"));
        assert!(m.wanted());
        assert_eq!(
            m.next_deadline(),
            Some(later + CLAIM_LIFETIME),
            "the renewed claim is the next to run out"
        );
        assert_eq!(m.tick(later + CLAIM_LIFETIME), Some(Action::Terminate));
        assert!(!m.wanted());
    }

    #[test]
    fn the_helpers_progress_wants_the_glow_with_nobody_claiming() {
        let t = t0();
        let mut m = Machine::default();
        assert_eq!(
            m.helper_progress(Helper::Telamon, true, t),
            Some(Action::Start)
        );
        // the legacy name says the same: nothing new
        assert_eq!(m.helper_progress(Helper::Legacy, true, t), None);
        assert_eq!(m.helper_progress(Helper::Telamon, true, t), None);
        // one name going quiet is not the end
        assert_eq!(m.helper_progress(Helper::Telamon, false, t), None);
        assert!(m.wanted());
        assert_eq!(
            m.helper_progress(Helper::Legacy, false, t),
            Some(Action::Terminate)
        );
    }

    #[test]
    fn a_claim_and_the_helper_hold_it_together() {
        let t = t0();
        let mut m = Machine::default();
        claim(&mut m, ":1.1", true, t);
        assert_eq!(m.helper_progress(Helper::Telamon, true, t), None);
        assert_eq!(claim(&mut m, ":1.1", false, t), None);
        assert_eq!(
            m.helper_progress(Helper::Telamon, false, t),
            Some(Action::Terminate)
        );
    }

    #[test]
    fn it_is_killed_when_it_ignores_the_term() {
        let t = t0();
        let mut m = Machine::default();
        claim(&mut m, ":1.1", true, t);
        assert_eq!(claim(&mut m, ":1.1", false, t), Some(Action::Terminate));
        assert_eq!(m.next_deadline(), Some(t + TERM_GRACE));
        assert_eq!(m.tick(t + TERM_GRACE - Duration::from_millis(1)), None);
        assert_eq!(m.tick(t + TERM_GRACE), Some(Action::Kill));
        // only once
        assert_eq!(m.tick(t + 2 * TERM_GRACE), None);
        assert!(m.has_process());
        assert_eq!(m.exited(t + 2 * TERM_GRACE), None);
        assert!(!m.has_process());
        assert_eq!(m.next_deadline(), None);
    }

    #[test]
    fn wanted_again_while_it_is_ending_starts_one_after_it_ended() {
        let t = t0();
        let mut m = Machine::default();
        claim(&mut m, ":1.1", true, t);
        claim(&mut m, ":1.1", false, t);
        // never two at once: nothing is started while the old one is there
        assert_eq!(claim(&mut m, ":1.2", true, t + S), None);
        assert!(m.has_process());
        assert_eq!(m.exited(t + 2 * S), Some(Action::Start));
        assert!(m.has_process());
    }

    #[test]
    fn a_glow_that_dies_is_started_again_with_longer_pauses_then_given_up_on() {
        let t = t0();
        let mut m = Machine::default();
        claim(&mut m, ":1.1", true, t);
        // 1st end: restart after 1 s
        let mut now = t + S;
        assert_eq!(m.exited(now), None);
        assert!(!m.has_process());
        assert_eq!(m.next_deadline(), Some(now + S));
        assert_eq!(m.tick(now + S - Duration::from_millis(1)), None);
        now += S;
        assert_eq!(m.tick(now), Some(Action::Start));
        // 2nd: after 2 s
        assert_eq!(m.exited(now), None);
        assert_eq!(m.next_deadline(), Some(now + 2 * S));
        now += 2 * S;
        assert_eq!(m.tick(now), Some(Action::Start));
        // 3rd: after 4 s
        assert_eq!(m.exited(now), None);
        assert_eq!(m.next_deadline(), Some(now + 4 * S));
        now += 4 * S;
        assert_eq!(m.tick(now), Some(Action::Start));
        // 4th within the minute: that is enough
        assert_eq!(m.exited(now), None);
        assert!(m.gave_up() && !m.has_process());
        assert_eq!(m.next_deadline(), Some(t + CLAIM_LIFETIME));
        // more of the same input does not start it
        assert_eq!(claim(&mut m, ":1.2", true, now), None);
        assert_eq!(claim(&mut m, ":1.1", false, now), None);
        assert!(!m.has_process());
        // idle once, wanted again: a fresh start with a fresh count
        assert_eq!(claim(&mut m, ":1.2", false, now), None);
        assert!(!m.gave_up());
        assert_eq!(claim(&mut m, ":1.3", true, now + S), Some(Action::Start));
        assert_eq!(m.exited(now + 2 * S), None);
        assert_eq!(m.next_deadline(), Some(now + 3 * S));
    }

    #[test]
    fn restarts_far_apart_are_not_counted_together() {
        let t = t0();
        let mut m = Machine::default();
        claim(&mut m, ":1.1", true, t);
        let mut now = t;
        for _ in 0..6 {
            // it dies, is started again after 1 s, and runs for a minute
            assert_eq!(m.exited(now), None);
            now += S;
            assert_eq!(m.tick(now), Some(Action::Start), "{now:?}");
            now += RESTART_WINDOW;
        }
        assert!(!m.gave_up());
    }

    #[test]
    fn no_longer_wanted_cancels_a_pending_restart() {
        let t = t0();
        let mut m = Machine::default();
        claim(&mut m, ":1.1", true, t);
        assert_eq!(m.exited(t + S), None);
        assert!(m.next_deadline().is_some());
        // released while waiting: no process to end, none to start
        assert_eq!(claim(&mut m, ":1.1", false, t + 2 * S), None);
        assert_eq!(m.tick(t + 10 * S), None);
        assert!(!m.has_process());
        assert_eq!(m.next_deadline(), None);
    }

    #[test]
    fn a_glow_that_cannot_start_is_not_tried_again_until_idle() {
        let t = t0();
        let mut m = Machine::default();
        assert_eq!(claim(&mut m, ":1.1", true, t), Some(Action::Start));
        m.start_failed();
        assert!(!m.has_process() && m.gave_up());
        assert_eq!(claim(&mut m, ":1.2", true, t), None);
        assert_eq!(m.tick(t + 100 * S), None);
        claim(&mut m, ":1.1", false, t);
        claim(&mut m, ":1.2", false, t);
        assert_eq!(claim(&mut m, ":1.1", true, t), Some(Action::Start));
    }

    #[test]
    fn a_runaway_of_callers_is_stopped() {
        let t = t0();
        let mut m = Machine::default();
        for i in 0..MAX_CLAIMS {
            m.claim(&format!(":1.{i}"), true, t).unwrap();
        }
        assert_eq!(m.claim(":1.99", true, t), Err(TooMany));
        // one that holds a claim may renew it
        assert!(m.claim(":1.0", true, t).is_ok());
        // and room is made again by releasing
        m.claim(":1.1", false, t).unwrap();
        assert!(m.claim(":1.99", true, t).is_ok());
    }
}
