// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Black-box checks of `--socket` (docs/engine-protocol-plan.md sections 1 and 2) on the
//! debug binary, against an account whose servers refuse connections.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use ms_ipc::{code, method, Client, ClientError, Message, SubscribeParams};
use serde_json::{json, Value};

const ACCOUNT_ID: &str = "c0ffee-harness";
const ACCOUNT: &str = r#"{"id":"c0ffee-harness","emailAddress":"me@example.com","provider":"imap","settings":{"imap_host":"127.0.0.1","imap_port":1,"imap_username":"me","imap_password":"x","imap_security":"none","smtp_host":"127.0.0.1","smtp_port":1,"smtp_username":"me","smtp_password":"x","smtp_security":"none","imap_allow_insecure_ssl":false,"smtp_allow_insecure_ssl":false}}"#;

fn mailsync(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mailsync"));
    c.env("CONFIG_DIR_PATH", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn migrate(config: &Path) {
    let status = mailsync(config)
        .args(["--mode", "migrate"])
        .output()
        .unwrap()
        .status;
    assert!(status.success());
}

fn wait_for_log(path: &Path, pattern: &str, timeout: Duration) -> String {
    let re = regex::Regex::new(pattern).unwrap();
    let start = Instant::now();
    loop {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if re.is_match(&text) {
            return text;
        }
        assert!(start.elapsed() < timeout, "{pattern} not in log:\n{text}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Engine {
    dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Engine {
    fn start() -> Self {
        Engine::start_with(&[])
    }

    fn start_with(env: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        migrate(dir.path());
        let socket = dir.path().join("run").join("e.sock");
        let mut cmd = mailsync(dir.path());
        cmd.args(["--mode", "sync", "--identity", "null", "--account", ACCOUNT])
            .arg("--socket")
            .arg(&socket);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let stdin = child.stdin.take();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        if line.is_empty() {
            let mut stderr = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
            panic!("exited with {:?}: {stderr}", child.wait().unwrap());
        }
        let expected = format!("{}\n", json!({ "listening": socket.to_string_lossy() }));
        assert_eq!(line, expected);
        Engine {
            dir,
            socket,
            child,
            stdin,
            stdout,
        }
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join(format!("mailsync-{ACCOUNT_ID}.log"))
    }

    fn client(&self) -> Client {
        let c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        c
    }

    fn hello(&self) -> Client {
        let mut c = self.client();
        c.hello("socket-test/1").unwrap();
        c
    }

    /// Kills the engine and checks that stdout carried nothing after the listening line and
    /// that stderr stayed empty.
    fn stop(mut self) {
        assert!(self.child.try_wait().unwrap().is_none(), "exited early");
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        let mut rest = String::new();
        self.stdout.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "", "stdout after the listening line");
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

fn task(id: &str) -> Value {
    json!({ "id": id, "__cls": "SendFeatureUsageEventTask", "aid": ACCOUNT_ID })
}

#[test]
fn hello_first_and_version_refusal() {
    let e = Engine::start();
    let mut c = e.client();

    let err = c.call(method::ENGINE_STATUS, Value::Null).unwrap_err();
    assert_eq!(err.code(), Some(code::HELLO_REQUIRED));

    let err = c
        .call(method::HELLO, json!({ "protocol": 2, "client": "x" }))
        .unwrap_err();
    let ClientError::Rpc(obj) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(obj.code, code::UNSUPPORTED_PROTOCOL);
    assert_eq!(obj.data, Some(json!({ "supported": [1] })));
    // Still not initialized.
    assert_eq!(
        c.call(method::ENGINE_STATUS, Value::Null)
            .unwrap_err()
            .code(),
        Some(code::HELLO_REQUIRED)
    );

    let hello = c.hello("socket-test/1").unwrap();
    assert_eq!(hello.protocol, 1);
    assert_eq!(hello.pid, e.child.id());
    assert_eq!(hello.accounts.len(), 1);
    assert_eq!(hello.accounts[0].account_id, ACCOUNT_ID);
    assert_eq!(hello.accounts[0].email, "me@example.com");
    assert_eq!(hello.accounts[0].state, "running");

    let status: ms_ipc::EngineStatusResult = c.call_as(method::ENGINE_STATUS, Value::Null).unwrap();
    assert_eq!(status.protocol, 1);
    assert_eq!(status.engine, hello.engine);
    assert_eq!(status.accounts[0].account_id, ACCOUNT_ID);
    e.stop();
}

#[test]
fn errors_parse_method_params_account() {
    let e = Engine::start();
    let mut c = e.hello();

    c.send_raw(b"garbage\n").unwrap();
    let Message::Response(r) = c.read_message().unwrap() else {
        panic!()
    };
    assert_eq!(r.id, None);
    assert_eq!(r.error.unwrap().code, code::PARSE_ERROR);

    c.send_raw(b"[1,2]\n").unwrap();
    let Message::Response(r) = c.read_message().unwrap() else {
        panic!()
    };
    assert_eq!(r.error.unwrap().code, code::INVALID_REQUEST);

    // A client notification gets no answer: the next response is the status call's.
    c.send_raw(br#"{"jsonrpc":"2.0","method":"sync.wake"}"#)
        .unwrap();
    c.call(method::ENGINE_STATUS, Value::Null).unwrap();

    let code_of = |c: &mut Client, m: &str, p: Value| c.call(m, p).unwrap_err().code();
    assert_eq!(
        code_of(&mut c, "folders.list", json!({})),
        Some(code::METHOD_NOT_FOUND)
    );
    assert_eq!(
        code_of(
            &mut c,
            method::TASK_CANCEL,
            json!({ "accountId": ACCOUNT_ID })
        ),
        Some(code::INVALID_PARAMS)
    );
    assert_eq!(
        code_of(
            &mut c,
            method::TASK_QUEUE,
            json!({ "accountId": ACCOUNT_ID, "task": [1] })
        ),
        Some(code::INVALID_PARAMS)
    );
    for (m, p) in [
        (
            method::TASK_CANCEL,
            json!({ "accountId": "nope", "taskId": "t" }),
        ),
        (
            method::TASK_QUEUE,
            json!({ "accountId": "nope", "task": task("t") }),
        ),
        (method::SYNC_WAKE, json!({ "accountId": "nope" })),
        (
            method::BODIES_FETCH,
            json!({ "accountId": "nope", "ids": [] }),
        ),
        (method::CALENDAR_SYNC, json!({ "accountId": "nope" })),
        (method::SUBSCRIBE, json!({ "accounts": ["nope"] })),
    ] {
        assert_eq!(code_of(&mut c, m, p), Some(code::UNKNOWN_ACCOUNT), "{m}");
    }
    e.stop();
}

#[test]
fn methods_and_subscribe_filter() {
    let e = Engine::start();
    let mut sub = e.hello();
    sub.subscribe(&SubscribeParams {
        accounts: Some(vec![ACCOUNT_ID.to_owned()]),
        classes: Some(vec!["Task".to_owned()]),
    })
    .unwrap();
    let mut c = e.hello();

    // The reply comes after the local phase: the task is in `remote`.
    let queued = c
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": ACCOUNT_ID, "task": task("t-sock-1") }),
        )
        .unwrap();
    assert_eq!(queued["id"], "t-sock-1");
    assert_eq!(queued["status"], "remote");

    // A failed local phase is a task-rejected error carrying the SyncError JSON.
    let mut bad = task("t-sock-2");
    bad["aid"] = json!("someone-else");
    let err = c
        .call(
            method::TASK_QUEUE,
            json!({ "accountId": ACCOUNT_ID, "task": bad }),
        )
        .unwrap_err();
    let ClientError::Rpc(obj) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(obj.code, code::TASK_REJECTED);
    assert!(obj.data.as_ref().unwrap()["key"].is_string(), "{obj:?}");

    assert_eq!(
        c.call(
            method::TASK_CANCEL,
            json!({ "accountId": ACCOUNT_ID, "taskId": "t-sock-1" })
        )
        .unwrap(),
        json!({})
    );
    assert_eq!(c.call(method::SYNC_WAKE, json!({})).unwrap(), json!({}));
    wait_for_log(
        &e.log(),
        r"(?m)\[main\] \[info\] Waking all workers\.\.\.\r?$",
        Duration::from_secs(10),
    );
    assert_eq!(
        c.call(
            method::BODIES_FETCH,
            json!({ "accountId": ACCOUNT_ID, "ids": ["m1"] })
        )
        .unwrap(),
        json!({})
    );
    let cal = c
        .call(method::CALENDAR_SYNC, json!({ "accountId": ACCOUNT_ID }))
        .unwrap();
    assert!(cal["started"].is_boolean(), "{cal}");

    // The subscriber sees the task's deltas, with the account id, and no other class:
    // ProcessState flows on this account (its servers refuse connections) but is filtered.
    let mut saw_cancel = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !saw_cancel && Instant::now() < deadline {
        let n = sub.next_notification().unwrap();
        assert_eq!(n.method, "delta");
        let p: ms_ipc::DeltaParams = serde_json::from_value(n.params).unwrap();
        assert_eq!(p.account_id, ACCOUNT_ID);
        assert_eq!(p.model_class, "Task");
        saw_cancel = p
            .model_jsons
            .iter()
            .any(|j| j["id"] == "t-sock-1" && j["should_cancel"] == true);
    }
    assert!(saw_cancel);

    // The non-subscriber got responses only.
    c.set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    assert!(c.read_message().unwrap_err().is_timeout());
    e.stop();
}

/// Deltas flushed before anyone subscribed reach the first unfiltered subscriber, as stdout
/// would have shown them; a filtered subscriber before it doesn't take them.
#[test]
fn first_subscriber_gets_the_backlog() {
    let e = Engine::start();
    let mut c = e.hello();
    c.call(
        method::TASK_QUEUE,
        json!({ "accountId": ACCOUNT_ID, "task": task("t-early") }),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(200));

    let mut filtered = e.hello();
    filtered
        .subscribe(&SubscribeParams {
            accounts: None,
            classes: Some(vec!["Task".to_owned()]),
        })
        .unwrap();
    filtered
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    assert!(filtered.read_message().unwrap_err().is_timeout());

    let mut sub = e.hello();
    let found = sub
        .subscribe(&SubscribeParams::default())
        .unwrap()
        .map(|n| n.unwrap())
        .any(|n| n.params["modelJSONs"][0]["id"] == "t-early");
    assert!(found);
    e.stop();
}

/// A request without an id runs but gets no answer, `subscribe` included.
#[test]
fn notifications_run_unanswered() {
    let e = Engine::start();
    let mut sub = e.hello();
    sub.send_raw(
        format!(
            "{}\n",
            json!({ "jsonrpc": "2.0", "method": method::SUBSCRIBE,
                    "params": { "classes": ["Task"] } })
        )
        .as_bytes(),
    )
    .unwrap();
    // Answered in order, so the subscription is in place once this returns.
    sub.call(method::ENGINE_STATUS, Value::Null).unwrap();

    let mut c = e.hello();
    c.send_raw(
        format!(
            "{}\n",
            json!({ "jsonrpc": "2.0", "method": method::TASK_QUEUE,
                    "params": { "accountId": ACCOUNT_ID, "task": task("t-notified") } })
        )
        .as_bytes(),
    )
    .unwrap();
    let n = sub.next_notification().unwrap();
    assert_eq!(n.params["modelJSONs"][0]["id"], "t-notified");
    c.call(method::ENGINE_STATUS, Value::Null).unwrap();
    e.stop();
}

/// SIGTERM, how the desktop stops an engine, removes the socket and discovery file, and the
/// engine still dies of the signal.
#[test]
fn sigterm_removes_the_socket_files() {
    use std::os::unix::process::ExitStatusExt;
    let mut e = Engine::start();
    let discovery = e.dir.path().join("run").join(format!("{ACCOUNT_ID}.json"));
    assert!(discovery.exists());
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(e.child.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());
    let status = e.child.wait().unwrap();
    assert_eq!(status.signal(), Some(15), "{status:?}");
    assert!(!e.socket.exists());
    assert!(!discovery.exists());
    let mut stderr = String::new();
    e.child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(stderr, "");
}

#[test]
fn stdin_still_takes_commands() {
    let mut e = Engine::start();
    let stdin = e.stdin.as_mut().unwrap();
    stdin.write_all(b"{\"type\":\"wake-workers\"}\n").unwrap();
    stdin.flush().unwrap();
    wait_for_log(
        &e.log(),
        r"(?m)\[main\] \[info\] Waking all workers\.\.\.\r?$",
        Duration::from_secs(10),
    );
    e.stop();
}

#[test]
fn files_and_modes() {
    let e = Engine::start();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&e.socket), 0o600);
    let run = e.dir.path().join("run");
    assert_eq!(mode(&run), 0o700);
    let discovery = run.join(format!("{ACCOUNT_ID}.json"));
    assert_eq!(mode(&discovery), 0o600);
    let d: Value = serde_json::from_slice(&std::fs::read(&discovery).unwrap()).unwrap();
    assert_eq!(d["pid"], e.child.id());
    assert_eq!(d["protocol"], 1);
    assert_eq!(d["accountId"], ACCOUNT_ID);
    assert_eq!(
        Path::new(d["socket"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        e.socket.canonicalize().unwrap()
    );
    e.stop();
}

/// A connection that never reads is dropped once its queue passes the bound, and the engine
/// carries on serving everyone else.
#[test]
fn slow_subscriber_is_dropped() {
    let e = Engine::start_with(&[("MAILSYNC_SOCKET_QUEUE_BYTES", "65536")]);
    let mut slow = e.hello();
    slow.subscribe(&SubscribeParams::default()).unwrap();

    let mut c = e.hello();
    let padding = "x".repeat(32 * 1024);
    for i in 0..200 {
        let mut t = task(&format!("t-slow-{i}"));
        t["padding"] = json!(padding);
        c.call(
            method::TASK_QUEUE,
            json!({ "accountId": ACCOUNT_ID, "task": t }),
        )
        .unwrap();
        let log = std::fs::read_to_string(e.log()).unwrap_or_default();
        if log.contains("-X Dropping socket connection") {
            break;
        }
    }
    wait_for_log(
        &e.log(),
        r"(?m)-X Dropping socket connection \d+: ",
        Duration::from_secs(10),
    );

    // The slow client now reads, well within the drop grace: the line in flight, then the
    // disconnect notification, then the close.
    let start = Instant::now();
    let mut disconnect = None;
    loop {
        match slow.read_message() {
            Ok(Message::Notification(n)) if n.method == method::DISCONNECT => {
                disconnect = Some(n.params)
            }
            Ok(_) => assert!(disconnect.is_none(), "a message after disconnect"),
            Err(ClientError::Closed) => break,
            Err(err) => panic!("{err:?}"),
        }
        assert!(start.elapsed() < Duration::from_secs(20), "never closed");
    }
    let reason = disconnect.expect("no disconnect notification")["reason"].clone();
    assert!(reason.as_str().unwrap().contains("fell behind"), "{reason}");
    c.call(method::ENGINE_STATUS, Value::Null).unwrap();
    e.hello().call(method::ENGINE_STATUS, Value::Null).unwrap();
    e.stop();
}

#[test]
fn stale_live_and_bad_socket_paths() {
    let dir = tempfile::tempdir().unwrap();
    migrate(dir.path());
    let run = |socket: &Path| {
        let mut child = mailsync(dir.path())
            .args(["--mode", "sync", "--identity", "null", "--account", ACCOUNT])
            .arg("--socket")
            .arg(socket)
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut first = String::new();
        stdout.read_line(&mut first).unwrap();
        if first.is_empty() {
            let mut stderr = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
            panic!("exited with {:?}: {stderr}", child.wait().unwrap());
        }
        (child, stdout, first)
    };

    // A socket file nobody listens on is replaced.
    let socket = dir.path().join("s.sock");
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    let (mut live, _live_out, first) = run(&socket);
    assert!(first.starts_with("{\"listening\":"), "{first}");
    Client::connect(&socket).unwrap().hello("t").unwrap();

    // A live one is not.
    let (mut second, mut out, first) = run(&socket);
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    assert_eq!(second.wait().unwrap().code(), Some(1));
    assert!(
        format!("{first}{rest}").starts_with("\n{\"error\":\"Another process is listening on "),
        "{first}{rest}"
    );
    Client::connect(&socket).unwrap().hello("t").unwrap();
    live.kill().unwrap();
    live.wait().unwrap();

    // Not a socket, and too long.
    let file = dir.path().join("plain");
    std::fs::write(&file, "").unwrap();
    let long = dir.path().join("d".repeat(120)).join("e.sock");
    for (path, expected) in [
        (&file, "is not a socket"),
        (&long, "Choose a shorter path."),
    ] {
        let (mut child, mut out, first) = run(path);
        let mut rest = String::new();
        out.read_to_string(&mut rest).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(1));
        let all = format!("{first}{rest}");
        assert!(all.starts_with("\n{\"error\":"), "{all}");
        assert!(all.contains(expected), "{all}");
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert_eq!(stderr, "");
    }
}

/// Takes 30 s: `cargo test -p mailsync -- --ignored`. Closing stdin still ends the engine,
/// and the socket and discovery file go with it.
#[test]
#[ignore]
fn exits_141_after_stdin_closes() {
    let mut e = Engine::start();
    e.hello().call(method::ENGINE_STATUS, Value::Null).unwrap();
    drop(e.stdin.take());
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
    assert!(!e
        .dir
        .path()
        .join("run")
        .join(format!("{ACCOUNT_ID}.json"))
        .exists());
}
