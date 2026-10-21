// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The FTS row of an Event or Contact, found by rowid. `content_id` is UNINDEXED in
//! `EventSearch` and `ContactSearch`, so a `WHERE content_id = ?` scans the whole table; the
//! rowid is kept on the model as `searchRowId`, as Thread keeps its `ThreadSearch` one.
//! A row written before that has none, and is found by `content_id` once, on its next save.
//! A new model never is: `Store::save` inserts it, so no row of its id can exist yet.

use std::cell::Cell;

use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use tern_core::Result;

use crate::model::json_i64;
use crate::Store;

const KEY: &str = "searchRowId";

thread_local! {
    static CONTENT_ID_LOOKUPS: Cell<u64> = const { Cell::new(0) };
}

/// How many `WHERE content_id = ?` scans [`assign`] has run on this thread, for tests.
#[doc(hidden)]
pub fn content_id_lookups() -> u64 {
    CONTENT_ID_LOOKUPS.with(Cell::get)
}

/// The rowid `data`'s search row has or will get, recorded on `data` before it is written.
/// A `searchRowId` that names another model's row (JSON copied from elsewhere) is not
/// trusted. `is_new`: the model is about to be inserted (version 1).
pub(crate) fn assign(
    store: &Store,
    table: &str,
    id: &str,
    is_new: bool,
    data: &mut Value,
) -> Result<i64> {
    let conn = store.conn();
    let recorded = json_i64(&data[KEY]);
    if recorded > 0 {
        let owner: Option<String> = conn
            .prepare_cached(&format!("SELECT content_id FROM {table} WHERE rowid = ?"))?
            .query_row([recorded], |r| r.get(0))
            .optional()?;
        if owner.is_none() || owner.as_deref() == Some(id) {
            return Ok(recorded);
        }
    }
    let existing: Option<i64> = if is_new {
        None
    } else {
        CONTENT_ID_LOOKUPS.with(|n| n.set(n.get() + 1));
        conn.prepare_cached(&format!("SELECT rowid FROM {table} WHERE content_id = ?"))?
            .query_row([id], |r| r.get(0))
            .optional()?
    };
    let rowid = match existing {
        Some(r) => r,
        None => conn
            .prepare_cached(&format!("SELECT COALESCE(MAX(rowid), 0) + 1 FROM {table}"))?
            .query_row([], |r| r.get(0))?,
    };
    data[KEY] = json!(rowid);
    Ok(rowid)
}

/// Writes the search row at `rowid`: `columns` set to `values`, inserted if it is not there.
pub(crate) fn write(
    store: &Store,
    table: &str,
    rowid: i64,
    id: &str,
    columns: &[&str],
    values: &[&dyn rusqlite::ToSql],
) -> Result<()> {
    let conn = store.conn();
    let sets: Vec<String> = columns.iter().map(|c| format!("{c} = ?")).collect();
    let mut params: Vec<&dyn rusqlite::ToSql> = values.to_vec();
    params.push(&rowid);
    let changed = conn
        .prepare_cached(&format!(
            "UPDATE {table} SET {} WHERE rowid = ?",
            sets.join(", ")
        ))?
        .execute(params.as_slice())?;
    if changed == 0 {
        let marks = vec!["?"; columns.len() + 2].join(", ");
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&rowid, &id];
        params.extend_from_slice(values);
        conn.prepare_cached(&format!(
            "INSERT INTO {table} (rowid, content_id, {}) VALUES ({marks})",
            columns.join(", ")
        ))?
        .execute(params.as_slice())?;
    }
    Ok(())
}

/// Deletes the search row of a removed model.
pub(crate) fn remove(store: &Store, table: &str, id: &str, data: &Value) -> Result<()> {
    let conn = store.conn();
    let recorded = json_i64(&data[KEY]);
    if recorded > 0 {
        let n = conn
            .prepare_cached(&format!(
                "DELETE FROM {table} WHERE rowid = ? AND content_id = ?"
            ))?
            .execute(rusqlite::params![recorded, id])?;
        if n > 0 {
            return Ok(());
        }
    }
    conn.prepare_cached(&format!("DELETE FROM {table} WHERE content_id = ?"))?
        .execute([id])?;
    Ok(())
}
