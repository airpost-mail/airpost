// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The metadata outbox (docs/backend-plan.md 4.1): pushes the Airpost server did not take yet,
//! and the backfill of local metadata after a new binding. Neither engine retries a failed
//! `SyncbackMetadataTask`, so without it a lost push would leave this device's version above
//! the server's and it would ignore every later echo.
//!
//! An Airpost table in `airpost.db`, created on first use, so a migrated database keeps the V12
//! schema the C++ engine wrote. The latest value per (account, object, plugin) wins: a push
//! replaces whatever is queued for its key under a new row id (AUTOINCREMENT, so ids are
//! never reused), and removing a sent entry by id never drops a newer one.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use airpost_core::constants::{DATABASE_FILE, SQLITE_BUSY_TIMEOUT_MS};
use airpost_core::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::metadata::MetadataPush;

const CREATE: &str = "CREATE TABLE IF NOT EXISTS MetadataOutbox (\
    id INTEGER PRIMARY KEY AUTOINCREMENT, accountId TEXT NOT NULL, objectType TEXT NOT NULL, \
    objectId TEXT NOT NULL, pluginId TEXT NOT NULL, version INTEGER NOT NULL, value TEXT NOT NULL, \
    headerMessageId TEXT, backfill INTEGER NOT NULL DEFAULT 0, \
    UNIQUE (accountId, objectType, objectId, pluginId))";

/// One queued record.
#[derive(Clone, Debug, PartialEq)]
pub struct OutboxEntry {
    pub id: i64,
    pub push: MetadataPush,
    /// Sent through `/airpost/v1/backfill`, where the server keeps a value it already has.
    pub backfill: bool,
}

pub struct Outbox {
    conn: Mutex<Connection>,
}

impl Outbox {
    /// Opens the outbox in `config_dir`'s `airpost.db`.
    pub fn open(config_dir: &Path) -> Result<Outbox> {
        let conn = Connection::open(config_dir.join(DATABASE_FILE))?;
        conn.busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))?;
        conn.execute_batch(CREATE)?;
        Ok(Outbox {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queues a push, replacing what is queued for its key.
    pub fn put(&self, push: &MetadataPush) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM MetadataOutbox WHERE accountId = ? AND objectType = ? AND objectId = ? AND pluginId = ?",
            params![push.account_id, class(push), push.model_id, push.plugin_id],
        )?;
        insert(&tx, push, false)?;
        tx.commit()?;
        Ok(())
    }

    /// Drops everything queued for `account_id` (it was meant for another server or user) and
    /// queues `records` as backfill.
    pub fn rebind(&self, account_id: &str, records: &[MetadataPush]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM MetadataOutbox WHERE accountId = ?",
            [account_id],
        )?;
        for r in records {
            insert(&tx, r, true)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The oldest `limit` entries of `account_id`.
    pub fn pending(&self, account_id: &str, limit: usize) -> Result<Vec<OutboxEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, objectType, objectId, pluginId, version, value, headerMessageId, backfill \
             FROM MetadataOutbox WHERE accountId = ? ORDER BY id LIMIT ?",
        )?;
        let rows = stmt.query_map(params![account_id, limit as i64], |r| {
            let value: String = r.get(5)?;
            Ok(OutboxEntry {
                id: r.get(0)?,
                push: MetadataPush {
                    account_id: account_id.to_owned(),
                    model_class: r.get(1)?,
                    model_id: r.get(2)?,
                    plugin_id: r.get(3)?,
                    version: r.get(4)?,
                    value: serde_json::from_str(&value).unwrap_or(serde_json::Value::Null),
                    header_message_id: r.get(6)?,
                },
                backfill: r.get::<_, i64>(7)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn remove(&self, ids: &[i64]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM MetadataOutbox WHERE id = ?", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Drops what is queued for `push`'s key: a newer value of it reached the server.
    pub fn remove_key(&self, push: &MetadataPush) -> Result<()> {
        self.conn().execute(
            "DELETE FROM MetadataOutbox WHERE accountId = ? AND objectType = ? AND objectId = ? AND pluginId = ?",
            params![push.account_id, class(push), push.model_id, push.plugin_id],
        )?;
        Ok(())
    }

    pub fn is_queued(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM MetadataOutbox WHERE id = ?",
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The entries of `entries` that are still queued under the same id.
    pub fn still_queued(&self, entries: Vec<OutboxEntry>) -> Result<Vec<OutboxEntry>> {
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            if self.is_queued(e.id)? {
                out.push(e);
            }
        }
        Ok(out)
    }

    pub fn len(&self, account_id: &str) -> Result<usize> {
        let n: Option<i64> = self
            .conn()
            .query_row(
                "SELECT count(*) FROM MetadataOutbox WHERE accountId = ?",
                [account_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(n.unwrap_or(0) as usize)
    }

    pub fn is_empty(&self, account_id: &str) -> Result<bool> {
        Ok(self.len(account_id)? == 0)
    }
}

/// Keys use the lowercase wire class, so `Thread` and `thread` are one key.
fn class(push: &MetadataPush) -> String {
    push.model_class.to_ascii_lowercase()
}

fn insert(conn: &Connection, push: &MetadataPush, backfill: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO MetadataOutbox (accountId, objectType, objectId, pluginId, version, value, headerMessageId, backfill) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (accountId, objectType, objectId, pluginId) DO NOTHING",
        params![
            push.account_id,
            class(push),
            push.model_id,
            push.plugin_id,
            push.version,
            push.value.to_string(),
            push.header_message_id,
            backfill as i64,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn push(object: &str, plugin: &str, value: serde_json::Value) -> MetadataPush {
        MetadataPush {
            account_id: "a1".into(),
            model_id: object.into(),
            model_class: "Thread".into(),
            plugin_id: plugin.into(),
            value,
            version: 2,
            header_message_id: Some("m@x".into()),
        }
    }

    #[test]
    fn latest_push_per_key_wins_and_removal_keeps_newer_entries() {
        let dir = tempfile_dir();
        let o = Outbox::open(dir.path()).unwrap();
        o.put(&push("t:1", "snooze", json!({"a": 1}))).unwrap();
        let first = o.pending("a1", 10).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].push.model_class, "thread");
        o.put(&push("t:1", "snooze", json!({"a": 2}))).unwrap();
        o.put(&push("t:2", "snooze", json!({}))).unwrap();
        // The first entry was replaced, so removing it by id is a no-op.
        o.remove(&[first[0].id]).unwrap();
        let now = o.pending("a1", 10).unwrap();
        assert_eq!(now.len(), 2);
        assert_eq!(now[0].push.value, json!({"a": 2}));
        assert!(!now[0].backfill);
        assert_eq!(now[0].push.header_message_id.as_deref(), Some("m@x"));
        assert!(o.pending("other", 10).unwrap().is_empty());
    }

    #[test]
    fn rebind_clears_and_queues_backfill_without_replacing_pushes() {
        let dir = tempfile_dir();
        let o = Outbox::open(dir.path()).unwrap();
        o.put(&push("t:old", "snooze", json!({"a": 1}))).unwrap();
        o.rebind("a1", &[push("t:1", "snooze", json!({"b": 1}))])
            .unwrap();
        let p = o.pending("a1", 10).unwrap();
        assert_eq!(p.len(), 1);
        assert!(p[0].backfill);
        assert_eq!(p[0].push.model_id, "t:1");
        o.put(&push("t:1", "snooze", json!({"b": 2}))).unwrap();
        let p = o.pending("a1", 10).unwrap();
        assert_eq!((p.len(), p[0].backfill), (1, false));
        assert_eq!(o.len("a1").unwrap(), 1);
    }

    fn tempfile_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }
}
