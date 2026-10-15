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
pub mod foreground;
pub mod mail_processor;
pub mod mailbox_model;
pub mod metadata_expiration;
pub mod metadata_worker;
pub mod tasks;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use ms_core::{Account, Clock, Identity};
use ms_net::{MetadataBackend, TokenManager};
use ms_store::DeltaStream;

pub use foreground::ForegroundControl;
pub use mailbox_model::MailboxModel;
pub use tasks::{RemoteTaskRunner, TaskRunnerFactory};

/// What every worker thread is given. Cheap to clone; everything shared is behind an `Arc`.
#[derive(Clone)]
pub struct WorkerContext {
    pub account: Arc<Account>,
    pub identity: Option<Arc<Identity>>,
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
