// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `query` parity suite (docs/desktop-client-plan.md stage 7): every query in
//! `fixtures/model_queries.json`, captured from the desktop's call sites by
//! `apps/desktop/spec/model-query-capture-spec.ts`, runs as the desktop's own SQL
//! (`Query.sql()`) and through the engine's compiler on one fixture database, and the two
//! must return the same rows in the same order. The ids the desktop queries name are the
//! ones [`build`] writes.

use std::path::Path;

use ms_core::ManualClock;
use ms_store::model_query::{compile, ModelQuery, QueryOutput};
use ms_store::read::ReadStore;
use ms_store::schema;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const BASE: i64 = 1_700_000_000;
const NULL_PLACEHOLDER: &str = "!NULLVALUE!";

struct Category {
    id: &'static str,
    aid: &'static str,
    role: &'static str,
    path: &'static str,
    label: bool,
}

const CATEGORIES: &[Category] = &[
    Category {
        id: "f-a1-inbox",
        aid: "a1",
        role: "inbox",
        path: "INBOX",
        label: false,
    },
    Category {
        id: "f-a1-sent",
        aid: "a1",
        role: "sent",
        path: "Sent",
        label: false,
    },
    Category {
        id: "f-a1-drafts",
        aid: "a1",
        role: "drafts",
        path: "Drafts",
        label: false,
    },
    Category {
        id: "f-a1-trash",
        aid: "a1",
        role: "trash",
        path: "Trash",
        label: false,
    },
    Category {
        id: "f-a1-spam",
        aid: "a1",
        role: "spam",
        path: "Spam",
        label: false,
    },
    Category {
        id: "f-a1-work",
        aid: "a1",
        role: "",
        path: "Work",
        label: false,
    },
    Category {
        id: "f-a2-inbox",
        aid: "a2",
        role: "inbox",
        path: "INBOX",
        label: false,
    },
    Category {
        id: "f-a2-sent",
        aid: "a2",
        role: "sent",
        path: "[Gmail]/Sent Mail",
        label: false,
    },
    Category {
        id: "f-a2-trash",
        aid: "a2",
        role: "trash",
        path: "[Gmail]/Trash",
        label: false,
    },
    Category {
        id: "l-a2-important",
        aid: "a2",
        role: "important",
        path: "[Gmail]/Important",
        label: true,
    },
    Category {
        id: "l-a2-travel",
        aid: "a2",
        role: "",
        path: "Travel",
        label: true,
    },
];

fn category(id: &str) -> &'static Category {
    CATEGORIES.iter().find(|c| c.id == id).unwrap()
}

const NAMES: &[&str] = &[
    "Ann Smith",
    "Bob Jones",
    "Anna Lee",
    "Bobby Fischer",
    "Carol King",
    "Dan Brown",
    "Annette Moss",
    "Eve Polastri",
    "Frank Ocean",
    "Grace Hopper",
    "Hal Abelson",
    "Ida Wells",
    "Jo Ann Marsh",
    "Ken Thompson",
    "Lea Bob",
    "Mo Farah",
    "Ned Kelly",
    "Ola Nordmann",
    "Pia Ann",
    "Quinn Fabray",
];

fn thread_categories(i: i64) -> (Vec<&'static str>, bool) {
    let mut cats = Vec::new();
    if i <= 24 {
        if i == 23 {
            cats.push("f-a1-spam");
        } else {
            match i % 6 {
                0 => cats.push("f-a1-trash"),
                4 => cats.push("f-a1-sent"),
                5 => cats.extend(["f-a1-inbox", "f-a1-work"]),
                _ => cats.push("f-a1-inbox"),
            }
        }
    } else {
        cats.push(if i % 4 == 0 {
            "f-a2-sent"
        } else {
            "f-a2-inbox"
        });
        if i % 3 == 0 {
            cats.push("l-a2-travel");
        }
        if i % 5 == 0 {
            cats.push("l-a2-important");
        }
        if i == 39 {
            cats = vec!["f-a2-trash"];
        }
    }
    let in_all_mail = !cats
        .iter()
        .any(|c| matches!(category(c).role, "trash" | "spam"));
    (cats, in_all_mail)
}

fn exec(conn: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) {
    conn.execute(sql, args).unwrap();
}

/// The database every captured query runs against.
fn build(dir: &Path) {
    let mut out = Vec::new();
    ms_store::migrate::migrate(dir, &ManualClock::new(BASE), &mut out).unwrap();
    let conn = schema::open_connection(dir).unwrap();
    let tx = conn.unchecked_transaction().unwrap();

    for c in CATEGORIES {
        let table = if c.label { "Label" } else { "Folder" };
        let data = json!({"__cls": table, "id": c.id, "aid": c.aid, "v": 1, "path": c.path,
                          "role": c.role, "localStatus": {}});
        exec(
            &tx,
            &format!(
                "INSERT INTO {table} (id, accountId, version, data, path, role) VALUES (?,?,1,?,?,?)"
            ),
            &[&c.id, &c.aid, &data.to_string(), &c.path, &c.role],
        );
    }

    for i in 1..=40i64 {
        let id = format!("t{i:02}");
        let aid = if i <= 24 { "a1" } else { "a2" };
        let lmrt = BASE + ((i * 13) % 40) * 1000 + 250;
        let lmst = BASE + ((i * 31) % 40) * 1000 + 500;
        let fmt = BASE + i * 1000;
        let unread = i % 3;
        let starred = i64::from(i % 4 == 0);
        let (cats, in_all_mail) = thread_categories(i);
        let refs = |label: bool| -> Vec<Value> {
            cats.iter()
                .map(|c| category(c))
                .filter(|c| c.label == label)
                .map(|c| json!({"id": c.id, "role": c.role, "path": c.path, "_refs": 1, "_u": 0}))
                .collect()
        };
        let mut metadata = Vec::new();
        if i % 5 == 2 {
            metadata.push(json!({"pluginId": "thread-snooze", "v": 1, "value": {"expiration": BASE + 90_000}}));
        }
        let subject = format!("Subject {i:02}");
        let data = json!({
            "__cls": "Thread", "id": id, "aid": aid, "v": 1, "subject": subject,
            "snippet": format!("snippet {i}"), "unread": unread, "starred": starred,
            "lmrt": lmrt, "lmst": lmst, "lmt": lmrt, "fmt": fmt, "inAllMail": in_all_mail,
            "folders": refs(false), "labels": refs(true),
            "participants": [{"email": format!("p{i}@x.test"), "name": format!("P {i}")}],
            "attachmentCount": 0, "metadata": metadata,
        });
        exec(
            &tx,
            "INSERT INTO Thread (id, accountId, version, data, subject, snippet, unread, starred, \
             firstMessageTimestamp, lastMessageTimestamp, lastMessageReceivedTimestamp, \
             lastMessageSentTimestamp, inAllMail, isSearchIndexed, participants, hasAttachments) \
             VALUES (?,?,1,?,?,?,?,?,?,?,?,?,?,1,'',0)",
            &[
                &id,
                &aid,
                &data.to_string(),
                &subject,
                &format!("snippet {i}"),
                &unread,
                &starred,
                &fmt,
                &lmrt,
                &lmrt,
                &lmst,
                &i64::from(in_all_mail),
            ],
        );
        for c in &cats {
            exec(
                &tx,
                "INSERT INTO ThreadCategory (id, value, inAllMail, unread, \
                 lastMessageReceivedTimestamp, lastMessageSentTimestamp) VALUES (?,?,?,?,?,?)",
                &[
                    &id,
                    c,
                    &i64::from(in_all_mail),
                    &i64::from(unread > 0),
                    &lmrt,
                    &lmst,
                ],
            );
        }
        if i % 5 == 2 {
            exec(
                &tx,
                "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) \
                 VALUES (?,?,'Thread','thread-snooze',?)",
                &[&id, &aid, &(BASE + 90_000)],
            );
        }
        let body = format!("words about topic {i}");
        exec(
            &tx,
            "INSERT INTO ThreadSearch (content_id, subject, to_, from_, categories, body) \
             VALUES (?,?,?,?,?,?)",
            &[
                &id,
                &subject,
                &"me@x.test",
                &format!("p{i}@x.test"),
                &cats.join(" "),
                &body,
            ],
        );

        for k in 1..=2i64 {
            let mid = format!("m{i:02}-{k}");
            let date = lmrt - (2 - k) * 100;
            let unread_msg = k == 2 && unread > 0;
            let mut metadata = Vec::new();
            let mut plugins = Vec::new();
            if k == 2 {
                match i % 7 {
                    1 => plugins.push("open-tracking"),
                    2 => plugins.push("link-tracking"),
                    3 => plugins.extend(["open-tracking", "link-tracking"]),
                    _ => {}
                }
            }
            for p in &plugins {
                metadata.push(json!({"pluginId": p, "v": 1, "value": {"uid": mid}}));
            }
            let data = json!({
                "__cls": "Message", "id": mid, "aid": aid, "v": 1, "threadId": id,
                "hMsgId": format!("<{mid}@x.test>"), "subject": format!("Subject {i:02}"),
                "date": date, "draft": false, "unread": unread_msg, "starred": starred == 1 && k == 2,
                "from": [{"email": format!("p{i}@x.test"), "name": format!("P {i}")}],
                "to": [{"email": "me@x.test"}], "cc": [], "bcc": [], "replyTo": [], "files": [],
                "events": [], "snippet": "", "metadata": metadata,
            });
            exec(
                &tx,
                "INSERT INTO Message (id, accountId, version, data, headerMessageId, subject, date, \
                 draft, unread, starred, threadId) VALUES (?,?,1,?,?,?,?,0,?,?,?)",
                &[
                    &mid, &aid, &data.to_string(), &format!("<{mid}@x.test>"),
                    &format!("Subject {i:02}"), &date, &i64::from(unread_msg),
                    &i64::from(starred == 1 && k == 2), &id,
                ],
            );
            for p in &plugins {
                exec(
                    &tx,
                    "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value) \
                     VALUES (?,?,'Message',?)",
                    &[&mid, &aid, p],
                );
            }
            if i == 4 && k == 1 {
                exec(
                    &tx,
                    "INSERT INTO MessageBody (id, value) VALUES (?, NULL)",
                    &[&mid],
                );
            } else if k == 1 || i % 2 == 0 {
                exec(
                    &tx,
                    "INSERT INTO MessageBody (id, value, fetchedAt) VALUES (?,?,?)",
                    &[&mid, &format!("<p>Body {mid}</p>"), &BASE],
                );
            }
        }
    }

    for n in 1..=6i64 {
        let id = format!("d{n:02}");
        let aid = if n <= 4 { "a1" } else { "a2" };
        let thread = if n == 1 { "t06" } else { "" };
        let date = BASE + 100_000 + n * 10;
        let metadata: Vec<Value> = if n == 3 {
            vec![json!({"pluginId": "open-tracking", "v": 1, "value": {}})]
        } else {
            vec![]
        };
        let data = json!({
            "__cls": "Message", "id": id, "aid": aid, "v": 1, "threadId": thread,
            "hMsgId": format!("<{id}@x.test>"), "subject": format!("Draft {n}"), "date": date,
            "draft": true, "unread": false, "starred": false, "from": [{"email": "me@x.test"}],
            "to": [], "cc": [], "bcc": [], "replyTo": [], "files": [], "events": [],
            "metadata": metadata,
        });
        exec(
            &tx,
            "INSERT INTO Message (id, accountId, version, data, headerMessageId, subject, date, \
             draft, unread, starred, threadId) VALUES (?,?,1,?,?,?,?,1,0,0,?)",
            &[
                &id,
                &aid,
                &data.to_string(),
                &format!("<{id}@x.test>"),
                &format!("Draft {n}"),
                &date,
                &thread,
            ],
        );
        if n == 3 {
            exec(
                &tx,
                "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value) \
                 VALUES (?,?,'Message','open-tracking')",
                &[&id, &aid],
            );
        }
        if n % 2 == 0 {
            exec(
                &tx,
                "INSERT INTO MessageBody (id, value) VALUES (?,?)",
                &[&id, &format!("<p>draft {n}</p>")],
            );
        }
    }

    for (n, name) in NAMES.iter().enumerate() {
        let n = n as i64 + 1;
        let id = format!("c{n:02}");
        let aid = if n <= 12 { "a1" } else { "a2" };
        let email = format!("c{n:02}@x.test");
        let refs = (n * 7) % 20;
        let hidden = n % 9 == 0;
        let source = if n % 5 == 0 { "carddav" } else { "mail" };
        let groups: Vec<&str> = match n {
            1 => vec!["g1"],
            2 => vec!["g1", "g2"],
            3 | 4 => vec!["g2"],
            14 => vec!["g3"],
            _ => vec![],
        };
        let book = (source == "carddav").then_some(if aid == "a1" { "b1" } else { "b2" });
        let data = json!({"__cls": "Contact", "id": id, "aid": aid, "v": 1, "email": email,
                          "name": name, "refs": refs, "h": hidden, "s": source, "gis": groups,
                          "info": {}});
        exec(
            &tx,
            "INSERT INTO Contact (id, data, accountId, email, version, refs, hidden, source, bookId) \
             VALUES (?,?,?,?,1,?,?,?,?)",
            &[&id, &data.to_string(), &aid, &email, &refs, &i64::from(hidden), &source, &book],
        );
        exec(
            &tx,
            "INSERT INTO ContactSearch (content_id, content) VALUES (?,?)",
            &[&id, &format!("{name} {email}")],
        );
        for g in groups {
            exec(
                &tx,
                "INSERT INTO ContactContactGroup (id, value) VALUES (?,?)",
                &[&id, &g],
            );
        }
    }
    for (id, aid, name) in [
        ("g1", "a1", "Friends"),
        ("g2", "a1", "Work"),
        ("g3", "a2", "Family"),
    ] {
        let data = json!({"__cls": "ContactGroup", "id": id, "aid": aid, "v": 1, "name": name});
        exec(
            &tx,
            "INSERT INTO ContactGroup (id, accountId, bookId, data, version, name) VALUES (?,?,?,?,1,?)",
            &[&id, &aid, &if aid == "a1" { "b1" } else { "b2" }, &data.to_string(), &name],
        );
    }
    for (id, aid) in [("b1", "a1"), ("b2", "a2")] {
        let data =
            json!({"__cls": "ContactBook", "id": id, "aid": aid, "v": 1, "source": "carddav"});
        exec(
            &tx,
            "INSERT INTO ContactBook (id, accountId, data, version) VALUES (?,?,?,1)",
            &[&id, &aid, &data.to_string()],
        );
    }

    for (id, aid, name) in [
        ("cal1", "a1", "Home"),
        ("cal2", "a1", "Work"),
        ("cal3", "a2", "Team"),
    ] {
        let data = json!({"__cls": "Calendar", "id": id, "aid": aid, "v": 1, "name": name,
                          "color": "#336699", "read_only": false});
        exec(
            &tx,
            "INSERT INTO Calendar (id, data, accountId) VALUES (?,?,?)",
            &[&id, &data.to_string(), &aid],
        );
    }
    for n in 1..=15i64 {
        let id = format!("e{n:02}");
        let cal = match n {
            1 | 3 | 4 => "cal1",
            2 => "cal2",
            _ => ["cal1", "cal2", "cal3"][(n % 3) as usize],
        };
        let aid = if cal == "cal3" { "a2" } else { "a1" };
        let uid = if n == 4 {
            "uid-03".to_owned()
        } else {
            format!("uid-{n:02}")
        };
        let rid = if n == 4 { "20231115T100000Z" } else { "" };
        let start = BASE + n * 5000;
        let end = if n == 10 {
            start + 80_000
        } else {
            start + 3000
        };
        let title = if n % 3 == 0 {
            format!("Lunch with {n}")
        } else {
            format!("Meeting {n}")
        };
        let data = json!({"__cls": "Event", "id": id, "aid": aid, "v": 1, "cid": cal,
                          "ics": format!("BEGIN:VCALENDAR\r\nSUMMARY:{title}\r\nEND:VCALENDAR"),
                          "icsuid": uid, "rid": rid, "rs": start, "re": end, "status": "confirmed"});
        exec(
            &tx,
            "INSERT INTO Event (id, data, accountId, etag, calendarId, recurrenceStart, \
             recurrenceEnd, icsuid, recurrenceId) VALUES (?,?,?,'e',?,?,?,?,?)",
            &[&id, &data.to_string(), &aid, &cal, &start, &end, &uid, &rid],
        );
        exec(
            &tx,
            "INSERT INTO EventSearch (content_id, title, description, location, participants) \
             VALUES (?,?,'','Room 1','')",
            &[&id, &title],
        );
    }

    for (n, (aid, status)) in [
        ("a1", "local"),
        ("a1", "complete"),
        ("a2", "remote"),
        ("a2", "cancelled"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("k{}", n + 1);
        let data =
            json!({"__cls": "ChangeStarredTask", "id": id, "aid": aid, "v": 1, "status": status});
        exec(
            &tx,
            "INSERT INTO Task (id, version, data, accountId, status) VALUES (?,1,?,?,?)",
            &[&id, &data.to_string(), &aid, &status],
        );
    }
    for n in 1..=3i64 {
        let id = format!("file{n}");
        let data = json!({"__cls": "File", "id": id, "aid": "a1", "v": 1,
                          "filename": format!("f{n}.pdf"), "size": n * 100,
                          "contentType": "application/pdf", "messageId": format!("m0{n}-1")});
        exec(
            &tx,
            "INSERT INTO File (id, version, data, accountId, filename) VALUES (?,1,?,'a1',?)",
            &[&id, &data.to_string(), &format!("f{n}.pdf")],
        );
    }
    tx.commit().unwrap();
}

struct Captured {
    name: String,
    query: Value,
    sql: String,
}

fn captured() -> Vec<Captured> {
    let file: Value = serde_json::from_str(include_str!("fixtures/model_queries.json")).unwrap();
    file["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| Captured {
            name: q["name"].as_str().unwrap().to_owned(),
            query: q["query"].clone(),
            sql: q["sql"].as_str().unwrap().to_owned(),
        })
        .collect()
}

#[derive(Debug, PartialEq)]
enum Result_ {
    Rows(Vec<Value>),
    Ids(Vec<String>),
    Count(i64),
}

/// What the desktop reads with `Query.sql()` and `inflateResult`.
fn desktop(conn: &Connection, c: &Captured) -> Result_ {
    let q = &c.query;
    let mut stmt = conn
        .prepare(&c.sql)
        .unwrap_or_else(|e| panic!("{}: {e}", c.name));
    let mut rows = stmt.query([]).unwrap();
    if q["count"] == true {
        return Result_::Count(match rows.next().unwrap() {
            Some(r) => r.get(0).unwrap(),
            None => 0,
        });
    }
    if q["idsOnly"] == true {
        let mut ids = Vec::new();
        while let Some(r) = rows.next().unwrap() {
            ids.push(r.get(0).unwrap());
        }
        return Result_::Ids(ids);
    }
    let includes: Vec<String> = q["include"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let mut out = Vec::new();
    while let Some(r) = rows.next().unwrap() {
        let data: String = r.get(0).unwrap();
        let mut model: Value = serde_json::from_str(&data).unwrap();
        for (i, name) in includes.iter().enumerate() {
            let v: Option<String> = r.get(1 + i).unwrap();
            let v = v
                .filter(|s| s != NULL_PLACEHOLDER)
                .map_or(Value::Null, Value::String);
            model.as_object_mut().unwrap().insert(name.clone(), v);
        }
        out.push(model);
    }
    Result_::Rows(out)
}

fn engine(store: &ReadStore, c: &Captured) -> Result_ {
    let q: ModelQuery =
        serde_json::from_value(c.query.clone()).unwrap_or_else(|e| panic!("{}: {e}", c.name));
    match store
        .model_query(&q)
        .unwrap_or_else(|e| panic!("{}: {e}", c.name))
    {
        QueryOutput::Rows { rows, .. } => Result_::Rows(
            rows.into_iter()
                .map(|mut m| {
                    // `hidden` is the engine's addition to Message JSON.
                    m.as_object_mut().unwrap().remove("hidden");
                    m
                })
                .collect(),
        ),
        QueryOutput::Ids(ids) => Result_::Ids(ids),
        QueryOutput::Count(n) => Result_::Count(n),
    }
}

fn unordered(r: Result_) -> Result_ {
    let key = |v: &Value| v["id"].as_str().unwrap_or_default().to_owned();
    match r {
        Result_::Rows(mut rows) => {
            rows.sort_by_key(key);
            Result_::Rows(rows)
        }
        Result_::Ids(mut ids) => {
            ids.sort();
            Result_::Ids(ids)
        }
        other => other,
    }
}

fn len(r: &Result_) -> i64 {
    match r {
        Result_::Rows(r) => r.len() as i64,
        Result_::Ids(i) => i.len() as i64,
        Result_::Count(n) => *n,
    }
}

#[test]
fn engine_returns_what_the_desktop_reads() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let store = ReadStore::open(dir.path()).unwrap();
    let conn = Connection::open(dir.path().join("edgehill.db")).unwrap();
    let queries = captured();
    assert!(queries.len() >= 60, "{} queries captured", queries.len());
    for c in &queries {
        let (mut want, mut got) = (desktop(&conn, c), engine(&store, c));
        let ordered = !c.query["order"].as_array().unwrap().is_empty();
        if !ordered {
            want = unordered(want);
            got = unordered(got);
        }
        assert_eq!(got, want, "{}: {}", c.name, c.sql);
        if c.name != "empty-perspective" {
            assert!(len(&want) > 0, "{} reads nothing from the fixture", c.name);
        }
    }
}

fn plan(conn: &Connection, sql: &str, params: &[rusqlite::types::Value]) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            r.get::<_, String>(3)
        })
        .unwrap();
    rows.map(Result::unwrap).collect::<Vec<_>>().join("\n")
}

/// Statistics shaped like a large mailbox: a few hundred thousand category rows over a
/// few dozen categories.
fn large_mailbox_stats(dir: &Path) {
    let conn = Connection::open(dir.join("edgehill.db")).unwrap();
    conn.execute_batch(
        "ANALYZE;
         DELETE FROM sqlite_stat1 WHERE tbl IN ('ThreadCategory', 'Thread');
         INSERT INTO sqlite_stat1 (tbl, idx, stat) VALUES
           ('ThreadCategory', 'ThreadCategory_val_id', '300000 10000 1'),
           ('ThreadCategory', 'sqlite_autoindex_ThreadCategory_1', '300000 2 1'),
           ('ThreadCategory', 'ThreadListCategoryIndex', '300000 2 1 1 1 1'),
           ('ThreadCategory', 'ThreadListCategorySentIndex', '300000 2 1 1 1 1'),
           ('Thread', 'sqlite_autoindex_Thread_1', '150000 1');",
    )
    .unwrap();
}

#[test]
fn thread_list_plan_matches_the_desktop() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let plans = |name: &str| {
        let conn = Connection::open(dir.path().join("edgehill.db")).unwrap();
        let c = captured().into_iter().find(|c| c.name == name).unwrap();
        let q: ModelQuery = serde_json::from_value(c.query.clone()).unwrap();
        let compiled = compile(&q).unwrap();
        (
            plan(&conn, &compiled.sql, &compiled.params),
            plan(&conn, &c.sql, &[]),
        )
    };
    let names = [
        "inbox-page-1",
        "inbox-page-2",
        "unified-inbox",
        "sent",
        "starred-all-accounts",
    ];
    for name in names {
        let (ours, desktop) = plans(name);
        assert_eq!(ours, desktop, "{name}");
    }
    // The plan for a thread list is the desktop's own: the category subselect searches
    // ThreadCategory by value and looks threads up by id, never scanning Thread. SQLite
    // picks `ThreadCategory_val_id` over `ThreadListCategoryIndex` for it, with or without
    // statistics, for the desktop's SQL as much as for ours.
    large_mailbox_stats(dir.path());
    for name in names {
        let (ours, desktop) = plans(name);
        assert_eq!(ours, desktop, "{name}");
        if name != "starred-all-accounts" {
            assert!(
                ours.contains("SEARCH ThreadCategory USING"),
                "{name}:\n{ours}"
            );
            assert!(!ours.contains("SCAN Thread"), "{name}:\n{ours}");
        }
    }
    // The web's lists, by cursor and per account, keep that plan: the deduped subselect and
    // the account narrowing don't scan Thread or ThreadCategory.
    let conn = Connection::open(dir.path().join("edgehill.db")).unwrap();
    for accounts in [Value::Null, json!(["a2"])] {
        let q: ModelQuery = serde_json::from_value(json!({
            "class": "Thread", "accounts": accounts,
            "where": [{"attr": "categories", "op": "containsAny", "value": ["f-a1-inbox", "f-a2-inbox"]},
                      {"attr": "inAllMail", "op": "=", "value": true}],
            "order": [{"attr": "lastMessageReceivedTimestamp", "dir": "desc"}],
            "limit": 50, "cursor": "[1700000000,\"t09\"]",
        }))
        .unwrap();
        let c = compile(&q).unwrap();
        let ours = plan(&conn, &c.sql, &c.params);
        assert!(
            ours.contains("SEARCH ThreadCategory USING"),
            "{accounts}:\n{ours}"
        );
        assert!(!ours.contains("SCAN Thread"), "{accounts}:\n{ours}");
    }
}

#[test]
fn contact_search_across_accounts() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let store = ReadStore::open(dir.path()).unwrap();
    let q: ModelQuery = serde_json::from_value(json!({
        "class": "Contact",
        "search": {"text": "ann", "grammar": false},
        "where": [{"attr": "hidden", "op": "=", "value": false}],
        "order": [{"attr": "refs", "dir": "desc"}],
        "limit": 10,
    }))
    .unwrap();
    let QueryOutput::Rows { rows, .. } = store.model_query(&q).unwrap() else {
        panic!("rows")
    };
    let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    // Prefix matches on any word, both accounts, hidden ones left out (c09 is hidden but
    // isn't an "ann"), most referenced first.
    let mut want: Vec<(i64, &str)> = NAMES
        .iter()
        .enumerate()
        .map(|(i, n)| ((i as i64 + 1) * 7 % 20, *n))
        .filter(|(_, n)| n.split(' ').any(|w| w.to_lowercase().starts_with("ann")))
        .collect();
    want.sort_by(|a, b| b.0.cmp(&a.0));
    assert_eq!(names, want.iter().map(|(_, n)| *n).collect::<Vec<_>>());
    let accounts: std::collections::BTreeSet<&str> =
        rows.iter().map(|r| r["aid"].as_str().unwrap()).collect();
    assert_eq!(accounts.len(), 2);
}

#[test]
fn accounts_and_cursor_pages() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let store = ReadStore::open(dir.path()).unwrap();
    let run = |v: Value| {
        let q: ModelQuery = serde_json::from_value(v).unwrap();
        match store.model_query(&q).unwrap() {
            QueryOutput::Rows { rows, next_cursor } => (
                rows.iter()
                    .map(|r| r["id"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>(),
                next_cursor,
            ),
            other => panic!("{other:?}"),
        }
    };
    let all = |accounts: Value, cats: Value| {
        let (ids, next) = run(json!({
            "class": "Thread", "accounts": accounts,
            "where": [{"attr": "categories", "op": "containsAny", "value": cats},
                      {"attr": "inAllMail", "op": "=", "value": true}],
            "limit": 1000,
        }));
        assert_eq!(next, None);
        ids
    };
    let unified = all(Value::Null, json!(["f-a1-inbox", "f-a2-inbox"]));
    // The account filter narrows the category subselect.
    let only_a2 = all(json!(["a2"]), json!(["f-a1-inbox", "f-a2-inbox"]));
    assert_eq!(
        only_a2,
        unified
            .iter()
            .filter(|t| t.as_str() >= "t25")
            .cloned()
            .collect::<Vec<_>>()
    );
    assert!(all(json!([]), json!(["f-a1-inbox"])).is_empty());

    // Pages of 4 by cursor cover the same threads in the same order, on the subselect and
    // on the join (distinct).
    for distinct in [false, true] {
        let mut cursor = String::new();
        let mut paged = Vec::new();
        loop {
            let (ids, next) = run(json!({
                "class": "Thread",
                "where": [{"attr": "categories", "op": "containsAny", "value": ["f-a1-inbox", "f-a2-inbox"]},
                          {"attr": "inAllMail", "op": "!=", "value": 0}],
                "limit": 4, "cursor": cursor, "distinct": distinct,
            }));
            assert!(ids.len() <= 4);
            paged.extend(ids);
            match next {
                Some(Some(c)) => cursor = c,
                Some(None) => break,
                None => panic!("a cursor query answers nextCursor"),
            }
        }
        assert_eq!(paged, unified, "distinct {distinct}");
    }

    // Starred across accounts, by cursor on the plain table, oldest first.
    let mut cursor = String::new();
    let mut paged = Vec::new();
    loop {
        let (ids, next) = run(json!({
            "class": "Thread",
            "where": {"attr": "starred", "op": "=", "value": true},
            "order": [{"attr": "lastMessageReceivedTimestamp", "dir": "asc"}],
            "limit": 3, "cursor": cursor,
        }));
        paged.extend(ids);
        match next.unwrap() {
            Some(c) => cursor = c,
            None => break,
        }
    }
    let conn = Connection::open(dir.path().join("edgehill.db")).unwrap();
    let want: Vec<String> = conn
        .prepare(
            "SELECT id FROM Thread WHERE starred != 0 ORDER BY lastMessageReceivedTimestamp ASC",
        )
        .unwrap()
        .query_map(params![], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(paged, want);
}

/// Threads in more than one listed category, and categories of accounts the query doesn't
/// read, still give full pages by cursor and by offset on the thread list's subselect.
#[test]
fn overlapping_categories_and_accounts_page_fully() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let store = ReadStore::open(dir.path()).unwrap();
    let conn = Connection::open(dir.path().join("edgehill.db")).unwrap();
    let rows = |v: Value| {
        let q: ModelQuery = serde_json::from_value(v).unwrap();
        match store.model_query(&q).unwrap() {
            QueryOutput::Rows { rows, next_cursor } => (
                rows.iter()
                    .map(|r| r["id"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>(),
                next_cursor,
            ),
            other => panic!("{other:?}"),
        }
    };
    let base = |accounts: Value, cats: &[&str]| {
        json!({
            "class": "Thread", "accounts": accounts,
            "where": [{"attr": "categories", "op": "containsAny", "value": cats},
                      {"attr": "inAllMail", "op": "=", "value": true}],
        })
    };
    let by_cursor = |q: Value, limit: i64| {
        let mut cursor = String::new();
        let mut paged = Vec::new();
        loop {
            let mut q = q.clone();
            q["limit"] = json!(limit);
            q["cursor"] = json!(cursor);
            let (ids, next) = rows(q);
            paged.extend(ids);
            match next.expect("a cursor query answers nextCursor") {
                Some(c) => cursor = c,
                None => return paged,
            }
        }
    };
    let by_offset = |q: Value, limit: i64| {
        let mut paged = Vec::new();
        loop {
            let mut q = q.clone();
            q["limit"] = json!(limit);
            q["offset"] = json!(paged.len());
            let (ids, _) = rows(q);
            let done = (ids.len() as i64) < limit;
            paged.extend(ids);
            if done {
                return paged;
            }
        }
    };
    let want = |accounts: &[&str], cats: &[&str]| -> Vec<String> {
        let list = |v: &[&str]| {
            v.iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let account_clause = if accounts.is_empty() {
            String::new()
        } else {
            format!(" AND accountId IN ({})", list(accounts))
        };
        conn.prepare(&format!(
            "SELECT id FROM Thread WHERE inAllMail != 0 AND id IN \
             (SELECT id FROM ThreadCategory WHERE value IN ({})){account_clause} \
             ORDER BY lastMessageReceivedTimestamp DESC",
            list(cats)
        ))
        .unwrap()
        .query_map(params![], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };

    // Threads 5, 11, 17 and 23 are in both a1's inbox and work folders.
    let overlap = ["f-a1-inbox", "f-a1-work"];
    let expected = want(&[], &overlap);
    assert!(expected.len() > 4);
    for limit in [1, 2, 3, 4] {
        assert_eq!(
            by_cursor(base(Value::Null, &overlap), limit),
            expected,
            "limit {limit}"
        );
    }

    // a2 only, listing categories of both accounts, a1's first in the order.
    let both = ["f-a1-inbox", "f-a1-work", "f-a2-inbox", "l-a2-travel"];
    let expected = want(&["a2"], &both);
    assert!(expected.len() > 4);
    for limit in [1, 2, 3, 4] {
        assert_eq!(
            by_cursor(base(json!(["a2"]), &both), limit),
            expected,
            "cursor, limit {limit}"
        );
        // Offset pages follow the desktop's subselect, which can repeat a thread on two
        // pages or cut a page short when its categories overlap; with one category per
        // thread in a2 here the offset pages are exact.
        let a2_disjoint = ["f-a1-inbox", "f-a2-inbox"];
        assert_eq!(
            by_offset(base(json!(["a2"]), &a2_disjoint), limit),
            want(&["a2"], &a2_disjoint),
            "offset, limit {limit}"
        );
    }
}

/// Every allowlisted column and table exists in the schema, except the one the desktop
/// declares queryable without the engine ever writing it.
#[test]
fn allowlist_matches_the_schema() {
    use ms_store::model_query::{allowlist, AttrDef};
    let dir = tempfile::tempdir().unwrap();
    build(dir.path());
    let conn = Connection::open(dir.path().join("edgehill.db")).unwrap();
    let columns = |table: &str| -> Vec<String> {
        conn.prepare(&format!("PRAGMA table_info(`{table}`)"))
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let mut missing = Vec::new();
    for (class, def) in &allowlist().classes {
        let own = columns(class);
        assert!(!own.is_empty(), "no table {class}");
        for (attr, a) in &def.attributes {
            let (table, column) = match a {
                AttrDef::String { column }
                | AttrDef::Number { column }
                | AttrDef::Boolean { column }
                | AttrDef::Datetime { column }
                | AttrDef::Object { column } => (class.as_str(), column.as_str()),
                AttrDef::Collection { join_table, .. } => (join_table.as_str(), "value"),
                AttrDef::JoinedData { table } => (table.as_str(), "value"),
            };
            if !columns(table).iter().any(|c| c == column) {
                missing.push(format!("{class}.{attr}"));
            }
        }
    }
    assert_eq!(missing, ["Event.status"]);
}
