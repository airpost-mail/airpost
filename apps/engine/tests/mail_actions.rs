// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `messages.unsubscribe`, `threads.snooze` and `threads.unsnooze` on the socket
//! (docs/desktop-client-plan.md 3.3) on the debug binary, against a seeded database, an
//! account whose servers refuse connections and a local HTTP server for one-click. The moves
//! and the mail they cause on a server are covered by the harness (`unsubscribe-mailto`,
//! `snooze-method`).

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ms_ipc::{code, feature, method, Client, MessagesUnsubscribeResult, TasksListResult};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const ACCOUNT_ID: &str = "c0ffee-harness";
const ACCOUNT: &str = r#"{"id":"c0ffee-harness","emailAddress":"me@example.com","name":"Me","provider":"imap","settings":{"imap_host":"127.0.0.1","imap_port":1,"imap_username":"me","imap_password":"x","imap_security":"none","smtp_host":"127.0.0.1","smtp_port":1,"smtp_username":"me","smtp_password":"x","smtp_security":"none","imap_allow_insecure_ssl":false,"smtp_allow_insecure_ssl":false}}"#;

/// A local HTTP server answering each request with `status` after `delay`, reporting every
/// request it read.
fn http_server(status: &'static str, delay: Duration) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let Ok(n) = conn.read(&mut chunk) else { return };
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    let complete = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        body.len() >= len
                    });
                    if complete || n == 0 {
                        break;
                    }
                }
                std::thread::sleep(delay);
                let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
                let _ = conn.write_all(
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                );
            });
        }
    });
    (base, rx)
}

fn mailsync(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mailsync"));
    c.env("CONFIG_DIR_PATH", config)
        .env("MAILSYNC_UNSUBSCRIBE_ALLOW_LOCAL", "1")
        .env("MAILSYNC_FILES_WAIT_MS", "1000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn insert_message(c: &Connection, id: &str, unsub: Option<&str>, post: Option<&str>) {
    let mut data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": "Message", "v": 1,
        "threadId": "t1", "date": 1_700_000_000, "subject": "News",
        "to": [{"name": "", "email": "me@example.com"}], "from": [{"name": "List", "email": "news@lists.example.net"}]});
    if let Some(u) = unsub {
        data["hListUnsub"] = json!(u);
    }
    if let Some(p) = post {
        data["hListUnsubPost"] = json!(p);
    }
    c.execute(
        "INSERT INTO Message (id, accountId, version, data, date, threadId) VALUES (?, ?, 1, ?, ?, ?)",
        params![id, ACCOUNT_ID, data.to_string(), 1_700_000_000, "t1"],
    )
    .unwrap();
}

fn seed(config: &Path, base: &str) {
    let c = Connection::open(config.join("tern.db")).unwrap();
    let one_click = format!("<mailto:leave@lists.example.net>, <{base}/ok?u=1>");
    insert_message(
        &c,
        "m-oneclick",
        Some(&one_click),
        Some("List-Unsubscribe=One-Click"),
    );
    let failing = format!("<{base}/fail>");
    insert_message(
        &c,
        "m-fails",
        Some(&failing),
        Some("List-Unsubscribe=One-Click"),
    );
    insert_message(
        &c,
        "m-mailto",
        Some("<mailto:leave@lists.example.net?subject=stop>, <https://lists.example.net/u>"),
        None,
    );
    insert_message(
        &c,
        "m-web",
        Some("<https://lists.example.net/u?id=9>"),
        None,
    );
    insert_message(&c, "m-badmailto", Some("<mailto:not-an-address>"), None);
    // Body fetched, no header: nothing to wait for.
    insert_message(&c, "m-none", None, None);
    c.execute(
        "INSERT INTO MessageBody (id, value) VALUES ('m-none', '<p>hi</p>')",
        [],
    )
    .unwrap();
    // No body yet: the header may come with it, so the engine asks and waits.
    insert_message(&c, "m-unfetched", None, None);
}

struct Engine {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
}

impl Engine {
    fn start(base: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let status = mailsync(dir.path())
            .args(["--mode", "migrate"])
            .output()
            .unwrap()
            .status;
        assert!(status.success());
        seed(dir.path(), base);
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
        c.hello("mail-actions-test/1").unwrap();
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

fn unsubscribe(
    c: &mut Client,
    message: &str,
    method_name: Option<&str>,
) -> MessagesUnsubscribeResult {
    let mut p = json!({"accountId": ACCOUNT_ID, "messageId": message});
    if let Some(m) = method_name {
        p["method"] = json!(m);
    }
    c.call_as(method::MESSAGES_UNSUBSCRIBE, p).unwrap()
}

fn error_code(c: &mut Client, m: &str, p: Value) -> Option<i64> {
    c.call(m, p).unwrap_err().code()
}

#[test]
fn unsubscribe_by_each_method() {
    let (base, requests) = http_server("200 OK", Duration::ZERO);
    let e = Engine::start(&base);
    let mut c = e.hello();
    let status = c.call(method::ENGINE_STATUS, json!({})).unwrap();
    let features = status["features"].as_array().unwrap();
    assert!(features.contains(&json!(feature::UNSUBSCRIBE)), "{status}");
    assert!(features.contains(&json!(feature::SNOOZE)), "{status}");

    let r = unsubscribe(&mut c, "m-oneclick", None);
    assert_eq!((r.method.as_str(), r.ok), ("one-click", true), "{r:?}");
    assert_eq!(r.url.as_deref(), Some(format!("{base}/ok?u=1").as_str()));
    let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        request.starts_with("POST /ok?u=1 HTTP/1.1\r\n"),
        "{request}"
    );
    assert!(
        request.ends_with("\r\n\r\nList-Unsubscribe=One-Click"),
        "{request}"
    );

    let r = unsubscribe(&mut c, "m-oneclick", Some("web"));
    assert_eq!((r.method.as_str(), r.ok), ("web", true), "{r:?}");
    assert!(requests.try_recv().is_err(), "a web link is never fetched");

    let r = unsubscribe(&mut c, "m-web", None);
    assert_eq!(
        (r.method.as_str(), r.ok, r.url.as_deref()),
        ("web", true, Some("https://lists.example.net/u?id=9"))
    );

    let r = unsubscribe(&mut c, "m-mailto", None);
    assert_eq!((r.method.as_str(), r.ok), ("mailto", true), "{r:?}");
    let task_id = r.task_id.unwrap();
    let tasks: TasksListResult = c
        .call_as(
            method::TASKS_LIST,
            json!({"accountId": ACCOUNT_ID, "class": "SendDraftTask"}),
        )
        .unwrap();
    let task = tasks
        .tasks
        .iter()
        .find(|t| t["id"] == json!(task_id))
        .unwrap();
    let draft = &task["draft"];
    assert_eq!(draft["to"][0]["email"], "leave@lists.example.net");
    assert_eq!(
        draft["from"][0],
        json!({"name": "Me", "email": "me@example.com"})
    );
    assert_eq!(draft["subject"], "stop");
    assert_eq!(draft["plaintext"], true);

    let r = unsubscribe(&mut c, "m-badmailto", None);
    assert!(!r.ok && r.error.unwrap().contains("not an email address"));

    for (message, wanted) in [
        ("m-none", None),
        ("m-web", Some("mailto")),
        ("m-mailto", Some("one-click")),
        ("nope", None),
    ] {
        let mut p = json!({"accountId": ACCOUNT_ID, "messageId": message});
        if let Some(m) = wanted {
            p["method"] = json!(m);
        }
        assert_eq!(
            error_code(&mut c, method::MESSAGES_UNSUBSCRIBE, p),
            Some(code::NOT_FOUND),
            "{message} {wanted:?}"
        );
    }
    assert_eq!(
        error_code(
            &mut c,
            method::MESSAGES_UNSUBSCRIBE,
            json!({"accountId": ACCOUNT_ID, "messageId": "m-web", "method": "body-link"})
        ),
        Some(code::INVALID_PARAMS)
    );
    assert_eq!(
        error_code(
            &mut c,
            method::MESSAGES_UNSUBSCRIBE,
            json!({"accountId": "other", "messageId": "m-web"})
        ),
        Some(code::UNKNOWN_ACCOUNT)
    );

    let started = Instant::now();
    assert_eq!(
        error_code(
            &mut c,
            method::MESSAGES_UNSUBSCRIBE,
            json!({"accountId": ACCOUNT_ID, "messageId": "m-unfetched"})
        ),
        Some(code::NOT_FOUND)
    );
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "waited for the body"
    );

    // A relayed device without the admin scope may unsubscribe.
    let mut relayed = e.relayed();
    let r = unsubscribe(&mut relayed, "m-web", None);
    assert!(r.ok);
    e.stop();
}

/// A failing server is reported in the result; a slow one doesn't hold the connection's
/// later requests.
#[test]
fn one_click_failure_and_ordering() {
    let (base, _requests) = http_server("503 Service Unavailable", Duration::from_millis(1500));
    let e = Engine::start(&base);
    let mut c = e.hello();
    c.send_raw(
        format!(
            "{}\n",
            json!({"jsonrpc": "2.0", "id": "slow", "method": method::MESSAGES_UNSUBSCRIBE,
                   "params": {"accountId": ACCOUNT_ID, "messageId": "m-fails"}})
        )
        .as_bytes(),
    )
    .unwrap();
    let started = Instant::now();
    c.call(method::ENGINE_STATUS, json!({})).unwrap();
    assert!(started.elapsed() < Duration::from_millis(1200));
    let slow = loop {
        match c.read_message().unwrap() {
            ms_ipc::Message::Response(r) => break r,
            _ => continue,
        }
    };
    assert_eq!(slow.id, Some(ms_ipc::Id::Str("slow".into())));
    let r: MessagesUnsubscribeResult = serde_json::from_value(slow.result.unwrap()).unwrap();
    assert_eq!((r.method.as_str(), r.ok), ("one-click", false));
    assert!(r.error.unwrap().contains("503"));
    e.stop();
}

#[test]
fn snooze_params() {
    let (base, _requests) = http_server("200 OK", Duration::ZERO);
    let e = Engine::start(&base);
    let mut c = e.hello();
    let future = 4_000_000_000_i64;
    assert_eq!(
        error_code(
            &mut c,
            method::THREADS_SNOOZE,
            json!({"accountId": ACCOUNT_ID, "threadIds": ["t1"], "until": 1_000})
        ),
        Some(code::INVALID_PARAMS)
    );
    assert_eq!(
        error_code(
            &mut c,
            method::THREADS_SNOOZE,
            json!({"accountId": ACCOUNT_ID, "threadIds": ["nope"], "until": future})
        ),
        Some(code::NOT_FOUND)
    );
    assert_eq!(
        error_code(
            &mut c,
            method::THREADS_UNSNOOZE,
            json!({"accountId": ACCOUNT_ID, "threadIds": []})
        ),
        Some(code::NOT_FOUND)
    );
    assert_eq!(
        error_code(
            &mut c,
            method::THREADS_SNOOZE,
            json!({"accountId": "other", "threadIds": ["t1"], "until": future})
        ),
        Some(code::UNKNOWN_ACCOUNT)
    );
    e.stop();
}
