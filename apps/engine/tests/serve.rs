// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Black-box checks of `--mode serve` (docs/engine-protocol-plan.md section 4) on the debug
//! binary: accounts added over stdin, whose servers refuse connections, served on the
//! socket.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use ms_ipc::{code, method, Client, ClientError, SubscribeParams};
use serde_json::{json, Value};

// Ids starting with c-f have no start delay.
const A: &str = "c0ffee-serve-a";
const B: &str = "d00d-serve-b";
const WAIT: Duration = Duration::from_secs(30);

fn account(id: &str) -> Value {
    json!({
        "id": id,
        "emailAddress": format!("{id}@example.com"),
        "provider": "imap",
        "settings": {
            "imap_host": "127.0.0.1", "imap_port": 1, "imap_username": "me",
            "imap_password": "x", "imap_security": "none",
            "smtp_host": "127.0.0.1", "smtp_port": 1, "smtp_username": "me",
            "smtp_password": "x", "smtp_security": "none",
            "imap_allow_insecure_ssl": false, "smtp_allow_insecure_ssl": false
        }
    })
}

fn task(id: &str, account_id: &str) -> Value {
    json!({ "id": id, "__cls": "SendFeatureUsageEventTask", "aid": account_id })
}

fn mailsync(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mailsync"));
    c.env("CONFIG_DIR_PATH", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn wait_for_log(path: &Path, pattern: &str) -> String {
    let re = regex::Regex::new(pattern).unwrap();
    let start = Instant::now();
    loop {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if re.is_match(&text) {
            return text;
        }
        assert!(start.elapsed() < WAIT, "{pattern} not in {path:?}:\n{text}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The parent's end of the control channel. Every stdout line must be a JSON-RPC response
/// or a `delta` notification.
struct Parent {
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
    next_id: i64,
    /// Notifications read while waiting for responses.
    notes: Vec<Value>,
}

impl Parent {
    fn send(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(format!("{line}\n").as_bytes()).unwrap();
        stdin.flush().unwrap();
        id
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, ms_ipc::ErrorObject> {
        let id = self.send(method, params);
        loop {
            let msg = self
                .lines
                .recv_timeout(WAIT)
                .unwrap_or_else(|_| panic!("no answer to {method}"));
            if msg.get("method").is_some() {
                assert_eq!(msg["method"], "delta", "{msg}");
                self.notes.push(msg);
                continue;
            }
            assert_eq!(msg["id"], id, "{msg}");
            if let Some(e) = msg.get("error") {
                return Err(serde_json::from_value(e.clone()).unwrap());
            }
            return Ok(msg["result"].clone());
        }
    }

    fn add(&mut self, account: Value) -> Value {
        self.call(
            method::ACCOUNT_ADD,
            json!({ "account": account, "identity": null }),
        )
        .unwrap()
    }
}

struct Serve {
    dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
    parent: Parent,
}

impl Serve {
    fn start() -> Self {
        Serve::start_with(true, &[])
    }

    fn start_with(explicit_socket: bool, env: &[(&str, String)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let status = mailsync(dir.path())
            .args(["--mode", "migrate"])
            .output()
            .unwrap()
            .status;
        assert!(status.success());
        let socket = if explicit_socket {
            dir.path().join("s").join("e.sock")
        } else {
            dir.path().join("run").join("engine.sock")
        };
        let mut cmd = mailsync(dir.path());
        cmd.args(["--mode", "serve"]);
        if explicit_socket {
            cmd.arg("--socket").arg(&socket);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { return };
                let v: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line:?}"));
                assert_eq!(v["jsonrpc"], "2.0", "{line}");
                if tx.send(v).is_err() {
                    return;
                }
            }
        });
        let mut s = Serve {
            dir,
            socket,
            child,
            parent: Parent {
                stdin,
                lines,
                next_id: 1,
                notes: Vec::new(),
            },
        };
        // The first answer comes once the socket is bound.
        let hello = s
            .parent
            .call(method::HELLO, json!({ "protocol": 1, "client": "parent" }))
            .unwrap();
        assert_eq!(hello["accounts"], json!([]));
        s
    }

    fn log(&self, id: &str) -> PathBuf {
        self.dir.path().join(format!("mailsync-{id}.log"))
    }

    fn discovery(&self) -> Value {
        let path = self.dir.path().join("run").join("engine.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn hello(&self) -> Client {
        let mut c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(WAIT)).unwrap();
        c.hello("serve-test/1").unwrap();
        c
    }

    fn stderr(&mut self) -> String {
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        stderr
    }

    /// Kills the engine and checks that stderr stayed empty.
    fn stop(mut self) {
        assert!(self.child.try_wait().unwrap().is_none(), "exited early");
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        assert_eq!(self.stderr(), "");
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reads notifications until one matches `pred`.
#[track_caller]
fn next_matching(c: &mut Client, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        assert!(Instant::now() < deadline, "no matching notification");
        let n = c.next_notification().unwrap();
        let v = json!({ "method": n.method, "params": n.params });
        if pred(&v) {
            return v;
        }
    }
}

fn is_state(v: &Value, id: &str, state: &str) -> bool {
    v["method"] == method::ACCOUNT_STATE
        && v["params"]["accountId"] == id
        && v["params"]["state"] == state
}

fn states(status: &Value) -> Vec<(String, String)> {
    status["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["accountId"].as_str().unwrap().to_owned(),
                a["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn running(ids: &[&str]) -> Vec<(String, String)> {
    ids.iter()
        .map(|id| ((*id).to_owned(), "running".to_owned()))
        .collect()
}

#[test]
fn two_accounts_listed_routed_and_logged_apart() {
    let mut e = Serve::start();
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams {
        accounts: None,
        classes: Some(vec!["Task".to_owned()]),
    })
    .unwrap();

    let added = e.parent.add(account(A));
    assert_eq!(added, json!({ "accountId": A, "state": "running" }));
    assert_eq!(e.parent.add(account(B))["state"], "running");
    // The subscriber is told, whatever its class filter.
    for id in [A, B] {
        next_matching(&mut sub, |v| is_state(v, id, "starting"));
        next_matching(&mut sub, |v| is_state(v, id, "running"));
    }

    let mut c = e.hello();
    let hello = c.hello("again").unwrap();
    let ids: Vec<_> = hello
        .accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.email.as_str(), a.state.as_str()))
        .collect();
    assert_eq!(
        ids,
        [
            (A, "c0ffee-serve-a@example.com", "running"),
            (B, "d00d-serve-b@example.com", "running")
        ]
    );
    let status = c.call(method::ENGINE_STATUS, Value::Null).unwrap();
    assert_eq!(states(&status), running(&[A, B]));
    let list = c.call(method::ACCOUNT_LIST, json!({})).unwrap();
    assert_eq!(states(&list), running(&[A, B]));
    // The parent channel answers the socket methods too.
    let status = e.parent.call(method::ENGINE_STATUS, json!({})).unwrap();
    assert_eq!(states(&status), running(&[A, B]));

    // Each task lands in its own account's main thread and table rows.
    for (id, t) in [(A, "t-a"), (B, "t-b")] {
        let queued = c
            .call(
                method::TASK_QUEUE,
                json!({ "accountId": id, "task": task(t, id) }),
            )
            .unwrap();
        assert_eq!(queued["status"], "remote", "{queued}");
        let got = c
            .call(method::TASKS_GET, json!({ "accountId": id, "id": t }))
            .unwrap();
        assert_eq!(got["task"]["aid"], id);
    }
    // B's task sent to A runs A's local phase, which refuses another account's task.
    let err = c
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": A, "task": task("t-cross", B) }),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some(code::TASK_REJECTED));
    assert_eq!(
        c.call(method::TASKS_GET, json!({ "accountId": B, "id": "t-a" }))
            .unwrap_err()
            .code(),
        Some(code::NOT_FOUND)
    );
    for (m, p) in [
        (
            method::TASK_QUEUE,
            json!({ "accountId": "nope", "task": task("t", "nope") }),
        ),
        (method::SYNC_WAKE, json!({ "accountId": "nope" })),
        (method::FOLDERS_LIST, json!({ "accountId": "nope" })),
        (method::SUBSCRIBE, json!({ "accounts": ["nope"] })),
    ] {
        assert_eq!(
            c.call(m, p).unwrap_err().code(),
            Some(code::UNKNOWN_ACCOUNT),
            "{m}"
        );
    }
    // Each account flushes on its own thread, so the two deltas may arrive in either order.
    let mut seen = std::collections::BTreeMap::new();
    while seen.len() < 2 {
        let v = next_matching(&mut sub, |v| {
            v["method"] == "delta"
                && ["t-a", "t-b"]
                    .iter()
                    .any(|t| v["params"]["modelJSONs"][0]["id"] == *t)
        });
        let task_id = v["params"]["modelJSONs"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        seen.entry(task_id)
            .or_insert_with(|| v["params"]["accountId"].clone());
    }
    assert_eq!(seen["t-a"], A);
    assert_eq!(seen["t-b"], B);

    // Credentials only come in on stdin.
    for m in [
        method::ACCOUNT_ADD,
        method::ACCOUNT_REMOVE,
        method::ENGINE_SHUTDOWN,
    ] {
        let p = json!({ "account": account("e0-other"), "accountId": A });
        assert_eq!(c.call(m, p).unwrap_err().code(), Some(code::WRONG_CHANNEL));
    }
    assert_eq!(
        e.parent
            .call(method::SUBSCRIBE, json!({}))
            .unwrap_err()
            .code,
        code::WRONG_CHANNEL
    );
    let again = e
        .parent
        .call(
            method::ACCOUNT_ADD,
            json!({ "account": account(A), "identity": null }),
        )
        .unwrap_err();
    assert_eq!(again.code, code::ACCOUNT_EXISTS);
    let bad = e
        .parent
        .call(
            method::ACCOUNT_ADD,
            json!({ "account": {"id": "x"}, "identity": null }),
        )
        .unwrap_err();
    assert_eq!(bad.code, code::INVALID_PARAMS);

    // sync.wake without an account wakes each one.
    assert_eq!(c.call(method::SYNC_WAKE, json!({})).unwrap(), json!({}));
    let waking = r"(?m)\[main\] \[info\] Waking all workers\.\.\.\r?$";
    let log_a = wait_for_log(&e.log(A), waking);
    let log_b = wait_for_log(&e.log(B), waking);

    // One log per account, in the usual format and thread names; the rest in engine.log.
    assert!(log_a.contains("Starting Sync (c0ffee-serve-a@example.com)"));
    assert!(!log_a.contains("d00d-serve-b"), "{log_a}");
    assert!(log_b.contains("Starting Sync (d00d-serve-b@example.com)"));
    assert!(!log_b.contains("c0ffee-serve-a"), "{log_b}");
    let line = regex::Regex::new(
        r"(?m)^\d+ \[\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d{3}\] \[background\] \[(info|error)\] ",
    )
    .unwrap();
    wait_for_log(&e.log(A), line.as_str());
    let engine_log = wait_for_log(
        &e.dir.path().join("engine.log"),
        r"Socket connection \d+: hello from",
    );
    assert!(!engine_log.contains("Waking all workers"), "{engine_log}");

    let d = e.discovery();
    assert_eq!(d["pid"], e.child.id());
    assert_eq!(d["protocol"], 1);
    assert_eq!(d["accounts"], json!([A, B]));
    assert_eq!(
        Path::new(d["socket"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        e.socket.canonicalize().unwrap()
    );
    assert!(d.get("accountId").is_none());
    e.stop();
}

#[test]
fn remove_stops_one_account_and_keeps_the_other() {
    let mut e = Serve::start();
    e.parent.add(account(A));
    e.parent.add(account(B));
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams::default()).unwrap();
    let mut c = e.hello();
    c.call(
        method::TASK_QUEUE,
        json!({ "accountId": A, "task": task("t-kept", A) }),
    )
    .unwrap();

    let started = Instant::now();
    assert_eq!(
        e.parent
            .call(method::ACCOUNT_REMOVE, json!({ "accountId": A }))
            .unwrap(),
        json!({ "stillRunning": [] })
    );
    assert!(started.elapsed() < Duration::from_secs(6));
    next_matching(&mut sub, |v| is_state(v, A, "removed"));
    let status = c.call(method::ENGINE_STATUS, Value::Null).unwrap();
    assert_eq!(states(&status), running(&[B]));
    assert_eq!(e.discovery()["accounts"], json!([B]));
    assert_eq!(
        c.call(
            method::TASK_QUEUE,
            json!({ "accountId": A, "task": task("t", A) })
        )
        .unwrap_err()
        .code(),
        Some(code::UNKNOWN_ACCOUNT)
    );
    assert_eq!(
        e.parent
            .call(method::ACCOUNT_REMOVE, json!({ "accountId": A }))
            .unwrap_err()
            .code,
        code::UNKNOWN_ACCOUNT
    );
    let queued = c
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": B, "task": task("t-b", B) }),
        )
        .unwrap();
    assert_eq!(queued["status"], "remote");

    // Removing keeps the rows: added again, the account still has its task.
    assert_eq!(e.parent.add(account(A))["state"], "running");
    assert_eq!(e.discovery()["accounts"], json!([B, A]));
    let got = c
        .call(method::TASKS_GET, json!({ "accountId": A, "id": "t-kept" }))
        .unwrap();
    assert_eq!(got["task"]["id"], "t-kept");
    e.stop();
}

#[test]
fn a_failure_stops_only_its_account() {
    let mut e = Serve::start();
    e.parent.add(account(A));
    e.parent.add(account(B));
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams::default()).unwrap();
    let mut c = e.hello();

    e.parent
        .call(method::TEST_CRASH, json!({ "accountId": A }))
        .unwrap();
    let stopped = next_matching(&mut sub, |v| is_state(v, A, "stopped"));
    assert_eq!(stopped["params"]["error"]["key"], "test", "{stopped}");

    let status = c.call(method::ENGINE_STATUS, Value::Null).unwrap();
    assert_eq!(
        states(&status),
        [
            (A.to_owned(), "stopped".to_owned()),
            (B.to_owned(), "running".to_owned())
        ]
    );
    assert_eq!(status["accounts"][0]["error"]["key"], "test");
    let err = c
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": A, "task": task("t", A) }),
        )
        .unwrap_err();
    let ClientError::Rpc(obj) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(obj.code, code::ACCOUNT_STOPPED);
    assert_eq!(obj.data.as_ref().unwrap()["state"], "stopped");
    // Reads still answer for a stopped account.
    c.call(method::FOLDERS_LIST, json!({ "accountId": A }))
        .unwrap();
    let queued = c
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": B, "task": task("t-b", B) }),
        )
        .unwrap();
    assert_eq!(queued["status"], "remote");

    // A panic on an account thread stops that account, not the process.
    e.parent
        .call(method::TEST_PANIC, json!({ "accountId": B }))
        .unwrap();
    let stopped = next_matching(&mut sub, |v| is_state(v, B, "stopped"));
    assert!(stopped["params"]["error"].is_object(), "{stopped}");
    wait_for_log(
        &e.log(B),
        r"\[main\] \[error\] Unhandled panic on thread main: .*test\.panic",
    );

    // Restarting is the client's call.
    assert_eq!(e.parent.add(account(A))["state"], "running");
    let queued = c
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": A, "task": task("t-a2", A) }),
        )
        .unwrap();
    assert_eq!(queued["status"], "remote");
    let log_a = std::fs::read_to_string(e.log(A)).unwrap();
    assert!(!log_a.contains("[critical]"), "{log_a}");
    e.stop();
}

/// A one-request HTTP server answering the OAuth refresh with a rotated refresh token.
fn token_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            let _ = reader.read_exact(&mut body);
            let reply =
                r#"{"access_token":"at","expires_in":3600,"refresh_token":"rotated-s3cret"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    format!("http://{addr}/token")
}

#[test]
fn secrets_go_to_stdout_only() {
    let url = token_server();
    let mut e = Serve::start_with(true, &[("MAILSYNC_GOOGLE_TOKEN_URL", url)]);
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams::default()).unwrap();

    let mut oauth = account(A);
    oauth["provider"] = json!("gmail");
    let settings = oauth["settings"].as_object_mut().unwrap();
    settings.remove("imap_password");
    settings.remove("smtp_password");
    settings.insert("refresh_token".into(), json!("old"));
    settings.insert("refresh_client_id".into(), json!("cid"));
    e.parent.add(oauth);

    // The background worker refreshes before connecting.
    let deadline = Instant::now() + WAIT;
    let secret = loop {
        if let Some(n) = e
            .parent
            .notes
            .iter()
            .find(|n| n["params"]["modelClass"] == "ProcessAccountSecretsUpdated")
        {
            break n.clone();
        }
        assert!(Instant::now() < deadline, "no secrets delta on stdout");
        e.parent.call(method::ENGINE_STATUS, json!({})).unwrap();
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(secret["method"], "delta");
    assert_eq!(secret["params"]["accountId"], A);
    assert_eq!(
        secret["params"]["modelJSONs"][0]["settings"]["refresh_token"],
        "rotated-s3cret"
    );

    // The subscriber saw A's other deltas but never the secrets.
    let mut c = e.hello();
    c.call(
        method::TASK_QUEUE,
        json!({ "accountId": A, "task": task("t-after", A) }),
    )
    .unwrap();
    let mut seen = Vec::new();
    loop {
        let n = sub.next_notification().unwrap();
        let text = n.params.to_string();
        let done = text.contains("t-after");
        seen.push(text);
        if done {
            break;
        }
    }
    assert!(!seen.concat().contains("s3cret"), "{seen:?}");
    assert!(!seen.concat().contains("ProcessAccountSecretsUpdated"));
    e.stop();
}

#[test]
fn shutdown_stops_everything_and_cleans_up() {
    let mut e = Serve::start_with(false, &[]);
    let discovery = e.dir.path().join("run").join("engine.json");
    assert!(e.socket.exists(), "the default socket path");
    assert!(discovery.exists());
    e.parent.add(account(A));
    e.parent.add(account(B));

    let started = Instant::now();
    assert_eq!(
        e.parent.call(method::ENGINE_SHUTDOWN, json!({})).unwrap(),
        json!({})
    );
    let status = e.child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!e.socket.exists());
    assert!(!discovery.exists());
    assert_eq!(e.stderr(), "");
    let log_a = std::fs::read_to_string(e.log(A)).unwrap();
    assert!(log_a.contains("Starting Sync"), "{log_a}");
}

#[test]
fn bad_lines_on_stdin_get_errors() {
    let mut e = Serve::start();
    let stdin = e.parent.stdin.as_mut().unwrap();
    stdin
        .write_all(b"garbage\n{\"type\":\"wake-workers\"}\n")
        .unwrap();
    stdin.flush().unwrap();
    for expected in [code::PARSE_ERROR, code::INVALID_REQUEST] {
        let msg = e.parent.lines.recv_timeout(WAIT).unwrap();
        assert_eq!(msg["id"], Value::Null);
        assert_eq!(msg["error"]["code"], expected, "{msg}");
    }
    assert_eq!(
        e.parent.call("no.such.method", json!({})).unwrap_err().code,
        code::METHOD_NOT_FOUND
    );
    // A request without an id runs unanswered.
    let stdin = e.parent.stdin.as_mut().unwrap();
    let note = json!({ "jsonrpc": "2.0", "method": method::ENGINE_STATUS });
    stdin.write_all(format!("{note}\n").as_bytes()).unwrap();
    e.parent.call(method::ENGINE_STATUS, json!({})).unwrap();
    let mut c = e.hello();
    assert!(matches!(
        c.call(method::ENGINE_STATUS, Value::Null),
        Ok(Value::Object(_))
    ));
    e.stop();
}

/// Takes 30 s: `cargo test -p mailsync -- --ignored`. Closing stdin ends the engine with
/// 141, and the socket and discovery file go with it.
#[test]
#[ignore]
fn exits_141_after_stdin_closes() {
    let mut e = Serve::start();
    e.parent.add(account(A));
    drop(e.parent.stdin.take());
    let start = Instant::now();
    let status = loop {
        if let Some(s) = e.child.try_wait().unwrap() {
            break s;
        }
        assert!(start.elapsed() < Duration::from_secs(45), "did not exit");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(141));
    assert!(start.elapsed() >= Duration::from_secs(30));
    assert!(!e.socket.exists());
    assert!(!e.dir.path().join("run").join("engine.json").exists());
}

#[test]
fn a_second_engine_on_the_socket_exits_in_use() {
    let mut e = Serve::start();
    let out = mailsync(e.dir.path())
        .args(["--mode", "serve", "--socket"])
        .arg(&e.socket)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(ms_core::constants::EXIT_SOCKET_IN_USE)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"error\":\"Another process is listening on "),
        "{stdout}"
    );
    // The first engine keeps its socket and discovery file.
    assert_eq!(e.discovery()["pid"], json!(e.child.id()));
    e.hello();
    e.parent.call(method::ENGINE_STATUS, json!({})).unwrap();
    e.stop();
}
