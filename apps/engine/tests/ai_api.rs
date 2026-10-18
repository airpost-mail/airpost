// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `ai.*` on `serve`'s socket (docs/ai-plan.md section 3) against ms-ai's fake provider,
//! with a database seeded before launch and an account whose servers refuse connections.

#![cfg(unix)]

#[path = "../../../crates/ms-ai/tests/common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use common::{
    anthropic_stream, ollama_stream, openai_stream, stream_reply, MockServer, Reply, ALL_KINDS,
    KEY, NDJSON, SSE,
};
use ms_ai::ProviderKind;
use ms_ipc::{code, feature, method, Client, ClientError, Message, Request};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

// Ids starting with c-f have no start delay.
const ACCOUNT_ID: &str = "c0ffee-ai";
const WAIT: Duration = Duration::from_secs(30);
const SECRET: &str = "SECRET";

fn account() -> Value {
    json!({
        "id": ACCOUNT_ID,
        "name": "Bo Chen",
        "emailAddress": "bo@example.com",
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

fn mailsync(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mailsync"));
    c.env("CONFIG_DIR_PATH", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    c
}

fn insert_folder(c: &Connection, id: &str, role: &str) {
    let data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": "Folder", "v": 1, "path": id,
        "role": role});
    c.execute(
        "INSERT INTO Folder (id, accountId, version, data, path, role) VALUES (?, ?, 1, ?, ?, ?)",
        params![id, ACCOUNT_ID, data.to_string(), id, role],
    )
    .unwrap();
}

/// The participants as the store keeps them: everyone on any message, drafts and Spam
/// included.
fn insert_thread(c: &Connection, id: &str, subject: &str) {
    let data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": "Thread", "v": 1, "subject": subject,
        "participants": [{"name": "Ann Lee", "email": "ann@acme.io"},
                         {"name": "Bo Chen", "email": "bo@example.com"},
                         {"name": "Cy SECRET", "email": "cy@draft.example"},
                         {"name": "Dee SECRET", "email": "dee@spam.example"},
                         {"name": "Eve SECRET", "email": "eve@pgp.example"}]});
    c.execute(
        "INSERT INTO Thread (id, accountId, version, data, subject, lastMessageTimestamp, \
         lastMessageReceivedTimestamp) VALUES (?, ?, 1, ?, ?, 0, 0)",
        params![id, ACCOUNT_ID, data.to_string(), subject],
    )
    .unwrap();
}

struct Msg<'a> {
    id: &'a str,
    thread: &'a str,
    date: i64,
    from: (&'a str, &'a str),
    folder: &'a str,
    body: &'a str,
    extra: Value,
}

fn insert_message(c: &Connection, m: Msg<'_>) {
    let mut data = json!({"id": m.id, "aid": ACCOUNT_ID, "__cls": "Message", "v": 1,
        "threadId": m.thread, "date": m.date, "subject": "Lunch",
        "from": [{"name": m.from.0, "email": m.from.1}], "folders": {m.folder: 0},
        "files": [], "draft": false});
    for (k, v) in m.extra.as_object().unwrap() {
        data[k] = v.clone();
    }
    c.execute(
        "INSERT INTO Message (id, accountId, version, data, date, threadId) \
         VALUES (?, ?, 1, ?, ?, ?)",
        params![m.id, ACCOUNT_ID, data.to_string(), m.date, m.thread],
    )
    .unwrap();
    c.execute(
        "INSERT INTO MessageBody (id, value, fetchedAt) VALUES (?, ?, 0)",
        params![m.id, m.body],
    )
    .unwrap();
}

/// `t1` holds what may be sent and, marked SECRET, what never is: a draft to someone new, a
/// message only in Spam, an S/MIME one and quoted text. `t-spam` is all Spam. `t-pgp` has,
/// oldest first, an inline-PGP message, one longer than a small window's read cap and a
/// short one with a longer signature.
fn seed(config: &Path) {
    let c = Connection::open(config.join("edgehill.db")).unwrap();
    insert_folder(&c, "f-inbox", "inbox");
    insert_folder(&c, "f-spam", "spam");
    insert_folder(&c, "f-drafts", "drafts");
    insert_thread(&c, "t1", "Lunch");
    insert_thread(&c, "t-spam", "Prize");
    insert_thread(&c, "t-pgp", "Keys");
    let ann = ("Ann Lee", "ann@acme.io");
    let bo = ("Bo Chen", "bo@example.com");
    let dee = ("Dee SECRET", "dee@spam.example");
    let eve = ("Eve SECRET", "eve@pgp.example");
    let older = "Older words. ".repeat(700);
    let newest = format!("Newest note.\n-- \n{}", "Legal text. ".repeat(800));
    let base = 1_760_000_000;
    let rows = [
        Msg {
            id: "m1",
            thread: "t1",
            date: base,
            from: ann,
            folder: "f-inbox",
            body: "<p>Lunch on Friday? Mail me at ann@acme.io.</p>",
            extra: json!({}),
        },
        Msg {
            id: "m2",
            thread: "t1",
            date: base + 10,
            from: bo,
            folder: "f-drafts",
            body: "<p>DRAFT SECRET</p>",
            extra: json!({"draft": true,
                "to": [{"name": "Cy SECRET", "email": "cy@draft.example"}]}),
        },
        Msg {
            id: "m3",
            thread: "t1",
            date: base + 20,
            from: dee,
            folder: "f-spam",
            body: "<p>SPAM SECRET</p>",
            extra: json!({}),
        },
        Msg {
            id: "m4",
            thread: "t1",
            date: base + 30,
            from: ann,
            folder: "f-inbox",
            body: "<p>ENCRYPTED SECRET</p>",
            extra: json!({"files": [{"id": "f1", "filename": "smime.p7m",
                "contentType": "application/pkcs7-mime"}]}),
        },
        Msg {
            id: "m5",
            thread: "t1",
            date: base + 40,
            from: bo,
            folder: "f-inbox",
            body: "<p>Friday works.</p><blockquote type=\"cite\">QUOTED SECRET</blockquote>",
            extra: json!({"files": [{"id": "f2", "filename": "menu.pdf",
                "contentType": "application/pdf"}]}),
        },
        Msg {
            id: "m6",
            thread: "t-spam",
            date: base,
            from: ann,
            folder: "f-spam",
            body: "<p>You won</p>",
            extra: json!({}),
        },
        Msg {
            id: "p1",
            thread: "t-pgp",
            date: base,
            from: eve,
            folder: "f-inbox",
            body: "-----BEGIN PGP MESSAGE-----\nSECRET\n-----END PGP MESSAGE-----",
            extra: json!({"plaintext": true, "snippet": "-----BEGIN PGP MESSAGE----- SECRET"}),
        },
        Msg {
            id: "p2",
            thread: "t-pgp",
            date: base + 10,
            from: bo,
            folder: "f-inbox",
            body: &older,
            extra: json!({"plaintext": true}),
        },
        Msg {
            id: "p3",
            thread: "t-pgp",
            date: base + 20,
            from: ann,
            folder: "f-inbox",
            body: &newest,
            extra: json!({"plaintext": true}),
        },
    ];
    for m in rows {
        insert_message(&c, m);
    }
}

struct Serve {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
    _stdin: ChildStdin,
}

impl Serve {
    fn start() -> Serve {
        let dir = tempfile::tempdir().unwrap();
        let migrated = mailsync(dir.path())
            .args(["--mode", "migrate"])
            .output()
            .unwrap();
        assert!(migrated.status.success());
        seed(dir.path());
        let socket = dir.path().join("s").join("e.sock");
        let mut child = mailsync(dir.path())
            .args(["--mode", "serve", "--socket"])
            .arg(&socket)
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut call = |id: i64, method: &str, params: Value| {
            let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
            writeln!(stdin, "{line}").unwrap();
            loop {
                let mut line = String::new();
                assert!(stdout.read_line(&mut line).unwrap() > 0, "engine exited");
                let v: Value = serde_json::from_str(&line).unwrap();
                if v["id"] == id {
                    assert!(v.get("result").is_some(), "{v}");
                    return;
                }
            }
        };
        // The parent's hello is answered once the socket is bound.
        call(1, method::HELLO, json!({"protocol": 1}));
        call(
            2,
            method::ACCOUNT_ADD,
            json!({"account": account(), "identity": null}),
        );
        // Keep stdout drained so the engine never blocks on it.
        std::thread::spawn(move || for _ in stdout.lines() {});
        Serve {
            _dir: dir,
            socket,
            child,
            _stdin: stdin,
        }
    }

    fn client(&self) -> Client {
        let mut c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(WAIT)).unwrap();
        c.hello("ai-test/1").unwrap();
        c
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn config(server: &MockServer) -> Value {
    json!({"id": "local", "kind": "ollama", "url": server.url(), "model": "m-1"})
}

/// A config of `kind` pointing at `server`, with a key (Anthropic always needs one).
fn config_of(kind: ProviderKind, server: &MockServer) -> Value {
    let (name, url) = match kind {
        ProviderKind::OpenAi => ("openai", format!("{}/v1", server.url())),
        ProviderKind::Anthropic => ("anthropic", server.url()),
        ProviderKind::Ollama => ("ollama", server.url()),
    };
    json!({"id": "local", "kind": name, "url": url, "apiKey": KEY, "model": "m-1"})
}

fn settings() -> Value {
    json!({"accounts": [ACCOUNT_ID], "features": {"summary": true, "reply": {"enabled": true}}})
}

fn thread_params(server: &MockServer, extra: Value) -> Value {
    let mut p = json!({"accountId": ACCOUNT_ID, "threadId": "t1", "config": config(server),
        "settings": settings()});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

fn ollama(texts: &[&str]) -> MockServer {
    provider(ProviderKind::Ollama, texts)
}

/// Streams `texts` in `kind`'s wire format; Ollama's model list too.
fn provider(kind: ProviderKind, texts: &[&str]) -> MockServer {
    let texts: Vec<String> = texts.iter().map(|t| (*t).to_owned()).collect();
    MockServer::start(move |req| match req.path() {
        "/api/tags" => Reply::json(
            200,
            json!({"models": [{"name": "m-1", "model": "m-1"}, {"name": "m-2", "model": "m-2"}]}),
        ),
        _ => {
            let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
            stream_reply(kind, &texts)
        }
    })
}

/// 50 chunks 40 ms apart, the first after `first`.
fn slow_ollama(first: Duration) -> MockServer {
    slow_stream(ProviderKind::Ollama, first)
}

/// [`slow_ollama`] in `kind`'s wire format.
fn slow_stream(kind: ProviderKind, first: Duration) -> MockServer {
    let texts: Vec<String> = (0..50).map(|i| format!("w{i} ")).collect();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let (content_type, chunks) = match kind {
        ProviderKind::OpenAi => (SSE, openai_stream("m-1", &texts, Some((100, 10)))),
        ProviderKind::Anthropic => (SSE, anthropic_stream("m-1", &texts, (100, 10))),
        ProviderKind::Ollama => (NDJSON, ollama_stream("m-1", &texts, Some((100, 10)))),
    };
    let reply =
        Reply::stream(content_type, chunks, Duration::from_millis(40)).first_chunk_after(first);
    MockServer::start(move |_| reply.clone())
}

fn rpc_error(e: ClientError) -> ms_ipc::ErrorObject {
    match e {
        ClientError::Rpc(e) => e,
        other => panic!("{other:?}"),
    }
}

/// The `AiError` key of a `-32011` answer.
#[track_caller]
fn ai_key(r: Result<Value, ClientError>) -> String {
    let e = rpc_error(r.unwrap_err());
    assert_eq!(e.code, code::AI_FAILED, "{e:?}");
    e.data.unwrap()["key"].as_str().unwrap().to_owned()
}

/// Sends `req` and collects the `ai.delta` texts for `stream` until its response.
fn stream(c: &mut Client, req: Request, stream: &str) -> (Vec<String>, ms_ipc::Response) {
    let id = req.id.clone();
    c.send_raw(&ms_ipc::encode(&req)).unwrap();
    let mut texts = Vec::new();
    loop {
        match c.read_message().unwrap() {
            Message::Notification(n) if n.method == method::AI_DELTA => {
                assert_eq!(n.params["streamId"], stream);
                texts.push(n.params["text"].as_str().unwrap().to_owned());
            }
            Message::Response(r) if r.id == id => return (texts, r),
            _ => {}
        }
    }
}

#[test]
fn hello_announces_ai_and_its_capabilities() {
    let e = Serve::start();
    let mut c = e.client();
    let hello = c.call(method::HELLO, json!({"protocol": 1})).unwrap();
    assert!(hello["features"]
        .as_array()
        .unwrap()
        .contains(&json!(feature::AI)));
    let ai = &hello["capabilities"]["ai"];
    for f in ["summary", "reply"] {
        assert_eq!(
            ai[f],
            json!({"available": true, "credentials": "request"}),
            "{f}"
        );
    }
    assert!(ai.get("triage").is_none());
}

/// Nothing is read or sent until the account is opted in and the feature is on.
#[test]
fn opt_in_and_feature_switches_are_checked_first() {
    let e = Serve::start();
    let server = ollama(&["Fine."]);
    let mut c = e.client();
    let call =
        |c: &mut Client, extra: Value| c.call(method::AI_SUMMARIZE, thread_params(&server, extra));
    assert_eq!(
        ai_key(call(&mut c, json!({"settings": null}))),
        "not-enabled"
    );
    assert_eq!(
        ai_key(call(
            &mut c,
            json!({"settings": {"accounts": ["other"],
            "features": {"summary": true}}})
        )),
        "not-enabled"
    );
    let off = call(
        &mut c,
        json!({"settings": {"accounts": [ACCOUNT_ID], "features": {"summary": false, "reply": true}}}),
    );
    let e2 = rpc_error(off.unwrap_err());
    assert_eq!(e2.data.as_ref().unwrap()["key"], "not-enabled");
    assert!(e2.message.contains("summary"), "{}", e2.message);
    // reply is off for suggestions and drafts alike.
    let reply_off = json!({"settings": {"accounts": [ACCOUNT_ID], "features": {"summary": true}}});
    for m in [method::AI_SUGGEST_REPLIES, method::AI_DRAFT_REPLY] {
        assert_eq!(
            ai_key(c.call(m, thread_params(&server, reply_off.clone()))),
            "not-enabled",
            "{m}"
        );
    }
    assert_eq!(
        ai_key(call(&mut c, json!({"config": null, "provider": "local"}))),
        "not-configured"
    );
    assert_eq!(server.count(), 0);

    let unknown = call(&mut c, json!({"accountId": "nope"})).unwrap_err();
    assert_eq!(unknown.code(), Some(code::UNKNOWN_ACCOUNT));
    let missing = call(&mut c, json!({"threadId": "t-none"})).unwrap_err();
    assert_eq!(missing.code(), Some(code::NOT_FOUND));
    assert_eq!(
        ai_key(call(&mut c, json!({"threadId": "t-spam"}))),
        "unsupported"
    );
    let bad = call(&mut c, json!({"config": {"id": "x", "kind": "nope"}})).unwrap_err();
    assert_eq!(bad.code(), Some(code::INVALID_PARAMS));
    assert_eq!(server.count(), 0);
}

/// The text streams as `ai.delta` to the calling connection only, in order, ahead of the
/// response with the whole text, in each provider's wire format; what the provider gets
/// leaves out drafts, Spam, encrypted mail, their people, quoted text and addresses.
#[test]
fn summarize_streams_to_the_caller_only() {
    for kind in ALL_KINDS {
        summarize_streams_to_the_caller_only_with(kind);
    }
}

fn summarize_streams_to_the_caller_only_with(kind: ProviderKind) {
    let e = Serve::start();
    let server = provider(kind, &["Ann asks ", "about lunch ", "on Friday."]);
    let mut c = e.client();
    let mut other = e.client();
    other.call(method::SUBSCRIBE, json!({})).unwrap();

    let req = Request::new(
        7,
        method::AI_SUMMARIZE,
        thread_params(
            &server,
            json!({"streamId": "s1", "config": config_of(kind, &server)}),
        ),
    );
    let (texts, res) = stream(&mut c, req, "s1");
    assert_eq!(
        texts,
        ["Ann asks ", "about lunch ", "on Friday."],
        "{kind:?}"
    );
    let result = res.result.unwrap();
    assert_eq!(result["text"], "Ann asks about lunch on Friday.");
    assert_eq!(result["model"], "m-1");
    assert_eq!(result["cached"], false);
    assert_eq!(result["usage"]["inputTokens"], 100);

    let sent = server.last().body;
    for want in [
        "Lunch on Friday?",
        "Friday works.",
        "menu.pdf",
        "Ann Lee",
        "Bo Chen",
    ] {
        assert!(sent.contains(want), "{want} missing from {sent}");
    }
    assert!(!sent.contains(SECRET), "{sent}");
    assert!(!sent.contains("smime.p7m"), "{sent}");
    assert!(
        !sent.contains("@acme.io") && !sent.contains("bo@example.com"),
        "{sent}"
    );
    let body = server.last().json();
    assert_eq!(body["model"], "m-1");
    if kind == ProviderKind::Ollama {
        assert_eq!(body["options"]["num_ctx"], 8192);
    }

    // Cached: the whole text at once, nothing streamed.
    let req = Request::new(
        8,
        method::AI_SUMMARIZE,
        thread_params(
            &server,
            json!({"streamId": "s2", "config": config_of(kind, &server)}),
        ),
    );
    let (texts, res) = stream(&mut c, req, "s2");
    assert!(texts.is_empty());
    assert_eq!(res.result.unwrap()["cached"], true);
    assert_eq!(server.count(), 1);

    // The other connection got no ai.delta; its next message is its own answer.
    other
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    loop {
        match other.read_message() {
            Ok(Message::Notification(n)) => assert_ne!(n.method, method::AI_DELTA),
            Ok(m) => panic!("{m:?}"),
            Err(e) if e.is_timeout() => break,
            Err(e) => panic!("{e:?}"),
        }
    }

    let usage = c.call(method::AI_USAGE, json!({})).unwrap();
    assert_eq!(usage.as_array().unwrap().len(), 1, "{usage}");
    assert_eq!(usage[0]["provider"], "local");
    assert_eq!(usage[0]["feature"], "summary");
    assert_eq!(usage[0]["requests"], 1);
    assert_eq!(usage[0]["inputTokens"], 100, "{kind:?}");
    assert_eq!(usage[0]["outputTokens"], 10, "{kind:?}");
}

/// The read cap counts bodies without their signatures, and an inline-PGP message stays
/// out once the cap is spent.
#[test]
fn encrypted_mail_stays_out_past_the_read_cap() {
    let e = Serve::start();
    let server = ollama(&["ok"]);
    let mut c = e.client();
    let mut config = config(&server);
    // A read cap of 8000 characters: the newest signature is longer, the older body too.
    config["contextTokens"] = json!(1000);
    c.call(
        method::AI_SUMMARIZE,
        thread_params(&server, json!({"threadId": "t-pgp", "config": config})),
    )
    .unwrap();
    let sent = server.last().body;
    assert!(!sent.contains(SECRET), "{sent}");
    assert!(
        sent.contains("Message 1 of 2") && !sent.contains("of 3"),
        "{sent}"
    );
    assert!(
        sent.contains("Newest note.") && !sent.contains("Legal text"),
        "{sent}"
    );
    assert!(sent.contains("Older words."), "{sent}");
}

#[test]
fn addresses_go_only_when_the_settings_allow() {
    let e = Serve::start();
    let server = ollama(&["ok"]);
    let mut c = e.client();
    let mut s = settings();
    s["includeAddresses"] = json!(true);
    c.call(
        method::AI_SUMMARIZE,
        thread_params(&server, json!({"settings": s})),
    )
    .unwrap();
    assert!(server.last().body.contains("ann@acme.io"));
}

#[test]
fn suggestions_and_drafts() {
    let e = Serve::start();
    let answer = r#"{"suggestions": [{"label": "Yes", "text": "Friday works for me."},
        {"label": "No", "text": "Sorry, I can't make it."}]}"#;
    let server = ollama(&[answer]);
    let mut c = e.client();
    let s = c
        .call(
            method::AI_SUGGEST_REPLIES,
            thread_params(&server, json!({"count": 2})),
        )
        .unwrap();
    assert_eq!(s.as_array().unwrap().len(), 2);
    assert_eq!(
        s[0],
        json!({"label": "Yes", "text": "Friday works for me."})
    );

    let server = ollama(&["Hi Ann,\n\n", "Friday works."]);
    let req = Request::new(
        3,
        method::AI_DRAFT_REPLY,
        thread_params(
            &server,
            json!({"streamId": "d1", "instruction": "say I'll bring dessert"}),
        ),
    );
    let (texts, res) = stream(&mut c, req, "d1");
    assert_eq!(texts.concat(), "Hi Ann,\n\nFriday works.");
    assert_eq!(res.result.unwrap()["text"], "Hi Ann,\n\nFriday works.");
    assert!(server.last().body.contains("say I'll bring dessert"));
}

#[test]
fn models_and_test() {
    let e = Serve::start();
    let server = ollama(&["OK"]);
    let mut c = e.client();
    let models = c
        .call(method::AI_MODELS, json!({"config": config(&server)}))
        .unwrap();
    assert_eq!(models[0]["id"], "m-1");
    assert_eq!(models.as_array().unwrap().len(), 2);
    let report = c
        .call(method::AI_TEST, json!({"config": config(&server)}))
        .unwrap();
    assert_eq!(report["host"], "127.0.0.1");
    assert_eq!(report["local"], true);
    assert_eq!(report["model"], "m-1");
    assert_eq!(
        ai_key(c.call(method::AI_TEST, json!({"provider": "local"}))),
        "not-configured"
    );
}

/// `ai.cancel` is answered at once and stops the provider request mid-stream, in each
/// provider's wire format.
#[test]
fn cancel_stops_the_stream() {
    for kind in ALL_KINDS {
        cancel_stops_the_stream_with(kind);
    }
}

fn cancel_stops_the_stream_with(kind: ProviderKind) {
    let e = Serve::start();
    let server = slow_stream(kind, Duration::from_millis(40));
    let mut c = e.client();
    let req = Request::new(
        5,
        method::AI_SUMMARIZE,
        thread_params(
            &server,
            json!({"streamId": "s1", "config": config_of(kind, &server)}),
        ),
    );
    c.send_raw(&ms_ipc::encode(&req)).unwrap();
    // The same streamId can't start twice while it streams.
    let dup = Request::new(
        6,
        method::AI_DRAFT_REPLY,
        thread_params(&server, json!({"streamId": "s1"})),
    );
    c.send_raw(&ms_ipc::encode(&dup)).unwrap();
    let mut deltas = 0;
    let mut cancelled = false;
    let mut answers = Vec::new();
    while answers.len() < 3 {
        match c.read_message().unwrap() {
            Message::Notification(n) if n.method == method::AI_DELTA => {
                deltas += 1;
                if !cancelled && deltas == 2 {
                    cancelled = true;
                    let cancel = Request::new(9, method::AI_CANCEL, json!({"streamId": "s1"}));
                    c.send_raw(&ms_ipc::encode(&cancel)).unwrap();
                }
            }
            Message::Response(r) => answers.push(r),
            _ => {}
        }
    }
    let by_id = |id: i64| {
        answers
            .iter()
            .find(|r| r.id == Some(ms_ipc::Id::Num(id)))
            .unwrap()
            .clone()
    };
    assert_eq!(
        by_id(6).error.unwrap().code,
        code::INVALID_PARAMS,
        "duplicate streamId"
    );
    assert_eq!(by_id(9).result.unwrap(), json!({}));
    let e5 = by_id(5).error.unwrap();
    assert_eq!(e5.code, code::AI_FAILED);
    assert_eq!(e5.data.unwrap()["key"], "cancelled");
    assert!(deltas < 50, "{kind:?}: {deltas}");
    assert!(server.saw_abort(Duration::from_secs(5)), "{kind:?}");
}

/// Suggestions send no `ai.delta`, but their `streamId` lets `ai.cancel` stop them.
#[test]
fn cancel_stops_suggestions_by_stream_id() {
    let e = Serve::start();
    let server = slow_ollama(Duration::from_secs(2));
    let mut c = e.client();
    let req = Request::new(
        5,
        method::AI_SUGGEST_REPLIES,
        thread_params(&server, json!({"streamId": "r1"})),
    );
    c.send_raw(&ms_ipc::encode(&req)).unwrap();
    let start = Instant::now();
    while server.count() == 0 {
        assert!(start.elapsed() < WAIT, "the provider got no request");
        std::thread::sleep(Duration::from_millis(20));
    }
    let cancel = Request::new(6, method::AI_CANCEL, json!({"streamId": "r1"}));
    c.send_raw(&ms_ipc::encode(&cancel)).unwrap();
    let mut answers = Vec::new();
    while answers.len() < 2 {
        match c.read_message().unwrap() {
            Message::Notification(n) => assert_ne!(n.method, method::AI_DELTA),
            Message::Response(r) => answers.push(r),
            _ => {}
        }
    }
    let suggest = answers
        .iter()
        .find(|r| r.id == Some(ms_ipc::Id::Num(5)))
        .unwrap();
    let err = suggest.error.clone().unwrap();
    assert_eq!(err.code, code::AI_FAILED);
    assert_eq!(err.data.unwrap()["key"], "cancelled");
    assert!(server.saw_abort(Duration::from_secs(5)));
}

/// Closing the connection cancels its calls, also one still waiting for the first token.
#[test]
fn a_dropped_connection_cancels_the_provider_request() {
    let e = Serve::start();
    let server = slow_ollama(Duration::from_secs(2));
    let mut c = e.client();
    let req = Request::new(
        5,
        method::AI_SUMMARIZE,
        thread_params(&server, json!({"streamId": "s1"})),
    );
    c.send_raw(&ms_ipc::encode(&req)).unwrap();
    let start = Instant::now();
    while server.count() == 0 {
        assert!(start.elapsed() < WAIT, "the provider got no request");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(c);
    assert!(server.saw_abort(Duration::from_secs(5)));
    // The engine goes on serving.
    e.client().call(method::ENGINE_STATUS, json!({})).unwrap();
}

/// A relayed connection may not name the provider or the switches, and finds none of its
/// own yet.
#[test]
fn relayed_connections_may_not_pass_config_or_settings() {
    let e = Serve::start();
    let server = ollama(&["x"]);
    for scopes in [json!([]), json!(["admin"])] {
        let mut c = Client::connect(&e.socket).unwrap();
        c.set_read_timeout(Some(WAIT)).unwrap();
        c.call(
            method::HELLO,
            json!({"protocol": 1, "client": "gateway", "relayed": true, "scopes": scopes}),
        )
        .unwrap();
        for (m, p) in [
            (method::AI_TEST, json!({"config": config(&server)})),
            (method::AI_MODELS, json!({"config": config(&server)})),
            (method::AI_SUMMARIZE, thread_params(&server, json!({}))),
            (
                method::AI_SUMMARIZE,
                thread_params(&server, json!({"config": null, "provider": "local"})),
            ),
        ] {
            let err = c.call(m, p).unwrap_err();
            assert_eq!(err.code(), Some(code::FORBIDDEN_RELAYED), "{m}");
        }
        assert_eq!(
            ai_key(c.call(method::AI_TEST, json!({"provider": "local"}))),
            "not-configured"
        );
        assert_eq!(
            ai_key(c.call(
                method::AI_SUMMARIZE,
                json!({"accountId": ACCOUNT_ID, "threadId": "t1", "provider": "local"})
            )),
            "not-enabled"
        );
        c.call(method::AI_CANCEL, json!({"streamId": "s"})).unwrap();
        // Absent params read as `{}`.
        c.call(method::AI_USAGE, Value::Null).unwrap();
    }
    assert_eq!(server.count(), 0);
}
