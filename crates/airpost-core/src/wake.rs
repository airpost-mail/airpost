// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Sleep/wake signals: the worker sleep condvar (`MailUtils::sleepWorkerUntilWakeOrSec` /
//! `wakeAllWorkers`), which each account holds in its `WorkerContext`, and the metadata
//! expiration workers' wake handles (`MetadataExpirationWorkerForAccountId`,
//! `WakeAllMetadataExpirationWorkers`), kept here by account id because the store's save
//! path reaches them without a context.
//!
//! Derived from Mailspring-Sync `MailSync/MailUtils.cpp` and
//! `MailSync/MetadataExpirationWorker.cpp`, Copyright © 2017 Foundry 376, GPL-3.0. See
//! NOTICE.md.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A wake-up that reaches only threads already waiting, like a condvar `notify_all`. Once
/// closed, every sleep returns at once.
#[derive(Default)]
pub struct WakeSignal {
    state: Mutex<WakeState>,
    cv: Condvar,
}

#[derive(Default)]
struct WakeState {
    generation: u64,
    closed: bool,
}

impl WakeSignal {
    pub fn new() -> Self {
        WakeSignal::default()
    }

    /// Sleeps until `timeout` passes, [`WakeSignal::wake_all`] is called or the signal is
    /// closed. Returns true if woken early or closed.
    pub fn sleep(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let start = s.generation;
        while s.generation == start && !s.closed {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            s = self
                .cv
                .wait_timeout(s, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }

    pub fn wake_all(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.generation = s.generation.wrapping_add(1);
        self.cv.notify_all();
    }

    /// Ends every sleep, now and later.
    pub fn close(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.closed = true;
        self.cv.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed
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
    closed: bool,
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

    /// Ends the worker's wait, now and later: its account is stopping.
    pub fn close(&self) {
        let mut s = self.lock();
        s.closed = true;
        self.cv.notify_all();
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
    /// [`DeadlineSignal::schedule`] with an earlier time. Returns true if woken early or
    /// closed.
    pub fn wait_until(&self, wake_time: i64, now: &dyn Fn() -> i64) -> bool {
        // Re-read the clock at least this often, so a wall-clock jump (the machine slept)
        // does not stretch the sleep.
        const MAX_SLICE: Duration = Duration::from_secs(30);
        let mut s = self.lock();
        s.wake_time = wake_time;
        s.waiting = true;
        let start = s.generation;
        let woken = loop {
            if s.generation != start || s.closed {
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

/// Drops the account's expiration signal if it is still `signal`, so an account added again
/// gets a fresh one and a stop of an earlier run never drops a later run's.
pub fn remove_expiration_signal(account_id: &str, signal: &Arc<DeadlineSignal>) {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    if reg.get(account_id).is_some_and(|s| Arc::ptr_eq(s, signal)) {
        reg.remove(account_id);
    }
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
    fn close_ends_sleeps_now_and_later() {
        let s = Arc::new(WakeSignal::new());
        let s2 = s.clone();
        let t = thread::spawn(move || s2.sleep(Duration::from_secs(10)));
        thread::sleep(Duration::from_millis(50));
        s.close();
        assert!(t.join().unwrap());
        assert!(s.sleep(Duration::from_secs(10)));

        let d = Arc::new(DeadlineSignal::default());
        let waiter = {
            let d = d.clone();
            thread::spawn(move || d.wait_until(1_000_000, &|| 1_000))
        };
        while !d.is_waiting() {
            thread::sleep(Duration::from_millis(5));
        }
        d.close();
        assert!(waiter.join().unwrap());
        assert!(d.wait_until(1_000_000, &|| 1_000));
    }

    #[test]
    fn removed_expiration_signal_is_replaced() {
        let a = expiration_signal("remove-test");
        remove_expiration_signal("remove-test", &a);
        let b = expiration_signal("remove-test");
        assert!(!Arc::ptr_eq(&a, &b));
        // A late remove for the first signal leaves the second registered.
        remove_expiration_signal("remove-test", &a);
        assert!(Arc::ptr_eq(&b, &expiration_signal("remove-test")));
    }
}
