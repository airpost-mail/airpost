// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! An account's `main` thread: the command handler (`runListenOnMainThread`). Commands
//! from stdin and, with `--socket` or in `serve` mode, from socket connections arrive over
//! the account's channel as [`Command`] values and are handled in order.
//! The thread owns the `Store` the local phases run on.
//!
//! Derived from Mailspring-Sync (`MailSync/main.cpp`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tern_core::constants::{thread, QUEUE_TASK_IDLE_DEBOUNCE_MS, STREAM_DELAY_MAIN_MS};
use tern_core::SyncError;
use tern_ipc::{code, ErrorObject};
use tern_net::unsubscribe::Mailto;
use tern_store::{File, Model, Task};
use tern_sync::{AccountFailure, WorkerContext};
use tern_tasks::mail_actions::{self, ActionError};
use tern_tasks::TaskProcessor;

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
    /// `files.put`: saves the File JSON (its bytes are already on disk) and answers with it.
    SaveFile(Value),
    /// `threads.snooze`: answers with [`tern_ipc::ThreadsSnoozeResult`].
    Snooze {
        thread_ids: Vec<String>,
        until: i64,
    },
    /// `threads.unsnooze`: answers with [`tern_ipc::ThreadsSnoozeResult`].
    Unsnooze {
        thread_ids: Vec<String>,
    },
    /// `messages.unsubscribe` by mail: queues the message and answers `{taskId}`.
    UnsubscribeMailto {
        message_id: String,
        mailto: Mailto,
    },
    /// `draft.create` (`crate::drafts_api`).
    DraftCreate(Box<crate::drafts_api::CreateCall>),
    /// `draft.send`.
    DraftSend(tern_ipc::DraftSendParams),
    /// `draft.cancelSend`.
    DraftCancelSend(tern_ipc::DraftCancelSendParams),
    /// `draft.validate`, with every account's addresses.
    DraftValidate(tern_ipc::DraftValidateParams, tern_compose::Identities),
    /// The `events.*` and `contacts.*` writes (`crate::pim_api`).
    Pim(Box<crate::pim_api::Call>),
    /// stdin only.
    TestCrash,
    /// stdin only.
    TestSegfault,
    /// `serve` mode's `test.panic`, debug builds only: a panic on the account's `main`
    /// thread, which stops only that account.
    TestPanic,
}

/// A method's result, or the error to answer with.
pub type Reply = Result<Value, ErrorObject>;

/// A reply now, or one another thread works out later, out of order with the connection's
/// later requests (T21).
pub enum Deferred {
    Now(Reply),
    Later(Box<dyn FnOnce() -> Reply + Send>),
}

pub struct Command {
    pub request: Request,
    /// Where the result goes; `None` for stdin, which answers nothing.
    pub reply: Option<mpsc::Sender<Reply>>,
}

/// Opens the thread's store, cleans up tasks left by the last run, calls `start` (which
/// starts the transports), then handles commands until the account stops or every sender
/// is gone. A database error on the way stops the account.
pub fn run(ctx: WorkerContext, commands: mpsc::Receiver<Command>, start: impl FnOnce()) {
    let store = match tern_store::Store::open(&ctx.config_dir, ctx.deltas.clone()) {
        Ok(s) => s,
        Err(e) => {
            let message = format!("Could not open the database: {e}");
            return ctx.fail(AccountFailure::new(message, e.to_json()));
        }
    };
    store.set_stream_delay(STREAM_DELAY_MAIN_MS);
    let tasks = TaskProcessor::for_context(&ctx, &store);
    if let Err(e) = tasks.cleanup_tasks_after_launch() {
        fail_on_task_error(&ctx, &e);
        return;
    }

    start();
    let dispatcher = Dispatcher {
        ctx: &ctx,
        tasks: &tasks,
        fg_wake_queued: Arc::new(AtomicBool::new(false)),
        calendar_running: Arc::new(AtomicBool::new(false)),
    };
    while let Some(command) = tern_sync::runtime::next_command(&ctx, &commands) {
        let result = dispatcher.handle(command.request);
        if let Some(reply) = command.reply {
            let _ = reply.send(result);
        }
    }
}

/// A database error from a task's local phase or a cancel leaves the task as it was and
/// stops the account (the C++ main loop let it escape and aborted). Returns the reply for
/// the command that hit it.
fn fail_on_task_error(ctx: &WorkerContext, e: &tern_core::Error) -> ErrorObject {
    let message = format!("An unexpected error occurred during program execution: {e}");
    ctx.fail(AccountFailure::new(message, e.to_json()));
    ErrorObject::new(
        code::INTERNAL_ERROR,
        "the account stopped on a database error",
    )
    .with_data(e.to_json())
}

fn action_error(e: ActionError) -> ErrorObject {
    match e {
        ActionError::NotFound(why) | ActionError::Unavailable(why) => {
            ErrorObject::new(code::NOT_FOUND, why)
        }
        ActionError::Database(e) => internal(e),
    }
}

fn internal(e: tern_core::Error) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

impl crate::drafts_api::QueueTask for Dispatcher<'_> {
    fn queue_task(&self, task: Value) -> Reply {
        Dispatcher::queue_task(self, task)
    }
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
                    return Err(fail_on_task_error(self.ctx, &e));
                }
                Ok(json!({}))
            }
            Request::WakeWorkers => {
                tracing::info!("{}", tern_core::logmsg::waking_all_workers());
                self.ctx.background_mark_all.store(true, Ordering::SeqCst);
                self.ctx.wake_workers();
                // The network may have changed while asleep; don't leave IDLE hanging on a
                // dead connection, and try the sends deferred while it was down.
                self.ctx.foreground.retry_deferred();
                Ok(json!({}))
            }
            Request::NeedBodies(ids) => {
                self.ctx.foreground.queue_bodies(ids);
                self.ctx.foreground.interrupt();
                Ok(json!({}))
            }
            Request::SyncCalendar => Ok(json!({ "started": self.sync_calendar() })),
            Request::SaveFile(json) => {
                let mut file = File::from_json(json).map_err(internal)?;
                self.tasks.store.save(&mut file).map_err(internal)?;
                Ok(file.data)
            }
            Request::Snooze { thread_ids, until } => {
                let tasks = mail_actions::snooze_tasks(
                    self.tasks.store,
                    self.ctx.account.id(),
                    &thread_ids,
                    until,
                );
                self.queue_all(tasks)
            }
            Request::Unsnooze { thread_ids } => {
                let tasks = mail_actions::unsnooze_tasks(
                    self.tasks.store,
                    self.ctx.account.id(),
                    &thread_ids,
                );
                self.queue_all(tasks)
            }
            Request::UnsubscribeMailto { message_id, mailto } => {
                let task = mail_actions::unsubscribe_send_task(
                    self.tasks.store,
                    &self.ctx.account,
                    &message_id,
                    &mailto,
                )
                .map_err(action_error)?;
                let id = task.id().to_owned();
                self.queue_task(task.data)?;
                Ok(json!({ "taskId": id }))
            }
            Request::DraftCreate(call) => self.drafts().create(*call),
            Request::DraftSend(p) => self.drafts().send(p),
            Request::DraftCancelSend(p) => self.drafts().cancel_send(p),
            Request::DraftValidate(p, accounts) => self.drafts().validate(p, accounts),
            Request::Pim(call) => crate::pim_api::Here {
                store: self.tasks.store,
                account: &self.ctx.account,
                queue: self,
            }
            .handle(*call),
            Request::TestCrash => {
                let e = SyncError::fatal("test", "triggered via cin");
                self.ctx
                    .fail(AccountFailure::new(e.to_json().to_string(), e.to_json()));
                Ok(json!({}))
            }
            Request::TestSegfault => platform::raise_segfault(),
            Request::TestPanic => panic!("triggered via test.panic"),
        }
    }

    fn drafts(&self) -> crate::drafts_api::Here<'_> {
        crate::drafts_api::Here {
            store: self.tasks.store,
            account: &self.ctx.account,
            config_dir: &self.ctx.config_dir,
            queue: self,
            metadata: &*self.ctx.metadata,
        }
    }

    /// Queues `tasks` in order, stopping at the first whose local phase fails, and answers
    /// with their ids.
    fn queue_all(&self, tasks: Result<Vec<Task>, ActionError>) -> Reply {
        let mut ids = Vec::new();
        for task in tasks.map_err(action_error)? {
            ids.push(task.id().to_owned());
            self.queue_task(task.data)?;
        }
        Ok(json!({ "taskIds": ids }))
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
            let spawned = self.ctx.spawn(thread::QUEUE_TASK_DEBOUNCE, move || {
                std::thread::sleep(Duration::from_millis(QUEUE_TASK_IDLE_DEBOUNCE_MS));
                fg.interrupt();
                flag.store(false, Ordering::SeqCst);
            });
            if spawned.is_err() {
                self.fg_wake_queued.store(false, Ordering::SeqCst);
            }
        }
        reply
    }

    fn perform_local(&self, mut task: Task) -> Reply {
        if let Err(e) = self.tasks.perform_local(&mut task) {
            return Err(fail_on_task_error(self.ctx, &e));
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
        let spawned = self.ctx.spawn(thread::CALENDAR, move || {
            tern_sync::cal_contacts::run_calendar_pass(c);
            flag.store(false, Ordering::SeqCst);
        });
        if spawned.is_err() {
            self.calendar_running.store(false, Ordering::SeqCst);
            return false;
        }
        true
    }
}
