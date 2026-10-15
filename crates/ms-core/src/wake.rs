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

/// The metadata expiration worker's wake handle (`MetadataExpirationWorker::_wakeTime`,
/// `_wakeCv`). The worker publishes the time it sleeps until; saving metadata that expires
/// earlier ends that sleep so the worker re-scans and picks the new, earlier time.
///
/// Like the C++ condition variable, a wake reaches only a worker that is waiting: one that
/// is scanning reads the new expiration from the database itself. Before the first scan the
/// wake time is the epoch, so nothing wakes the worker during its start delay.
#[derive(Default)]
pub struct DeadlineSignal {
    state: Mutex<DeadlineState>,
    cv: Condvar,
}

#[derive(Default)]
struct DeadlineState {
    /// Epoch seconds the worker sleeps until.
    wake_time: i64,
    waiting: bool,
    /// Bumped by each wake that reaches a waiting worker.
    generation: u64,
}

impl DeadlineSignal {
    fn lock(&self) -> std::sync::MutexGuard<'_, DeadlineState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `isSavingMetadataWithExpiration`: wakes the worker if `expiration` (epoch seconds) is
    /// before the time it sleeps until.
    pub fn schedule(&self, expiration: i64) {
        let mut s = self.lock();
        if expiration < s.wake_time {
            if s.waiting {
                s.generation = s.generation.wrapping_add(1);
                self.cv.notify_all();
            }
        } else {
            tracing::info!(
                "Metadata expiration {expiration} is further in future than wake time {}",
                s.wake_time
            );
        }
    }

    /// `WakeAllMetadataExpirationWorkers`: re-check for metadata that expired while the
    /// machine slept.
    pub fn wake(&self) {
        self.schedule(0);
    }

    /// The time the worker last slept or sleeps until, in epoch seconds.
    pub fn wake_time(&self) -> i64 {
        self.lock().wake_time
    }

    /// Whether the worker is waiting in [`DeadlineSignal::wait_until`].
    pub fn is_waiting(&self) -> bool {
        self.lock().waiting
    }

    /// Publishes `wake_time` and sleeps until `now()` (epoch seconds) reaches it or a
    /// [`DeadlineSignal::schedule`] with an earlier time. Returns true if woken early.
    pub fn wait_until(&self, wake_time: i64, now: &dyn Fn() -> i64) -> bool {
        // Re-read the clock at least this often, so a wall-clock jump (the machine slept)
        // does not stretch the sleep.
        const MAX_SLICE: Duration = Duration::from_secs(30);
        let mut s = self.lock();
        s.wake_time = wake_time;
        s.waiting = true;
        let start = s.generation;
        let woken = loop {
            if s.generation != start {
                break true;
            }
            let secs = wake_time - now();
            if secs <= 0 {
                break false;
            }
            let wait = Duration::from_secs(secs as u64).min(MAX_SLICE);
            s = self
                .cv
                .wait_timeout(s, wait)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        };
        s.waiting = false;
        woken
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<DeadlineSignal>>> {
    static REG: OnceLock<Mutex<HashMap<String, Arc<DeadlineSignal>>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

/// The expiration signal of an account (`MetadataExpirationWorkerForAccountId`), created
/// on first use. The store calls [`DeadlineSignal::schedule`] when it saves metadata with an
/// expiration; the worker waits on it.
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
    fn schedule_wakes_only_a_waiting_worker_for_an_earlier_time() {
        let now = || 1_000;
        let d = Arc::new(DeadlineSignal::default());
        // Nothing is waiting: the wake is lost, as with a condvar.
        d.schedule(-1);
        assert!(!d.wait_until(999, &now), "already due");
        assert_eq!(d.wake_time(), 999);

        let waiter = {
            let d = d.clone();
            thread::spawn(move || d.wait_until(1_000_000, &|| 1_000))
        };
        while !d.is_waiting() {
            thread::sleep(Duration::from_millis(5));
        }
        d.schedule(2_000_000);
        thread::sleep(Duration::from_millis(50));
        assert!(!waiter.is_finished(), "a later expiration does not wake");
        d.schedule(5_000);
        assert!(waiter.join().unwrap());
        assert!(!d.is_waiting());
    }

    #[test]
    fn wake_all_workers_wakes_registered_expiration_workers() {
        let d = expiration_signal("wake-all-test");
        let waiter = {
            let d = d.clone();
            thread::spawn(move || d.wait_until(1_000_000, &|| 1_000))
        };
        while !d.is_waiting() {
            thread::sleep(Duration::from_millis(5));
        }
        wake_all_workers();
        assert!(waiter.join().unwrap());
    }
}
