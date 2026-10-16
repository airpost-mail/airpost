// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The socket read methods (docs/engine-protocol-plan.md section 3) on the debug binary,
//! against a database seeded before launch and an account whose servers refuse connections.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use ms_ipc::{code, method, Client, ThreadsListResult};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const ACCOUNT_ID: &str = "c0ffee-harness";
const OTHER: &str = "other-acc";
const ACCOUNT: &str = r#"{"id":"c0ffee-harness","emailAddress":"me@example.com","provider":"imap","settings":{"imap_host":"127.0.0.1","imap_port":1,"imap_username":"me","imap_password":"x","imap_security":"none","smtp_host":"127.0.0.1","smtp_port":1,"smtp_username":"me","smtp_password":"x","smtp_security":"none","imap_allow_insecure_ssl":false,"smtp_allow_insecure_ssl":false}}"#;
const THREADS: usize = 120;
const BASE_TS: i64 = 1_600_000_000;

fn mailsync(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mailsync"));
    c.env("CONFIG_DIR_PATH", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn db(config: &Path) -> Connection {
    let c = Connection::open(config.join("edgehill.db")).unwrap();
    c.busy_timeout(Duration::from_secs(60)).unwrap();
    c
}

fn thread_id(i: usize) -> String {
    format!("t:{i:03}")
}

/// Three threads share each timestamp, so paging has to break ties on the id.
fn thread_ts(i: usize) -> i64 {
    BASE_TS + (i / 3) as i64
}

fn insert_thread(c: &Connection, account: &str, id: &str, ts: i64, categories: &[&str]) {
    insert_thread_in(c, account, id, ts, categories, true);
}

/// A thread whose `ThreadCategory` rows carry `in_all_mail`.
fn insert_thread_in(
    c: &Connection,
    account: &str,
    id: &str,
    ts: i64,
    categories: &[&str],
    in_all_mail: bool,
) {
    let data = json!({"id": id, "aid": account, "__cls": "Thread", "v": 1, "lmt": ts,
        "subject": format!("Subject {id}")});
    c.execute(
        "INSERT INTO Thread (id, accountId, version, data, subject, lastMessageTimestamp, \
         lastMessageReceivedTimestamp) VALUES (?, ?, 1, ?, ?, ?, ?)",
        params![
            id,
            account,
            data.to_string(),
            data["subject"].as_str(),
            ts,
            ts
        ],
    )
    .unwrap();
    for cat in categories {
        c.execute(
            "INSERT INTO ThreadCategory (id, value, inAllMail, unread, \
             lastMessageReceivedTimestamp, lastMessageSentTimestamp) VALUES (?, ?, ?, 0, ?, ?)",
            params![id, cat, in_all_mail, ts, ts],
        )
        .unwrap();
    }
}

fn insert_category(c: &Connection, table: &str, id: &str, path: &str, role: &str) {
    let data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": table, "v": 1, "path": path,
        "role": role});
    c.execute(
        &format!(
            "INSERT INTO {table} (id, accountId, version, data, path, role) \
             VALUES (?, ?, 1, ?, ?, ?)"
        ),
        params![id, ACCOUNT_ID, data.to_string(), path, role],
    )
    .unwrap();
}

fn insert_message(c: &Connection, id: &str, thread: &str, date: i64, files: Value) {
    let data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": "Message", "v": 1,
        "threadId": thread, "date": date, "files": files});
    c.execute(
        "INSERT INTO Message (id, accountId, version, data, date, threadId) \
         VALUES (?, ?, 1, ?, ?, ?)",
        params![id, ACCOUNT_ID, data.to_string(), date, thread],
    )
    .unwrap();
}

fn insert_contact(
    c: &Connection,
    account: &str,
    id: &str,
    email: &str,
    name: &str,
    refs: i64,
    hidden: bool,
) {
    let data = json!({"id": id, "aid": account, "__cls": "Contact", "v": 1, "email": email,
        "name": name, "refs": refs});
    c.execute(
        "INSERT INTO Contact (id, data, accountId, email, version, refs, hidden) \
         VALUES (?, ?, ?, ?, 1, ?, ?)",
        params![
            id,
            data.to_string().into_bytes(),
            account,
            email,
            refs,
            hidden
        ],
    )
    .unwrap();
    let content = format!("{} {name}", email.replacen('@', " ", 1));
    c.execute(
        "INSERT INTO ContactSearch (content_id, content) VALUES (?, ?)",
        params![id, content],
    )
    .unwrap();
}

/// Even threads are in the inbox folder, odd ones in Archive; every fifth also has the
/// Work label. `t:000` has two messages, the first with a body and a file.
fn seed(config: &Path) {
    let c = db(config);
    insert_category(&c, "Folder", "f-inbox", "INBOX", "inbox");
    insert_category(&c, "Folder", "f-arch", "Archive", "archive");
    insert_category(&c, "Label", "l-work", "Work", "");
    insert_category(&c, "Label", "l-imp", "[Gmail]/Important", "important");
    c.execute(
        "INSERT INTO ThreadCounts (categoryId, unread, total) VALUES ('f-inbox', 2, 60)",
        [],
    )
    .unwrap();
    for i in 0..THREADS {
        let folder = if i % 2 == 0 { "f-inbox" } else { "f-arch" };
        let mut cats = vec![folder];
        if i % 5 == 0 {
            cats.push("l-work");
        }
        insert_thread(&c, ACCOUNT_ID, &thread_id(i), thread_ts(i), &cats);
    }
    insert_thread(&c, OTHER, "t:other", BASE_TS + 10_000, &["f-inbox"]);

    insert_message(
        &c,
        "m-1",
        "t:000",
        BASE_TS,
        json!([{"id": "file-1", "filename": "a.pdf", "messageId": "m-1"}]),
    );
    insert_message(&c, "m-2", "t:000", BASE_TS + 5, json!([]));
    insert_message(&c, "m-3", "t:001", BASE_TS, json!([]));
    c.execute(
        "INSERT INTO MessageBody (id, value, fetchedAt) VALUES ('m-1', '<p>Hello body</p>', 0)",
        [],
    )
    .unwrap();
    // The placeholder a pending fetch leaves.
    c.execute(
        "INSERT INTO MessageBody (id, value) VALUES ('m-3', NULL)",
        [],
    )
    .unwrap();

    for (id, subject, body) in [
        ("t:001", "Quarterly report", "numbers inside"),
        (
            "t:002",
            r#"say "hello" NEAR(world)"#,
            "a -minus sign and col:on",
        ),
        ("t:other", "Quarterly report", "another account"),
    ] {
        c.execute(
            "INSERT INTO ThreadSearch (content_id, subject, to_, from_, categories, body) \
             VALUES (?, ?, '', '', '', ?)",
            params![id, subject, body],
        )
        .unwrap();
    }

    insert_contact(
        &c,
        ACCOUNT_ID,
        "c-1",
        "ann@example.com",
        "Ann Smith",
        5,
        false,
    );
    insert_contact(
        &c,
        ACCOUNT_ID,
        "c-2",
        "annie@example.org",
        "Annie",
        9,
        false,
    );
    insert_contact(
        &c,
        ACCOUNT_ID,
        "c-3",
        "ann.hidden@example.com",
        "Ann",
        50,
        true,
    );
    insert_contact(&c, OTHER, "c-4", "ann@other.com", "Ann Other", 99, false);

    for (id, account, status) in [
        ("task-done", ACCOUNT_ID, "complete"),
        ("task-other", OTHER, "complete"),
    ] {
        let data = json!({"id": id, "aid": account, "__cls": "ChangeStarredTask", "v": 1,
            "status": status, "error": null});
        c.execute(
            "INSERT INTO Task (id, version, data, accountId, status) VALUES (?, 1, ?, ?, ?)",
            params![id, data.to_string(), account, status],
        )
        .unwrap();
    }
}

struct Engine {
    dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
}

impl Engine {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let status = mailsync(dir.path())
            .args(["--mode", "migrate"])
            .output()
            .unwrap()
            .status;
        assert!(status.success());
        seed(dir.path());
        let socket = dir.path().join("run").join("e.sock");
        let mut child = mailsync(dir.path())
            .args(["--mode", "sync", "--identity", "null", "--account", ACCOUNT])
            .arg("--socket")
            .arg(&socket)
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        if line.is_empty() {
            let mut stderr = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
            panic!("exited with {:?}: {stderr}", child.wait().unwrap());
        }
        assert!(line.contains("listening"), "{line}");
        Engine { dir, socket, child }
    }

    fn hello(&self) -> Client {
        let mut c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        c.hello("read-api-test/1").unwrap();
        c
    }

    /// Kills the engine and checks that stderr stayed empty.
    fn stop(mut self) {
        assert!(self.child.try_wait().unwrap().is_none(), "exited early");
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert_eq!(stderr, "");
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn ids(models: &[Value]) -> Vec<String> {
    models
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The ids of `threads`, newest first by `(lastMessageTimestamp, id)`.
fn expected_order(threads: impl Iterator<Item = usize>) -> Vec<String> {
    let mut v: Vec<(i64, String)> = threads.map(|i| (thread_ts(i), thread_id(i))).collect();
    v.sort_by(|a, b| b.cmp(a));
    v.into_iter().map(|(_, id)| id).collect()
}

fn list(c: &mut Client, params: Value) -> ThreadsListResult {
    c.call_as(method::THREADS_LIST, params).unwrap()
}

/// Every page of `params` until `next` is null.
fn list_all(c: &mut Client, mut params: Value) -> (Vec<String>, usize) {
    let mut all = Vec::new();
    let mut pages = 0;
    loop {
        let page = list(c, params.clone());
        pages += 1;
        all.extend(ids(&page.threads));
        match page.next {
            Some(next) => params["before"] = json!(next),
            None => return (all, pages),
        }
    }
}

fn error_code(c: &mut Client, m: &str, p: Value) -> Option<i64> {
    c.call(m, p).unwrap_err().code()
}

#[test]
fn folders_and_labels() {
    let e = Engine::start();
    let mut c = e.hello();
    let r: ms_ipc::FoldersListResult = c
        .call_as(method::FOLDERS_LIST, json!({ "accountId": ACCOUNT_ID }))
        .unwrap();
    assert_eq!(ids(&r.folders), ["f-arch", "f-inbox"]);
    assert_eq!(r.folders[1]["role"], "inbox");
    assert_eq!(r.counts.len(), 1);
    assert_eq!(
        r.counts["f-inbox"],
        ms_ipc::CategoryCounts {
            unread: 2,
            total: 60
        }
    );
    let r: ms_ipc::LabelsListResult = c
        .call_as(method::LABELS_LIST, json!({ "accountId": ACCOUNT_ID }))
        .unwrap();
    assert_eq!(ids(&r.labels), ["l-work", "l-imp"]);
    assert!(r.counts.is_empty());
    assert_eq!(
        error_code(&mut c, method::FOLDERS_LIST, json!({ "accountId": OTHER })),
        Some(code::UNKNOWN_ACCOUNT)
    );
    assert_eq!(
        error_code(&mut c, method::LABELS_LIST, json!({})),
        Some(code::INVALID_PARAMS)
    );
    e.stop();
}

#[test]
fn threads_list_pages_and_filters() {
    let e = Engine::start();
    let mut c = e.hello();

    let first = list(&mut c, json!({ "accountId": ACCOUNT_ID }));
    assert_eq!(first.threads.len(), 50, "the default limit");
    assert_eq!(first.threads[0]["__cls"], "Thread");

    let (all, pages) = list_all(&mut c, json!({ "accountId": ACCOUNT_ID, "limit": 7 }));
    assert_eq!(pages, THREADS.div_ceil(7));
    assert_eq!(all, expected_order(0..THREADS), "no other account's thread");

    let (inbox, _) = list_all(
        &mut c,
        json!({ "accountId": ACCOUNT_ID, "role": "inbox", "limit": 9 }),
    );
    assert_eq!(inbox, expected_order((0..THREADS).step_by(2)));
    let (arch, _) = list_all(
        &mut c,
        json!({ "accountId": ACCOUNT_ID, "folderId": "f-arch", "limit": 500 }),
    );
    assert_eq!(arch, expected_order((1..THREADS).step_by(2)));
    let (work, pages) = list_all(
        &mut c,
        json!({ "accountId": ACCOUNT_ID, "labelId": "l-work", "limit": 10 }),
    );
    assert_eq!(work, expected_order((0..THREADS).step_by(5)));
    assert_eq!(pages, THREADS.div_ceil(5).div_ceil(10));
    let empty = list(
        &mut c,
        json!({ "accountId": ACCOUNT_ID, "labelId": "l-imp" }),
    );
    assert!(empty.threads.is_empty() && empty.next.is_none());

    // Threads added while paging: newer ones don't shift later pages, older ones show up.
    let page = list(&mut c, json!({ "accountId": ACCOUNT_ID, "limit": 30 }));
    let db = db(e.dir.path());
    insert_thread(&db, ACCOUNT_ID, "t:new", BASE_TS + 1_000, &["f-inbox"]);
    insert_thread(&db, ACCOUNT_ID, "t:old", BASE_TS - 1, &["f-inbox"]);
    let (rest, _) = list_all(
        &mut c,
        json!({ "accountId": ACCOUNT_ID, "limit": 30, "before": page.next.unwrap() }),
    );
    let mut seen = ids(&page.threads);
    seen.extend(rest);
    let mut expected = expected_order(0..THREADS);
    expected.push("t:old".into());
    assert_eq!(seen, expected);
    e.stop();
}

#[test]
fn threads_list_clamps_limit() {
    let e = Engine::start();
    let db = db(e.dir.path());
    let extra = ms_ipc::READ_MAX_LIMIT + 10 - THREADS;
    for i in 0..extra {
        insert_thread(&db, ACCOUNT_ID, &format!("t:x{i:03}"), BASE_TS - 1, &[]);
    }
    let mut c = e.hello();
    let page = list(&mut c, json!({ "accountId": ACCOUNT_ID, "limit": 100_000 }));
    assert_eq!(page.threads.len(), ms_ipc::READ_MAX_LIMIT);
    let rest = list(
        &mut c,
        json!({ "accountId": ACCOUNT_ID, "limit": 100_000, "before": page.next.unwrap() }),
    );
    assert_eq!(rest.threads.len(), 10);
    assert!(rest.next.is_none());
    let one = list(&mut c, json!({ "accountId": ACCOUNT_ID, "limit": 0 }));
    assert_eq!(ids(&one.threads), [thread_id(THREADS - 1)]);
    assert!(one.next.is_some());
    e.stop();
}

/// As in the desktop, a category lists only threads in all mail unless it is spam or
/// trash.
#[test]
fn threads_list_hides_threads_outside_all_mail() {
    let e = Engine::start();
    let db = db(e.dir.path());
    insert_category(&db, "Folder", "f-trash", "Trash", "trash");
    insert_category(&db, "Folder", "f-spam", "Spam", "spam");
    insert_thread_in(
        &db,
        ACCOUNT_ID,
        "t:trashed",
        BASE_TS + 1_000,
        &["f-trash", "l-work", "l-imp"],
        false,
    );
    insert_thread_in(
        &db,
        ACCOUNT_ID,
        "t:spam",
        BASE_TS + 1_000,
        &["f-spam", "f-inbox"],
        false,
    );
    let mut c = e.hello();
    let first = |c: &mut Client, p: Value| -> Vec<String> {
        let mut p = p;
        p["accountId"] = json!(ACCOUNT_ID);
        p["limit"] = json!(1);
        ids(&list(c, p).threads)
    };
    assert_eq!(first(&mut c, json!({ "labelId": "l-work" })), ["t:115"]);
    assert_eq!(
        first(&mut c, json!({ "role": "important" })),
        Vec::<String>::new()
    );
    assert_eq!(first(&mut c, json!({ "folderId": "f-inbox" })), ["t:118"]);
    assert_eq!(
        first(&mut c, json!({ "folderId": "f-trash" })),
        ["t:trashed"]
    );
    assert_eq!(first(&mut c, json!({ "role": "trash" })), ["t:trashed"]);
    assert_eq!(first(&mut c, json!({ "role": "spam" })), ["t:spam"]);
    e.stop();
}

#[test]
fn threads_list_errors() {
    let e = Engine::start();
    let mut c = e.hello();
    for (params, expected) in [
        (
            json!({ "accountId": ACCOUNT_ID, "folderId": "nope" }),
            code::NOT_FOUND,
        ),
        (
            json!({ "accountId": ACCOUNT_ID, "labelId": "nope" }),
            code::NOT_FOUND,
        ),
        // A label id is not a folder id.
        (
            json!({ "accountId": ACCOUNT_ID, "folderId": "l-work" }),
            code::NOT_FOUND,
        ),
        (
            json!({ "accountId": ACCOUNT_ID, "role": "spam" }),
            code::NOT_FOUND,
        ),
        (
            json!({ "accountId": ACCOUNT_ID, "folderId": "f-inbox", "labelId": "l-work" }),
            code::INVALID_PARAMS,
        ),
        // l-work's role is "", which must not resolve to it.
        (
            json!({ "accountId": ACCOUNT_ID, "role": "" }),
            code::INVALID_PARAMS,
        ),
        (
            json!({ "accountId": ACCOUNT_ID, "before": "garbage" }),
            code::INVALID_PARAMS,
        ),
        (
            json!({ "accountId": ACCOUNT_ID, "limit": -1 }),
            code::INVALID_PARAMS,
        ),
        (json!({ "accountId": OTHER }), code::UNKNOWN_ACCOUNT),
    ] {
        assert_eq!(
            error_code(&mut c, method::THREADS_LIST, params.clone()),
            Some(expected),
            "{params}"
        );
    }
    e.stop();
}

#[test]
fn messages_and_thread_get() {
    let e = Engine::start();
    let mut c = e.hello();
    let get = |c: &mut Client, id: &str, body: Option<bool>| -> ms_ipc::MessagesGetResult {
        let mut p = json!({ "accountId": ACCOUNT_ID, "id": id });
        if let Some(b) = body {
            p["body"] = json!(b);
        }
        c.call_as(method::MESSAGES_GET, p).unwrap()
    };
    let r = get(&mut c, "m-1", None);
    assert_eq!(r.message["id"], "m-1");
    assert_eq!(r.body, None, "not asked for");
    assert_eq!(ids(&r.files), ["file-1"]);
    let r = get(&mut c, "m-1", Some(true));
    assert_eq!(r.body.as_deref(), Some("<p>Hello body</p>"));
    assert_eq!(get(&mut c, "m-2", Some(true)).body, None, "never fetched");
    assert_eq!(get(&mut c, "m-3", Some(true)).body, None, "fetch pending");
    assert_eq!(
        error_code(
            &mut c,
            method::MESSAGES_GET,
            json!({ "accountId": ACCOUNT_ID, "id": "nope" })
        ),
        Some(code::NOT_FOUND)
    );

    let r: ms_ipc::ThreadsGetResult = c
        .call_as(
            method::THREADS_GET,
            json!({ "accountId": ACCOUNT_ID, "id": "t:000" }),
        )
        .unwrap();
    assert_eq!(r.thread["id"], "t:000");
    assert_eq!(ids(&r.messages), ["m-1", "m-2"]);
    assert!(r.messages.iter().all(|m| m.get("body").is_none()));
    assert_eq!(
        error_code(
            &mut c,
            method::THREADS_GET,
            json!({ "accountId": ACCOUNT_ID, "id": "t:other" })
        ),
        Some(code::NOT_FOUND),
        "another account's thread"
    );
    e.stop();
}

#[test]
fn tasks_get() {
    let e = Engine::start();
    let mut c = e.hello();
    let r: ms_ipc::TasksGetResult = c
        .call_as(
            method::TASKS_GET,
            json!({ "accountId": ACCOUNT_ID, "id": "task-done" }),
        )
        .unwrap();
    assert_eq!(
        (r.task["id"].as_str(), r.task["status"].as_str()),
        (Some("task-done"), Some("complete"))
    );
    for id in ["task-other", "nope"] {
        assert_eq!(
            error_code(
                &mut c,
                method::TASKS_GET,
                json!({ "accountId": ACCOUNT_ID, "id": id })
            ),
            Some(code::NOT_FOUND),
            "{id}"
        );
    }
    e.stop();
}

#[test]
fn search_quotes_user_text() {
    let e = Engine::start();
    let mut c = e.hello();
    let mut search = |q: &str| -> Vec<String> {
        let r: ms_ipc::SearchResult = c
            .call_as(
                method::SEARCH,
                json!({ "accountId": ACCOUNT_ID, "query": q }),
            )
            .unwrap();
        ids(&r.threads)
    };
    assert_eq!(search("quarterly"), ["t:001"], "only this account");
    assert_eq!(search("Quarterly REPORT"), ["t:001"]);
    assert_eq!(
        search("quarterly hello"),
        Vec::<String>::new(),
        "terms are ANDed"
    );
    assert_eq!(search(r#"say "hello" NEAR(world)"#), ["t:002"]);
    assert_eq!(search("-minus col:on"), ["t:002"]);
    for q in [
        "\"",
        "\"\"",
        "AND",
        "OR NOT",
        "*",
        "a*",
        "^x",
        "(",
        ")",
        "NEAR(",
        "{x y}:z",
        "col:on:",
        "' OR 1=1 --",
        "   ",
        "",
    ] {
        search(q);
    }
    assert!(search("AND OR").is_empty());

    let mut contacts = |q: &str| -> Vec<String> {
        let r: ms_ipc::ContactsSearchResult = c
            .call_as(
                method::CONTACTS_SEARCH,
                json!({ "accountId": ACCOUNT_ID, "query": q }),
            )
            .unwrap();
        ids(&r.contacts)
    };
    assert_eq!(
        contacts("ann"),
        ["c-2", "c-1"],
        "by refs, no hidden or other account"
    );
    assert_eq!(contacts("ann smi"), ["c-1"]);
    assert_eq!(contacts("example.org"), ["c-2"]);
    for q in ["\"", "ann\" OR \"", "NOT", "*", "ann@", "-"] {
        contacts(q);
    }
    e.stop();
}

/// A task's local phase waiting on the write lock holds the main thread; reads on another
/// connection still answer.
#[test]
fn reads_answer_while_the_dispatcher_is_busy() {
    let e = Engine::start();
    let lock = db(e.dir.path());
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();

    let mut writer = e.hello();
    let (tx, rx) = mpsc::channel();
    let queued = std::thread::spawn(move || {
        let task =
            json!({ "id": "t-busy", "__cls": "SendFeatureUsageEventTask", "aid": ACCOUNT_ID });
        let r = writer.call(
            method::TASK_QUEUE,
            json!({ "accountId": ACCOUNT_ID, "task": task }),
        );
        let _ = tx.send(());
        r
    });
    std::thread::sleep(Duration::from_millis(300));

    let mut reader = e.hello();
    let page = list(&mut reader, json!({ "accountId": ACCOUNT_ID, "limit": 3 }));
    assert_eq!(page.threads.len(), 3);
    assert!(
        rx.try_recv().is_err(),
        "the task's local phase is still waiting"
    );

    lock.execute_batch("ROLLBACK").unwrap();
    let r = queued.join().unwrap().unwrap();
    assert_eq!(r["status"], "remote");
    e.stop();
}
