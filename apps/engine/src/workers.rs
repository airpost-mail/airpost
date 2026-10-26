// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Worker lanes for the calls the engine answers out of order (T21): the socket's reads
//! (`read_pool.rs`), `integrations.*`, `ai.*`, `files.*` downloads and `serve`'s slow parent
//! methods. Each lane is a named, bounded set
//! of threads with a bounded queue. Threads start as work arrives, up to the lane's limit,
//! and end after [`IDLE`] without work; a call that finds the queue full is answered
//! [`code::ENGINE_BUSY`] at once.
//!
//! A job runs with the account it names set as its thread's account (for the log). In a
//! contained lane a panic is answered as an internal error and the thread goes on; the panic
//! hook lets those threads unwind ([`is_contained`]). Elsewhere a panic aborts the process,
//! as on any engine thread.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use tern_core::constants::thread;
use tern_ipc::{code, ErrorObject};

use crate::dispatch::Reply;

/// A thread with nothing to do for this long ends.
const IDLE: Duration = Duration::from_secs(30);

/// The socket connections' reads, each up to `READ_TIMEOUT_MS`; a connection runs at most
/// `read_pool::POOL_SIZE` at once.
pub static READS: Lane = Lane::new(thread::READS, 16, 256, true);
/// Provider HTTP calls, each up to 30 s (tern-integrations' timeout).
pub static INTEGRATIONS: Lane = Lane::new(thread::INTEGRATIONS, 8, 64, true);
/// Model calls, each up to 120 s (tern-ai's total timeout); a streamed one holds its thread
/// while it streams.
pub static AI: Lane = Lane::new(thread::AI, 8, 32, true);
/// `files.get` waiting for a download; a message's inline images come together. A call
/// queued here is still answered within its wait: the deadline starts when it arrives.
pub static FILES: Lane = Lane::new(thread::FILES, 32, 512, true);
/// `messages.unsubscribe` waiting for a body or making a one-click POST; `unsubscribe.rs`
/// caps those at 16 and 8 engine-wide, so every call gets a thread at once.
pub static UNSUBSCRIBE: Lane = Lane::new(thread::UNSUBSCRIBE, 24, 8, true);
pub static ACCOUNT_TEST: Lane = Lane::new(thread::ACCOUNT_TEST, 8, 32, true);
/// Discovery (a 5 s MX lookup, autoconfig fetches), OAuth sign-ins and tested adds.
pub static ACCOUNT_SETUP: Lane = Lane::new(thread::ACCOUNT_SETUP, 8, 32, true);
pub static ACCOUNT_RESET: Lane = Lane::new(thread::ACCOUNT_RESET, 4, 32, true);
/// `engine.resetDatabase` and `engine.setLogLevel`, one at a time. Not contained: they
/// stop and restart every account, and a panic halfway leaves the engine in no known state.
pub static ENGINE_MAINTENANCE: Lane = Lane::new(thread::ENGINE_MAINTENANCE, 1, 16, false);

const CONTAINED: &[&Lane] = &[
    &READS,
    &INTEGRATIONS,
    &AI,
    &FILES,
    &UNSUBSCRIBE,
    &ACCOUNT_TEST,
    &ACCOUNT_SETUP,
    &ACCOUNT_RESET,
];

/// Whether `thread_name` is a contained lane's, whose panics are answered rather than
/// fatal.
pub fn is_contained(thread_name: &str) -> bool {
    CONTAINED.iter().any(|l| l.name == thread_name)
}

/// The answer to a call whose lane is full.
pub fn busy(lane: &Lane) -> ErrorObject {
    ErrorObject::new(
        code::ENGINE_BUSY,
        format!(
            "the engine is busy: {} calls are waiting for {} workers; try again later",
            lane.max_queued, lane.name
        ),
    )
}

type Work = Box<dyn FnOnce() -> Reply + Send>;
type Answer = Box<dyn FnOnce(Reply) + Send>;

struct Job {
    account: Option<Arc<str>>,
    work: Work,
    answer: Answer,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    threads: usize,
    idle: usize,
    /// Threads running a job.
    busy: usize,
}

pub struct Lane {
    name: &'static str,
    max_threads: usize,
    max_queued: usize,
    contained: bool,
    state: Mutex<Option<State>>,
    ready: Condvar,
}

impl Lane {
    pub const fn new(
        name: &'static str,
        max_threads: usize,
        max_queued: usize,
        contained: bool,
    ) -> Lane {
        Lane {
            name,
            max_threads,
            max_queued,
            contained,
            state: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<State>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queues `work`; `answer` gets its result on the worker thread. When the queue is full
    /// `answer` gets [`busy`] at once, on the caller's thread.
    pub fn submit(
        &'static self,
        account: Option<Arc<str>>,
        work: impl FnOnce() -> Reply + Send + 'static,
        answer: impl FnOnce(Reply) + Send + 'static,
    ) {
        let mut guard = self.lock();
        let state = guard.get_or_insert_with(State::default);
        // Queued jobs that a free thread (idle or about to start) will take aren't waiting.
        let free = self.max_threads.saturating_sub(state.busy);
        if state.queue.len() >= self.max_queued + free {
            drop(guard);
            answer(Err(busy(self)));
            return;
        }
        state.queue.push_back(Job {
            account,
            work: Box::new(work),
            answer: Box::new(answer),
        });
        if state.queue.len() > state.idle && state.threads < self.max_threads {
            state.threads += 1;
            drop(guard);
            crate::modes::spawn(self.name, move || self.run());
        } else {
            drop(guard);
            self.ready.notify_one();
        }
    }

    fn run(&'static self) {
        let mut finished = false;
        while let Some(job) = self.next_job(finished) {
            finished = true;
            tern_core::log::set_thread_account(job.account);
            let result = if self.contained {
                catch_unwind(AssertUnwindSafe(job.work)).unwrap_or_else(|_| {
                    Err(ErrorObject::new(
                        code::INTERNAL_ERROR,
                        "the call panicked; see the log",
                    ))
                })
            } else {
                (job.work)()
            };
            (job.answer)(result);
            tern_core::log::set_thread_account(None);
        }
    }

    /// The next job, or `None` once this thread has waited [`IDLE`] for one; the thread then
    /// ends. `finished` says the thread just finished a job.
    fn next_job(&self, finished: bool) -> Option<Job> {
        let mut guard = self.lock();
        if finished {
            if let Some(state) = guard.as_mut() {
                state.busy -= 1;
            }
        }
        loop {
            let state = guard.get_or_insert_with(State::default);
            if let Some(job) = state.queue.pop_front() {
                state.busy += 1;
                return Some(job);
            }
            state.idle += 1;
            let (next, wait) = self
                .ready
                .wait_timeout(guard, IDLE)
                .unwrap_or_else(|e| e.into_inner());
            guard = next;
            let state = guard.get_or_insert_with(State::default);
            state.idle -= 1;
            if wait.timed_out() && state.queue.is_empty() {
                state.threads -= 1;
                return None;
            }
        }
    }

    #[cfg(test)]
    fn threads(&self) -> usize {
        self.lock().as_ref().map_or(0, |s| s.threads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;

    #[test]
    fn runs_up_to_its_threads_at_once_then_queues_then_refuses() {
        static LANE: Lane = Lane::new("workersTest", 2, 2, true);
        let (started_tx, started) = mpsc::channel::<()>();
        let (release_tx, release) = mpsc::channel::<()>();
        let release = Arc::new(Mutex::new(release));
        let (answers_tx, answers) = mpsc::channel::<Reply>();
        for _ in 0..5 {
            let started_tx = started_tx.clone();
            let release = release.clone();
            let answers_tx = answers_tx.clone();
            LANE.submit(
                None,
                move || {
                    started_tx.send(()).unwrap();
                    release.lock().unwrap().recv().unwrap();
                    Ok(json!(1))
                },
                move |r| answers_tx.send(r).unwrap(),
            );
        }
        // Two run, two wait, the fifth is refused at once.
        for _ in 0..2 {
            started.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let refused = answers.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(refused.unwrap_err().code, code::ENGINE_BUSY);
        assert!(started.recv_timeout(Duration::from_millis(100)).is_err());
        assert_eq!(LANE.threads(), 2);
        for _ in 0..4 {
            release_tx.send(()).unwrap();
        }
        for _ in 0..4 {
            assert_eq!(
                answers
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap(),
                json!(1)
            );
        }
    }

    #[test]
    fn a_panic_in_a_contained_lane_is_answered_and_the_thread_goes_on() {
        static LANE: Lane = Lane::new("workersPanicTest", 1, 4, true);
        let (tx, rx) = mpsc::channel::<Reply>();
        let tx2 = tx.clone();
        LANE.submit(
            Some("acct".into()),
            || panic!("boom"),
            move |r| tx.send(r).unwrap(),
        );
        LANE.submit(
            None,
            || Ok(json!(tern_core::log::thread_account().is_none())),
            move |r| tx2.send(r).unwrap(),
        );
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first.unwrap_err().code, code::INTERNAL_ERROR);
        // The same thread ran the second job, without the first job's account.
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(),
            json!(true)
        );
        assert_eq!(LANE.threads(), 1);
    }

    #[test]
    fn jobs_carry_their_account() {
        static LANE: Lane = Lane::new("workersAccountTest", 1, 4, true);
        let (tx, rx) = mpsc::channel::<Reply>();
        LANE.submit(
            Some("acct-1".into()),
            || Ok(json!(tern_core::log::thread_account().as_deref())),
            move |r| tx.send(r).unwrap(),
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(),
            json!("acct-1")
        );
    }
}
