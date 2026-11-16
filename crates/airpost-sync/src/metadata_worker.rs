// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `metadata` thread: pulls plugin metadata from the backend and applies it to local
//! models. Not started for [`tern_net::LocalOnly`].
//!
//! Derived from Mailspring-Sync `MailSync/MetadataWorker.cpp` (Copyright © 2017 Foundry 376,
//! GPL-3.0). See NOTICE.md. The server transport is the backend's business; this thread
//! keeps the cursor and its binding to a server and user, applies what arrives and backs off
//! on errors.

use std::sync::Arc;

use serde_json::Value;
use tern_core::{Metadata, Result};
use tern_net::{MetadataBackend, MetadataDelta, MetadataPush};
use tern_store::{Message, Query, Store};
use tracing::{info, warn};

use crate::WorkerContext;

/// `backoffSeconds`: the wait after each consecutive retryable failure.
const BACKOFF_SECONDS: [u64; 10] = [3, 3, 5, 10, 20, 30, 60, 120, 300, 300];

/// The thread body (`MetadataWorker::run`). Returns at once for a local-only backend;
/// otherwise when the account stops.
pub fn run_metadata_worker(ctx: WorkerContext) {
    if ctx.metadata.is_local_only() {
        return;
    }
    let store = match Store::open(&ctx.config_dir, ctx.deltas.clone()) {
        Ok(s) => s,
        Err(e) => return ctx.fail_with(&e),
    };
    let worker = MetadataWorker::new(ctx.account.id(), &store, ctx.metadata.clone());
    let mut backoff_step = 0;
    while !ctx.is_stopping() {
        match worker.pull_until(&|| ctx.is_stopping()) {
            Ok(()) => backoff_step = 0,
            Err(e) if e.is_transient() => {
                let secs = BACKOFF_SECONDS[backoff_step];
                info!("Will retry in {secs} sec.");
                ctx.sleep_until_wake_or(secs);
                backoff_step = (backoff_step + 1).min(BACKOFF_SECONDS.len() - 1);
            }
            Err(e) => return ctx.fail_with(&e),
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

    fn binding_key(&self) -> String {
        format!("metadata-binding-{}", self.account_id)
    }

    /// The saved cursor; `None` before the first pull and after a binding change discards
    /// it, which makes the backend deliver everything again. `Store::reset_for_account`
    /// writes `"0"`, which the backend also treats as the start.
    pub fn cursor(&self) -> Result<Option<String>> {
        Ok(self
            .store
            .get_key_value(&self.cursor_key())?
            .filter(|c| !c.is_empty()))
    }

    fn set_cursor(&self, cursor: &str) -> Result<()> {
        self.store.save_key_value(&self.cursor_key(), cursor)
    }

    /// Checks the binding, drains the outbox, then pulls from the saved cursor until the
    /// backend closes the stream, applying each delta as it arrives and saving the cursor
    /// the backend hands back.
    pub fn pull_once(&self) -> Result<()> {
        self.pull_until(&|| false)
    }

    /// [`MetadataWorker::pull_once`], ending the stream early once `stop` is true.
    pub fn pull_until(&self, stop: &dyn Fn() -> bool) -> Result<()> {
        self.check_binding()?;
        let cursor = self.cursor()?;
        let mut failed = None;
        let resume = self.backend.pull(cursor.as_deref(), stop, &mut |delta| {
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

    /// A cursor is a position on one server for one user. A missing or different binding
    /// (first pull, a restored server, another user, or a cursor `import-legacy` carried from
    /// Mailspring) discards the cursor and re-sends local state as backfill. The binding is
    /// saved last, so a crash in between repeats the whole step.
    fn check_binding(&self) -> Result<()> {
        let Some(binding) = self.backend.binding()? else {
            return Ok(());
        };
        if self.store.get_key_value(&self.binding_key())?.as_deref() != Some(binding.as_str()) {
            self.store.save_key_value(&self.cursor_key(), "")?;
            let local = local_metadata(self.store, &self.account_id)?;
            self.backend.rebind(&local)?;
            self.store.save_key_value(&self.binding_key(), &binding)?;
        }
        self.backend.drain_outbox()
    }

    /// `onDelta`.
    fn on_delta(&self, delta: MetadataDelta) -> Result<()> {
        match delta {
            MetadataDelta::Metadata(m) => apply_metadata(self.store, &m),
            MetadataDelta::AnchoredThreadMetadata(m, anchor) => {
                apply_anchored_thread_metadata(self.store, &m, &anchor)
            }
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
/// (`tern_store::model::metadata_before_save`).
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

/// Thread metadata from another device, whose thread id may differ here: applied to the thread
/// that holds the anchor message, else to the thread of that id, else kept under the pending
/// key `import-legacy` uses, which the anchor message's first save attaches.
pub fn apply_anchored_thread_metadata(store: &Store, m: &Metadata, anchor: &str) -> Result<()> {
    let local_thread: Option<String> = store
        .conn()
        .prepare_cached(
            "SELECT threadId FROM Message WHERE headerMessageId = ? AND accountId = ? AND threadId != '' ORDER BY rowid LIMIT 1",
        )?
        .query_map(rusqlite::params![anchor, m.account_id], |r| r.get(0))?
        .next()
        .transpose()?;
    let exists = |id: &str| -> Result<bool> {
        Ok(store
            .find_generic(
                "thread",
                &Query::new()
                    .equal("id", id)
                    .equal("accountId", m.account_id.as_str()),
            )?
            .is_some())
    };
    let target = match local_thread {
        Some(t) => t,
        None if exists(&m.object_id)? => m.object_id.clone(),
        None => {
            info!(
                "Received metadata V{} for thread {} before its message <{anchor}>; waiting for it.",
                m.version, m.object_id
            );
            let pending = Metadata {
                object_id: tern_store::legacy_import::pending_thread_key(anchor, &m.object_id),
                object_type: "Thread".into(),
                ..m.clone()
            };
            return store.transaction("applyMetadataJSON", |s| {
                s.save_detached_plugin_metadata(&pending)
            });
        }
    };
    apply_metadata(
        store,
        &Metadata {
            object_id: target,
            ..m.clone()
        },
    )
}

/// Every metadata value the account holds, as pushes: what a new binding backfills. Thread
/// values carry their anchor.
pub fn local_metadata(store: &Store, account_id: &str) -> Result<Vec<MetadataPush>> {
    let rows: Vec<(String, String)> = store
        .conn()
        .prepare_cached(
            "SELECT DISTINCT id, objectType FROM ModelPluginMetadata WHERE accountId = ? ORDER BY objectType, id",
        )?
        .query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for (id, class) in rows {
        let query = Query::new()
            .equal("id", id.as_str())
            .equal("accountId", account_id);
        let Some(model) = store.find_generic(&class, &query)? else {
            continue;
        };
        let header_message_id = match class.to_ascii_lowercase().as_str() {
            "message" => store
                .find::<Message>(&Query::new().equal("id", id.as_str()))?
                .map(|m| m.header_message_id().to_owned())
                .filter(|h| !h.is_empty()),
            "thread" => thread_anchor(store, &id)?,
            _ => None,
        };
        let Some(entries) = model.json().get("metadata").and_then(Value::as_array) else {
            continue;
        };
        for e in entries {
            let value = e.get("value").cloned().unwrap_or(Value::Null);
            let Some(plugin_id) = e.get("pluginId").and_then(Value::as_str) else {
                continue;
            };
            out.push(MetadataPush {
                account_id: account_id.to_owned(),
                model_id: id.clone(),
                model_class: class.clone(),
                plugin_id: plugin_id.to_owned(),
                value,
                version: e.get("v").and_then(Value::as_i64).unwrap_or(0),
                header_message_id: header_message_id.clone(),
            });
        }
    }
    Ok(out)
}

/// The Message-ID of the message whose id forms `thread_id` (`t:<message id>`), if it is
/// still here.
pub fn thread_anchor(store: &Store, thread_id: &str) -> Result<Option<String>> {
    let Some(message_id) = thread_id.strip_prefix("t:") else {
        return Ok(None);
    };
    Ok(store
        .find::<Message>(&Query::new().equal("id", message_id))?
        .map(|m| m.header_message_id().to_owned())
        .filter(|h| !h.is_empty()))
}
