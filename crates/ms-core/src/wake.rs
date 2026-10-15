//! Process-wide sleep/wake signals: the worker sleep condvar (`MailUtils::sleepWorkerUntil
//! WakeOrSec` / `wakeAllWorkers`) and the metadata expiration workers' wake handles
//! (`MetadataExpirationWorkerForAccountId`, `WakeAllMetadataExpirationWorkers`).

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A wake-up that reaches only threads already waiting, like a condvar `notify_all`.
#[derive(Default)]
pub struct WakeSignal {
    generation: Mutex<u64>,
    cv: Condvar,
}

impl WakeSignal {
    pub fn new() -> Self {
        WakeSignal::default()
    }

    /// Sleeps until `timeout` passes or [`WakeSignal::wake_all`] is called. Returns true if
    /// woken early.
    pub fn sleep(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut gen = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        let start = *gen;
        while *gen == start {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            gen = self
                .cv
                .wait_timeout(gen, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }

    pub fn wake_all(&self) {
        let mut gen = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        *gen = gen.wrapping_add(1);
        self.cv.notify_all();
    }
}

fn worker_signal() -> &'static WakeSignal {
    static SIGNAL: OnceLock<WakeSignal> = OnceLock::new();
    SIGNAL.get_or_init(WakeSignal::new)
}

/// Sleeps a sync worker for `secs`, or until `wake-workers`.
pub fn sleep_worker_until_wake_or(secs: u64) -> bool {
    worker_signal().sleep(Duration::from_secs(secs))
}

/// `wake-workers`: ends every worker sleep and wakes every metadata expiration worker so
/// it re-checks for metadata that expired while the machine slept.
pub fn wake_all_workers() {
    worker_signal().wake_all();
    for signal in expiration_signals() {
        signal.wake();
    }
}

/// The metadata expiration worker's timer: it sleeps until the earliest known expiration,
/// and saving metadata with an earlier one pulls the deadline in.
#[derive(Default)]
pub struct DeadlineSignal {
    state: Mutex<DeadlineState>,
    cv: Condvar,
}

#[derive(Default)]
struct DeadlineState {
    /// Epoch seconds of the next wake, if any.
    deadline: Option<i64>,
    woken: bool,
}

impl DeadlineSignal {
    /// `isSavingMetadataWithExpiration`: wake at `expiration` unless already due earlier.
    pub fn schedule(&self, expiration: i64) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.deadline.is_none_or(|d| expiration < d) {
            s.deadline = Some(expiration);
            self.cv.notify_all();
        }
    }

    pub fn wake(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.woken = true;
        self.cv.notify_all();
    }

    pub fn deadline(&self) -> Option<i64> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .deadline
    }

    /// Replaces the deadline (the worker computes it from the database after each run).
    pub fn set_deadline(&self, deadline: Option<i64>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .deadline = deadline;
    }

    /// Waits until the deadline (measured with `now`, epoch seconds), a wake, or `max`.
    /// Clears the wake flag.
    pub fn wait(&self, now: &dyn Fn() -> i64, max: Duration) {
        let limit = Instant::now() + max;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if s.woken {
                s.woken = false;
                return;
            }
            let remaining_max = limit.saturating_duration_since(Instant::now());
            let until_deadline = match s.deadline {
                Some(d) => {
                    let secs = d - now();
                    if secs <= 0 {
                        return;
                    }
                    Duration::from_secs(secs as u64)
                }
                None => remaining_max,
            };
            let wait = until_deadline.min(remaining_max);
            if wait.is_zero() {
                return;
            }
            s = self
                .cv
                .wait_timeout(s, wait)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<DeadlineSignal>>> {
    static REG: OnceLock<Mutex<HashMap<String, Arc<DeadlineSignal>>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

/// The expiration signal of an account, created on first use. The store calls
/// [`DeadlineSignal::schedule`] when it saves metadata with an expiration; the worker waits
/// on it.
pub fn expiration_signal(account_id: &str) -> Arc<DeadlineSignal> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(account_id.to_owned())
        .or_default()
        .clone()
}

fn expiration_signals() -> Vec<Arc<DeadlineSignal>> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn wake_ends_sleep() {
        let s = Arc::new(WakeSignal::new());
        let s2 = s.clone();
        let t = thread::spawn(move || s2.sleep(Duration::from_secs(10)));
        thread::sleep(Duration::from_millis(50));
        s.wake_all();
        assert!(t.join().unwrap());
        assert!(!s.sleep(Duration::from_millis(10)));
    }

    #[test]
    fn deadline_schedule_keeps_earliest() {
        let d = DeadlineSignal::default();
        d.schedule(100);
        d.schedule(200);
        assert_eq!(d.deadline(), Some(100));
        d.schedule(50);
        assert_eq!(d.deadline(), Some(50));
        // already due
        d.wait(&|| 60, Duration::from_secs(5));
        d.wake();
        d.set_deadline(None);
        d.wait(&|| 0, Duration::from_secs(5));
    }
}
