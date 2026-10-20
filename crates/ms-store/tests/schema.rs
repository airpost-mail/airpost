// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The schema `migrate` creates must be the C++ engine's V12 schema exactly: the client
//! and `invariants.py` query it by name, and the placement upsert's ON CONFLICT target
//! depends on the partial index WHERE clauses.

use std::path::{Path, PathBuf};

use ms_core::ManualClock;
use rusqlite::Connection;
use serde_json::{json, Value};

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
    let outcome = ms_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
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

/// Parses the `static vector<string> NAME = { … };` arrays of `constants.h` (adjacent
/// string literals concatenated, `\"` unescaped, comments skipped).
fn parse_arrays(src: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut arrays = std::collections::HashMap::new();
    let mut rest = src;
    while let Some(pos) = rest.find("static vector<string> ") {
        rest = &rest[pos + "static vector<string> ".len()..];
        let name_end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap();
        let name = rest[..name_end].to_owned();
        let body_start = rest.find('{').unwrap() + 1;
        let chars: Vec<char> = rest[body_start..].chars().collect();
        let (mut items, mut cur, mut i) = (Vec::new(), None::<String>, 0);
        while i < chars.len() {
            match chars[i] {
                '"' => {
                    let s = cur.get_or_insert_with(String::new);
                    i += 1;
                    while chars[i] != '"' {
                        if chars[i] == '\\' {
                            i += 1;
                        }
                        s.push(chars[i]);
                        i += 1;
                    }
                }
                '/' if chars[i + 1] == '/' => {
                    while chars[i] != '\n' {
                        i += 1;
                    }
                }
                ',' => items.extend(cur.take()),
                '}' => {
                    items.extend(cur.take());
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        arrays.insert(name, items);
    }
    arrays
}

fn constants_h() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy/mailsync-cpp/MailSync/constants.h")
}

#[test]
fn matches_live_replay_of_cpp_migrations() {
    let Ok(src) = std::fs::read_to_string(constants_h()) else {
        eprintln!("legacy C++ sources not present; fixture comparison only");
        return;
    };
    let arrays = parse_arrays(&src);
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("replay.db")).unwrap();
    for step in [
        "V1_SETUP_QUERIES",
        "V3_SETUP_QUERIES",
        "V4_SETUP_QUERIES",
        "V6_SETUP_QUERIES",
        "V7_SETUP_QUERIES",
        "V8_SETUP_QUERIES",
        "V9_SETUP_QUERIES",
        "V10_SETUP_QUERIES",
        "V10_INDEX_QUERIES",
    ] {
        for sql in &arrays[step] {
            conn.execute_batch(sql)
                .unwrap_or_else(|e| panic!("{step}: {sql}: {e}"));
        }
    }
    let (_d, actual) = migrated_schema();
    assert_eq!(actual, schema_of(&conn));

    // ACCOUNT_RESET_QUERIES are copied into schema.rs; keep them in step.
    assert_eq!(
        arrays["ACCOUNT_RESET_QUERIES"],
        ms_store::schema::ACCOUNT_RESET_QUERIES
    );
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
    let outcome = ms_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
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
    let outcome = ms_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());

    // No full VACUUM months later either: the new database vacuums incrementally.
    clock.advance(31 * 24 * 3600);
    let mut out = Vec::new();
    let outcome = ms_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    assert_eq!(outcome, Default::default());
    assert!(out.is_empty());
    let conn = ms_store::schema::open_connection(dir.path()).unwrap();
    let mode: i64 = conn
        .pragma_query_value(None, "auto_vacuum", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, 2, "incremental");
    let limit: i64 = conn
        .pragma_query_value(None, "journal_size_limit", |r| r.get(0))
        .unwrap();
    assert_eq!(limit, 64 * 1024 * 1024);
}
