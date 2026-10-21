// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `files.get`, `files.read`, `files.put` and the relayed rules for them
//! on the debug binary,
//! against a seeded database and an account whose servers refuse connections. The body
//! fetch `files.get` triggers is covered by the harness (`test_mcp.py`'s `get_attachment`).

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine as _;
use ms_ipc::{code, feature, method, Client, FilesGetResult, FilesPutResult, FilesReadResult};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const ACCOUNT_ID: &str = "c0ffee-harness";
const ACCOUNT: &str = r#"{"id":"c0ffee-harness","emailAddress":"me@example.com","provider":"imap","settings":{"imap_host":"127.0.0.1","imap_port":1,"imap_username":"me","imap_password":"x","imap_security":"none","smtp_host":"127.0.0.1","smtp_port":1,"smtp_username":"me","smtp_password":"x","smtp_security":"none","imap_allow_insecure_ssl":false,"smtp_allow_insecure_ssl":false}}"#;
const ON_DISK: &[u8] = b"0123456789abcdef";

fn mailsync(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mailsync"));
    c.env("CONFIG_DIR_PATH", config)
        .env("MAILSYNC_FILES_WAIT_MS", "1500")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn insert_file(c: &Connection, id: &str, message: &str, name: &str, size: i64) {
    let data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": "File", "v": 1, "messageId": message,
        "filename": name, "contentType": "application/pdf", "size": size, "contentId": null});
    c.execute(
        "INSERT INTO File (id, accountId, version, data, filename) VALUES (?, ?, 1, ?, ?)",
        params![id, ACCOUNT_ID, data.to_string(), name],
    )
    .unwrap();
}

fn seed(config: &Path) {
    let c = Connection::open(config.join("tern.db")).unwrap();
    insert_file(&c, "filedisk1", "m-1", "a.pdf", ON_DISK.len() as i64);
    // Its message has no copy on any server, so the fetch never writes it.
    insert_file(&c, "filegone1", "m-2", "b.pdf", 99);
    let dir = config.join("files/fi/le/filedisk1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), ON_DISK).unwrap();
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
        assert!(line.contains("listening"), "{line}");
        // Deltas go to stdout in sync mode; keep the pipe drained.
        std::thread::spawn(move || std::io::copy(&mut stdout, &mut std::io::sink()));
        Engine { dir, socket, child }
    }

    fn client(&self) -> Client {
        let c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        c
    }

    fn hello(&self) -> Client {
        let mut c = self.client();
        c.hello("files-test/1").unwrap();
        c
    }

    fn relayed(&self, scopes: &[&str]) -> Client {
        let mut c = self.client();
        c.call(
            method::HELLO,
            json!({ "protocol": 1, "client": "tern-gateway/test", "relayed": true,
                    "scopes": scopes, "device": { "id": "d1", "name": "Test" } }),
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

fn get(c: &mut Client, file_id: &str) -> FilesGetResult {
    c.call_as(
        method::FILES_GET,
        json!({"accountId": ACCOUNT_ID, "fileId": file_id}),
    )
    .unwrap()
}

fn error_code(c: &mut Client, m: &str, p: Value) -> Option<i64> {
    c.call(m, p).unwrap_err().code()
}

#[test]
fn get_and_read() {
    let e = Engine::start();
    let mut c = e.hello();
    let hello = c.call(method::ENGINE_STATUS, json!({})).unwrap();
    assert!(hello["features"]
        .as_array()
        .unwrap()
        .contains(&json!(feature::FILES)));

    let got = get(&mut c, "filedisk1");
    assert!(got.downloaded);
    assert_eq!(got.size, ON_DISK.len() as u64);
    assert_eq!(got.content_type, "application/pdf");
    assert_eq!(got.filename, "a.pdf");
    let path = PathBuf::from(got.path.unwrap());
    assert!(path.starts_with(e.dir.path().join("files")));
    assert_eq!(std::fs::read(path).unwrap(), ON_DISK);

    let read = |c: &mut Client, offset: u64, len: Value| -> FilesReadResult {
        c.call_as(
            method::FILES_READ,
            json!({"accountId": ACCOUNT_ID, "fileId": "filedisk1", "offset": offset, "len": len}),
        )
        .unwrap()
    };
    let decode = |r: &FilesReadResult| {
        base64::engine::general_purpose::STANDARD
            .decode(&r.data)
            .unwrap()
    };
    let first = read(&mut c, 0, json!(10));
    assert_eq!(decode(&first), &ON_DISK[..10]);
    assert!(!first.eof);
    assert_eq!(first.size, 16);
    let rest = read(&mut c, 10, Value::Null);
    assert_eq!(decode(&rest), &ON_DISK[10..]);
    assert!(rest.eof);
    let past = read(&mut c, 100, json!(5));
    assert_eq!((past.offset, past.data.as_str(), past.eof), (16, "", true));

    assert_eq!(
        error_code(
            &mut c,
            method::FILES_GET,
            json!({"accountId": ACCOUNT_ID, "fileId": "nope"})
        ),
        Some(code::NOT_FOUND)
    );
    assert_eq!(
        error_code(
            &mut c,
            method::FILES_GET,
            json!({"accountId": "other", "fileId": "filedisk1"})
        ),
        Some(code::UNKNOWN_ACCOUNT)
    );
    assert_eq!(
        error_code(
            &mut c,
            method::FILES_READ,
            json!({"accountId": ACCOUNT_ID, "fileId": "filegone1"})
        ),
        Some(code::NOT_FOUND)
    );
    e.stop();
}

/// A file that never arrives: the answer comes after the wait, and the connection's later
/// requests are answered before it.
#[test]
fn get_waits_out_of_order() {
    let e = Engine::start();
    let mut c = e.hello();
    c.send_raw(
        format!(
            "{}\n",
            json!({"jsonrpc": "2.0", "id": "slow", "method": method::FILES_GET,
                   "params": {"accountId": ACCOUNT_ID, "fileId": "filegone1"}})
        )
        .as_bytes(),
    )
    .unwrap();
    let started = Instant::now();
    let status = c.call(method::ENGINE_STATUS, json!({})).unwrap();
    assert!(status["features"].is_array());
    assert!(started.elapsed() < Duration::from_millis(1200));
    let slow = loop {
        match c.read_message().unwrap() {
            ms_ipc::Message::Response(r) => break r,
            _ => continue,
        }
    };
    assert_eq!(slow.id, Some(ms_ipc::Id::Str("slow".into())));
    let got: FilesGetResult = serde_json::from_value(slow.result.unwrap()).unwrap();
    assert!(!got.downloaded);
    assert!(got.path.is_none());
    assert_eq!(got.size, 99);
    assert!(started.elapsed() >= Duration::from_millis(1400));
    e.stop();
}

#[test]
fn put_by_data_and_path() {
    let e = Engine::start();
    let mut c = e.hello();
    let put: FilesPutResult = c
        .call_as(
            method::FILES_PUT,
            json!({"accountId": ACCOUNT_ID, "name": "notes:1.txt", "contentType": "Text/Plain",
                   "data": base64::engine::general_purpose::STANDARD.encode(b"hello")}),
        )
        .unwrap();
    assert_eq!(put.file_id.len(), 40);
    assert_eq!(put.file["id"], json!(put.file_id));
    assert_eq!(put.file["__cls"], "File");
    assert_eq!(put.file["aid"], ACCOUNT_ID);
    assert_eq!(put.file["filename"], "notes:1.txt");
    assert_eq!(put.file["contentType"], "text/plain");
    assert_eq!(put.file["size"], 5);
    let got = get(&mut c, &put.file_id);
    assert!(got.downloaded);
    let path = PathBuf::from(got.path.unwrap());
    assert_eq!(path.file_name().unwrap(), "notes-1.txt");
    assert_eq!(std::fs::read(&path).unwrap(), b"hello");

    let src = e.dir.path().join("upload.bin");
    std::fs::write(&src, vec![7u8; 70_000]).unwrap();
    let by_path: FilesPutResult = c
        .call_as(
            method::FILES_PUT,
            json!({"accountId": ACCOUNT_ID, "name": "big.bin", "path": src}),
        )
        .unwrap();
    assert_eq!(by_path.file["size"], 70_000);
    assert_eq!(by_path.file["contentType"], "application/octet-stream");
    assert!(src.is_file(), "the caller's file is copied, not moved");
    assert_eq!(get(&mut c, &by_path.file_id).size, 70_000);

    let bad = |c: &mut Client, p: Value| error_code(c, method::FILES_PUT, p);
    let base = |extra: Value| {
        let mut p = json!({"accountId": ACCOUNT_ID, "name": "x", "data": ""});
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        p
    };
    let invalid = Some(code::INVALID_PARAMS);
    assert_eq!(bad(&mut c, base(json!({"name": " "}))), invalid);
    assert_eq!(bad(&mut c, base(json!({"contentType": "nope"}))), invalid);
    assert_eq!(bad(&mut c, base(json!({"data": "!!"}))), invalid);
    assert_eq!(bad(&mut c, base(json!({"path": "/etc/hosts"}))), invalid);
    assert_eq!(
        bad(
            &mut c,
            json!({"accountId": ACCOUNT_ID, "name": "x", "path": "relative.txt"})
        ),
        invalid
    );
    assert_eq!(
        bad(&mut c, json!({"accountId": ACCOUNT_ID, "name": "x"})),
        invalid
    );
    e.stop();
}

#[test]
fn relayed_rules() {
    let e = Engine::start();
    let forbidden = Some(code::FORBIDDEN_RELAYED);
    let mut browser = e.relayed(&[]);
    for m in [method::FILES_GET, method::FILES_READ, method::FILES_PUT] {
        assert_eq!(
            error_code(
                &mut browser,
                m,
                json!({"accountId": ACCOUNT_ID, "fileId": "filedisk1", "name": "x", "data": ""})
            ),
            forbidden,
            "{m}"
        );
    }
    // The raw-message export by file id is open to the web; by path it isn't.
    let export = |filepath: Value| {
        json!({"accountId": ACCOUNT_ID, "task": {"id": "t-eml", "__cls": "GetMessageRFC2822Task",
               "aid": ACCOUNT_ID, "messageId": "m-1", "filepath": filepath}})
    };
    let task = browser
        .call(method::TASK_QUEUE, export(Value::Null))
        .unwrap();
    assert_eq!(task["fileId"].as_str().unwrap().len(), 40);
    assert_eq!(
        error_code(
            &mut browser,
            method::TASK_QUEUE,
            export(json!("/tmp/x.eml"))
        ),
        forbidden
    );

    let mut admin = e.relayed(&["admin"]);
    assert!(get(&mut admin, "filedisk1").downloaded);
    let src = e.dir.path().join("secret.txt");
    std::fs::write(&src, b"secret").unwrap();
    assert_eq!(
        error_code(
            &mut admin,
            method::FILES_PUT,
            json!({"accountId": ACCOUNT_ID, "name": "x", "path": src})
        ),
        forbidden
    );
    // Positional params fill `path` as well; they're refused too.
    assert_eq!(
        error_code(
            &mut admin,
            method::FILES_PUT,
            json!([ACCOUNT_ID, "x", null, null, src])
        ),
        forbidden
    );
    let put: FilesPutResult = admin
        .call_as(
            method::FILES_PUT,
            json!({"accountId": ACCOUNT_ID, "name": "x", "data": "aGk="}),
        )
        .unwrap();
    assert_eq!(put.file["size"], 2);
    e.stop();
}
