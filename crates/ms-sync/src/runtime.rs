// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! One account's threads in a process that may hold several:
//! starting them, stopping them cooperatively, and turning a failure on one of
//! them into a stop of that account alone.
//!
//! A failure is a worker's non-retryable error, a database error in a task's local phase,
//! `test-crash`, or a panic on one of the account's threads (caught at the thread's top).
//! It is reported once, to the [`FailureHandler`] given to [`WorkerContext::new`], on the
//! failing thread and before the stop is requested. `sync` mode's handler logs the failure
//! as critical and aborts, as the C++ worker loops did, so nothing after it runs there.
//!
//! The stop is cooperative: workers check [`WorkerContext::is_stopping`] at their loop
//! points, and the stop ends their sleeps and the foreground IDLE. A thread blocked in a
//! network call notices only when the call returns; [`AccountRuntime::stop`] waits for it
//! up to a timeout and then leaves it to finish on its own.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ms_core::constants::{thread, ACCOUNT_STOP_POLL_MS};
use ms_core::wake::{expiration_signal, remove_expiration_signal, DeadlineSignal, WakeSignal};
use serde_json::Value;

use crate::WorkerContext;

/// What stopped an account.
#[derive(Clone, Debug)]
pub struct AccountFailure {
    /// The thread it happened on.
    pub thread: String,
    /// The line `sync` mode logs as critical before aborting.
    pub message: String,
    /// The `SyncError` JSON (`unhandled-exception` for anything else).
    pub error: Value,
}

impl AccountFailure {
    /// A failure on the calling thread.
    pub fn new(message: impl Into<String>, error: Value) -> Self {
        AccountFailure {
            thread: std::thread::current().name().unwrap_or("").to_owned(),
            message: message.into(),
            error,
        }
    }

    /// A worker loop's error, worded as the C++ `logCurrentExceptionWithStackTrace` was.
    pub fn from_error(e: &ms_core::Error) -> Self {
        AccountFailure::new(crate::worker_error_text(e), e.to_json())
    }

    fn from_panic(thread: &str, payload: &(dyn Any + Send)) -> Self {
        let what = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic without a message".to_owned());
        let message = format!("Unhandled panic on thread {thread}: {what}");
        AccountFailure {
            thread: thread.to_owned(),
            error: ms_core::error::unexpected_error_json(&message),
            message,
        }
    }
}

/// Called once per account with the failure that stops it, on the failing thread. It must
/// not call [`AccountRuntime::stop`] for the same account from there: hand the stop off.
pub type FailureHandler = Arc<dyn Fn(&AccountFailure) + Send + Sync>;

/// The account's stop flag, failure and threads, shared by every clone of its
/// [`WorkerContext`].
pub struct AccountControl {
    account_id: Arc<str>,
    stopping: AtomicBool,
    /// Closed when the stop is requested; ends the sleeps that `wake-workers` must not end.
    stop_signal: WakeSignal,
    /// The expiration signal registered for this run of the account. A thread left from an
    /// earlier run that stops late closes its own run's signal, not this one.
    expiration: Arc<DeadlineSignal>,
    failure: Mutex<Option<AccountFailure>>,
    on_failure: FailureHandler,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl AccountControl {
    pub(crate) fn new(account_id: &str, on_failure: FailureHandler) -> Self {
        AccountControl {
            account_id: account_id.into(),
            stopping: AtomicBool::new(false),
            stop_signal: WakeSignal::new(),
            expiration: expiration_signal(account_id),
            failure: Mutex::new(None),
            on_failure,
            threads: Mutex::new(Vec::new()),
        }
    }

    fn threads(&self) -> std::sync::MutexGuard<'_, Vec<JoinHandle<()>>> {
        self.threads.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl WorkerContext {
    /// Whether the account is stopping; its threads return at their next loop point.
    pub fn is_stopping(&self) -> bool {
        self.control.stopping.load(Ordering::SeqCst)
    }

    /// Sleeps for `secs` or until `wake-workers` or the stop (`sleepWorkerUntilWakeOrSec`).
    /// Returns true if woken early.
    pub fn sleep_until_wake_or(&self, secs: u64) -> bool {
        self.wake.sleep(Duration::from_secs(secs))
    }

    /// Sleeps for `duration` or until the stop; `wake-workers` does not end it. Returns
    /// whether the account is stopping.
    pub fn pause(&self, duration: Duration) -> bool {
        self.control.stop_signal.sleep(duration);
        self.is_stopping()
    }

    /// `wake-workers` for this account: ends the workers' sleeps and wakes the metadata
    /// expiration worker so it re-checks for metadata that expired while the machine slept.
    pub fn wake_workers(&self) {
        self.wake.wake_all();
        self.control.expiration.wake();
    }

    /// The metadata expiration worker's wake handle for this run of the account.
    pub fn expiration_signal(&self) -> &Arc<DeadlineSignal> {
        &self.control.expiration
    }

    /// Asks every thread of the account to return: sets the flag, ends every sleep and
    /// interrupts IDLE. Does not wait; see [`AccountRuntime::stop`].
    pub fn request_stop(&self) {
        self.control.stopping.store(true, Ordering::SeqCst);
        self.control.stop_signal.close();
        self.wake.close();
        self.control.expiration.close();
        self.foreground.interrupt();
    }

    /// Reports `failure` (the first one per account only) and stops the account. The
    /// caller returns from its thread body afterwards.
    pub fn fail(&self, failure: AccountFailure) {
        let first = {
            let mut slot = self
                .control
                .failure
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = Some(failure.clone());
                true
            } else {
                false
            }
        };
        if first {
            (self.control.on_failure)(&failure);
        }
        self.request_stop();
    }

    /// [`WorkerContext::fail`] with a worker loop's error.
    pub fn fail_with(&self, e: &ms_core::Error) {
        self.fail(AccountFailure::from_error(e));
    }

    /// The failure that stopped the account, if one did.
    pub fn failure(&self) -> Option<AccountFailure> {
        self.control
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Spawns one of the account's threads, named `name`: its log lines go to the account's
    /// log, a panic on it stops the account, and [`AccountRuntime::stop`] waits for it. Once
    /// the account is stopping nothing is spawned.
    pub fn spawn(&self, name: &str, f: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
        if self.is_stopping() {
            return Ok(());
        }
        let ctx = self.clone();
        let thread_name = name.to_owned();
        let handle = ms_core::log::spawn_account_thread(
            Some(self.control.account_id.clone()),
            name,
            move || {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(f)) {
                    ctx.fail(AccountFailure::from_panic(&thread_name, &*payload));
                }
            },
        )?;
        let mut threads = self.control.threads();
        threads.retain(|h| !h.is_finished());
        threads.push(handle);
        Ok(())
    }
}

/// The body of an account's `main` thread: the command handler, given the receiving end of
/// the account's command channel and a sender for the transports it starts.
pub type Dispatcher<C> = Box<dyn FnOnce(WorkerContext, Receiver<C>, Sender<C>) + Send>;

/// One account's running threads: `background` (which starts `foreground`),
/// `calContacts`, `metadata` unless the backend is local-only, `metadataExpiration`, and
/// `main` running the dispatcher, fed by the account's command channel.
pub struct AccountRuntime<C> {
    ctx: WorkerContext,
    commands: Mutex<Option<Sender<C>>>,
}

impl<C: Send + 'static> AccountRuntime<C> {
    /// Starts the account's threads. Without a dispatcher no `main` thread runs (`sync
    /// --orphan` without a socket) and commands sent to the channel are never read.
    pub fn start(ctx: WorkerContext, dispatcher: Option<Dispatcher<C>>) -> Self {
        let (tx, rx) = mpsc::channel();
        let c = ctx.clone();
        start_thread(&ctx, thread::BACKGROUND, move || {
            crate::background::run_background(c)
        });
        let c = ctx.clone();
        start_thread(&ctx, thread::CAL_CONTACTS, move || {
            crate::cal_contacts::run_cal_contacts(c)
        });
        if !ctx.metadata.is_local_only() {
            let c = ctx.clone();
            start_thread(&ctx, thread::METADATA, move || {
                crate::metadata_worker::run_metadata_worker(c)
            });
        }
        let c = ctx.clone();
        start_thread(&ctx, thread::METADATA_EXPIRATION, move || {
            crate::metadata_expiration::run_metadata_expiration(c)
        });
        if let Some(dispatcher) = dispatcher {
            let (c, sender) = (ctx.clone(), tx.clone());
            start_thread(&ctx, thread::MAIN, move || dispatcher(c, rx, sender));
        }
        AccountRuntime {
            ctx,
            commands: Mutex::new(Some(tx)),
        }
    }

    pub fn context(&self) -> &WorkerContext {
        &self.ctx
    }

    /// A sender for the account's `main` thread; `None` once the account is stopping.
    pub fn commands(&self) -> Option<Sender<C>> {
        if self.ctx.is_stopping() {
            return None;
        }
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The names of the account's threads that have not exited.
    pub fn running_threads(&self) -> Vec<String> {
        self.ctx
            .control
            .threads()
            .iter()
            .filter(|h| !h.is_finished())
            .map(|h| h.thread().name().unwrap_or("").to_owned())
            .collect()
    }

    /// Blocks until the `main` thread exits, or `background` without one. `sync` mode's
    /// process main thread waits here.
    pub fn wait(&self) {
        let wanted = |h: &JoinHandle<()>, name: &str| h.thread().name() == Some(name);
        let handle = {
            let mut threads = self.ctx.control.threads();
            let pick = threads
                .iter()
                .position(|h| wanted(h, thread::MAIN))
                .or_else(|| threads.iter().position(|h| wanted(h, thread::BACKGROUND)));
            pick.map(|i| threads.swap_remove(i))
        };
        if let Some(h) = handle {
            let _ = h.join();
        }
    }

    /// Requests the stop and waits up to `timeout` for the account's threads to exit.
    /// Returns the names of those still running, which are left to finish on their own.
    /// The calling thread is never waited for, so a thread of the account may call it.
    pub fn stop(&self, timeout: Duration) -> Vec<String> {
        self.ctx.request_stop();
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let me = std::thread::current().id();
        let deadline = Instant::now() + timeout;
        let mut pending: Vec<JoinHandle<()>> = Vec::new();
        loop {
            // Threads spawned while the stop was being requested register late.
            pending.append(&mut self.ctx.control.threads());
            pending.retain(|h| h.thread().id() != me);
            let (done, running): (Vec<_>, Vec<_>) =
                pending.into_iter().partition(JoinHandle::is_finished);
            for h in done {
                let _ = h.join();
            }
            pending = running;
            if pending.is_empty() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        remove_expiration_signal(self.ctx.account.id(), &self.ctx.control.expiration);
        let stragglers: Vec<String> = pending
            .iter()
            .map(|h| h.thread().name().unwrap_or("").to_owned())
            .collect();
        if !stragglers.is_empty() {
            tracing::warn!(
                "-X Threads of account {} still running after the stop: {}",
                self.ctx.account.id(),
                stragglers.join(", ")
            );
        }
        stragglers
    }
}

/// The next command for an account's `main` thread, or `None` once the account is stopping
/// or every sender is gone.
pub fn next_command<C>(ctx: &WorkerContext, commands: &Receiver<C>) -> Option<C> {
    loop {
        if ctx.is_stopping() {
            return None;
        }
        match commands.recv_timeout(Duration::from_millis(ACCOUNT_STOP_POLL_MS)) {
            Ok(c) => return (!ctx.is_stopping()).then_some(c),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// A thread the account cannot run without: failing to start it ends the process, as it did
/// in the C++ (the process is out of threads).
fn start_thread(ctx: &WorkerContext, name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = ctx.spawn(name, f) {
        ms_core::critical!("Could not start the {name} thread: {e}");
        ms_core::log::flush();
        std::process::abort();
    }
}
