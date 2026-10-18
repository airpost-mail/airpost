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
        let (s, hello) = Serve::spawn_in(dir, explicit_socket, env);
        assert_eq!(hello["accounts"], json!([]));
        s
    }

    /// Stops this engine with `engine.shutdown` and starts another on the same config
    /// directory; returns it and its `hello`.
    fn restart(mut self, env: &[(&str, String)]) -> (Self, Value) {
        self.parent
            .call(method::ENGINE_SHUTDOWN, json!({}))
            .unwrap();
        assert_eq!(self.child.wait().unwrap().code(), Some(0));
        assert_eq!(self.stderr(), "");
        let explicit = !self.socket.ends_with("run/engine.sock");
        let dir = std::mem::replace(&mut self.dir, tempfile::tempdir().unwrap());
        Serve::spawn_in(dir, explicit, env)
    }

    fn spawn_in(
        dir: tempfile::TempDir,
        explicit_socket: bool,
        env: &[(&str, String)],
    ) -> (Self, Value) {
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
        (s, hello)
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
    assert_eq!(
        added,
        json!({ "accountId": A, "state": "running", "stored": true })
    );
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
    let mut clear = account("e1-clear");
    clear["settings"]["smtp_security"] = json!("None");
    let refused = e
        .parent
        .call(
            method::ACCOUNT_ADD,
            json!({ "account": clear, "identity": null }),
        )
        .unwrap_err();
    assert_eq!(refused.code, code::INVALID_PARAMS);
    assert!(
        refused
            .message
            .starts_with("Account has an invalid smtp_security: \"None\""),
        "{}",
        refused.message
    );

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

/// Retries far enough out that the tests below see each stop as it is.
fn no_retry() -> (&'static str, String) {
    ("TERN_ENGINE_RESTART_DELAY_MS", "600000".to_owned())
}

#[test]
fn a_failure_stops_only_its_account() {
    let mut e = Serve::start_with(true, &[no_retry()]);
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
    assert!(stopped["params"]["retryAt"].as_i64().is_some(), "{stopped}");

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

    // account.add restarts it before the retry would.
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

/// An HTTP server answering every OAuth refresh with a rotated refresh token; the request
/// bodies it got.
fn token_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = bodies.clone();
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
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&body).into_owned());
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
    (format!("http://{addr}/token"), bodies)
}

fn oauth_account(id: &str) -> Value {
    let mut oauth = account(id);
    oauth["provider"] = json!("gmail");
    let settings = oauth["settings"].as_object_mut().unwrap();
    settings.remove("imap_password");
    settings.remove("smtp_password");
    settings.insert("refresh_token".into(), json!("old"));
    settings.insert("refresh_client_id".into(), json!("cid"));
    oauth
}

/// Without a secret store the account isn't registered, and its rotated token goes to the
/// parent on stdout, never to a socket subscriber.
#[test]
fn secrets_go_to_stdout_only() {
    let (url, _) = token_server();
    let mut e = Serve::start_with(
        true,
        &[
            ("MAILSYNC_GOOGLE_TOKEN_URL", url),
            ("TERN_ENGINE_SECRET_STORE", "none".to_owned()),
        ],
    );
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams::default()).unwrap();

    let added = e.parent.add(oauth_account(A));
    assert_eq!(added["stored"], false, "{added}");

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

#[test]
fn account_test_on_stdin_runs_beside_other_calls() {
    let mut e = Serve::start();
    // A server that accepts and never greets keeps the test's IMAP step waiting.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let mut slow = account(A);
    slow["settings"]["imap_port"] = json!(port);
    let pending = e
        .parent
        .send(method::ACCOUNT_TEST, json!({ "account": slow }));
    // Answered while the test still waits.
    let listed = e.parent.call(method::ACCOUNT_LIST, Value::Null).unwrap();
    assert_eq!(listed["accounts"], json!([]));
    drop(silent);

    let answer = loop {
        let msg = e
            .parent
            .lines
            .recv_timeout(Duration::from_secs(120))
            .unwrap();
        if msg["id"] == pending {
            break msg;
        }
    };
    let result = &answer["result"];
    assert!(result["error"].is_string(), "{answer}");
    assert_eq!(result["error_service"], "imap");
    assert_eq!(result["account"], Value::Null);
    assert!(result["log"].is_string());

    // Refused: an account the engine cannot read, and the socket.
    let bad = e
        .parent
        .call(method::ACCOUNT_TEST, json!({ "account": { "id": "x" } }))
        .unwrap_err();
    assert_eq!(bad.code, code::INVALID_PARAMS);
    let mut c = e.hello();
    for m in [
        method::ACCOUNT_TEST,
        method::ACCOUNT_RESET,
        method::ENGINE_RESET_DATABASE,
        method::ENGINE_SET_LOG_LEVEL,
    ] {
        let err = c.call(m, json!({})).unwrap_err();
        assert_eq!(err.code(), Some(code::WRONG_CHANNEL), "{m}");
    }
    e.stop();
}

#[test]
fn account_reset_forgets_the_account_and_deletes_its_rows() {
    let mut e = Serve::start();
    e.parent.add(account(A));
    e.parent.add(account(B));
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams::default()).unwrap();
    let mut c = e.hello();
    for (t, id) in [("t-a", A), ("t-b", B)] {
        c.call(
            method::TASK_QUEUE,
            json!({ "accountId": id, "task": task(t, id) }),
        )
        .unwrap();
    }

    assert_eq!(
        e.parent
            .call(method::ACCOUNT_RESET, json!({ "accountId": A }))
            .unwrap(),
        json!({})
    );
    next_matching(&mut sub, |v| is_state(v, A, "removed"));
    let status = c.call(method::ENGINE_STATUS, Value::Null).unwrap();
    // Still registered, so listed as stopped until it is added again.
    assert_eq!(
        states(&status),
        [
            (B.to_owned(), "running".to_owned()),
            (A.to_owned(), "stopped".to_owned())
        ]
    );

    // Added again, A starts empty; B kept its rows.
    assert_eq!(e.parent.add(account(A))["state"], "running");
    let gone = c
        .call(method::TASKS_GET, json!({ "accountId": A, "id": "t-a" }))
        .unwrap_err();
    assert_eq!(gone.code(), Some(code::NOT_FOUND));
    let kept = c
        .call(method::TASKS_GET, json!({ "accountId": B, "id": "t-b" }))
        .unwrap();
    assert_eq!(kept["task"]["id"], "t-b");

    // An account the engine doesn't hold (removed earlier) is reset all the same.
    e.parent
        .call(method::ACCOUNT_REMOVE, json!({ "accountId": B }))
        .unwrap();
    assert_eq!(
        e.parent
            .call(method::ACCOUNT_RESET, json!({ "accountId": B }))
            .unwrap(),
        json!({})
    );
    assert_eq!(e.parent.add(account(B))["state"], "running");
    let gone = c
        .call(method::TASKS_GET, json!({ "accountId": B, "id": "t-b" }))
        .unwrap_err();
    assert_eq!(gone.code(), Some(code::NOT_FOUND));
    e.stop();
}

#[test]
fn reset_database_starts_over_and_sessions_reopen_it() {
    let mut e = Serve::start();
    e.parent.add(account(A));
    e.parent.add(account(B));
    let mut c = e.hello();
    c.call(
        method::TASK_QUEUE,
        json!({ "accountId": A, "task": task("t-a", A) }),
    )
    .unwrap();
    // The session's read connection is open on the old file.
    c.call(method::TASKS_GET, json!({ "accountId": A, "id": "t-a" }))
        .unwrap();
    // Metadata waiting for its message is carried into the new database (TD32).
    let db = e.dir.path().join("edgehill.db");
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "INSERT INTO DetatchedPluginMetadata (objectId, objectType, accountId, pluginId, value, version) \
             VALUES ('<kept@x>', 'Message', ?, 'send-later', '{\"expiration\":1}', 2)",
            [A],
        )
        .unwrap();

    let result = e
        .parent
        .call(method::ENGINE_RESET_DATABASE, Value::Null)
        .unwrap();
    assert_eq!(result, json!({ "removed": [A, B] }));
    let kept: i64 = rusqlite::Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT version FROM DetatchedPluginMetadata WHERE objectId = '<kept@x>'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 2);
    let status = c.call(method::ENGINE_STATUS, Value::Null).unwrap();
    let stopped = |id: &str| (id.to_owned(), "stopped".to_owned());
    assert_eq!(states(&status), [stopped(A), stopped(B)]);
    assert_eq!(e.discovery()["accounts"], json!([]));
    assert!(e.dir.path().join("edgehill.db").exists());

    assert_eq!(e.parent.add(account(A))["state"], "running");
    let gone = c
        .call(method::TASKS_GET, json!({ "accountId": A, "id": "t-a" }))
        .unwrap_err();
    assert_eq!(gone.code(), Some(code::NOT_FOUND));
    e.stop();
}

#[test]
fn set_log_level_restarts_running_accounts() {
    let mut e = Serve::start();
    e.parent.add(account(A));
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams::default()).unwrap();
    let until = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        + 60_000;
    let result = e
        .parent
        .call(
            method::ENGINE_SET_LOG_LEVEL,
            json!({ "verboseUntil": until }),
        )
        .unwrap();
    assert_eq!(result, json!({ "restarted": [A] }));
    next_matching(&mut sub, |v| is_state(v, A, "removed"));
    next_matching(&mut sub, |v| is_state(v, A, "running"));
    let mut c = e.hello();
    let status = c.call(method::ENGINE_STATUS, Value::Null).unwrap();
    assert_eq!(states(&status), running(&[A]));
    // Accounts added later get it too, and turning it off restarts them again.
    e.parent.add(account(B));
    let result = e
        .parent
        .call(method::ENGINE_SET_LOG_LEVEL, json!({ "verboseUntil": 0 }))
        .unwrap();
    assert_eq!(result, json!({ "restarted": [A, B] }));
    e.stop();
}

#[test]
fn logs_tail_reads_the_end_of_a_log() {
    let mut e = Serve::start();
    e.parent.add(account(A));
    wait_for_log(&e.log(A), "Starting");
    let mut c = e.hello();
    let tail = c
        .call(
            method::ENGINE_LOGS_TAIL,
            json!({ "accountId": A, "bytes": 40 }),
        )
        .unwrap();
    assert_eq!(tail["text"].as_str().unwrap().len(), 40);
    assert_eq!(tail["truncated"], true);
    let whole = c
        .call(method::ENGINE_LOGS_TAIL, json!({ "accountId": A }))
        .unwrap();
    assert!(whole["text"].as_str().unwrap().contains(A));
    let engine = c.call(method::ENGINE_LOGS_TAIL, json!({})).unwrap();
    assert!(engine["text"]
        .as_str()
        .unwrap()
        .contains("Starting the engine"));
    let none = c
        .call(
            method::ENGINE_LOGS_TAIL,
            json!({ "accountId": "e0e0-none" }),
        )
        .unwrap();
    assert_eq!(none, json!({ "text": "", "truncated": false }));
    let bad = c
        .call(
            method::ENGINE_LOGS_TAIL,
            json!({ "accountId": "../config" }),
        )
        .unwrap_err();
    assert_eq!(bad.code(), Some(code::INVALID_PARAMS));
    // The parent channel answers it too.
    let tail = e
        .parent
        .call(
            method::ENGINE_LOGS_TAIL,
            json!({ "accountId": A, "bytes": 10 }),
        )
        .unwrap();
    assert_eq!(tail["text"].as_str().unwrap().len(), 10);
    e.stop();
}

#[test]
fn settings_round_trip_and_notify_the_other_connections() {
    let mut e = Serve::start();
    let mut writer = e.hello();
    let mut watcher = e.hello();
    let status = writer.call(method::ENGINE_STATUS, json!({})).unwrap();
    let features = status["features"].as_array().unwrap();
    assert!(features.contains(&json!("settings")));
    assert!(features.contains(&json!(ms_ipc::feature::EXPIRATION_EXECUTOR)));
    for c in [&mut writer, &mut watcher] {
        // A class filter doesn't hold settings.changed back.
        c.call(method::SUBSCRIBE, json!({ "classes": ["Task"] }))
            .unwrap();
    }
    let empty = writer.call(method::SETTINGS_GET, json!({})).unwrap();
    assert_eq!(empty["values"], json!({}));
    let store_id = empty["storeId"].as_str().unwrap().to_owned();

    let values =
        json!({ "core.sending.undoSend": 15000, "signatures": { "s1": { "body": "<b>hi</b>" } } });
    assert_eq!(
        writer
            .call(method::SETTINGS_SET, json!({ "values": values }))
            .unwrap(),
        json!({})
    );
    let n = next_matching(&mut watcher, |v| v["method"] == method::SETTINGS_CHANGED);
    assert_eq!(n["params"], json!({ "values": values }));
    let got = watcher
        .call(
            method::SETTINGS_GET,
            json!({ "keys": ["core.sending.undoSend", "core.composing.html"] }),
        )
        .unwrap();
    assert_eq!(
        got,
        json!({ "values": { "core.sending.undoSend": 15000 }, "storeId": store_id })
    );
    let mut file: Value =
        serde_json::from_slice(&std::fs::read(e.dir.path().join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(file["$storeId"], json!(store_id));
    file.as_object_mut().unwrap().remove("$storeId");
    assert_eq!(file, values);

    // The parent's change reaches every subscriber, the writer included; a cleared key is
    // null in the notification.
    e.parent
        .call(
            method::SETTINGS_SET,
            json!({ "values": { "signatures": null, "core.sending.undoSend": 15000 } }),
        )
        .unwrap();
    for c in [&mut writer, &mut watcher] {
        let n = next_matching(c, |v| v["method"] == method::SETTINGS_CHANGED);
        assert_eq!(n["params"], json!({ "values": { "signatures": null } }));
    }
    // The writer saw nothing of its own first change.
    writer
        .call(
            method::SETTINGS_SET,
            json!({ "values": { "core.composing.html": false } }),
        )
        .unwrap();
    watcher
        .call(
            method::SETTINGS_SET,
            json!({ "values": { "core.composing.html": true } }),
        )
        .unwrap();
    let n = next_matching(&mut writer, |v| v["method"] == method::SETTINGS_CHANGED);
    assert_eq!(
        n["params"],
        json!({ "values": { "core.composing.html": true } })
    );

    let bad = writer
        .call(
            method::SETTINGS_SET,
            json!({ "values": { "core.themes": ["x"] } }),
        )
        .unwrap_err();
    assert_eq!(bad.code(), Some(code::INVALID_PARAMS));
    let bad = writer
        .call(
            method::SETTINGS_SET,
            json!({ "values": { "core.sending.undoSend": "5s" } }),
        )
        .unwrap_err();
    assert_eq!(bad.code(), Some(code::INVALID_PARAMS));
    e.stop();
}

/// A relayed device without the admin scope may change the plain settings but not the ones
/// the desktop renders as HTML (signatures) or that act on mail (ms-ipc's policy).
#[test]
fn relayed_settings_writes_are_limited() {
    let e = Serve::start();
    let mut relayed = Client::connect(&e.socket).unwrap();
    relayed.set_read_timeout(Some(WAIT)).unwrap();
    relayed
        .call(
            method::HELLO,
            json!({ "protocol": 1, "client": "tern-gateway/test", "relayed": true,
                    "scopes": [], "device": { "id": "d1", "name": "Test browser" } }),
        )
        .unwrap();
    let html = json!({ "signatures": { "s1": { "body": "<img src=x onerror=alert(1)>" } } });
    let refused = relayed
        .call(method::SETTINGS_SET, json!({ "values": html }))
        .unwrap_err();
    assert_eq!(refused.code(), Some(code::FORBIDDEN_RELAYED));
    relayed
        .call(
            method::SETTINGS_SET,
            json!({ "values": { "core.sending.undoSend": 0 } }),
        )
        .unwrap();
    let got = e.hello().call(method::SETTINGS_GET, json!({})).unwrap();
    assert_eq!(got["values"], json!({ "core.sending.undoSend": 0 }));
    e.stop();
}

fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn registered_accounts_start_with_the_next_engine_until_forgotten() {
    let mut e = Serve::start_with(true, &[no_retry()]);
    let mut secret = account(A);
    secret["settings"]["imap_password"] = json!("imap-s3cret");
    let added = e.parent.add(secret);
    assert_eq!(added["stored"], true, "{added}");
    let accounts = e.dir.path().join("accounts.json");
    let text = std::fs::read_to_string(&accounts).unwrap();
    assert!(text.contains(A) && !text.contains("s3cret"), "{text}");
    assert_eq!(file_mode(&accounts), 0o600);
    let sealed = std::fs::read(e.dir.path().join("secrets.enc")).unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("s3cret"));
    assert_eq!(file_mode(&e.dir.path().join("secrets.enc")), 0o600);
    assert_eq!(file_mode(e.dir.path()), 0o700);

    let (mut e, hello) = e.restart(&[no_retry()]);
    assert_eq!(hello["accounts"][0]["accountId"], A, "{hello}");
    assert_eq!(hello["credentials"]["store"], "file", "{hello}");
    assert_eq!(hello["credentials"]["accounts"], 1, "{hello}");
    assert!(hello["features"]
        .as_array()
        .unwrap()
        .contains(&json!("accounts")));
    assert!(hello["build"].as_str().unwrap().starts_with("sha256:"));
    let mut c = e.hello();
    let deadline = Instant::now() + WAIT;
    let list = loop {
        let list = c.call(method::ACCOUNT_LIST, Value::Null).unwrap();
        if list["accounts"][0]["state"] == "running" {
            break list;
        }
        assert!(Instant::now() < deadline, "{list}");
        std::thread::sleep(Duration::from_millis(100));
    };
    let json = &list["accounts"][0]["account"];
    assert_eq!(json["settings"]["imap_host"], "127.0.0.1", "{list}");
    assert!(!list.to_string().contains("s3cret"), "{list}");
    // The client adding it again finds it running.
    let again = e
        .parent
        .call(
            method::ACCOUNT_ADD,
            json!({ "account": account(A), "identity": null }),
        )
        .unwrap_err();
    assert_eq!(again.code, code::ACCOUNT_EXISTS);

    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams::default()).unwrap();
    e.parent
        .call(method::ACCOUNT_REMOVE, json!({ "accountId": A }))
        .unwrap();
    next_matching(&mut sub, |v| is_state(v, A, "removed"));
    let list = c.call(method::ACCOUNT_LIST, Value::Null).unwrap();
    assert_eq!(list["accounts"], json!([]));

    let (e, hello) = e.restart(&[]);
    assert_eq!(hello["accounts"], json!([]));
    assert_eq!(hello["credentials"]["accounts"], 0);
    e.stop();
}

#[test]
fn credentials_import_maps_a_keyset_with_duplicate_emails() {
    let mut e = Serve::start();
    let bare = |id: &str| {
        let mut a = account(id);
        a["emailAddress"] = json!("same@example.com");
        let settings = a["settings"].as_object_mut().unwrap();
        settings.remove("imap_password");
        settings.remove("smtp_password");
        a
    };
    let mut c = e.hello();
    let refused = c
        .call(
            method::CREDENTIALS_IMPORT,
            json!({ "accounts": [], "secrets": {} }),
        )
        .unwrap_err();
    let ClientError::Rpc(obj) = &refused else {
        panic!("{refused:?}")
    };
    assert_eq!(obj.code, code::WRONG_CHANNEL);

    let mut other = bare("e-no-keys");
    other["emailAddress"] = json!("other@example.com");
    let result = e
        .parent
        .call(
            method::CREDENTIALS_IMPORT,
            json!({
                "accounts": [bare(A), bare(B), other],
                "secrets": {
                    "same@example.com-imap": "pw", "same@example.com-smtp": "pw",
                    "integration-deepl-key": "k", "tern-server-token": "t"
                }
            }),
        )
        .unwrap();
    assert_eq!(result["imported"], json!([A, B]), "{result}");
    assert_eq!(result["failed"][0]["accountId"], "e-no-keys", "{result}");
    assert_eq!(
        result["secrets"],
        json!(["integration:deepl", "tern-server-token"])
    );
    // Imported, not started: the client adds them, without their secrets.
    let list = c.call(method::ACCOUNT_LIST, Value::Null).unwrap();
    assert_eq!(
        states(&list),
        [
            (A.to_owned(), "stopped".to_owned()),
            (B.to_owned(), "stopped".to_owned())
        ]
    );
    assert_eq!(e.parent.add(bare(A))["state"], "running");
    assert_eq!(e.parent.add(bare(B))["state"], "running");
    e.stop();
}

#[test]
fn a_stopped_account_is_retried_until_it_fails_too_often() {
    let mut e = Serve::start_with(true, &[("TERN_ENGINE_RESTART_DELAY_MS", "100".to_owned())]);
    e.parent.add(account(A));
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams::default()).unwrap();
    for n in 1..=5 {
        e.parent
            .call(method::TEST_CRASH, json!({ "accountId": A }))
            .unwrap();
        let stopped = next_matching(&mut sub, |v| is_state(v, A, "stopped"));
        if n < 5 {
            assert!(stopped["params"]["retryAt"].is_i64(), "{stopped}");
            next_matching(&mut sub, |v| is_state(v, A, "running"));
        } else {
            assert!(stopped["params"].get("retryAt").is_none(), "{stopped}");
            assert!(stopped["params"].get("authFailed").is_none(), "{stopped}");
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    let mut c = e.hello();
    let list = c.call(method::ACCOUNT_LIST, Value::Null).unwrap();
    assert_eq!(states(&list), [(A.to_owned(), "stopped".to_owned())]);
    // account.add clears the history.
    assert_eq!(e.parent.add(account(A))["state"], "running");
    e.stop();
}

#[test]
fn a_rotated_token_is_stored_and_used_after_a_restart() {
    let (url, bodies) = token_server();
    let env = [("MAILSYNC_GOOGLE_TOKEN_URL", url), no_retry()];
    let mut e = Serve::start_with(true, &env);
    let added = e.parent.add(oauth_account(A));
    assert_eq!(added["stored"], true, "{added}");
    wait_for_log(&e.log(A), "Stored the rotated refresh token");
    assert!(e
        .parent
        .notes
        .iter()
        .all(|n| n["params"]["modelClass"] != "ProcessAccountSecretsUpdated"));

    let (mut e, _) = e.restart(&env);
    let deadline = Instant::now() + WAIT;
    while !bodies
        .lock()
        .unwrap()
        .iter()
        .any(|b| b.contains("refresh_token=rotated-s3cret"))
    {
        assert!(Instant::now() < deadline, "{:?}", bodies.lock().unwrap());
        std::thread::sleep(Duration::from_millis(100));
    }
    e.parent.call(method::ENGINE_STATUS, json!({})).unwrap();
    assert!(e
        .parent
        .notes
        .iter()
        .all(|n| n["params"]["modelClass"] != "ProcessAccountSecretsUpdated"));
    e.stop();
}

#[test]
fn a_detached_engine_migrates_and_takes_parent_methods_on_the_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("s").join("e.sock");
    // No `--mode migrate` first: serve migrates.
    let mut child = mailsync(dir.path())
        .args(["--mode", "serve", "--detached", "--socket"])
        .arg(&socket)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let discovery = dir.path().join("run").join("engine.json");
    let deadline = Instant::now() + WAIT;
    while !discovery.exists() {
        assert!(Instant::now() < deadline, "no discovery file");
        std::thread::sleep(Duration::from_millis(50));
    }
    let d: Value = serde_json::from_slice(&std::fs::read(&discovery).unwrap()).unwrap();
    assert_eq!(d["detached"], true, "{d}");
    let mut c = Client::connect(&socket).unwrap();
    c.set_read_timeout(Some(WAIT)).unwrap();
    let hello = c.hello("serve-test/1").unwrap();
    assert!(hello.detached, "{hello:?}");

    let added = c
        .call(
            method::ACCOUNT_ADD,
            json!({ "account": account(A), "identity": null }),
        )
        .unwrap();
    assert_eq!(added["state"], "running", "{added}");
    let refused = c
        .call(
            method::CREDENTIALS_IMPORT,
            json!({ "accounts": [], "secrets": {} }),
        )
        .unwrap_err();
    let ClientError::Rpc(obj) = &refused else {
        panic!("{refused:?}")
    };
    assert_eq!(obj.code, code::WRONG_CHANNEL);
    assert_eq!(
        c.call(method::ENGINE_SHUTDOWN, json!({})).unwrap(),
        json!({})
    );
    let deadline = Instant::now() + WAIT;
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < deadline, "did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(0));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(stderr, "");
    assert!(!discovery.exists());
}
