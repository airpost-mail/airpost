// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The schema `migrate` creates must be the C++ engine's V12 schema exactly, plus what
//! Airpost's migrations add: the client and `invariants.py` query it by name, and the placement
//! upsert's ON CONFLICT target depends on the partial index WHERE clauses. A database at any
//! earlier Airpost version must reach the same schema with its rows intact.

use airpost_core::constants::{BASE_SCHEMA_VERSION, SCHEMA_VERSION};
use airpost_core::ManualClock;
use airpost_store::schema::MIGRATIONS;
use airpost_store::{DeltaStream, Model, Store, Thread};
use rusqlite::Connection;
use serde_json::{json, Value};

fn user_version(conn: &Connection) -> i32 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn v12_fixture() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/v12_schema.json")).unwrap()
}

/// The V12 fixture plus `fixtures/migrations.json`, in `schema_of` order.
fn current_fixture() -> Vec<Value> {
    let migrations: Value = serde_json::from_str(include_str!("fixtures/migrations.json")).unwrap();
    assert_eq!(migrations["version"], json!(SCHEMA_VERSION));
    let mut all = v12_fixture();
    all.extend(migrations["added"].as_array().unwrap().iter().cloned());
    let key = |v: &Value| {
        (
            v["type"].as_str().unwrap().to_owned(),
            v["name"].as_str().unwrap().to_owned(),
        )
    };
    all.sort_by_key(key);
    all
}

fn assert_schema(actual: &[Value], expected: &[Value]) {
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.iter().zip(expected) {
        assert_eq!(a, e);
    }
}

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
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert!(outcome.created && !outcome.moved_legacy);
    assert_eq!(String::from_utf8(out).unwrap(), "\nRunning Setup");
    let conn = Connection::open(dir.path().join("airpost.db")).unwrap();
    assert_eq!(user_version(&conn), SCHEMA_VERSION);
    let schema = schema_of(&conn);
    (dir, schema)
}

#[test]
fn matches_fixture() {
    let (_dir, actual) = migrated_schema();
    assert_schema(&actual, &current_fixture());
}

#[test]
fn v12_is_the_cxx_schema() {
    let conn = Connection::open_in_memory().unwrap();
    airpost_store::schema::create_v12(&conn).unwrap();
    assert_eq!(user_version(&conn), BASE_SCHEMA_VERSION);
    assert_schema(&schema_of(&conn), &v12_fixture());
}

#[test]
fn migrations_are_consecutive() {
    for (i, m) in MIGRATIONS.iter().enumerate() {
        assert_eq!(m.version, BASE_SCHEMA_VERSION + 1 + i as i32);
    }
    assert_eq!(MIGRATIONS.last().unwrap().version, SCHEMA_VERSION);
}

const ACCOUNT: &str = "acc1";

/// A profile as a build at schema version 12 left it: the V12 schema, written through the
/// store, with a removed thread's references still in `ThreadReference`.
fn write_v12_profile(dir: &std::path::Path) -> String {
    {
        let conn = airpost_store::schema::open_connection(dir).unwrap();
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")
            .unwrap();
        airpost_store::schema::create_v12(&conn).unwrap();
        airpost_store::migrate::save_key_value(&conn, "engine", "rust").unwrap();
    }
    let store = Store::open(dir, DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    let mut thread = Thread::create("m1", ACCOUNT, "Plans", 0);
    store.save(&mut thread).unwrap();
    store
        .upsert_thread_references(thread.id(), ACCOUNT, "<a@x>", &["<root@x>".to_owned()])
        .unwrap();
    store
        .conn()
        .execute_batch(
            "INSERT INTO ThreadReference (threadId, accountId, headerMessageId) VALUES ('t:gone', 'acc1', '<old@x>');
             INSERT INTO MessageBody (id, value, fetchedAt) VALUES ('m1', '<p>kept</p>', 1);
             REPLACE INTO `_State` (id, value) VALUES ('cursor-acc1', '42');",
        )
        .unwrap();
    thread.id().to_owned()
}

#[test]
fn a_v12_database_is_upgraded_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let thread_id = write_v12_profile(dir.path());
    let clock = ManualClock::new(1_700_000_000);

    let mut out = Vec::new();
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert!(outcome.upgraded && !outcome.created && !outcome.moved_legacy);
    assert_eq!(String::from_utf8(out).unwrap(), "\nRunning Migration");

    let conn = Connection::open(dir.path().join("airpost.db")).unwrap();
    assert_eq!(user_version(&conn), SCHEMA_VERSION);
    assert_schema(&schema_of(&conn), &current_fixture());
    let body: String = conn
        .query_row("SELECT value FROM MessageBody WHERE id = 'm1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(body, "<p>kept</p>");
    let refs: Vec<String> = conn
        .prepare("SELECT threadId FROM ThreadReference ORDER BY headerMessageId")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(refs, [thread_id.clone(), thread_id.clone()]);

    // TD7: the Message-ID lookup uses the index instead of scanning ThreadReference.
    let plan: Vec<String> = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT Thread.data FROM Thread INNER JOIN ThreadReference ON ThreadReference.threadId = Thread.id \
             WHERE ThreadReference.accountId = ? AND ThreadReference.headerMessageId IN (?, ?) LIMIT 1",
        )
        .unwrap()
        .query_map(["acc1", "<a@x>", "<root@x>"], |r| r.get(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        plan.iter()
            .any(|p| p.contains("USING INDEX ThreadReferenceLookup")),
        "{plan:?}"
    );
    drop(conn);

    let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    let found = store
        .find_thread_by_references(ACCOUNT, "<new@x>", &["<root@x>".to_owned()])
        .unwrap()
        .unwrap();
    assert_eq!(found.id(), thread_id);
    assert_eq!(
        store.get_key_value("cursor-acc1").unwrap().as_deref(),
        Some("42")
    );
    drop(store);

    // A second run has nothing to do.
    let mut out = Vec::new();
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(1_700_000_000);
    airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    {
        let conn = Connection::open(dir.path().join("airpost.db")).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
    }
    let err = airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap_err();
    assert!(err.to_string().contains("newer than this build"), "{err}");
    let conn = Connection::open(dir.path().join("airpost.db")).unwrap();
    assert_eq!(user_version(&conn), SCHEMA_VERSION + 1);
    assert!(!dir.path().join("airpost.legacy.db").exists());
}

#[test]
fn legacy_database_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("airpost.db");
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
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert!(outcome.moved_legacy && outcome.created);
    assert_eq!(String::from_utf8(out).unwrap(), "\nRunning Migration");
    assert!(dir.path().join("airpost.legacy.db").exists());
    assert!(dir.path().join("files").exists());
    assert_eq!(
        std::fs::read_dir(dir.path().join("files")).unwrap().count(),
        0
    );

    // A second run finds a Rust database and changes nothing.
    let mut out = Vec::new();
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());

    // No full VACUUM months later either: the new database vacuums incrementally.
    clock.advance(31 * 24 * 3600);
    let mut out = Vec::new();
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());
    let conn = airpost_store::schema::open_connection(dir.path()).unwrap();
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
    airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    {
        let conn = Connection::open(dir.path().join("airpost.db")).unwrap();
        conn.execute("REPLACE INTO `_State` (id, value) VALUES ('kept', '1')", [])
            .unwrap();
    }
    std::fs::rename(
        dir.path().join("airpost.db"),
        dir.path().join("edgehill.db"),
    )
    .unwrap();
    std::fs::write(dir.path().join("edgehill.legacy.db"), "legacy").unwrap();
    let mut out = Vec::new();
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());
    assert!(!dir.path().join("edgehill.db").exists());
    assert!(!dir.path().join("edgehill.legacy.db").exists());
    assert_eq!(
        std::fs::read(dir.path().join("airpost.legacy.db")).unwrap(),
        b"legacy"
    );
    let conn = Connection::open(dir.path().join("airpost.db")).unwrap();
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
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    assert!(outcome.moved_legacy && outcome.created);
    assert!(dir.path().join("airpost.legacy.db").exists());
    assert!(!dir.path().join("edgehill.db").exists());

    // An old-named database never replaces a current one.
    let dir = tempfile::tempdir().unwrap();
    airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    std::fs::write(dir.path().join("edgehill.db"), "old").unwrap();
    let outcome = airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
    assert_eq!(outcome, Default::default());
    assert_eq!(
        std::fs::read(dir.path().join("edgehill.db")).unwrap(),
        b"old"
    );
}
