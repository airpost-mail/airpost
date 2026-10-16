// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `metadataExpiration` thread: emits `metadata-expiration` deltas when snoozes,
//! send-later and reminders come due. The client acts on the delta (unsnoozes, sends) and
//! clears the metadata; until it does, every scan sends the model again.
//!
//! Derived from Mailspring-Sync `MailSync/MetadataExpirationWorker.cpp` (Copyright © 2017
//! Foundry 376, GPL-3.0). See NOTICE.md.
//!
//! The rows come from `ModelPluginMetadata`, which `MailModel::afterSave` keeps
//! (`ms_store::model::metadata_after_save`); a save with an earlier expiration than the
//! worker's wake time wakes it through [`ms_core::wake::expiration_signal`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use ms_core::constants::{
    DELTA_TYPE_METADATA_EXPIRATION, METADATA_EXPIRATION_CHUNK, METADATA_EXPIRATION_EMIT_DELAY_MS,
    METADATA_EXPIRATION_IDLE, METADATA_EXPIRATION_MIN_DELAY_AFTER_EMIT,
    METADATA_EXPIRATION_MIN_DELAY_IDLE, METADATA_EXPIRATION_START_DELAY,
};
use ms_core::{Clock, Result};
use ms_store::{DeltaItem, DeltaStream, Query, Store};
use tracing::{error, info};

use crate::WorkerContext;

/// The thread body (`MetadataExpirationWorker::run`). Returns when the account stops.
pub fn run_metadata_expiration(ctx: WorkerContext) {
    let signal = ctx.expiration_signal().clone();
    // Plugins may take longer than the main application to load.
    if ctx.pause(Duration::from_secs(METADATA_EXPIRATION_START_DELAY)) {
        return;
    }
    let store = match Store::open(&ctx.config_dir, ctx.deltas.clone()) {
        Ok(s) => s,
        Err(e) => return ctx.fail_with(&e),
    };
    let worker = MetadataExpirationWorker::new(
        ctx.account.id(),
        &store,
        ctx.deltas.clone(),
        ctx.clock.clone(),
    );
    while !ctx.is_stopping() {
        let next = match worker.scan() {
            Ok(next) => next,
            Err(e) => return ctx.fail_with(&e),
        };
        let clock = ctx.clock.clone();
        signal.wait_until(next, &|| clock.now());
        // Not at exactly the requested time, in case the client's clock differs slightly;
        // and a wake comes from inside the saving transaction, which has to commit before
        // the scan can see the new expiration.
        if ctx.pause(Duration::from_secs(1)) {
            return;
        }
    }
}

/// One account's scan: the half of `MetadataExpirationWorker::run` that touches the store.
pub struct MetadataExpirationWorker<'a> {
    account_id: String,
    store: &'a Store,
    deltas: Arc<DeltaStream>,
    clock: Arc<dyn Clock>,
    chunk_pause: Duration,
}

impl<'a> MetadataExpirationWorker<'a> {
    pub fn new(
        account_id: &str,
        store: &'a Store,
        deltas: Arc<DeltaStream>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        MetadataExpirationWorker {
            account_id: account_id.to_owned(),
            store,
            deltas,
            clock,
            chunk_pause: Duration::from_secs(1),
        }
    }

    /// The pause between chunks of 100 models of one class (1 s).
    pub fn with_chunk_pause(mut self, pause: Duration) -> Self {
        self.chunk_pause = pause;
        self
    }

    /// Emits a `metadata-expiration` delta for every model with metadata expiring at or
    /// before now, and returns the epoch second to wake at next: the next future
    /// expiration, else two hours on, and never sooner than 15 s after emitting (the client
    /// needs a moment to clear what it was sent) or 5 s otherwise (the undo-send window).
    pub fn scan(&self) -> Result<i64> {
        info!("Scanning for expired metadata");
        let now = self.clock.now();
        let due = self.due_by_class(now)?;
        for (class, ids) in &due {
            let chunks: Vec<&[String]> = ids.chunks(METADATA_EXPIRATION_CHUNK).collect();
            for (i, chunk) in chunks.iter().enumerate() {
                self.emit_chunk(class, chunk)?;
                if i + 1 < chunks.len() {
                    std::thread::sleep(self.chunk_pause);
                }
            }
        }

        let mut next = self
            .next_expiration_after(now)?
            .unwrap_or(now + METADATA_EXPIRATION_IDLE);
        let min_delay = if due.is_empty() {
            METADATA_EXPIRATION_MIN_DELAY_IDLE
        } else {
            METADATA_EXPIRATION_MIN_DELAY_AFTER_EMIT
        };
        next = next.max(now + min_delay);
        info!("-- Will wake for next expiration in {}sec", next - now);
        Ok(next)
    }

    /// Model ids with due metadata, by `objectType`. An id appears once per due plugin; the
    /// `IN` lookup collapses repeats within a chunk.
    fn due_by_class(&self, now: i64) -> Result<BTreeMap<String, Vec<String>>> {
        let mut stmt = self.store.conn().prepare_cached(
            "SELECT objectType, id FROM ModelPluginMetadata WHERE accountId = ? AND expiration <= ?",
        )?;
        let mut rows = stmt.query(rusqlite::params![self.account_id, now])?;
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let class: String = row.get(0)?;
            out.entry(class).or_default().push(row.get(1)?);
        }
        Ok(out)
    }

    fn emit_chunk(&self, class: &str, ids: &[String]) -> Result<()> {
        let query = Query::new().equal_any("id", ids.iter().map(String::as_str));
        let models = match self.store.find_all_generic(class, &query) {
            Ok(models) => models,
            // Only Message and Thread write rows, so this is a damaged row or model, which
            // must not hold up the snoozes of every other model.
            Err(e) if !matches!(e, ms_core::Error::Database(_)) => {
                error!("-X Could not load {class} models with expired metadata: {e}");
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        for model in models {
            info!("-- Sending expiration event for {class} {}", model.id());
            self.deltas.emit(
                vec![DeltaItem::new(
                    DELTA_TYPE_METADATA_EXPIRATION,
                    model.table(),
                    vec![model.delta_json()],
                )],
                METADATA_EXPIRATION_EMIT_DELAY_MS,
            );
        }
        Ok(())
    }

    fn next_expiration_after(&self, now: i64) -> Result<Option<i64>> {
        // Note: the C++ statement has no ORDER BY and takes the first row SQLite returns,
        // which is the earliest when the planner walks the expiration index; ordering makes
        // that the rule rather than a query-plan accident.
        let mut stmt = self.store.conn().prepare_cached(
            "SELECT expiration FROM ModelPluginMetadata WHERE accountId = ? AND expiration > ? ORDER BY expiration ASC LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![self.account_id, now])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }
}
