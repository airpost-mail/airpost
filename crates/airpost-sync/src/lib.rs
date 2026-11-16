// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The sync workers.
//!
//! Derived from Mailspring-Sync (`SyncWorker.*`, `MailProcessor.*`, `MetadataWorker.*`,
//! `MetadataExpirationWorker.*`, `DAVWorker.*`, `GoogleContactsWorker.*` and the worker
//! loops in `main.cpp`), Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Each `run_*` function is the body of one named thread and returns only when
//! its account stops ([`runtime`]). Each opens its own [`tern_store::Store`] on its thread.

pub mod background;
pub mod cal_contacts;
pub mod dav;
pub mod drafts;
pub mod expiration_executor;
pub mod folder_identity;
pub mod foreground;
pub mod google_contacts;
pub mod mail_processor;
pub mod mailbox_model;
pub mod metadata_expiration;
pub mod metadata_worker;
pub mod pending_sends;
pub mod runtime;
pub mod tasks;
mod worker;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tern_core::wake::WakeSignal;
use tern_core::{Account, Clock};
use tern_net::{MetadataBackend, TokenManager};
use tern_store::DeltaStream;

pub use expiration_executor::EngineSettings;
pub use foreground::ForegroundControl;
pub use mailbox_model::MailboxModel;
pub use runtime::{AccountFailure, AccountRuntime, FailureHandler};
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
    /// Builds the task runner the foreground worker uses for remote phases (tern-tasks).
    pub tasks: Arc<dyn TaskRunnerFactory>,
    /// The account's worker sleeps; `wake-workers` ends them.
    pub wake: Arc<WakeSignal>,
    /// The account's stop flag, failure handler and threads ([`runtime`]).
    pub control: Arc<runtime::AccountControl>,
    /// `serve`'s settings store. With it the expiration thread runs the expiration executor
    /// ([`expiration_executor`]); without it (`sync` mode) it only emits
    /// `metadata-expiration` deltas.
    pub settings: Option<Arc<dyn EngineSettings>>,
}

impl WorkerContext {
    /// A context with the system clock, quiet protocol logs and fresh per-account state.
    pub fn new(
        account: Arc<Account>,
        config_dir: PathBuf,
        deltas: Arc<DeltaStream>,
        tokens: Arc<TokenManager>,
        metadata: Arc<dyn MetadataBackend>,
        tasks: Arc<dyn TaskRunnerFactory>,
        on_failure: FailureHandler,
    ) -> Self {
        let control = Arc::new(runtime::AccountControl::new(account.id(), on_failure));
        WorkerContext {
            account,
            config_dir,
            deltas,
            clock: tern_core::clock::system(),
            verbose: false,
            foreground: Arc::new(ForegroundControl::new()),
            background_mark_all: Arc::new(AtomicBool::new(false)),
            tokens,
            metadata,
            tasks,
            wake: Arc::new(WakeSignal::new()),
            control,
            settings: None,
        }
    }
}

/// The `catch (SyncException &)` arm of the worker loops: an error that isn't transient
/// ([`tern_core::Error::is_transient`]) stops the account and returns false; otherwise it
/// reports a connection error when offline, logs `--sleeping` and returns true, and the
/// caller sleeps.
pub(crate) fn report_worker_error(ctx: &WorkerContext, e: &tern_core::Error) -> bool {
    if !e.is_transient() {
        ctx.fail_with(e);
        return false;
    }
    tracing::error!("{}", worker_error_text(e));
    // A thread finishing a call after its account stopped must not report the account
    // offline: a later run of it would never clear that.
    if e.is_offline() && !ctx.is_stopping() {
        ctx.deltas.begin_connection_error(ctx.account.id());
    }
    tracing::info!("{}", tern_core::logmsg::sleeping());
    true
}

/// `logCurrentExceptionWithStackTrace`, without the stack.
fn worker_error_text(e: &tern_core::Error) -> String {
    match e {
        tern_core::Error::Sync(s) => format!(
            "An exception occurred during program execution: {}",
            s.to_json()
        ),
        other => format!("An unexpected error occurred during program execution: {other}"),
    }
}
