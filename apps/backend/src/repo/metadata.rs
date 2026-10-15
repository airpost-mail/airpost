// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! The metadata version rules (docs/backend-plan.md 3.5) and the SQL behind them. Callers hold
//! the writer's transaction, so every seq is taken in commit order.

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::Value;

use super::{MetadataWrite, Written};
use crate::wire::{MetadataRecord, ObjectType, V_MAX};

/// Plugins whose value carries a `trackingId` the tracking endpoints look rows up by (3.6).
pub const TRACKING_PLUGINS: [&str; 2] = ["open-tracking", "link-tracking"];
/// Longest `trackingId` copied into `metadata.tracking_id`; the Tern format uses 22 chars.
const MAX_TRACKING_ID_CHARS: usize = 64;

/// `v` is read as `uint32_t` by the C++ engine.
pub fn clamp_v(v: u64) -> u32 {
    v.min(u64::from(V_MAX)) as u32
}

/// The push rule: one above the stored version (1 for a new row), or the pushed version if
/// that is larger. `None` and `Some(0)` both mean the device does not know.
pub fn push_version(stored: Option<u32>, pushed: Option<u64>) -> u32 {
    let base = stored.map_or(1, |v| u64::from(v) + 1);
    clamp_v(base.max(pushed.unwrap_or(0)))
}

/// The backfill rule for an existing row: the stored value stays, and when the device's
/// version is at least the stored one, `v` becomes one above the device's, so the device
/// adopts the server's value from the stream. `None` leaves the row alone.
pub fn backfill_version(stored: u32, pushed: Option<u64>) -> Option<u32> {
    let pushed = pushed.unwrap_or(0);
    (pushed > 0 && pushed >= u64::from(stored)).then(|| clamp_v(pushed.saturating_add(1)))
}

/// `value.trackingId` for the tracking plugins.
pub fn tracking_id(plugin_id: &str, value: &Value) -> Option<String> {
    if !TRACKING_PLUGINS.contains(&plugin_id) {
        return None;
    }
    value
        .get("trackingId")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty() && t.chars().count() <= MAX_TRACKING_ID_CHARS)
        .map(str::to_owned)
}

pub(super) fn next_seq(tx: &Transaction) -> rusqlite::Result<i64> {
    tx.query_row(
        "UPDATE seq SET value = value + 1 WHERE id = 1 RETURNING value",
        [],
        |r| r.get(0),
    )
}

fn stored_v(
    tx: &Transaction,
    user_id: &str,
    aid: &str,
    w: &MetadataWrite,
) -> rusqlite::Result<Option<u32>> {
    tx.query_row(
        "SELECT v FROM metadata
         WHERE user_id = ?1 AND aid = ?2 AND object_type = ?3 AND object_id = ?4 AND plugin_id = ?5",
        params![user_id, aid, w.object_type.as_str(), w.object_id, w.plugin_id],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map(|v| v.map(|v| clamp_v(v.max(0) as u64)))
}

/// Inserts or updates the row in place. Never `INSERT OR REPLACE`, which would give the row a
/// new rowid and move it across page offsets (3.3).
fn upsert(
    tx: &Transaction,
    user_id: &str,
    aid: &str,
    w: &MetadataWrite,
    v: u32,
    now: i64,
) -> rusqlite::Result<Written> {
    let seq = next_seq(tx)?;
    let value = serde_json::to_string(&w.value).expect("a Value always serializes");
    tx.execute(
        "INSERT INTO metadata (user_id, aid, object_type, object_id, plugin_id, v, value,
                               header_message_id, tracking_id, seq, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
         ON CONFLICT (user_id, aid, object_type, object_id, plugin_id) DO UPDATE SET
             v = excluded.v,
             value = excluded.value,
             header_message_id = COALESCE(excluded.header_message_id, metadata.header_message_id),
             tracking_id = excluded.tracking_id,
             seq = excluded.seq,
             updated_at = excluded.updated_at",
        params![
            user_id,
            aid,
            w.object_type.as_str(),
            w.object_id,
            w.plugin_id,
            v,
            value,
            w.header_message_id,
            tracking_id(&w.plugin_id, &w.value),
            seq,
            now
        ],
    )?;
    Ok(Written { v, seq })
}

/// 3.5 push, steps 1 and 4. The tracking merge (steps 2 and 3) arrives with B2-T.
pub(super) fn push(
    tx: &Transaction,
    user_id: &str,
    aid: &str,
    w: &MetadataWrite,
    now: i64,
) -> rusqlite::Result<Written> {
    let v = push_version(stored_v(tx, user_id, aid, w)?, w.version);
    upsert(tx, user_id, aid, w, v, now)
}

/// 3.5 backfill for one record. `None` when the row was left alone.
pub(super) fn backfill(
    tx: &Transaction,
    user_id: &str,
    aid: &str,
    w: &MetadataWrite,
    now: i64,
) -> rusqlite::Result<Option<Written>> {
    let Some(stored) = stored_v(tx, user_id, aid, w)? else {
        return upsert(tx, user_id, aid, w, push_version(None, w.version), now).map(Some);
    };
    let Some(v) = backfill_version(stored, w.version) else {
        return Ok(None);
    };
    let seq = next_seq(tx)?;
    tx.execute(
        "UPDATE metadata SET v = ?6, seq = ?7, updated_at = ?8,
                header_message_id = COALESCE(?9, header_message_id)
         WHERE user_id = ?1 AND aid = ?2 AND object_type = ?3 AND object_id = ?4 AND plugin_id = ?5",
        params![
            user_id,
            aid,
            w.object_type.as_str(),
            w.object_id,
            w.plugin_id,
            v,
            seq,
            now,
            w.header_message_id
        ],
    )?;
    Ok(Some(Written { v, seq }))
}

const RECORD_COLUMNS: &str = "object_type, object_id, aid, plugin_id, v, value, header_message_id";

fn conversion_error(
    col: usize,
    e: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
}

#[derive(Debug, thiserror::Error)]
#[error("unknown object type {0:?}")]
struct UnknownObjectType(String);

fn record_from_row(r: &Row) -> rusqlite::Result<MetadataRecord> {
    let object_type: String = r.get(0)?;
    let object_type = ObjectType::parse(&object_type)
        .ok_or_else(|| conversion_error(0, UnknownObjectType(object_type)))?;
    let value: String = r.get(5)?;
    let value = serde_json::from_str(&value).map_err(|e| conversion_error(5, e))?;
    Ok(MetadataRecord {
        object_type,
        object_id: r.get(1)?,
        aid: r.get(2)?,
        plugin_id: r.get(3)?,
        v: clamp_v(r.get::<_, i64>(4)?.max(0) as u64),
        value,
        header_message_id: r.get(6)?,
    })
}

pub(super) fn head(c: &Connection, user_id: &str, aid: &str) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM metadata WHERE user_id = ?1 AND aid = ?2",
        params![user_id, aid],
        |r| r.get(0),
    )
}

pub(super) fn page(
    c: &Connection,
    user_id: &str,
    aid: &str,
    limit: usize,
    offset: u64,
) -> rusqlite::Result<Vec<MetadataRecord>> {
    let mut stmt = c.prepare_cached(&format!(
        "SELECT {RECORD_COLUMNS} FROM metadata WHERE user_id = ?1 AND aid = ?2
         ORDER BY id LIMIT ?3 OFFSET ?4"
    ))?;
    let offset = i64::try_from(offset).unwrap_or(i64::MAX);
    let rows = stmt.query_map(params![user_id, aid, limit as i64, offset], record_from_row)?;
    rows.collect()
}

pub(super) fn since(
    c: &Connection,
    user_id: &str,
    aid: &str,
    after: i64,
    limit: usize,
) -> rusqlite::Result<Vec<(i64, MetadataRecord)>> {
    let mut stmt = c.prepare_cached(&format!(
        "SELECT {RECORD_COLUMNS}, seq FROM metadata WHERE user_id = ?1 AND aid = ?2 AND seq > ?3
         ORDER BY seq LIMIT ?4"
    ))?;
    let rows = stmt.query_map(params![user_id, aid, after, limit as i64], |r| {
        Ok((r.get(7)?, record_from_row(r)?))
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn push_version_table() {
        let max = u64::from(V_MAX);
        // (stored, pushed, expected)
        let cases: &[(Option<u32>, Option<u64>, u32)] = &[
            (None, None, 1),
            (None, Some(0), 1),
            (None, Some(1), 1),
            (None, Some(7), 7),
            (Some(1), None, 2),
            (Some(1), Some(0), 2),
            (Some(1), Some(2), 2),
            (Some(5), Some(3), 6),
            (Some(5), Some(6), 6),
            (Some(5), Some(9), 9),
            (None, Some(max + 10), V_MAX),
            (Some(V_MAX - 1), None, V_MAX),
            (Some(V_MAX), None, V_MAX),
            (Some(V_MAX), Some(u64::MAX), V_MAX),
        ];
        for &(stored, pushed, want) in cases {
            assert_eq!(push_version(stored, pushed), want, "{stored:?} {pushed:?}");
        }
    }

    #[test]
    fn backfill_version_table() {
        // (stored, pushed, expected)
        let cases: &[(u32, Option<u64>, Option<u32>)] = &[
            (1, None, None),
            (1, Some(0), None),
            (3, Some(2), None),
            (3, Some(3), Some(4)),
            (3, Some(8), Some(9)),
            (V_MAX, Some(u64::from(V_MAX)), Some(V_MAX)),
            (1, Some(u64::MAX), Some(V_MAX)),
        ];
        for &(stored, pushed, want) in cases {
            assert_eq!(
                backfill_version(stored, pushed),
                want,
                "{stored} {pushed:?}"
            );
        }
    }

    #[test]
    fn tracking_ids() {
        let v = json!({"trackingId": "AAAAAAAAAAAAAAAAAAAAAA", "open_count": 0});
        assert_eq!(
            tracking_id("open-tracking", &v).as_deref(),
            Some("AAAAAAAAAAAAAAAAAAAAAA")
        );
        assert_eq!(
            tracking_id("link-tracking", &v).as_deref(),
            Some("AAAAAAAAAAAAAAAAAAAAAA")
        );
        assert_eq!(tracking_id("thread-snooze", &v), None);
        assert_eq!(tracking_id("open-tracking", &json!({})), None);
        assert_eq!(tracking_id("open-tracking", &json!([])), None);
        assert_eq!(
            tracking_id("open-tracking", &json!({"trackingId": 5})),
            None
        );
        assert_eq!(
            tracking_id("open-tracking", &json!({"trackingId": ""})),
            None
        );
        assert_eq!(
            tracking_id("open-tracking", &json!({"trackingId": "x".repeat(65)})),
            None
        );
    }
}
