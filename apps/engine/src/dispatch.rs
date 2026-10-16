// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The main thread's command handler (`runListenOnMainThread`). Commands from stdin and,
//! with `--socket`, from socket connections arrive over one channel as [`Command`] values
//! and are handled in order (docs/engine-protocol-plan.md S0). The main thread owns the
//! `Store` the local phases run on.
//!
//! Derived from Mailspring-Sync (`MailSync/main.cpp`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use ms_core::constants::{thread, EXIT_OK, QUEUE_TASK_IDLE_DEBOUNCE_MS, STREAM_DELAY_MAIN_MS};
use ms_core::SyncError;
use ms_ipc::{code, ErrorObject};
use ms_store::{Model, Task};
use ms_sync::WorkerContext;
use ms_tasks::TaskProcessor;
use serde_json::{json, Value};

use crate::platform;

/// What a command asks for. The stdin `type`s, decoded.
#[derive(Debug)]
pub enum Request {
    /// `queue-task`: the task JSON as sent. Anything but an object skips the local phase
    /// but still wakes the foreground worker, as stdin always has.
    QueueTask(Value),
    CancelTask(String),
    WakeWorkers,
    NeedBodies(Vec<String>),
    SyncCalendar,
    /// stdin only.
    TestCrash,
    /// stdin only.
    TestSegfault,
}

/// A method's result, or the error to answer with.
pub type Reply = Result<Value, ErrorObject>;

pub struct Command {
    pub request: Request,
    /// Where the result goes; `None` for stdin, which answers nothing.
    pub reply: Option<mpsc::Sender<Reply>>,
}

/// Opens the main thread's store, cleans up tasks left by the last run, calls `start` with
/// the sender the transports use, then handles commands until every sender is gone.
pub fn run(ctx: WorkerContext, start: impl FnOnce(mpsc::Sender<Command>)) -> i32 {
    let store = match ms_store::Store::open(&ctx.config_dir, ctx.deltas.clone()) {
        Ok(s) => s,
        Err(e) => {
            ms_core::critical!("Could not open the database: {e}");
            ms_core::log::flush();
            std::process::abort();
        }
    };
    store.set_stream_delay(STREAM_DELAY_MAIN_MS);
    let tasks = TaskProcessor::for_context(&ctx, &store);
    if let Err(e) = tasks.cleanup_tasks_after_launch() {
        abort_on_task_error(&e);
    }

    let (tx, rx) = mpsc::channel();
    start(tx);
    let dispatcher = Dispatcher {
        ctx: &ctx,
        tasks: &tasks,
        fg_wake_queued: Arc::new(AtomicBool::new(false)),
        calendar_running: Arc::new(AtomicBool::new(false)),
    };
    for command in rx {
        let result = dispatcher.handle(command.request);
        if let Some(reply) = command.reply {
            let _ = reply.send(result);
        }
    }
    EXIT_OK
}

/// A database error from a task's local phase or a cancel leaves the task as it was; the
/// C++ main loop let it escape and aborted.
fn abort_on_task_error(e: &ms_core::Error) -> ! {
    ms_core::critical!("An unexpected error occurred during program execution: {e}");
    ms_core::log::flush();
    std::process::abort();
}

struct Dispatcher<'a> {
    ctx: &'a WorkerContext,
    tasks: &'a TaskProcessor<'a>,
    fg_wake_queued: Arc<AtomicBool>,
    calendar_running: Arc<AtomicBool>,
}

impl Dispatcher<'_> {
    fn handle(&self, request: Request) -> Reply {
        match request {
            Request::QueueTask(task) => self.queue_task(task),
            Request::CancelTask(id) => {
                // A task that has started (or finished) cannot always be dequeued, but a
                // deleted draft's pending saves can.
                if let Err(e) = self.tasks.cancel(&id) {
                    abort_on_task_error(&e);
                }
                Ok(json!({}))
            }
            Request::WakeWorkers => {
                tracing::info!("{}", ms_core::logmsg::waking_all_workers());
                self.ctx.background_mark_all.store(true, Ordering::SeqCst);
                ms_core::wake::wake_all_workers();
                // The network may have changed while asleep; don't leave IDLE hanging on a
                // dead connection.
                self.ctx.foreground.interrupt();
                Ok(json!({}))
            }
            Request::NeedBodies(ids) => {
                self.ctx.foreground.queue_bodies(ids);
                self.ctx.foreground.interrupt();
                Ok(json!({}))
            }
            Request::SyncCalendar => Ok(json!({ "started": self.sync_calendar() })),
            Request::TestCrash => {
                let e = SyncError::fatal("test", "triggered via cin");
                ms_core::critical!("{}", e.to_json());
                ms_core::log::flush();
                std::process::abort();
            }
            Request::TestSegfault => platform::raise_segfault(),
        }
    }

    /// Runs the local phase and answers with the task as it left it; a failed local phase
    /// answers with its error, the task (status `complete`) still arrives as a delta.
    fn queue_task(&self, mut json: Value) -> Reply {
        let mut reply = Err(ErrorObject::new(
            code::INVALID_PARAMS,
            "the task must be an object",
        ));
        if let Some(obj) = json.as_object_mut() {
            obj.insert("v".into(), Value::from(0));
            reply = match Task::from_json(json) {
                Ok(task) => self.perform_local(task),
                Err(e) => Err(ErrorObject::new(
                    code::INVALID_PARAMS,
                    format!("invalid task: {e}"),
                )),
            };
        }

        // Interrupt the foreground IDLE so it runs the remote phase, after a short
        // wait so tasks queued back to back go together instead of fighting the
        // foreground worker for the database.
        if self
            .fg_wake_queued
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let flag = self.fg_wake_queued.clone();
            let fg = self.ctx.foreground.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(QUEUE_TASK_IDLE_DEBOUNCE_MS));
                fg.interrupt();
                flag.store(false, Ordering::SeqCst);
            });
        }
        reply
    }

    fn perform_local(&self, mut task: Task) -> Reply {
        if let Err(e) = self.tasks.perform_local(&mut task) {
            abort_on_task_error(&e);
        }
        if task.error().is_null() {
            Ok(task.data)
        } else {
            Err(
                ErrorObject::new(code::TASK_REJECTED, "the task's local phase failed")
                    .with_data(task.error().clone()),
            )
        }
    }

    /// Starts a calendar pass unless one is running. Returns whether it started one.
    fn sync_calendar(&self) -> bool {
        if self
            .calendar_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let flag = self.calendar_running.clone();
        let c = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(thread::CALENDAR.into())
            .spawn(move || {
                ms_sync::cal_contacts::run_calendar_pass(c);
                flag.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.calendar_running.store(false, Ordering::SeqCst);
            return false;
        }
        true
    }
}
