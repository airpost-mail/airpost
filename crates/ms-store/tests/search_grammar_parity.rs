// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The search grammar parity suite (docs/desktop-client-plan.md stage 9). Every query in
//! `fixtures/search_queries.json`, captured from the desktop's `SearchQueryParser` and
//! `LocalSearchQueryBackend` by `apps/desktop/spec/search-query-capture-spec.ts`, must
//! parse to the same AST here (or fail on both sides), and its desktop WHERE clause and the
//! engine's must find the same threads on one fixture database. Date terms, which the engine
//! reads on its own terms, are checked here against two zones.

use std::path::Path;

use ms_core::ManualClock;
use ms_store::model_query::{ModelQuery, QueryOutput};
use ms_store::read::{ReadStore, ThreadSearch};
use ms_store::schema;
use ms_store::search_dates::DateContext;
use ms_store::search_grammar::{self, Where};
use rusqlite::{params_from_iter, Connection};
use serde_json::{json, Value};

const AID: &str = "a1";

struct T {
    id: &'static str,
    subject: &'static str,
    from: &'static str,
    to: &'static str,
    categories: &'static str,
    body: &'static str,
    unread: i64,
    starred: i64,
    attachments: i64,
    /// `lastMessageReceivedTimestamp`.
    lmrt: i64,
}

/// 2024-03-01 23:30 UTC: still the 1st in Lisbon, already the 2nd in Tokyo.
const LATE_ON_THE_1ST: i64 = 1_709_335_800;

const THREADS: &[T] = &[
    T {
        id: "t01",
        subject: "Q3 budget review",
        from: "ann@x.test Ann Lee",
        to: "me@x.test Me",
        categories: "inbox",
        body: "the budget numbers for q3",
        unread: 1,
        starred: 0,
        attachments: 1,
        lmrt: 1_704_000_000,
    },
    T {
        id: "t02",
        subject: "Lisbon trip",
        from: "bob@x.test Bob",
        to: "me@x.test Me",
        categories: "inbox Travel",
        body: "flights to lisbon booked, hello",
        unread: 0,
        starred: 1,
        attachments: 0,
        lmrt: 1_704_100_000,
    },
    T {
        id: "t03",
        subject: "Re: budget",
        from: "me@x.test Me",
        to: "ann@x.test Ann Lee",
        categories: "sent",
        body: "replying about the budget",
        unread: 0,
        starred: 0,
        attachments: 0,
        lmrt: 1_704_200_000,
    },
    T {
        id: "t04",
        subject: "Invoice 1042",
        from: "carol@shop.test Carol",
        to: "me@x.test Me",
        categories: "inbox",
        body: "please find the invoice attached",
        unread: 1,
        starred: 0,
        attachments: 2,
        lmrt: 1_704_300_000,
    },
    T {
        id: "t05",
        subject: "Old budget",
        from: "ann@x.test Ann Lee",
        to: "me@x.test Me",
        categories: "trash",
        body: "deleted budget draft",
        unread: 1,
        starred: 1,
        attachments: 0,
        lmrt: 1_704_400_000,
    },
    T {
        id: "t06",
        subject: "Spam offer",
        from: "spam@bad.test",
        to: "me@x.test",
        categories: "spam",
        body: "lisbon trip prize",
        unread: 1,
        starred: 0,
        attachments: 0,
        lmrt: 1_704_500_000,
    },
    T {
        id: "t07",
        subject: "foo bar",
        from: "dave@x.test Dave",
        to: "bob@x.test Bob, Mark mhahnenb@gmail.com",
        categories: "inbox",
        body: "baz blah",
        unread: 0,
        starred: 0,
        attachments: 0,
        lmrt: 1_704_600_000,
    },
    T {
        id: "t08",
        subject: "Lisbon photos",
        from: "carol@shop.test Carol",
        to: "me@x.test Me",
        categories: "sent Travel",
        body: "photos from the trip",
        unread: 1,
        starred: 0,
        attachments: 3,
        lmrt: 1_704_700_000,
    },
    T {
        id: "t09",
        subject: "late night",
        from: "erin@x.test Erin",
        to: "me@x.test",
        categories: "inbox",
        body: "sent just before midnight in lisbon",
        unread: 0,
        starred: 0,
        attachments: 0,
        lmrt: LATE_ON_THE_1ST,
    },
    T {
        id: "t10",
        subject: "next morning",
        from: "erin@x.test Erin",
        to: "me@x.test",
        categories: "inbox",
        body: "the day after",
        unread: 0,
        starred: 0,
        attachments: 0,
        lmrt: LATE_ON_THE_1ST + 12 * 3600,
    },
];

fn build(dir: &Path) {
    let mut out = Vec::new();
    ms_store::migrate::migrate(dir, &ManualClock::new(1_700_000_000), &mut out).unwrap();
    let conn = schema::open_connection(dir).unwrap();
    for t in THREADS {
        let data = json!({
            "__cls": "Thread", "id": t.id, "aid": AID, "v": 1, "subject": t.subject,
            "unread": t.unread, "starred": t.starred, "attachmentCount": t.attachments,
            "lmrt": t.lmrt, "lmt": t.lmrt,
        });
        conn.execute(
            "INSERT INTO Thread (id, accountId, version, data, subject, unread, starred, \
             lastMessageTimestamp, lastMessageReceivedTimestamp, inAllMail) \
             VALUES (?,?,1,?,?,?,?,?,?,1)",
            rusqlite::params![
                t.id,
                AID,
                data.to_string(),
                t.subject,
                t.unread,
                t.starred,
                t.lmrt,
                t.lmrt
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ThreadSearch (content_id, subject, to_, from_, categories, body) \
             VALUES (?,?,?,?,?,?)",
            rusqlite::params![t.id, t.subject, t.to, t.from, t.categories, t.body],
        )
        .unwrap();
    }
}

fn ids_where(conn: &Connection, w: &Where) -> Vec<String> {
    let sql = format!("SELECT id FROM Thread WHERE {} ORDER BY id", w.sql);
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map(params_from_iter(w.params.iter()), |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn captured() -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/search_queries.json");
    let file: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    file["queries"].as_array().unwrap().clone()
}

#[test]
fn engine_parses_and_finds_what_the_desktop_does() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let conn = Connection::open(dir.path().join("edgehill.db")).unwrap();
    let dates = DateContext::utc(Some(1_710_000_000));
    let (mut compared, mut non_empty) = (0, 0);
    for q in captured() {
        let text = q["text"].as_str().unwrap();
        let parsed = search_grammar::parse(text);
        if q["error"] == json!(true) {
            assert!(
                parsed.is_err(),
                "{text:?}: the desktop fails, the engine parsed {parsed:?}"
            );
            continue;
        }
        let ast = parsed.unwrap_or_else(|e| panic!("{text:?}: {e}"));
        assert_eq!(serde_json::to_value(&ast).unwrap(), q["ast"], "{text:?}");
        let Some(desktop_where) = q["where"].as_str() else {
            continue;
        };
        let desktop = ids_where(
            &conn,
            &Where {
                sql: desktop_where.to_owned(),
                params: Vec::new(),
            },
        );
        let engine = ids_where(
            &conn,
            &search_grammar::compile_thread(&ast, &dates).unwrap(),
        );
        assert_eq!(engine, desktop, "{text:?}");
        compared += 1;
        non_empty += usize::from(!desktop.is_empty());
    }
    assert!(compared >= 50, "{compared} compared");
    assert!(non_empty >= 30, "only {non_empty} queries find a thread");
}

fn query_ids(store: &ReadStore, search: Value) -> Vec<String> {
    let q: ModelQuery = serde_json::from_value(json!({
        "class": "Thread", "search": search,
        "order": [{"attr": "lastMessageReceivedTimestamp", "dir": "asc"}],
    }))
    .unwrap();
    match store.model_query(&q).unwrap() {
        QueryOutput::Rows { rows, .. } => rows
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn dates_resolve_in_the_request_zone() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let store = ReadStore::open(dir.path()).unwrap();
    let in_zone = |text: &str, tz: &str| {
        query_ids(
            &store,
            json!({"text": text, "grammar": true, "tz": tz, "now": LATE_ON_THE_1ST + 3600}),
        )
    };
    // t09 is late on the 1st in Lisbon and early on the 2nd in Tokyo; t10 is the 2nd in
    // both.
    assert_eq!(in_zone("erin after:2024-03-02", "Europe/Lisbon"), ["t10"]);
    assert_eq!(
        in_zone("erin after:2024-03-02", "Asia/Tokyo"),
        ["t09", "t10"]
    );
    assert_eq!(in_zone("erin before:2024-03-02", "Europe/Lisbon"), ["t09"]);
    assert!(in_zone("erin before:2024-03-02", "Asia/Tokyo").is_empty());
    // `now` is 00:30 on the 2nd in UTC and Lisbon, 09:30 in Tokyo: today is the 2nd in
    // both, and yesterday the 1st.
    assert_eq!(in_zone("erin after:today", "Europe/Lisbon"), ["t10"]);
    assert_eq!(in_zone("erin after:today", "Asia/Tokyo"), ["t09", "t10"]);
    assert_eq!(
        in_zone("erin after:yesterday", "Europe/Lisbon"),
        ["t09", "t10"]
    );
    assert_eq!(
        in_zone("erin after:\"march 2\" before:\"3 days ago\"", "UTC"),
        Vec::<String>::new()
    );
    assert_eq!(in_zone("erin since:\"mar 1 2024\"", "UTC"), ["t09", "t10"]);
    // The engine's own zone and clock don't enter: without `now` a relative date is an
    // error, and without `tz` dates are UTC.
    let no_now: ModelQuery = serde_json::from_value(json!({
        "class": "Thread", "search": {"text": "after:yesterday", "grammar": true}
    }))
    .unwrap();
    assert!(store.model_query(&no_now).is_err());
    assert_eq!(
        query_ids(
            &store,
            json!({"text": "erin after:2024-03-02", "grammar": true})
        ),
        ["t10"]
    );
}

#[test]
fn search_reads_the_grammar_across_accounts() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let store = ReadStore::open(dir.path()).unwrap();
    let accounts = vec![AID.to_owned()];
    let grammar = |text: &'static str| ThreadSearch::Grammar {
        text,
        dates: DateContext::utc(None),
    };
    let ids = |s: ThreadSearch<'_>, accounts: &[String]| -> Vec<String> {
        store
            .search_threads(accounts, &s, 50)
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        ids(grammar("budget NOT in:trash"), &accounts),
        ["t03", "t01"]
    );
    assert_eq!(ids(grammar("is:starred"), &accounts), ["t05", "t02"]);
    assert_eq!(
        ids(ThreadSearch::Plain("from:ann"), &accounts),
        Vec::<String>::new(),
        "plain text has no fields"
    );
    assert!(ids(grammar("budget"), &["other".to_owned()]).is_empty());
    assert!(store.search_threads(&accounts, &grammar("(x"), 50).is_err());
}

/// The longest and deepest queries the grammar takes run on SQLite: FTS5's parser has a
/// fixed stack, which a right-nested chain of 34 terms overflowed.
#[test]
fn long_and_deep_queries_run_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let store = ReadStore::open(dir.path()).unwrap();
    let accounts = vec![AID.to_owned()];
    let run = |q: &str| {
        let s = ThreadSearch::Grammar {
            text: q,
            dates: DateContext::utc(None),
        };
        store
            .search_threads(&accounts, &s, 50)
            .map(|v| v.len())
            .map_err(|e| format!("{e:?}"))
    };
    let depth = search_grammar::MAX_MATCH_DEPTH;
    // `x NOT (y NOT (...))` nests one level a term; `a OR (b (a OR (...)))` too.
    let nots = |n: usize| vec!["budget"; n].join(" NOT ");
    let alternating = |n: usize| {
        (0..n - 1)
            .map(|i| {
                if i % 2 == 0 {
                    "budget OR ("
                } else {
                    "budget ("
                }
            })
            .collect::<String>()
            + "budget"
            + &")".repeat(n - 1)
    };
    for (q, found) in [
        (vec!["budget"; search_grammar::MAX_TOKENS].join(" "), 3),
        (
            vec!["budget"; search_grammar::MAX_TOKENS / 2].join(" OR "),
            3,
        ),
        (vec!["budget review"; 67].join(" OR "), 1),
        (nots(depth), 0),
        (nots(depth - 1), 3),
        (alternating(depth), 3),
        // The rest on `Thread` columns, nested as deep as the token cap allows.
        (vec!["budget is:unread"; 50].join(" "), 2),
        (vec!["is:unread"; 50].join(" NOT "), 0),
        (vec!["budget OR is:starred"; 33].join(" OR "), 4),
    ] {
        assert_eq!(run(&q), Ok(found), "{q}");
    }
    for q in [nots(depth + 1), alternating(depth + 1)] {
        let e = run(&q).unwrap_err();
        assert!(
            e.starts_with("Invalid(") && e.contains("nests too deeply"),
            "{q}: {e}"
        );
    }
}

#[test]
fn terms_without_letters_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let store = ReadStore::open(dir.path()).unwrap();
    let accounts = vec![AID.to_owned()];
    let run = |q: &str| {
        let s = ThreadSearch::Grammar {
            text: q,
            dates: DateContext::utc(None),
        };
        store
            .search_threads(&accounts, &s, 50)
            .map(|v| v.len())
            .map_err(|e| format!("{e:?}"))
    };
    for q in [
        "budget - review",
        "budget : x",
        "budget \"\"",
        "from:\"...\"",
    ] {
        let e = run(q).unwrap_err();
        assert!(e.starts_with("Invalid("), "{q}: {e}");
    }
    assert_eq!(run("subject:re:budget"), Ok(1));
    assert_eq!(run("subject:budget"), Ok(3));
}
