// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Schema migrations. `PRAGMA user_version` holds the number of
//! migrations applied. Append new migrations; never edit a released one.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

/// Migration 1 creates the v1 tables. `assets` arrives with B2.
///
/// `metadata` has an explicit `id INTEGER PRIMARY KEY` instead of the composite primary key:
/// pages are ordered by rowid, and SQLite only keeps rowids stable across `VACUUM` (and
/// `VACUUM INTO` backups) when they are aliased by an INTEGER PRIMARY KEY. The composite key
/// is a UNIQUE constraint, the target of every `INSERT … ON CONFLICT DO UPDATE`.
pub(crate) const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    first_name    TEXT NOT NULL DEFAULT '',
    last_name     TEXT NOT NULL DEFAULT '',
    password_hash TEXT NOT NULL,
    tracking_ns   TEXT NOT NULL UNIQUE,
    created_at    INTEGER NOT NULL,
    disabled_at   INTEGER
) STRICT;

CREATE TABLE tokens (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hash         BLOB NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at   INTEGER
) STRICT;
CREATE INDEX tokens_user ON tokens(user_id);

CREATE TABLE accounts (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    aid        TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, aid)
) STRICT;

CREATE TABLE metadata (
    id                INTEGER PRIMARY KEY,
    user_id           TEXT NOT NULL,
    aid               TEXT NOT NULL,
    object_type       TEXT NOT NULL,
    object_id         TEXT NOT NULL,
    plugin_id         TEXT NOT NULL,
    v                 INTEGER NOT NULL,
    value             TEXT NOT NULL,
    header_message_id TEXT,
    tracking_id       TEXT,
    seq               INTEGER NOT NULL,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    UNIQUE (user_id, aid, object_type, object_id, plugin_id),
    FOREIGN KEY (user_id, aid) REFERENCES accounts(user_id, aid) ON DELETE CASCADE
) STRICT;
CREATE INDEX metadata_seq ON metadata(user_id, aid, seq);
CREATE INDEX metadata_tracking ON metadata(user_id, plugin_id, tracking_id);

-- Global, monotonic. Every metadata write takes the next value inside its own transaction.
CREATE TABLE seq (
    id    INTEGER PRIMARY KEY CHECK (id = 1),
    value INTEGER NOT NULL
) STRICT;
INSERT INTO seq (id, value) VALUES (1, 0);

CREATE TABLE tracking_events (
    id          INTEGER PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tracking_id TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('open', 'click')),
    link_index  INTEGER,
    rid         TEXT,
    ts          INTEGER NOT NULL,
    applied     INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX tracking_events_lookup ON tracking_events(user_id, tracking_id);
CREATE INDEX tracking_events_ts ON tracking_events(ts);

CREATE TABLE claims (
    user_id    TEXT NOT NULL,
    aid        TEXT NOT NULL,
    object_id  TEXT NOT NULL,
    plugin_id  TEXT NOT NULL,
    expiration INTEGER NOT NULL,
    token_id   INTEGER REFERENCES tokens(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, aid, object_id, plugin_id, expiration),
    FOREIGN KEY (user_id, aid) REFERENCES accounts(user_id, aid) ON DELETE CASCADE
) STRICT;
CREATE INDEX claims_created ON claims(created_at);
"#];

/// The schema version this build expects.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub fn schema_version(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
}

/// Applies pending `migrations` ([`MIGRATIONS`], or a longer list in tests), then makes sure
/// the instance has a server id.
pub(crate) fn migrate(
    conn: &mut Connection,
    migrations: &[&str],
    new_server_id: impl FnOnce() -> String,
) -> Result<(), MigrateError> {
    let current = schema_version(conn)?;
    let supported = migrations.len() as u32;
    if current > supported {
        return Err(MigrateError::TooNew {
            found: current,
            supported,
        });
    }
    for (i, sql) in migrations.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as u32)?;
        tx.commit()?;
    }
    let existing: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'server_id'", [], |r| {
            r.get(0)
        })
        .optional()?;
    if existing.is_none() {
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('server_id', ?1)",
            [new_server_id()],
        )?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    TooNew { found: u32, supported: u32 },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}
