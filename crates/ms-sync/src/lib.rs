// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The sync workers.
//!
//! Derived from Mailspring-Sync (`SyncWorker.*`, `MailProcessor.*`, `MetadataWorker.*`,
//! `MetadataExpirationWorker.*`, `DAVWorker.*`, `GoogleContactsWorker.*` and the worker
//! loops in `main.cpp`), Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Each `run_*` function is the body of one named thread (plan 4.1) and never returns
//! while the process lives. Each opens its own [`ms_store::Store`] on its thread.

pub mod background;
pub mod cal_contacts;
pub mod dav;
pub mod foreground;
pub mod google_contacts;
pub mod mail_processor;
pub mod mailbox_model;
pub mod metadata_expiration;
pub mod metadata_worker;
pub mod tasks;
mod worker;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use ms_core::{Account, Clock};
use ms_net::{MetadataBackend, TokenManager};
use ms_store::DeltaStream;

pub use foreground::ForegroundControl;
pub use mailbox_model::MailboxModel;
pub use tasks::{RemoteTaskRunner, TaskRunnerFactory};

/// What every worker thread is given. Cheap to clone; everything shared is behind an `Arc`.
#[derive(Clone)]
pub struct WorkerContext {
    pub account: Arc<Account>,
    pub config_dir: PathBuf,
    pub deltas: Arc<DeltaStream>,
    pub clock: Arc<dyn Clock>,
    /// `--verbose`: log IMAP and SMTP traffic.
    pub verbose: bool,
    /// The foreground worker's IDLE interrupt and body queue, shared with `main`.
    pub foreground: Arc<ForegroundControl>,
    /// Set by `wake-workers`: the background worker marks every folder busy at the start
    /// of its next pass, on its own thread (`bgWorkerShouldMarkAll`).
    pub background_mark_all: Arc<AtomicBool>,
    pub tokens: Arc<TokenManager>,
    pub metadata: Arc<dyn MetadataBackend>,
    /// Builds the task runner the foreground worker uses for remote phases (ms-tasks).
    pub tasks: Arc<dyn TaskRunnerFactory>,
}

/// A failure no retry can fix (a bug, a non-retryable `SyncError`): log it as critical and
/// abort, as the worker loops in `main.cpp` did.
pub(crate) fn abort_worker(e: &ms_core::Error) -> ! {
    ms_core::critical!("{}", worker_error_text(e));
    ms_core::log::flush();
    std::process::abort();
}

/// The `catch (SyncException &)` arm of the worker loops: aborts on a non-retryable error,
/// reports a connection error when offline, and logs `--sleeping`. The caller sleeps.
pub(crate) fn report_worker_error(ctx: &WorkerContext, e: &ms_core::Error) {
    if !e.is_retryable() {
        abort_worker(e);
    }
    tracing::error!("{}", worker_error_text(e));
    if e.is_offline() {
        ctx.deltas.begin_connection_error(ctx.account.id());
    }
    tracing::info!("{}", ms_core::logmsg::sleeping());
}

/// `logCurrentExceptionWithStackTrace`, without the stack.
fn worker_error_text(e: &ms_core::Error) -> String {
    match e {
        ms_core::Error::Sync(s) => format!(
            "An exception occurred during program execution: {}",
            s.to_json()
        ),
        other => format!("An unexpected error occurred during program execution: {other}"),
    }
}
