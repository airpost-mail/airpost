// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Live queries, the delta feed's sequence numbers and the read limits on the socket of the
//! debug binary, against an account whose servers refuse connections. Queued tasks stop in
//! `remote` there, which makes the Task table the rows that change.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tern_ipc::{code, feature, method, Client, ClientError, SubscribeParams};

const ACCOUNT_ID: &str = "c0ffee-harness";
const ACCOUNT: &str = r#"{"id":"c0ffee-harness","emailAddress":"me@example.com","provider":"imap","settings":{"imap_host":"127.0.0.1","imap_port":1,"imap_username":"me","imap_password":"x","imap_security":"none","smtp_host":"127.0.0.1","smtp_port":1,"smtp_username":"me","smtp_password":"x","smtp_security":"none","imap_allow_insecure_ssl":false,"smtp_allow_insecure_ssl":false}}"#;

fn tern_engine(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tern-engine"));
    c.env("CONFIG_DIR_PATH", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

struct Engine {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
}

impl Engine {
    fn start() -> Self {
        Engine::start_with(&[], |_| {})
    }

    fn start_with(env: &[(&str, &str)], seed: impl FnOnce(&Connection)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let status = tern_engine(dir.path())
            .args(["--mode", "migrate"])
            .output()
            .unwrap()
            .status;
        assert!(status.success());
        seed(&Connection::open(dir.path().join("tern.db")).unwrap());
        let socket = dir.path().join("run").join("e.sock");
        let mut cmd = tern_engine(dir.path());
        cmd.args(["--mode", "sync", "--identity", "null", "--account", ACCOUNT])
            .arg("--socket")
            .arg(&socket);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        if line.is_empty() {
            let mut stderr = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
            panic!("exited with {:?}: {stderr}", child.wait().unwrap());
        }
        assert!(line.contains("listening"), "{line}");
        std::thread::spawn(move || std::io::copy(&mut stdout, &mut std::io::sink()));
        Engine {
            _dir: dir,
            socket,
            child,
        }
    }

    fn hello(&self) -> Client {
        let mut c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        c.hello("live-test/1").unwrap();
        c
    }

    fn relayed(&self) -> Client {
        let mut c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        c.call(
            method::HELLO,
            json!({ "protocol": 1, "client": "tern-gateway/test", "relayed": true,
                    "scopes": [], "device": { "id": "d1", "name": "Test" } }),
        )
        .unwrap();
        c
    }

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

fn queue(c: &mut Client, id: &str) {
    let task = json!({ "id": id, "__cls": "NoOpTestTask", "aid": ACCOUNT_ID });
    let r = c
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": ACCOUNT_ID, "task": task }),
        )
        .unwrap();
    assert_eq!(r["status"], "remote");
}

/// Remote tasks by id, the window's rows.
fn remote_tasks() -> Value {
    json!({ "class": "Task", "where": {"attr": "status", "op": "=", "value": "remote"},
            "order": [{"attr": "id", "dir": "asc"}] })
}

/// The next `method` notification for `sub_id`, skipping the rest.
fn next_for(c: &mut Client, name: &str, sub_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < deadline, "no {name} for {sub_id}");
        let n = c.next_notification().unwrap();
        if n.method == name && n.params["subId"] == sub_id {
            return n.params;
        }
    }
}

/// The notifications that arrive within `wait`.
fn drain(c: &mut Client, wait: Duration) -> Vec<(String, Value)> {
    c.set_read_timeout(Some(wait)).unwrap();
    let mut out = Vec::new();
    while let Ok(n) = c.next_notification() {
        out.push((n.method, n.params));
    }
    c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    out
}

fn ids(rows: &Value) -> Vec<&str> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect()
}

fn error_code(r: Result<Value, ClientError>) -> Option<i64> {
    match r {
        Ok(v) => panic!("answered {:.200}", v.to_string()),
        Err(e) => e.code(),
    }
}

#[test]
fn a_live_query_follows_its_window() {
    let e = Engine::start();
    let mut c = e.hello();
    let status = c.call(method::ENGINE_STATUS, json!({})).unwrap();
    for f in [
        feature::LIVE_QUERIES,
        feature::EVENTS_SUBSCRIBE,
        feature::DELTA_SEQ,
        feature::TASK_DONE,
        feature::READ_LIMITS,
    ] {
        assert!(
            status["features"].as_array().unwrap().contains(&json!(f)),
            "{f}"
        );
    }

    let sub = c
        .call(
            method::QUERY_SUBSCRIBE,
            json!({ "query": remote_tasks(), "window": {"offset": 0, "limit": 2} }),
        )
        .unwrap();
    assert_eq!(
        (&sub["rows"], &sub["total"], &sub["version"]),
        (&json!([]), &json!(0), &json!(0))
    );
    let sub_id = sub["subId"].as_str().unwrap().to_owned();
    let mut writer = e.hello();

    queue(&mut writer, "t-5");
    let ch = next_for(&mut c, method::QUERY_CHANGED, &sub_id);
    assert_eq!(
        (&ch["version"], &ch["total"]),
        (&json!(1), &json!(1)),
        "{ch}"
    );
    assert_eq!(ch["ops"][0]["op"], "insert", "{ch}");
    assert_eq!(ch["ops"][0]["index"], 0);
    assert_eq!(ch["ops"][0]["row"]["id"], "t-5");
    assert_eq!(ch["ops"][0]["row"]["status"], "remote");

    queue(&mut writer, "t-3");
    let ch = next_for(&mut c, method::QUERY_CHANGED, &sub_id);
    assert_eq!(ch["version"], 2);
    assert_eq!(
        ch["ops"],
        json!([{"op": "insert", "index": 0, "row": ch["ops"][0]["row"]}])
    );
    assert_eq!(ch["ops"][0]["row"]["id"], "t-3");

    // Past the window: only the count changes.
    queue(&mut writer, "t-9");
    let ch = next_for(&mut c, method::QUERY_CHANGED, &sub_id);
    assert_eq!((&ch["total"], &ch["ops"]), (&json!(3), &json!([])), "{ch}");

    // Into the window's top: the last row leaves it.
    queue(&mut writer, "t-1");
    let ch = next_for(&mut c, method::QUERY_CHANGED, &sub_id);
    assert_eq!(ch["ops"][0], json!({"op": "remove", "id": "t-5"}), "{ch}");
    assert_eq!(ch["ops"][1]["op"], "insert");
    assert_eq!(
        (&ch["ops"][1]["index"], &ch["ops"][1]["row"]["id"]),
        (&json!(0), &json!("t-1"))
    );
    assert_eq!(ch["version"], 4);

    let w = c
        .call(
            method::QUERY_SET_WINDOW,
            json!({ "subId": sub_id, "window": {"offset": 2, "limit": 2} }),
        )
        .unwrap();
    assert_eq!(ids(&w["rows"]), ["t-5", "t-9"]);
    assert_eq!((&w["total"], &w["version"]), (&json!(4), &json!(5)));

    // A change to a row in the window is an update.
    writer
        .call(
            method::TASK_CANCEL,
            json!({ "accountId": ACCOUNT_ID, "taskId": "t-9" }),
        )
        .unwrap();
    let ch = next_for(&mut c, method::QUERY_CHANGED, &sub_id);
    assert_eq!(ch["version"], 6);
    assert_eq!(ch["ops"][0]["op"], "update", "{ch}");
    assert_eq!(ch["ops"][0]["row"]["id"], "t-9");
    assert_eq!(ch["ops"][0]["row"]["should_cancel"], true);

    c.call(method::QUERY_UNSUBSCRIBE, json!({ "subId": sub_id }))
        .unwrap();
    queue(&mut writer, "t-0");
    let after = drain(&mut c, Duration::from_millis(500));
    assert!(
        after.iter().all(|(m, _)| m != method::QUERY_CHANGED),
        "{after:?}"
    );
    // Gone, and unknown to setWindow.
    assert_eq!(
        error_code(c.call(
            method::QUERY_SET_WINDOW,
            json!({ "subId": sub_id, "window": {"offset": 0} })
        )),
        Some(code::NOT_FOUND)
    );
    e.stop();
}

#[test]
fn a_key_replaces_the_subscription_and_another_connection_cannot_touch_it() {
    let e = Engine::start();
    let mut c = e.hello();
    let first = c
        .call(
            method::QUERY_SUBSCRIBE,
            json!({ "query": remote_tasks(), "key": "tasks" }),
        )
        .unwrap();
    let second = c
        .call(
            method::QUERY_SUBSCRIBE,
            json!({ "query": remote_tasks(), "key": "tasks" }),
        )
        .unwrap();
    assert_ne!(first["subId"], second["subId"]);

    let mut other = e.hello();
    assert_eq!(
        error_code(other.call(
            method::QUERY_SET_WINDOW,
            json!({ "subId": second["subId"], "window": {"offset": 0} })
        )),
        Some(code::NOT_FOUND)
    );
    other
        .call(
            method::QUERY_UNSUBSCRIBE,
            json!({ "subId": second["subId"] }),
        )
        .unwrap();

    queue(&mut other, "t-1");
    let got = drain(&mut c, Duration::from_millis(800));
    let subs: Vec<&Value> = got
        .iter()
        .filter(|(m, _)| m == method::QUERY_CHANGED)
        .map(|(_, p)| &p["subId"])
        .collect();
    assert_eq!(subs, [&second["subId"]], "{got:?}");
    e.stop();
}

#[test]
fn live_query_params() {
    let e = Engine::start();
    let mut c = e.hello();
    let mut paged = remote_tasks();
    paged["limit"] = json!(10);
    assert_eq!(
        error_code(c.call(method::QUERY_SUBSCRIBE, json!({ "query": paged }))),
        Some(code::INVALID_PARAMS)
    );
    let mut misspelt = remote_tasks();
    misspelt["wher"] = json!({});
    assert_eq!(
        error_code(c.call(method::QUERY_SUBSCRIBE, json!({ "query": misspelt }))),
        Some(code::INVALID_PARAMS)
    );
    let mut elsewhere = remote_tasks();
    elsewhere["accounts"] = json!(["nope"]);
    assert_eq!(
        error_code(c.call(method::QUERY_SUBSCRIBE, json!({ "query": elsewhere }))),
        Some(code::UNKNOWN_ACCOUNT)
    );
    assert_eq!(
        error_code(c.call(
            method::QUERY_SUBSCRIBE,
            json!({ "query": {"class": "Nope"} })
        )),
        Some(code::INVALID_PARAMS)
    );

    // A relayed device subscribes too, with its window capped.
    let mut r = e.relayed();
    let sub = r
        .call(
            method::QUERY_SUBSCRIBE,
            json!({ "query": remote_tasks(), "window": {"offset": 0, "limit": 100000} }),
        )
        .unwrap();
    assert_eq!(sub["rows"], json!([]));
    e.stop();
}

#[test]
fn deltas_carry_seq_and_a_reconnect_resumes() {
    let e = Engine::start();
    let mut watch = e.hello();
    let start = watch
        .call(
            method::SUBSCRIBE,
            serde_json::to_value(SubscribeParams {
                accounts: None,
                classes: Some(vec!["Task".into()]),
                since: None,
            })
            .unwrap(),
        )
        .unwrap();
    let feed = start["feed"].as_str().unwrap().to_owned();
    let mut c = e.hello();
    queue(&mut c, "t-1");
    let first = loop {
        let n = watch.next_notification().unwrap();
        if n.method == method::DELTA {
            break n.params;
        }
    };
    let seen = first["seq"].as_u64().unwrap();
    assert!(
        seen > start["seq"].as_u64().unwrap(),
        "{first} after {start}"
    );
    drop(watch);

    queue(&mut c, "t-2");
    queue(&mut c, "t-3");
    std::thread::sleep(Duration::from_millis(1500));
    let mut back = e.hello();
    let r = back
        .call(
            method::SUBSCRIBE,
            json!({ "classes": ["Task"], "since": {"feed": feed, "seq": seen} }),
        )
        .unwrap();
    assert_eq!(r["resumed"], true, "{r}");
    let replayed = drain(&mut back, Duration::from_millis(500));
    let tasks: Vec<&str> = replayed
        .iter()
        .filter(|(m, _)| m == method::DELTA)
        .flat_map(|(_, p)| p["modelJSONs"].as_array().unwrap())
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(
        tasks.contains(&"t-2") && tasks.contains(&"t-3"),
        "{replayed:?}"
    );
    let seqs: Vec<u64> = replayed
        .iter()
        .filter_map(|(_, p)| p["seq"].as_u64())
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]) && seqs[0] > seen,
        "{seqs:?}"
    );

    let mut stranger = e.hello();
    let r = stranger
        .call(
            method::SUBSCRIBE,
            json!({ "since": {"feed": "elsewhere", "seq": 1} }),
        )
        .unwrap();
    assert_eq!(r["resumed"], false);
    e.stop();
}

#[test]
fn a_read_past_the_timeout_is_stopped() {
    // Remote, since the engine keeps only the newest hundred finished tasks at launch.
    let e = Engine::start_with(&[("TERN_ENGINE_READ_TIMEOUT_MS", "0")], |db| {
        for i in 0..5000 {
            let id = format!("old-{i:05}");
            let data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": "NoOpTestTask", "v": 1,
                "status": "remote", "error": null});
            db.execute(
                "INSERT INTO Task (id, version, data, accountId, status) VALUES (?, 1, ?, ?, ?)",
                params![id, data.to_string(), ACCOUNT_ID, "remote"],
            )
            .unwrap();
        }
    });
    let mut c = e.hello();
    let all = json!({ "class": "Task", "order": [{"attr": "id", "dir": "desc"}] });
    assert_eq!(
        error_code(c.call(method::QUERY, all.clone())),
        Some(code::READ_TIMEOUT)
    );
    assert_eq!(
        error_code(c.call(method::QUERY_SUBSCRIBE, json!({ "query": all }))),
        Some(code::READ_TIMEOUT)
    );
    // The connection goes on.
    c.call(method::ENGINE_STATUS, json!({})).unwrap();
    e.stop();
}

/// A thread of two messages, Ann's (unread) and the user's reply, and the inbox.
fn seed_thread(db: &Connection) {
    let message =
        |id: &str, date: i64, from: Value, to: Value, subject: &str, unread: bool| {
            let data = json!({"id": id, "aid": ACCOUNT_ID, "v": 1, "__cls": "Message",
            "threadId": "t1", "subject": subject, "date": date, "hMsgId": format!("{id}@x"),
            "unread": unread, "starred": false, "draft": false, "folders": {}, "labels": [],
            "from": from, "to": to, "cc": [], "bcc": [], "replyTo": [], "files": [],
            "snippet": format!("{subject} text")});
            db.execute(
            "INSERT INTO Message (id, accountId, version, data, headerMessageId, subject, date, \
             draft, unread, starred, threadId) VALUES (?, ?, 1, ?, ?, ?, ?, 0, ?, 0, 't1')",
            params![id, ACCOUNT_ID, data.to_string(), format!("{id}@x"), subject, date, unread],
        )
        .unwrap();
        };
    let ann = json!([{"name": "Ann", "email": "ann@x.com"}]);
    let me = json!([{"email": "me@example.com"}]);
    message("m1", 1_700_000_000, ann.clone(), me.clone(), "Hi", true);
    message("m2", 1_700_000_100, me, ann, "Re: Hi", false);
    let thread = json!({"id": "t1", "aid": ACCOUNT_ID, "v": 1, "__cls": "Thread",
        "subject": "Hi", "unread": 1, "starred": 0, "lmt": 1_700_000_100, "fmt": 1_700_000_000,
        "lmst": 1_700_000_100, "lmrt": 1_700_000_000, "inAllMail": true, "attachmentCount": 0,
        "folders": [], "labels": [], "participants": []});
    db.execute(
        "INSERT INTO Thread (id, accountId, version, data, subject, unread, starred, \
         firstMessageTimestamp, lastMessageTimestamp, lastMessageReceivedTimestamp, \
         lastMessageSentTimestamp, inAllMail) VALUES ('t1', ?, 1, ?, 'Hi', 1, 0, ?, ?, ?, ?, 1)",
        params![
            ACCOUNT_ID,
            thread.to_string(),
            1_700_000_000,
            1_700_000_100,
            1_700_000_000,
            1_700_000_100
        ],
    )
    .unwrap();
    let inbox = json!({"id": "f1", "aid": ACCOUNT_ID, "v": 1, "__cls": "Folder",
        "path": "INBOX", "role": "inbox", "localStatus": {}});
    db.execute(
        "INSERT INTO Folder (id, accountId, version, data, path, role) \
         VALUES ('f1', ?, 1, ?, 'INBOX', 'inbox')",
        params![ACCOUNT_ID, inbox.to_string()],
    )
    .unwrap();
}

#[test]
fn thread_rows_carry_a_summary_that_follows_their_messages() {
    let e = Engine::start_with(&[], seed_thread);
    let mut c = e.hello();
    let status = c.call(method::ENGINE_STATUS, json!({})).unwrap();
    for f in [
        feature::THREAD_SUMMARY,
        feature::DERIVED_FIELDS,
        feature::READ_POOL,
    ] {
        assert!(
            status["features"].as_array().unwrap().contains(&json!(f)),
            "{f}"
        );
    }
    let threads = json!({"class": "Thread", "include": ["summary"]});
    let rows = c.call(method::QUERY, threads.clone()).unwrap()["rows"].clone();
    assert_eq!(
        rows[0]["summary"],
        json!({"messageCount": 2, "hasDraft": false, "sending": false,
               "hasAttachment": false, "snippet": "Re: Hi text", "lastReply": "replied",
               "senders": [{"name": "Ann", "email": "ann@x.com", "unread": true, "me": false},
                           {"email": "me@example.com", "unread": false, "me": true}]})
    );
    // Derived fields on the read methods' rows.
    let folders = c
        .call(method::FOLDERS_LIST, json!({"accountId": ACCOUNT_ID}))
        .unwrap();
    assert_eq!(
        (
            &folders["folders"][0]["displayName"],
            &folders["folders"][0]["sortOrder"]
        ),
        (&json!("Inbox"), &json!(0))
    );
    let m = c
        .call(
            method::QUERY,
            json!({"class": "Message", "where": {"attr": "id", "op": "=", "value": "m1"}}),
        )
        .unwrap();
    assert_eq!(m["rows"][0]["hidden"], false);

    let sub = c
        .call(method::QUERY_SUBSCRIBE, json!({ "query": threads }))
        .unwrap();
    let sub_id = sub["subId"].as_str().unwrap().to_owned();
    assert_eq!(sub["rows"][0]["summary"]["messageCount"], 2);

    // A change to a message alone re-reads its thread's row.
    let mut writer = e.hello();
    writer
        .call(method::SUBSCRIBE, json!({"classes": ["Message"]}))
        .unwrap();
    let task = json!({"id": "meta-1", "__cls": "SyncbackMetadataTask", "aid": ACCOUNT_ID,
        "modelId": "m1", "modelClassName": "Message", "pluginId": "live-test",
        "value": {"seen": true}});
    writer
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": ACCOUNT_ID, "task": task }),
        )
        .unwrap();
    let ch = next_for(&mut c, method::QUERY_CHANGED, &sub_id);
    let update = ch["ops"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["op"] == "update")
        .unwrap_or_else(|| panic!("no update: {ch}"));
    assert_eq!(update["row"]["id"], "t1");
    assert_eq!(update["row"]["summary"]["messageCount"], 2);
    // The socket's Message deltas carry the derived fields.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < deadline, "no Message delta");
        let n = writer.next_notification().unwrap();
        if n.method == method::DELTA && n.params["modelClass"] == "Message" {
            assert_eq!(n.params["modelJSONs"][0]["hidden"], false, "{}", n.params);
            break;
        }
    }
    e.stop();
}

#[test]
fn a_slow_read_holds_back_no_other_request() {
    let e = Engine::start_with(
        &[("TERN_ENGINE_TEST_SLOW_READ", "folders.list:1500")],
        |_| {},
    );
    let mut c = e.hello();
    let request = |id: u64, name: &str, params: Value| {
        let r = json!({"jsonrpc": "2.0", "id": id, "method": name, "params": params});
        format!("{r}\n").into_bytes()
    };
    c.send_raw(&request(
        900,
        method::FOLDERS_LIST,
        json!({"accountId": ACCOUNT_ID}),
    ))
    .unwrap();
    c.send_raw(&request(901, method::QUERY, remote_tasks()))
        .unwrap();
    c.send_raw(&request(902, method::ENGINE_STATUS, json!({})))
        .unwrap();
    let mut order = Vec::new();
    while order.len() < 3 {
        if let tern_ipc::Message::Response(r) = c.read_message().unwrap() {
            assert!(r.error.is_none(), "{:?}", r.error);
            order.push(r.id.unwrap());
        }
    }
    assert_eq!(order.last(), Some(&tern_ipc::Id::Num(900)), "{order:?}");
    e.stop();
}
