// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Plugin metadata kept across an account reset (`--mode reset`, `account.reset`) and a
//! database rebuild (`engine.resetDatabase`), docs/tech-debt.md TD32. With the local-only
//! metadata backend (D3) nothing replays it after a resync, so scheduled sends, snoozes and
//! reminders would be lost.
//!
//! Model ids don't survive a resync (a thread id is `t:<first message inserted>`), so each
//! value goes into `DetatchedPluginMetadata` under the keys `import-legacy` uses: message
//! metadata under the message's Message-ID, thread metadata under each of the thread's
//! Message-IDs and the old thread id. The first save of a message with that Message-ID
//! attaches them again (`legacy_import::attach_pending_message_metadata` and
//! `attach_pending_thread_metadata`).

use std::path::Path;

use ms_core::Result;
use rusqlite::{params, Connection};
use serde_json::Value;

use crate::legacy_import::{pending_message_key, pending_thread_key};
use crate::model::{json_i64, json_size, json_str};

/// One `DetatchedPluginMetadata` row; `value` is its JSON text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashedMetadata {
    pub object_id: String,
    pub object_type: String,
    pub account_id: String,
    pub plugin_id: String,
    pub value: String,
    pub version: i64,
}

/// The metadata of every message and thread of `account_id` (every account with `None`)
/// as pending rows. With `None` the rows already waiting in `DetatchedPluginMetadata` are
/// included, since the whole database is about to go.
pub fn collect(conn: &Connection, account_id: Option<&str>) -> Result<Vec<StashedMetadata>> {
    let mut out = Vec::new();
    collect_messages(conn, account_id, &mut out)?;
    collect_threads(conn, account_id, &mut out)?;
    if account_id.is_none() {
        let mut stmt = conn.prepare(
            "SELECT objectId, objectType, accountId, pluginId, value, version FROM DetatchedPluginMetadata",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(StashedMetadata {
                object_id: r.get(0)?,
                object_type: r.get(1)?,
                account_id: r.get(2)?,
                plugin_id: r.get(3)?,
                value: text(r.get_ref(4)?),
                version: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
            })
        })?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

/// Writes `rows` into `DetatchedPluginMetadata`; of two rows with one key the higher
/// version wins, as `import-legacy` keeps them.
pub fn save(conn: &Connection, rows: &[StashedMetadata]) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO DetatchedPluginMetadata (objectId, objectType, accountId, pluginId, value, version) VALUES (?,?,?,?,?,?) \
         ON CONFLICT (objectId, accountId, pluginId) DO UPDATE SET objectType = excluded.objectType, value = excluded.value, version = excluded.version \
         WHERE excluded.version > DetatchedPluginMetadata.version",
    )?;
    for r in rows {
        stmt.execute(params![
            r.object_id,
            r.object_type,
            r.account_id,
            r.plugin_id,
            r.value,
            r.version
        ])?;
    }
    Ok(())
}

/// [`collect`] over the database in `config_dir`, for every account. No database means
/// nothing to keep.
pub fn collect_database(config_dir: &Path) -> Result<Vec<StashedMetadata>> {
    if !config_dir.join(ms_core::constants::DATABASE_FILE).exists() {
        return Ok(Vec::new());
    }
    let conn = crate::schema::open_connection(config_dir)?;
    collect(&conn, None)
}

/// [`save`] into the database in `config_dir`, in one transaction.
pub fn save_database(config_dir: &Path, rows: &[StashedMetadata]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let conn = crate::schema::open_connection(config_dir)?;
    let tx = conn.unchecked_transaction()?;
    save(&tx, rows)?;
    tx.commit()?;
    Ok(())
}

fn text(v: rusqlite::types::ValueRef<'_>) -> String {
    match v {
        rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
            String::from_utf8_lossy(b).into_owned()
        }
        _ => "null".to_owned(),
    }
}

/// `(pluginId, value, version)` of each non-empty value in a model's `metadata`.
fn entries(data: &Value) -> Vec<(String, Value, i64)> {
    data.get("metadata")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|m| json_size(&m["value"]) > 0)
                .map(|m| {
                    (
                        json_str(m, "pluginId").to_owned(),
                        m["value"].clone(),
                        json_i64(&m["v"]),
                    )
                })
                .filter(|(plugin, ..)| !plugin.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn models_with_metadata(
    conn: &Connection,
    table: &str,
    account_id: Option<&str>,
) -> Result<Vec<(String, String, Value)>> {
    let sql = format!(
        "SELECT id, accountId, data FROM `{table}` WHERE id IN \
         (SELECT id FROM ModelPluginMetadata WHERE objectType = ?1 AND (?2 IS NULL OR accountId = ?2))"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![table, account_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            text(r.get_ref(2)?),
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, aid, data) = row?;
        let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
        out.push((id, aid, data));
    }
    Ok(out)
}

fn collect_messages(
    conn: &Connection,
    account_id: Option<&str>,
    out: &mut Vec<StashedMetadata>,
) -> Result<()> {
    for (id, aid, data) in models_with_metadata(conn, "Message", account_id)? {
        let header_message_id = json_str(&data, "hMsgId");
        if ms_core::ids::is_deletion_placeholder_id(&id) || header_message_id.is_empty() {
            continue;
        }
        for (plugin_id, value, version) in entries(&data) {
            out.push(StashedMetadata {
                object_id: pending_message_key(header_message_id),
                object_type: "Message".into(),
                account_id: aid.clone(),
                plugin_id,
                value: value.to_string(),
                version,
            });
        }
    }
    Ok(())
}

fn collect_threads(
    conn: &Connection,
    account_id: Option<&str>,
    out: &mut Vec<StashedMetadata>,
) -> Result<()> {
    let mut header_ids = conn.prepare(
        "SELECT DISTINCT headerMessageId FROM Message WHERE threadId = ? AND accountId = ? \
         AND headerMessageId IS NOT NULL AND headerMessageId != '' AND id NOT LIKE 'deleted-%'",
    )?;
    for (id, aid, data) in models_with_metadata(conn, "Thread", account_id)? {
        let metadata = entries(&data);
        if metadata.is_empty() {
            continue;
        }
        let headers: Vec<String> = header_ids
            .query_map(params![id, aid], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for h in &headers {
            for (plugin_id, value, version) in &metadata {
                out.push(StashedMetadata {
                    object_id: pending_thread_key(h, &id),
                    object_type: "Thread".into(),
                    account_id: aid.clone(),
                    plugin_id: plugin_id.clone(),
                    value: value.to_string(),
                    version: *version,
                });
            }
        }
    }
    Ok(())
}
