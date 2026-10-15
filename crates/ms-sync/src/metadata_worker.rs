//! The `metadata` thread: pulls plugin metadata from the backend and applies it to local
//! models. Not started for [`ms_net::LocalOnly`] (plan D3).
//!
//! Derived from Mailspring-Sync `MailSync/MetadataWorker.cpp` (Copyright © 2017 Foundry 376,
//! GPL-3.0). See NOTICE.md. The identity-server transport (`/metadata`, `/deltas/…/head`,
//! `/deltas/…/streaming`) is the backend's business; this thread keeps the cursor, applies
//! what arrives and backs off on errors.

use std::sync::Arc;

use ms_core::{Metadata, Result};
use ms_net::{MetadataBackend, MetadataDelta};
use ms_store::{Query, Store};
use tracing::{info, warn};

use crate::WorkerContext;

/// `backoffSeconds`: the wait after each consecutive retryable failure.
const BACKOFF_SECONDS: [u64; 10] = [3, 3, 5, 10, 20, 30, 60, 120, 300, 300];

/// The thread body (`MetadataWorker::run`). Returns at once for a local-only backend;
/// otherwise never returns.
pub fn run_metadata_worker(ctx: WorkerContext) {
    if ctx.metadata.is_local_only() {
        return;
    }
    let store = match Store::open(&ctx.config_dir, ctx.deltas.clone()) {
        Ok(s) => s,
        Err(e) => crate::abort_worker(&e),
    };
    let worker = MetadataWorker::new(ctx.account.id(), &store, ctx.metadata.clone());
    let mut backoff_step = 0;
    loop {
        match worker.pull_once() {
            Ok(()) => backoff_step = 0,
            Err(e) if e.is_retryable() => {
                let secs = BACKOFF_SECONDS[backoff_step];
                info!("Will retry in {secs} sec.");
                ms_core::wake::sleep_worker_until_wake_or(secs);
                backoff_step = (backoff_step + 1).min(BACKOFF_SECONDS.len() - 1);
            }
            Err(e) => crate::abort_worker(&e),
        }
    }
}

/// One account's pull, on the `metadata` thread's store.
pub struct MetadataWorker<'a> {
    account_id: String,
    store: &'a Store,
    backend: Arc<dyn MetadataBackend>,
}

impl<'a> MetadataWorker<'a> {
    pub fn new(account_id: &str, store: &'a Store, backend: Arc<dyn MetadataBackend>) -> Self {
        MetadataWorker {
            account_id: account_id.to_owned(),
            store,
            backend,
        }
    }

    fn cursor_key(&self) -> String {
        format!("cursor-{}", self.account_id)
    }

    /// The saved cursor; `None` before the first pull and after `Store::reset_for_account`
    /// clears it, which makes the backend deliver everything again.
    pub fn cursor(&self) -> Result<Option<String>> {
        Ok(self
            .store
            .get_key_value(&self.cursor_key())?
            .filter(|c| !c.is_empty()))
    }

    fn set_cursor(&self, cursor: &str) -> Result<()> {
        self.store.save_key_value(&self.cursor_key(), cursor)
    }

    /// One `pull` from the saved cursor until the backend closes the stream, applying each
    /// delta as it arrives and saving the cursor the backend hands back.
    pub fn pull_once(&self) -> Result<()> {
        let cursor = self.cursor()?;
        let mut failed = None;
        let resume = self.backend.pull(cursor.as_deref(), &mut |delta| {
            if failed.is_some() {
                return;
            }
            if let Err(e) = self.on_delta(delta) {
                failed = Some(e);
            }
        })?;
        if let Some(e) = failed {
            return Err(e);
        }
        self.set_cursor(&resume)
    }

    /// `onDelta`.
    fn on_delta(&self, delta: MetadataDelta) -> Result<()> {
        match delta {
            MetadataDelta::Metadata(m) => apply_metadata(self.store, &m),
            MetadataDelta::Cursor(c) => self.set_cursor(&c),
            MetadataDelta::IdentityRefreshNeeded => {
                self.store.delta_stream().send_identity_refresh_needed();
                Ok(())
            }
        }
    }
}

/// `applyMetadataJSON`: attaches `m` to its model when the model has an older version, or
/// keeps it in `DetatchedPluginMetadata`, where the model's first save picks it up
/// (`ms_store::model::metadata_before_save`).
pub fn apply_metadata(store: &Store, m: &Metadata) -> Result<()> {
    // `findGeneric` asserted on any other class; a backend must not be able to end the
    // process with one bad record.
    if !["message", "thread", "contact"].contains(&m.object_type.to_ascii_lowercase().as_str()) {
        warn!(
            "Received metadata V{} for unsupported type {} ({}), ignoring.",
            m.version, m.object_type, m.object_id
        );
        return Ok(());
    }
    store.transaction("applyMetadataJSON", |s| {
        let query = Query::new()
            .equal("id", m.object_id.as_str())
            .equal("accountId", m.account_id.as_str());
        let model = s.find_generic(&m.object_type, &query)?;
        info!(
            "Received metadata V{} for ({} - {})",
            m.version, m.object_type, m.object_id
        );
        match model {
            Some(mut model) => {
                if model
                    .upsert_metadata(&m.plugin_id, &m.value, Some(m.version))
                    .is_some_and(|v| v > 0)
                {
                    info!(" -- Saved on to local model.");
                    model.save(s)?;
                } else {
                    info!(" -- Ignored. Local model has >= version.");
                }
            }
            None => {
                info!(" -- Local model is not present. Saving to waiting table.");
                s.save_detached_plugin_metadata(m)?;
            }
        }
        Ok(())
    })
}
