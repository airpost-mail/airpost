// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The schema `migrate` creates must be the C++ engine's V12 schema exactly: the client
//! and `invariants.py` query it by name, and the placement upsert's ON CONFLICT target
//! depends on the partial index WHERE clauses.

use rusqlite::Connection;
use serde_json::{json, Value};
use tern_core::ManualClock;

fn schema_of(conn: &Connection) -> Vec<Value> {
    let mut stmt = conn
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
        .unwrap();
    stmt.query_map([], |r| {
        Ok(json!({
            "type": r.get::<_, String>(0)?,
            "name": r.get::<_, String>(1)?,
            "tbl_name": r.get::<_, String>(2)?,
            "sql": r.get::<_, Option<String>>(3)?,
        }))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

fn migrated_schema() -> (tempfile::TempDir, Vec<Value>) {
    let dir = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    let clock = ManualClock::new(1_700_000_000);
    let outcome = tern_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert!(outcome.created && !outcome.moved_legacy);
    let conn = Connection::open(dir.path().join("tern.db")).unwrap();
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 12);
    let schema = schema_of(&conn);
    (dir, schema)
}

#[test]
fn matches_fixture() {
    let fixture: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/v12_schema.json")).unwrap();
    let (_dir, actual) = migrated_schema();
    assert_eq!(actual.len(), fixture.len());
    for (a, e) in actual.iter().zip(&fixture) {
        assert_eq!(a, e);
    }
}

#[test]
fn legacy_database_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tern.db");
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE `_State` (id VARCHAR(40) PRIMARY KEY, value TEXT); PRAGMA user_version = 12;",
        )
        .unwrap();
    }
    std::fs::create_dir_all(dir.path().join("files/ab/cd")).unwrap();
    std::fs::write(dir.path().join("files/ab/cd/x.txt"), "x").unwrap();

    let clock = ManualClock::new(1_700_000_000);
    let mut out = Vec::new();
    let outcome = tern_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert!(outcome.moved_legacy && outcome.created);
    assert_eq!(String::from_utf8(out).unwrap(), "\nRunning Migration");
    assert!(dir.path().join("tern.legacy.db").exists());
    assert!(dir.path().join("files").exists());
    assert_eq!(
        std::fs::read_dir(dir.path().join("files")).unwrap().count(),
        0
    );

    // A second run finds a Rust database and changes nothing.
    let mut out = Vec::new();
    let outcome = tern_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());

    // No full VACUUM months later either: the new database vacuums incrementally.
    clock.advance(31 * 24 * 3600);
    let mut out = Vec::new();
    let outcome = tern_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());
    let conn = tern_store::schema::open_connection(dir.path()).unwrap();
    let mode: i64 = conn
        .pragma_query_value(None, "auto_vacuum", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, 2, "incremental");
    let limit: i64 = conn
        .pragma_query_value(None, "journal_size_limit", |r| r.get(0))
        .unwrap();
    assert_eq!(limit, 64 * 1024 * 1024);
}

#[test]
fn a_database_under_the_old_name_is_renamed() {
    let clock = ManualClock::new(1_700_000_000);

    // A Rust database keeps its rows under the new name.
    let dir = tempfile::tempdir().unwrap();
    tern_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    {
        let conn = Connection::open(dir.path().join("tern.db")).unwrap();
        conn.execute("REPLACE INTO `_State` (id, value) VALUES ('kept', '1')", [])
            .unwrap();
    }
    std::fs::rename(dir.path().join("tern.db"), dir.path().join("edgehill.db")).unwrap();
    std::fs::write(dir.path().join("edgehill.legacy.db"), "legacy").unwrap();
    let mut out = Vec::new();
    let outcome = tern_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());
    assert!(!dir.path().join("edgehill.db").exists());
    assert!(!dir.path().join("edgehill.legacy.db").exists());
    assert_eq!(
        std::fs::read(dir.path().join("tern.legacy.db")).unwrap(),
        b"legacy"
    );
    let conn = Connection::open(dir.path().join("tern.db")).unwrap();
    let kept: String = conn
        .query_row("SELECT value FROM `_State` WHERE id = 'kept'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kept, "1");

    // A C++ database is renamed, then moved aside as usual.
    let dir = tempfile::tempdir().unwrap();
    {
        let conn = Connection::open(dir.path().join("edgehill.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE `_State` (id VARCHAR(40) PRIMARY KEY, value TEXT); PRAGMA user_version = 12;",
        )
        .unwrap();
    }
    let outcome = tern_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    assert!(outcome.moved_legacy && outcome.created);
    assert!(dir.path().join("tern.legacy.db").exists());
    assert!(!dir.path().join("edgehill.db").exists());

    // An old-named database never replaces a current one.
    let dir = tempfile::tempdir().unwrap();
    tern_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    std::fs::write(dir.path().join("edgehill.db"), "old").unwrap();
    let outcome = tern_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    assert_eq!(outcome, Default::default());
    assert_eq!(
        std::fs::read(dir.path().join("edgehill.db")).unwrap(),
        b"old"
    );
}
