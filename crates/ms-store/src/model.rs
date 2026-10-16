// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The model trait (`MailModel`) and the pieces every model shares: the base columns,
//! `__cls`, plugin metadata and the `ModelPluginMetadata` / `DetatchedPluginMetadata`
//! bookkeeping of the base save hooks.
//!
//! Derived from Mailspring-Sync `MailSync/Models/MailModel.*`, Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! Models are their JSON, as in the C++: `data` is what the client reads and writes, and it
//! carries keys the engine never looks at, which a save must preserve. Typed accessors read
//! and write that JSON. This replaces the plan's `Serialize + DeserializeOwned` bound, which
//! would drop unknown keys on a round trip.

use std::collections::BTreeMap;

use ms_core::{Error, Result};
use rusqlite::types::ToSql;
use rusqlite::Statement;
use serde_json::{json, Map, Value};

use crate::Store;

pub trait Model: Sized {
    /// The table name, which is also the delta `modelClass` and the default `__cls`.
    const TABLE: &'static str;

    /// Every column a save writes, `id` and `data` included (`columnsForQuery`). The insert
    /// and update statements bind them by name as `:<column>`.
    fn columns() -> &'static [&'static str];

    fn from_json(data: Value) -> Result<Self>;
    fn json(&self) -> &Value;
    fn json_mut(&mut self) -> &mut Value;

    /// Binds every column in [`Model::columns`] as `:<column>`.
    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()>;

    fn id(&self) -> &str {
        self.json()["id"].as_str().unwrap_or("")
    }

    fn account_id(&self) -> &str {
        self.json()["aid"].as_str().unwrap_or("")
    }

    fn version(&self) -> i64 {
        json_i64(&self.json()["v"])
    }

    fn increment_version(&mut self) {
        let v = self.version() + 1;
        self.json_mut()["v"] = Value::from(v);
    }

    /// Whether the model carries plugin metadata (`supportsMetadata`): Message and Thread.
    fn supports_metadata() -> bool {
        false
    }

    /// Sets `__cls` to the table name unless the JSON already names a class (a Task's
    /// `__cls` is its task type). `MailModel::toJSON` did this on every serialisation.
    fn ensure_cls(&mut self) {
        if let Some(obj) = self.json_mut().as_object_mut() {
            if !obj.contains_key("__cls") {
                obj.insert("__cls".into(), Value::from(Self::TABLE));
            }
        }
    }

    /// `toJSON`: the JSON with `__cls` filled in.
    fn to_json(&self) -> Value {
        let mut v = self.json().clone();
        if let Some(obj) = v.as_object_mut() {
            obj.entry("__cls")
                .or_insert_with(|| Value::from(Self::TABLE));
        }
        v
    }

    /// The JSON sent in deltas (`toJSONDispatch`); Message adds body and sync flags.
    fn delta_json(&self) -> Value {
        self.to_json()
    }

    /// `upsertMetadata`: sets the value of one plugin's metadata. With `version` the write
    /// is skipped (`None`) unless it is newer than what the model holds; without, the
    /// version is bumped. Returns the version written.
    fn upsert_metadata(
        &mut self,
        plugin_id: &str,
        value: &Value,
        version: Option<i64>,
    ) -> Option<i64> {
        upsert_metadata(self.json_mut(), plugin_id, value, version)
    }

    fn before_save(&mut self, _store: &Store) -> Result<()> {
        Ok(())
    }

    fn after_save(&mut self, _store: &Store) -> Result<()> {
        Ok(())
    }

    fn after_remove(&mut self, _store: &Store) -> Result<()> {
        Ok(())
    }
}

/// A JSON number as i64, accepting the floats a JS client writes for integral values.
pub fn json_i64(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| u as i64))
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Value::Bool(b) => *b as i64,
        _ => 0,
    }
}

pub fn json_str<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn json_bool(data: &Value, key: &str) -> bool {
    match data.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(v @ Value::Number(_)) => json_i64(v) != 0,
        _ => false,
    }
}

/// A fresh model JSON, `{id, aid, v}` (`MailModel(id, accountId, version)`).
pub fn base_json(id: &str, account_id: &str, version: i64) -> Value {
    json!({ "id": id, "aid": account_id, "v": version })
}

pub(crate) fn object_or_err(data: &Value, table: &str) -> Result<()> {
    if data.is_object() {
        Ok(())
    } else {
        Err(Error::other(format!("{table} JSON is not an object")))
    }
}

/// Binds one named parameter, failing like SQLiteCpp when the statement has no such name.
pub fn bind_named(stmt: &mut Statement<'_>, name: &str, value: impl ToSql) -> Result<()> {
    match stmt.parameter_index(name)? {
        Some(idx) => {
            stmt.raw_bind_parameter(idx, value)?;
            Ok(())
        }
        None => Err(Error::other(format!("no SQL parameter named {name}"))),
    }
}

/// `MailModel::bindToQuery`: `:id`, `:data`, `:accountId`, `:version`.
pub fn bind_base<M: Model>(model: &M, stmt: &mut Statement<'_>) -> Result<()> {
    bind_named(stmt, ":id", model.id())?;
    bind_named(stmt, ":data", serde_json::to_string(&model.to_json())?)?;
    bind_named(stmt, ":accountId", model.account_id())?;
    bind_named(stmt, ":version", model.version())?;
    Ok(())
}

/// Plugin ids and versions of a model's `metadata` array, for the change check in
/// [`metadata_after_save`].
pub type MetadataVersions = BTreeMap<String, i64>;

pub fn metadata_versions(data: &Value) -> MetadataVersions {
    let mut out = MetadataVersions::new();
    if let Some(arr) = data.get("metadata").and_then(Value::as_array) {
        for m in arr {
            out.insert(json_str(m, "pluginId").to_owned(), json_i64(&m["v"]));
        }
    }
    out
}

fn upsert_metadata(
    data: &mut Value,
    plugin_id: &str,
    value: &Value,
    version: Option<i64>,
) -> Option<i64> {
    let obj = data.as_object_mut()?;
    let list = obj
        .entry("metadata")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    let arr = list.as_array_mut().expect("metadata is an array");
    for m in arr.iter_mut() {
        if json_str(m, "pluginId") == plugin_id {
            let current = json_i64(&m["v"]);
            if let Some(v) = version {
                if current >= v {
                    return None;
                }
            }
            let next = version.unwrap_or(current + 1);
            m["value"] = value.clone();
            m["v"] = Value::from(next);
            return Some(next);
        }
    }
    let next = version.unwrap_or(1);
    let mut entry = Map::new();
    entry.insert("pluginId".into(), Value::from(plugin_id));
    entry.insert("value".into(), value.clone());
    entry.insert("v".into(), Value::from(next));
    arr.push(Value::Object(entry));
    Some(next)
}

/// `MailModel::beforeSave` for metadata models: on the first save, attach metadata that
/// arrived before the model did.
pub fn metadata_before_save<M: Model>(model: &mut M, store: &Store) -> Result<()> {
    if model.version() != 1 || !M::supports_metadata() {
        return Ok(());
    }
    let pending = store.find_and_delete_detached_plugin_metadata(model.account_id(), model.id())?;
    for m in pending {
        tracing::info!("-- Attaching waiting metadata for {}", m.plugin_id);
        model.upsert_metadata(&m.plugin_id, &m.value, Some(m.version));
    }
    Ok(())
}

/// `MailModel::afterSave` for metadata models: rewrites the model's `ModelPluginMetadata`
/// rows when any plugin's version changed since `initial`, and wakes the expiration worker
/// when the earliest `expiration` among the values is before the time it sleeps until. Metadata with an empty value
/// has no row, which is how a plugin "removes" metadata while its version keeps counting.
pub fn metadata_after_save<M: Model>(
    model: &M,
    initial: &MetadataVersions,
    store: &Store,
) -> Result<()> {
    if !M::supports_metadata() {
        return Ok(());
    }
    let current = metadata_versions(model.json());
    if &current == initial {
        return Ok(());
    }
    let conn = store.conn();
    conn.prepare_cached("DELETE FROM ModelPluginMetadata WHERE id = ?")?
        .execute([model.id()])?;
    let mut insert = conn.prepare_cached(
        "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) VALUES (?,?,?,?, ?)",
    )?;
    let mut lowest: Option<i64> = None;
    if let Some(arr) = model.json().get("metadata").and_then(Value::as_array) {
        for m in arr {
            let value = &m["value"];
            if json_size(value) == 0 {
                continue;
            }
            let expiration = value
                .get("expiration")
                .filter(|e| e.is_number())
                .map(json_i64);
            if let Some(e) = expiration {
                lowest = Some(lowest.map_or(e, |l| l.min(e)));
            }
            insert.execute(rusqlite::params![
                model.id(),
                model.account_id(),
                M::TABLE,
                json_str(m, "pluginId"),
                expiration,
            ])?;
        }
    }
    if let Some(e) = lowest {
        ms_core::wake::expiration_signal(model.account_id()).schedule(e);
    }
    Ok(())
}

/// `MailModel::afterRemove` for metadata models.
pub fn metadata_after_remove<M: Model>(model: &M, store: &Store) -> Result<()> {
    if !M::supports_metadata() {
        return Ok(());
    }
    store
        .conn()
        .prepare_cached("DELETE FROM ModelPluginMetadata WHERE id = ?")?
        .execute([model.id()])?;
    Ok(())
}

/// nlohmann's `size()`: elements of arrays and objects, 0 for null, 1 for any scalar.
/// nlohmann's `size()`: 0 for null, the length of a container, 1 for any scalar.
pub(crate) fn json_size(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::Array(a) => a.len(),
        Value::Object(o) => o.len(),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_metadata_versions() {
        let mut data = json!({"id": "a"});
        assert_eq!(
            upsert_metadata(&mut data, "p", &json!({"x": 1}), None),
            Some(1)
        );
        assert_eq!(
            upsert_metadata(&mut data, "p", &json!({"x": 2}), None),
            Some(2)
        );
        assert_eq!(
            upsert_metadata(&mut data, "p", &json!({"x": 3}), Some(2)),
            None
        );
        assert_eq!(
            upsert_metadata(&mut data, "p", &json!({"x": 3}), Some(5)),
            Some(5)
        );
        assert_eq!(
            upsert_metadata(&mut data, "q", &json!({}), Some(3)),
            Some(3)
        );
        assert_eq!(data["metadata"][0]["value"]["x"], 3);
        assert_eq!(metadata_versions(&data).get("q"), Some(&3));
    }
}
