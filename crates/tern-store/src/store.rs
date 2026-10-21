// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `MailStore` and `MailStoreTransaction`. Derived from Mailspring-Sync
//! `MailSync/MailStore.*` and `MailStoreTransaction.*`, Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! The placement helpers are in [`crate::placement`] and the thread search and reference
//! writes in [`crate::search`]; both are `impl Store` blocks.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{params, params_from_iter, Connection};
use serde_json::Value;
use tern_core::constants::{
    DELTA_TYPE_PERSIST, DELTA_TYPE_UNPERSIST, FIND_LARGE_SET_CHUNK, STREAM_DELAY_WORKER_MS,
};
use tern_core::{Clock, Metadata, Result};

use crate::delta::{DeltaItem, DeltaStream};
use crate::models::generic::GenericClass;
use crate::models::{Contact, Folder, GenericModel, Label, Message, Thread};
use crate::{migrate, schema, Model, Query};

/// Bumped by every Label (and Folder) save or remove through any store in the process, so
/// each thread's cache reloads after another thread changed the categories.
static LABELS_VERSION: AtomicU64 = AtomicU64::new(1);
static FOLDERS_VERSION: AtomicU64 = AtomicU64::new(1);

/// Transactions waiting longer than this for the write lock, or holding it longer than
/// [`SLOW_TRANSACTION`], are logged.
const BUSY_TRANSACTION: Duration = Duration::from_millis(1000);
const SLOW_TRANSACTION: Duration = Duration::from_millis(100);

/// Rusqlite's default of 16 cached statements is fewer than one sync pass uses.
const STATEMENT_CACHE_CAPACITY: usize = 128;

#[derive(Default)]
struct TxState {
    depth: u32,
    deltas: Vec<DeltaItem>,
}

type FolderMap = HashMap<String, Rc<Folder>>;

struct CategoryCache<T> {
    version: u64,
    account_id: String,
    value: Rc<T>,
}

/// One thread's view of `tern.db`. Not `Send`: each thread opens its own, which is
/// what `MailStore::assertCorrectThread` checked at runtime.
pub struct Store {
    conn: Connection,
    deltas: Arc<DeltaStream>,
    clock: RefCell<Arc<dyn Clock>>,
    stream_delay_ms: Cell<u64>,
    tx: RefCell<TxState>,
    label_cache: RefCell<Option<CategoryCache<Vec<Label>>>>,
    folder_cache: RefCell<Option<CategoryCache<FolderMap>>>,
    _not_send: PhantomData<*const ()>,
}

impl Store {
    /// Opens `$CONFIG_DIR_PATH/tern.db` with the engine's connection settings and points
    /// SQLite's temp files at the config dir, which `main.cpp` did for every mode before
    /// dispatch. The schema must already exist (`migrate` runs first, in its own process).
    pub fn open(config_dir: &Path, deltas: Arc<DeltaStream>) -> Result<Store> {
        let conn = schema::open_connection(config_dir)?;
        schema::set_temp_directory(&conn, config_dir)?;
        Ok(Store::from_connection(conn, deltas))
    }

    pub fn from_connection(conn: Connection, deltas: Arc<DeltaStream>) -> Store {
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);
        Store {
            conn,
            deltas,
            clock: RefCell::new(tern_core::clock::system()),
            stream_delay_ms: Cell::new(STREAM_DELAY_WORKER_MS),
            tx: RefCell::new(TxState::default()),
            label_cache: RefCell::new(None),
            folder_cache: RefCell::new(None),
            _not_send: PhantomData,
        }
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn delta_stream(&self) -> &Arc<DeltaStream> {
        &self.deltas
    }

    /// The clock behind `MessageOrphan.since`; the system clock unless a test sets one.
    pub fn set_clock(&self, clock: Arc<dyn Clock>) {
        *self.clock.borrow_mut() = clock;
    }

    pub fn now(&self) -> i64 {
        self.clock.borrow().now()
    }

    /// The longest a delta from this store waits before it is flushed: 500 ms for workers
    /// (the default), 5 ms for the main thread.
    pub fn set_stream_delay(&self, ms: u64) {
        self.stream_delay_ms.set(ms);
    }

    /// The `_State` value for `key`, `None` when unset.
    pub fn get_key_value(&self, key: &str) -> Result<Option<String>> {
        migrate::get_key_value(&self.conn, key)
    }

    pub fn save_key_value(&self, key: &str, value: &str) -> Result<()> {
        migrate::save_key_value(&self.conn, key, value)
    }

    // -- Transactions -------------------------------------------------------------------

    /// Runs `f` inside `BEGIN IMMEDIATE … COMMIT` and emits the deltas it produced after the
    /// commit, in one batch. An error rolls back and drops the deltas. A nested call runs in
    /// a savepoint of the outer transaction: its error rolls back its own writes and deltas
    /// only, so a caller can recover from it (a constraint race on insert, say) and commit
    /// the rest. `name` is for the slow-transaction log lines.
    pub fn transaction<T>(&self, name: &str, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        let depth = self.tx.borrow().depth;
        if depth > 0 {
            return self.savepoint(depth, f);
        }

        let start = Instant::now();
        self.conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
        let began = Instant::now();
        self.tx.borrow_mut().depth = 1;
        let result = f(self);
        self.tx.borrow_mut().depth = 0;

        let value = match result {
            Ok(v) => v,
            Err(e) => {
                self.rollback();
                return Err(e);
            }
        };
        if let Err(e) = self.conn.execute_batch("COMMIT") {
            self.rollback();
            return Err(e.into());
        }
        let deltas = std::mem::take(&mut self.tx.borrow_mut().deltas);
        if !deltas.is_empty() {
            self.deltas.emit(deltas, self.stream_delay_ms.get());
        }

        let waited = began - start;
        let held = began.elapsed();
        if waited > BUSY_TRANSACTION {
            tracing::warn!(
                "[BUSY] Transaction={name} waited {}ms to acquire write lock",
                waited.as_millis()
            );
        }
        if held > SLOW_TRANSACTION {
            tracing::warn!(
                "[SLOW] Transaction={name} held write lock for {}ms",
                held.as_millis()
            );
        }
        Ok(value)
    }

    fn savepoint<T>(&self, depth: u32, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        let sp = format!("ms_store_{depth}");
        self.conn.execute_batch(&format!("SAVEPOINT {sp}"))?;
        let mark = self.tx.borrow().deltas.len();
        self.tx.borrow_mut().depth = depth + 1;
        let result = f(self);
        self.tx.borrow_mut().depth = depth;
        match result {
            Ok(v) => {
                self.conn.execute_batch(&format!("RELEASE {sp}"))?;
                Ok(v)
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch(&format!("ROLLBACK TO {sp}; RELEASE {sp}"));
                self.tx.borrow_mut().deltas.truncate(mark);
                Err(e)
            }
        }
    }

    /// Deltas describe writes that never happened; left in place they would go out with the
    /// next commit. Cached statements are dropped too, as `rollbackTransaction` did.
    fn rollback(&self) {
        let _ = self.conn.execute_batch("ROLLBACK");
        self.tx.borrow_mut().deltas.clear();
        self.conn.flush_prepared_statement_cache();
        self.invalidate_category_caches();
    }

    /// After a panic caught inside [`Store::transaction`]: rolls the open transaction back
    /// and drops its deltas, so the store can be used again.
    pub fn abandon_after_panic(&self) {
        if self.tx.borrow().depth > 0 {
            self.rollback();
            self.tx.borrow_mut().depth = 0;
        }
    }

    pub fn in_transaction(&self) -> bool {
        self.tx.borrow().depth > 0
    }

    /// Queues a delta: held until commit inside a transaction, otherwise sent with this
    /// store's delay.
    pub fn emit(&self, item: DeltaItem) {
        let mut tx = self.tx.borrow_mut();
        if tx.depth > 0 {
            tx.deltas.push(item);
        } else {
            drop(tx);
            self.deltas.emit(vec![item], self.stream_delay_ms.get());
        }
    }

    /// Drops the deltas of the open transaction (`unsafeEraseTransactionDeltas`). Only for
    /// writes that change engine-internal fields the client never reads; otherwise the
    /// client falls out of step.
    pub fn unsafe_erase_transaction_deltas(&self) {
        self.tx.borrow_mut().deltas.clear();
    }

    /// `--mode reset`: deletes everything of one account (`ACCOUNT_RESET_QUERIES`), resets
    /// its metadata cursor so metadata is fetched again on resync, and vacuums. Its plugin
    /// metadata is kept by Message-ID for the resync ([`crate::metadata_stash`]) when
    /// `keep_metadata`, and deleted otherwise (the account is being removed).
    pub fn reset_for_account(&self, account_id: &str, keep_metadata: bool) -> Result<()> {
        migrate::reset_account(&self.conn, account_id, keep_metadata)
    }

    /// Frees some unused pages and truncates the WAL ([`crate::schema::compact`]); the
    /// background worker calls it after its hourly cache cleanup.
    pub fn compact(&self) {
        crate::schema::compact(&self.conn);
    }

    // -- Models -------------------------------------------------------------------------

    /// Inserts (at version 1) or updates `model`, runs its hooks and emits a `persist`
    /// delta. The version is bumped first, so a model at version 0 is new.
    pub fn save<M: Model>(&self, model: &mut M) -> Result<()> {
        model.increment_version();
        model.before_save(self)?;
        model.ensure_cls();

        let sql = if model.version() > 1 {
            let pairs: Vec<String> = M::columns()
                .iter()
                .filter(|c| **c != "id")
                .map(|c| format!("{c} = :{c}"))
                .collect();
            format!("UPDATE {} SET {} WHERE id = :id", M::TABLE, pairs.join(","))
        } else {
            let cols = M::columns().join(",");
            let values: Vec<String> = M::columns().iter().map(|c| format!(":{c}")).collect();
            format!(
                "INSERT INTO {} ({cols}) VALUES ({})",
                M::TABLE,
                values.join(",")
            )
        };
        {
            let mut stmt = self.conn.prepare_cached(&sql)?;
            model.bind(&mut stmt)?;
            stmt.raw_execute()?;
        }

        model.after_save(self)?;
        self.bump_category_versions(M::TABLE);
        self.emit(DeltaItem::for_model(DELTA_TYPE_PERSIST, model));
        Ok(())
    }

    /// Deletes `model`, runs its hooks and emits an `unpersist` delta with its full JSON.
    pub fn remove<M: Model>(&self, model: &mut M) -> Result<()> {
        self.conn
            .prepare_cached(&format!("DELETE FROM {} WHERE id = ?", M::TABLE))?
            .execute([model.id()])?;
        model.after_remove(self)?;
        self.bump_category_versions(M::TABLE);
        self.emit(DeltaItem::for_model(DELTA_TYPE_UNPERSIST, model));
        Ok(())
    }

    /// `remove<T>(Query)`: deletes every match with one statement and emits them as one
    /// `unpersist`; with no matches the delta has `modelClass:"none"`. Like the C++, this
    /// runs no `after_remove` hooks, so it is only for models without derived rows.
    pub fn remove_all<M: Model>(&self, query: &Query) -> Result<()> {
        let models = self.find_all::<M>(query)?;
        let (where_sql, params) = query.where_sql();
        self.conn
            .prepare(&format!("DELETE FROM {}{where_sql}", M::TABLE))?
            .execute(params_from_iter(params))?;
        self.emit(DeltaItem::for_models(DELTA_TYPE_UNPERSIST, &models));
        Ok(())
    }

    fn bump_category_versions(&self, table: &str) {
        if table == Label::TABLE {
            LABELS_VERSION.fetch_add(1, Ordering::SeqCst);
        }
        if table == Label::TABLE || table == Folder::TABLE {
            FOLDERS_VERSION.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The caches may hold rows a rollback removed.
    fn invalidate_category_caches(&self) {
        *self.label_cache.borrow_mut() = None;
        *self.folder_cache.borrow_mut() = None;
    }

    pub fn find<M: Model>(&self, query: &Query) -> Result<Option<M>> {
        let (where_sql, params) = query.where_sql();
        let sql = format!(
            "SELECT data FROM {}{where_sql}{} LIMIT 1",
            M::TABLE,
            query.order_sql()
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut rows = stmt.query(params_from_iter(params))?;
        match rows.next()? {
            Some(row) => Ok(Some(M::from_json(parse_data(row)?)?)),
            None => Ok(None),
        }
    }

    pub fn find_all<M: Model>(&self, query: &Query) -> Result<Vec<M>> {
        let (tail, params) = query.to_sql();
        let sql = format!("SELECT data FROM {}{tail}", M::TABLE);
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut rows = stmt.query(params_from_iter(params))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(M::from_json(parse_data(row)?)?);
        }
        Ok(out)
    }

    /// [`Store::find_all`] keyed by a top-level JSON string field, e.g. `"id"`.
    pub fn find_all_map<M: Model>(&self, query: &Query, key: &str) -> Result<HashMap<String, M>> {
        Ok(self
            .find_all::<M>(query)?
            .into_iter()
            .map(|m| (m.json()[key].as_str().unwrap_or("").to_owned(), m))
            .collect())
    }

    /// `findLargeSet`: `col IN (set)` in chunks small enough for SQLite's parameter limit.
    pub fn find_large_set<M: Model>(&self, col: &str, set: &[String]) -> Result<Vec<M>> {
        let mut all = Vec::new();
        for chunk in set.chunks(FIND_LARGE_SET_CHUNK) {
            all.extend(self.find_all::<M>(&Query::new().equal_any(col, chunk.iter()))?);
        }
        Ok(all)
    }

    pub fn count<M: Model>(&self, query: &Query) -> Result<i64> {
        let (where_sql, params) = query.where_sql();
        let sql = format!("SELECT COUNT(*) FROM {}{where_sql}", M::TABLE);
        Ok(self
            .conn
            .query_row(&sql, params_from_iter(params), |r| r.get(0))?)
    }

    /// `findGeneric`: the model a metadata record names by class (`Message`, `Thread` or
    /// `Contact`, any case). Other classes are an error.
    pub fn find_generic(&self, model_class: &str, query: &Query) -> Result<Option<GenericModel>> {
        Ok(match GenericClass::parse(model_class)? {
            GenericClass::Message => self.find::<Message>(query)?.map(GenericModel::Message),
            GenericClass::Thread => self.find::<Thread>(query)?.map(GenericModel::Thread),
            GenericClass::Contact => self.find::<Contact>(query)?.map(GenericModel::Contact),
        })
    }

    pub fn find_all_generic(&self, model_class: &str, query: &Query) -> Result<Vec<GenericModel>> {
        Ok(match GenericClass::parse(model_class)? {
            GenericClass::Message => self
                .find_all::<Message>(query)?
                .into_iter()
                .map(GenericModel::Message)
                .collect(),
            GenericClass::Thread => self
                .find_all::<Thread>(query)?
                .into_iter()
                .map(GenericModel::Thread)
                .collect(),
            GenericClass::Contact => self
                .find_all::<Contact>(query)?
                .into_iter()
                .map(GenericModel::Contact)
                .collect(),
        })
    }

    // -- Folders and labels -------------------------------------------------------------

    /// All labels of an account, reloaded after any Label save or remove in the process.
    pub fn all_labels_cache(&self, account_id: &str) -> Result<Rc<Vec<Label>>> {
        let version = LABELS_VERSION.load(Ordering::SeqCst);
        if let Some(c) = self.label_cache.borrow().as_ref() {
            if c.version == version && c.account_id == account_id {
                return Ok(c.value.clone());
            }
        }
        let labels = Rc::new(self.find_all::<Label>(&Query::new().equal("accountId", account_id))?);
        *self.label_cache.borrow_mut() = Some(CategoryCache {
            version,
            account_id: account_id.to_owned(),
            value: labels.clone(),
        });
        Ok(labels)
    }

    /// Folders and Labels of the account keyed by id, reloaded after any Folder or Label is
    /// saved or removed through any store in this process. Message JSON carries only folder
    /// ids, so roles and paths are resolved here. Labels are included because Gmail's send
    /// path files a message under the Sent label.
    pub fn all_folders_cache(&self, account_id: &str) -> Result<Rc<FolderMap>> {
        let version = FOLDERS_VERSION.load(Ordering::SeqCst);
        if let Some(c) = self.folder_cache.borrow().as_ref() {
            if c.version == version && c.account_id == account_id {
                return Ok(c.value.clone());
            }
        }
        let mut map = HashMap::new();
        for f in self.find_all::<Folder>(&Query::new().equal("accountId", account_id))? {
            map.insert(f.id().to_owned(), Rc::new(f));
        }
        for l in self.find_all::<Label>(&Query::new().equal("accountId", account_id))? {
            map.insert(l.id().to_owned(), Rc::new(Folder::from(l)));
        }
        let map = Rc::new(map);
        *self.folder_cache.borrow_mut() = Some(CategoryCache {
            version,
            account_id: account_id.to_owned(),
            value: map.clone(),
        });
        Ok(map)
    }

    /// The Folder or Label with `folder_id`, from [`Store::all_folders_cache`].
    pub fn folder_by_id(&self, account_id: &str, folder_id: &str) -> Result<Option<Rc<Folder>>> {
        Ok(self.all_folders_cache(account_id)?.get(folder_id).cloned())
    }

    /// Writes the `localStatus` keys this pass changed relative to `initial_local_status`
    /// over the stored row (`saveFolderStatus`), so keys another thread or the harness
    /// wrote meanwhile survive. Nothing is written when nothing changed, or when the folder
    /// is gone.
    pub fn save_folder_status(
        &self,
        folder: &mut Folder,
        initial_local_status: &Value,
    ) -> Result<()> {
        let changed = folder.local_status().clone();
        if &changed == initial_local_status {
            return Ok(());
        }
        self.transaction("saveFolderStatus", |store| {
            let query = Query::new()
                .equal("accountId", folder.account_id())
                .equal("id", folder.id());
            let Some(mut current) = store.find::<Folder>(&query)? else {
                return Ok(());
            };
            if let Some(obj) = changed.as_object() {
                let status = current.local_status_mut();
                for (k, v) in obj {
                    if initial_local_status.get(k) != Some(v) {
                        status.insert(k.clone(), v.clone());
                    }
                }
            }
            store.save(&mut current)
        })
    }

    // -- Metadata -----------------------------------------------------------------------

    /// Metadata that arrived before its model, removed as it is returned.
    pub fn find_and_delete_detached_plugin_metadata(
        &self,
        account_id: &str,
        object_id: &str,
    ) -> Result<Vec<Metadata>> {
        let mut out = Vec::new();
        {
            let mut stmt = self.conn.prepare_cached(
                "SELECT version, value, pluginId, objectType FROM DetatchedPluginMetadata WHERE objectId = ? AND accountId = ?",
            )?;
            let mut rows = stmt.query(params![object_id, account_id])?;
            while let Some(row) = rows.next()? {
                let value = match row.get_ref(1)? {
                    rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
                        serde_json::from_slice(b)?
                    }
                    _ => Value::Null,
                };
                out.push(Metadata {
                    account_id: account_id.to_owned(),
                    version: row.get(0)?,
                    value,
                    plugin_id: row.get(2)?,
                    object_type: row.get(3)?,
                    object_id: object_id.to_owned(),
                });
            }
        }
        if !out.is_empty() {
            self.conn
                .prepare_cached(
                    "DELETE FROM DetatchedPluginMetadata WHERE objectId = ? AND accountId = ?",
                )?
                .execute(params![object_id, account_id])?;
        }
        Ok(out)
    }

    pub fn save_detached_plugin_metadata(&self, m: &Metadata) -> Result<()> {
        self.conn
            .prepare_cached(
                "REPLACE INTO DetatchedPluginMetadata (objectId, objectType, accountId, pluginId, value, version) VALUES (?,?,?,?,?,?)",
            )?
            .execute(params![
                m.object_id,
                m.object_type,
                m.account_id,
                m.plugin_id,
                serde_json::to_string(&m.value)?,
                m.version
            ])?;
        Ok(())
    }
}

fn parse_data(row: &rusqlite::Row<'_>) -> Result<Value> {
    // `data` is TEXT for most tables and BLOB for some (File, Event, Contact, Task).
    let raw = row.get_ref(0)?;
    let bytes = match raw {
        rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => b,
        _ => b"null",
    };
    Ok(serde_json::from_slice(bytes)?)
}
