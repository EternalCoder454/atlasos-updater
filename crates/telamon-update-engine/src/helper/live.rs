//! The helper's live progress: what the `Progress` property says, and how
//! often a change of it is announced.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::Instant;

use tokio::sync::watch;

use super::lock;
use crate::progress::Progress;

/// Receives progress while an operation runs (from any thread).
pub type ProgressSink = Arc<dyn Fn(Progress) + Send + Sync>;

/// At most one `PropertiesChanged` per this long (a stage change and the end
/// are announced at once).
pub const MIN_INTERVAL: Duration = Duration::from_millis(250);

/// The current progress as JSON; `""` when no operation reports any.
pub struct ProgressCell {
    tx: watch::Sender<String>,
    /// Updates after the operation ended (a late line from a pipe) are dropped.
    open: Mutex<bool>,
}

impl ProgressCell {
    pub fn new() -> Arc<Self> {
        Arc::new(ProgressCell {
            tx: watch::channel(String::new()).0,
            open: Mutex::new(false),
        })
    }

    /// The JSON now.
    pub fn current(&self) -> String {
        self.tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<String> {
        self.tx.subscribe()
    }

    /// Start reporting for `op`; dropping the guard clears the property.
    pub fn begin(self: &Arc<Self>, op: &'static str) -> (ProgressSink, ProgressGuard) {
        *lock(&self.open) = true;
        let cell = self.clone();
        let sink: ProgressSink = Arc::new(move |mut p: Progress| {
            p.op = op.into();
            if let Ok(json) = serde_json::to_string(&p) {
                let open = lock(&cell.open);
                if *open {
                    cell.tx.send_replace(json);
                }
            }
        });
        (sink, ProgressGuard(self.clone()))
    }
}

pub struct ProgressGuard(Arc<ProgressCell>);

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        let mut open = lock(&self.0.open);
        *open = false;
        self.0.tx.send_replace(String::new());
    }
}

fn stage(json: &str) -> Option<String> {
    serde_json::from_str::<Progress>(json).ok().map(|p| p.stage)
}

/// How long to hold back announcing `next` after `prev` was announced at
/// `last`: nothing for a new stage or the end, else until `MIN_INTERVAL`
/// has passed.
pub fn announce_delay(last: Option<Instant>, prev: &str, next: &str, now: Instant) -> Duration {
    if next.is_empty() || stage(prev) != stage(next) {
        return Duration::ZERO;
    }
    match last {
        Some(t) => MIN_INTERVAL.saturating_sub(now.saturating_duration_since(t)),
        None => Duration::ZERO,
    }
}

/// Call `announce` for each change of `rx`, throttled by [`announce_delay`],
/// until the sender is gone. `announce` gets the value now current.
pub async fn announce_changes<F, Fut>(mut rx: watch::Receiver<String>, mut announce: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    // nothing announced yet: the property starts as ""
    let mut sent = String::new();
    rx.mark_changed();
    let mut last: Option<Instant> = None;
    while rx.changed().await.is_ok() {
        loop {
            let value = rx.borrow_and_update().clone();
            if value == sent {
                break;
            }
            let wait = announce_delay(last, &sent, &value, Instant::now());
            if wait.is_zero() {
                announce().await;
                sent = value;
                last = Some(Instant::now());
                break;
            }
            // a newer value (maybe a stage change) cuts the wait short
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                r = rx.changed() => if r.is_err() { return },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(stage: &str, done: u64) -> String {
        serde_json::to_string(&Progress {
            op: "upgrade".into(),
            stage: stage.into(),
            done,
            total: 10,
            detail: String::new(),
        })
        .unwrap()
    }

    #[test]
    fn delay_rules() {
        let t0 = Instant::now();
        let a = json("downloading", 1);
        let b = json("downloading", 2);
        let c = json("installing", 0);
        let soon = t0 + Duration::from_millis(100);
        assert_eq!(
            announce_delay(Some(t0), &a, &b, soon),
            Duration::from_millis(150)
        );
        assert_eq!(
            announce_delay(Some(t0), &a, &b, t0 + MIN_INTERVAL),
            Duration::ZERO
        );
        assert_eq!(
            announce_delay(Some(t0), &a, &c, soon),
            Duration::ZERO,
            "stage change"
        );
        assert_eq!(
            announce_delay(Some(t0), &a, "", soon),
            Duration::ZERO,
            "the end"
        );
        assert_eq!(
            announce_delay(Some(t0), "", &a, soon),
            Duration::ZERO,
            "the start"
        );
        assert_eq!(announce_delay(None, &a, &b, soon), Duration::ZERO);
    }

    #[test]
    fn cell_reports_while_open_and_clears_on_drop() {
        let cell = ProgressCell::new();
        assert_eq!(cell.current(), "");
        let (sink, guard) = cell.begin("switch");
        sink(Progress {
            stage: "installing".into(),
            done: 1,
            total: 2,
            ..Progress::default()
        });
        let p: Progress = serde_json::from_str(&cell.current()).unwrap();
        assert_eq!((p.op.as_str(), p.done), ("switch", 1));
        drop(guard);
        assert_eq!(cell.current(), "");
        sink(Progress::default());
        assert_eq!(cell.current(), "", "late updates are dropped");
    }

    #[tokio::test(start_paused = true)]
    async fn announcements_are_throttled_but_stage_changes_and_the_end_are_not() {
        let cell = ProgressCell::new();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c2 = count.clone();
        let task = tokio::spawn(announce_changes(cell.subscribe(), move || {
            let c = c2.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }));
        let n = || count.load(std::sync::atomic::Ordering::SeqCst);
        let (sink, guard) = cell.begin("upgrade");
        let p = |stage: &str, done| Progress {
            stage: stage.into(),
            done,
            total: 100,
            ..Progress::default()
        };
        sink(p("downloading", 0));
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(n(), 1);
        for i in 1..=50 {
            sink(p("downloading", i));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // 500 ms of updates: about two announcements more, not fifty
        assert!((2..=4).contains(&n()), "{}", n());
        let before = n();
        sink(p("installing", 0));
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(n(), before + 1, "stage change at once");
        drop(guard);
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(n(), before + 2, "the end at once");
        task.abort();
    }
}
