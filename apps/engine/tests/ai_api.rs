// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `ai.*` on `serve`'s socket against airpost-ai's fake provider,
//! with a database seeded before launch and an account whose servers refuse connections.

#![cfg(unix)]

#[path = "../../../crates/airpost-ai/tests/common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use airpost_ai::ProviderKind;
use airpost_ipc::{code, feature, method, Client, ClientError, Message, Request};
use common::{
    anthropic_stream, ollama_stream, openai_stream, stream_reply, MockServer, Reply, ALL_KINDS,
    KEY, NDJSON, SSE,
};
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
        "aliases": ["Bo at work <bo@work.example>"],
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

fn airpost_engine(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_airpost-engine"));
    c.env("CONFIG_DIR_PATH", config)
        .env("AIRPOST_ENGINE_KEY_FILE", config.join("engine.key"))
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
    let c = Connection::open(config.join("airpost.db")).unwrap();
    insert_folder(&c, "f-inbox", "inbox");
    insert_folder(&c, "f-spam", "spam");
    insert_folder(&c, "f-drafts", "drafts");
    insert_thread(&c, "t1", "Lunch");
    c.execute(
        "INSERT INTO ThreadCategory (id, value, inAllMail, unread, lastMessageReceivedTimestamp, \
         lastMessageSentTimestamp) VALUES ('t1', 'f-inbox', 1, 0, 0, 0)",
        [],
    )
    .unwrap();
    insert_thread(&c, "t-spam", "Prize");
    insert_thread(&c, "t-pgp", "Keys");
    insert_thread(&c, "t-alias", "Invoice");
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
    // Bo answers from an alias after Ann's question.
    insert_message(
        &c,
        Msg {
            id: "a1",
            thread: "t-alias",
            date: base,
            from: ann,
            folder: "f-inbox",
            body: "<p>Did the invoice arrive?</p>",
            extra: json!({}),
        },
    );
    insert_message(
        &c,
        Msg {
            id: "a2",
            thread: "t-alias",
            date: base + 10,
            from: ("Bo at work", "Bo@Work.example"),
            folder: "f-inbox",
            body: "<p>ALIAS REPLY</p>",
            extra: json!({}),
        },
    );
}

struct Serve {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
    stdin: ChildStdin,
}

impl Serve {
    fn start() -> Serve {
        let dir = tempfile::tempdir().unwrap();
        let migrated = airpost_engine(dir.path())
            .args(["--mode", "migrate"])
            .output()
            .unwrap();
        assert!(migrated.status.success());
        seed(dir.path());
        let socket = dir.path().join("s").join("e.sock");
        let mut child = airpost_engine(dir.path())
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
            stdin,
        }
    }

    /// Sends `method` on the parent's stdin; its answer is not read.
    fn parent(&mut self, method: &str, params: Value) {
        let line = json!({"jsonrpc": "2.0", "id": 99, "method": method, "params": params});
        writeln!(self.stdin, "{line}").unwrap();
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

/// Generation requests `server` got, leaving out the model list the engine may fetch at any time.
fn generations(server: &MockServer) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.path() != "/api/tags")
        .count()
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

/// 50 chunks 150 ms apart, the first after `first`.
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
        Reply::stream(content_type, chunks, Duration::from_millis(150)).first_chunk_after(first);
    MockServer::start(move |_| reply.clone())
}

fn rpc_error(e: ClientError) -> airpost_ipc::ErrorObject {
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
fn stream(c: &mut Client, req: Request, stream: &str) -> (Vec<String>, airpost_ipc::Response) {
    let id = req.id.clone();
    c.send_raw(&airpost_ipc::encode(&req)).unwrap();
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
            json!({"available": false, "reason": "not-set-up"}),
            "{f}"
        );
    }
    let features = hello["features"].as_array().unwrap();
    assert!(features.contains(&json!(feature::AI_TRIAGE)));
    assert!(features.contains(&json!(feature::AI_KEYS)));
    assert_eq!(
        ai["triage"],
        json!({"available": false, "reason": "not-set-up"})
    );
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
    c.send_raw(&airpost_ipc::encode(&req)).unwrap();
    // The same streamId can't start twice while it streams.
    let dup = Request::new(
        6,
        method::AI_DRAFT_REPLY,
        thread_params(&server, json!({"streamId": "s1"})),
    );
    c.send_raw(&airpost_ipc::encode(&dup)).unwrap();
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
                    c.send_raw(&airpost_ipc::encode(&cancel)).unwrap();
                }
            }
            Message::Response(r) => answers.push(r),
            _ => {}
        }
    }
    let by_id = |id: i64| {
        answers
            .iter()
            .find(|r| r.id == Some(airpost_ipc::Id::Num(id)))
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
    c.send_raw(&airpost_ipc::encode(&req)).unwrap();
    let start = Instant::now();
    while server.count() == 0 {
        assert!(start.elapsed() < WAIT, "the provider got no request");
        std::thread::sleep(Duration::from_millis(20));
    }
    let cancel = Request::new(6, method::AI_CANCEL, json!({"streamId": "r1"}));
    c.send_raw(&airpost_ipc::encode(&cancel)).unwrap();
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
        .find(|r| r.id == Some(airpost_ipc::Id::Num(5)))
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
    c.send_raw(&airpost_ipc::encode(&req)).unwrap();
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

// -- Stored settings and triage --------------------------------------------------------------

/// The `core.ai.*` values for the Ollama fake at `server`, with `features`.
fn stored(server: &MockServer, features: Value) -> Value {
    json!({
        "core.ai.providers": [{"id": "local", "kind": "ollama", "name": "Laptop",
                               "url": server.url(), "model": "m-1"}],
        "core.ai.features": features,
        "core.ai.accounts": [ACCOUNT_ID],
    })
}

fn store_settings(c: &mut Client, values: Value) {
    c.call(method::SETTINGS_SET, json!({ "values": values }))
        .unwrap();
}

/// The thread's `airpost-ai-triage` value, if it has one.
fn triage_value(c: &mut Client, thread: &str) -> Option<Value> {
    let r = c
        .call(
            method::THREADS_GET,
            json!({"accountId": ACCOUNT_ID, "id": thread}),
        )
        .unwrap();
    r["thread"]["metadata"]
        .as_array()?
        .iter()
        .find(|m| m["pluginId"] == "airpost-ai-triage")
        .map(|m| m["value"].clone())
        .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
}

#[track_caller]
fn wait_for_triage(c: &mut Client, thread: &str) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(v) = triage_value(c, thread) {
            return v;
        }
        assert!(Instant::now() < deadline, "no triage value on {thread}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Sends `ai.triage.run` and reads its answer and progress until `done == total` or
/// `stopped`, keeping the `capabilities.changed` that came meanwhile.
fn run_job_seeing(c: &mut Client, params: Value) -> (Value, Vec<Value>, Vec<Value>) {
    let req = Request::new(airpost_ipc::Id::Num(9001), method::AI_TRIAGE_RUN, params);
    c.send_raw(&airpost_ipc::encode(&req)).unwrap();
    let mut answer = None;
    let mut progress = Vec::new();
    let mut capabilities = Vec::new();
    loop {
        match c.read_message().unwrap() {
            Message::Response(r) if r.id == req.id => {
                assert!(progress.is_empty(), "progress before the answer");
                answer = Some(r.into_result().unwrap());
            }
            Message::Notification(n) if n.method == method::AI_TRIAGE_PROGRESS => {
                let done = n.params["done"] == n.params["total"] || n.params["stopped"] == true;
                progress.push(n.params);
                if done {
                    return (answer.expect("answered"), progress, capabilities);
                }
            }
            Message::Notification(n) if n.method == method::CAPABILITIES_CHANGED => {
                capabilities.push(n.params["ai"]["triage"].clone());
            }
            _ => {}
        }
    }
}

fn run_job(c: &mut Client, params: Value) -> (Value, Vec<Value>) {
    let (answer, progress, _) = run_job_seeing(c, params);
    (answer, progress)
}

const TRIAGE_ANSWER: &str =
    r#"{"category": "needs-reply", "needsReply": true, "reason": "Ann asks about lunch."}"#;

#[test]
fn stored_settings_serve_calls_without_config() {
    let e = Serve::start();
    let server = ollama(&["From the store."]);
    let mut c = e.client();
    let bare = json!({"accountId": ACCOUNT_ID, "threadId": "t1"});
    assert_eq!(
        ai_key(c.call(method::AI_SUMMARIZE, bare.clone())),
        "not-enabled"
    );
    // Keys never go to the settings file.
    let keyed = json!({"core.ai.providers": [{"id": "k", "kind": "openai", "model": "m",
                                              "apiKey": SECRET}]});
    let err = c
        .call(method::SETTINGS_SET, json!({ "values": keyed }))
        .unwrap_err();
    assert_eq!(err.code(), Some(code::INVALID_PARAMS));
    store_settings(
        &mut c,
        stored(
            &server,
            json!({"summary": {"enabled": true, "provider": "local"}}),
        ),
    );
    let r = c.call(method::AI_SUMMARIZE, bare.clone()).unwrap();
    assert_eq!(r["text"], "From the store.");
    assert_eq!(server.last().json()["model"], "m-1");
    let models = c
        .call(method::AI_MODELS, json!({"provider": "local"}))
        .unwrap();
    assert_eq!(models.as_array().unwrap().len(), 2);
    // A request's own switches still win.
    let off = json!({"accountId": ACCOUNT_ID, "threadId": "t1",
                     "settings": {"accounts": [], "features": {}}});
    assert_eq!(ai_key(c.call(method::AI_SUMMARIZE, off)), "not-enabled");

    // A relayed device reads the stored settings, writes none of them, admin or not, and
    // runs on the stored provider.
    let mut relayed = Client::connect(&e.socket).unwrap();
    relayed.set_read_timeout(Some(WAIT)).unwrap();
    relayed
        .call(
            method::HELLO,
            json!({"protocol": 1, "client": "gateway", "relayed": true, "scopes": ["admin"]}),
        )
        .unwrap();
    let got = relayed
        .call(method::SETTINGS_GET, json!({"keys": ["core.ai.providers"]}))
        .unwrap();
    assert_eq!(got["values"]["core.ai.providers"][0]["id"], "local");
    let err = relayed
        .call(
            method::SETTINGS_SET,
            json!({"values": {"core.ai.accounts": []}}),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some(code::FORBIDDEN_RELAYED));
    let r = relayed
        .call(
            method::AI_SUMMARIZE,
            json!({"accountId": ACCOUNT_ID, "threadId": "t1", "refresh": true}),
        )
        .unwrap();
    assert_eq!(r["text"], "From the store.");
}

#[test]
fn daily_token_cap_refuses_calls_until_midnight() {
    let e = Serve::start();
    let server = ollama(&["Capped."]);
    let mut c = Recorded::connect(&e, json!({"protocol": 1, "client": "ai-test/1"}));
    c.call(method::SUBSCRIBE, json!({})).unwrap();
    let mut values = stored(
        &server,
        json!({"summary": {"enabled": true, "provider": "local"}}),
    );
    values["core.ai.limits"] = json!({"dailyTokens": {"local": 1}});
    c.call(method::SETTINGS_SET, json!({ "values": values }))
        .unwrap();
    let summary = |c: &mut Recorded| loop {
        let n = c.notification();
        if n.method == method::CAPABILITIES_CHANGED {
            return n.params["ai"]["summary"].clone();
        }
    };
    assert_eq!(summary(&mut c)["available"], true);
    let bare = json!({"accountId": ACCOUNT_ID, "threadId": "t1"});
    c.call(method::AI_SUMMARIZE, bare.clone()).unwrap();
    // The call that used up the cap says so, so a client doesn't offer the feature again.
    assert_eq!(
        summary(&mut c),
        json!({"available": false, "provider": "local", "local": true,
               "reason": "quota-exceeded"})
    );
    let err = c
        .call(
            method::AI_SUMMARIZE,
            json!({"accountId": ACCOUNT_ID, "threadId": "t1", "refresh": true}),
        )
        .unwrap_err();
    let data = err.data.unwrap();
    assert_eq!(data["key"], "quota-exceeded");
    assert!(data["retryAfter"].as_u64().unwrap() <= 25 * 3600);
    assert_eq!(server.count(), 1);
}

#[test]
fn triage_run_stores_results_and_lists_them() {
    let e = Serve::start();
    let server = ollama(&[TRIAGE_ANSWER]);
    let mut c = e.client();
    let run = json!({"accountId": ACCOUNT_ID, "threadIds": ["t1"]});
    assert_eq!(
        ai_key(c.call(method::AI_TRIAGE_RUN, run.clone())),
        "not-enabled"
    );
    store_settings(
        &mut c,
        stored(
            &server,
            json!({"triage": {"enabled": true, "provider": "local"}}),
        ),
    );
    let hello = c.call(method::HELLO, json!({"protocol": 1})).unwrap();
    assert_eq!(
        hello["capabilities"]["ai"]["triage"],
        json!({"available": true, "provider": "local", "local": true})
    );

    let (answer, progress) = run_job(&mut c, run.clone());
    assert_eq!(answer["queued"], 1);
    let job = answer["jobId"].as_str().unwrap();
    assert_eq!(progress, [json!({"jobId": job, "done": 1, "total": 1})]);
    let value = wait_for_triage(&mut c, "t1");
    assert_eq!(value["category"], "needs-reply");
    assert_eq!(value["needsReply"], true);
    assert_eq!(value["reason"], "Ann asks about lunch.");
    // The latest inbound message the model may read: m4 is encrypted, m5 is Bo's own.
    assert_eq!(value["messageId"], "m1");
    assert_eq!(value["model"], "m-1");
    assert_eq!(value["v"], airpost_ai::PROMPT_VERSION);
    // Only that message's body went, with the others' headers; nothing secret.
    let sent = server.last().json().to_string();
    assert!(sent.contains("Lunch on Friday?"), "{sent}");
    assert!(!sent.contains("Friday works."), "{sent}");
    assert!(!sent.contains(SECRET), "{sent}");

    let list = |c: &mut Client, p: Value| c.call(method::AI_TRIAGE_LIST, p).unwrap();
    assert_eq!(
        list(&mut c, json!({"category": "needs-reply", "limit": 10})),
        json!({"threadIds": ["t1"], "total": 1})
    );
    assert_eq!(
        list(&mut c, json!({"category": "newsletter", "limit": 10})),
        json!({"threadIds": [], "total": 0})
    );
    let err = c
        .call(
            method::AI_TRIAGE_LIST,
            json!({"category": "urgent", "limit": 10}),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some(code::INVALID_PARAMS));

    // Already triaged for m1 with this prompt: not asked again.
    let before = server.count();
    let (_, progress) = run_job(&mut c, run.clone());
    assert_eq!(progress.last().unwrap()["done"], 1);
    assert_eq!(server.count(), before);

    // A live list follows the stored results.
    let mut live = e.client();
    let sub = live
        .call(
            method::AI_TRIAGE_SUBSCRIBE,
            json!({"category": "needs-reply", "limit": 10, "key": "needs-reply"}),
        )
        .unwrap();
    assert_eq!(
        (&sub["threadIds"], &sub["total"]),
        (&json!(["t1"]), &json!(1))
    );
    let sub_id = sub["subId"].clone();

    // A correction by the user is never replaced.
    let corrected = json!({"category": "important", "needsReply": false, "source": "user"});
    c.call(
        method::TASK_QUEUE,
        json!({"accountId": ACCOUNT_ID, "task": {
            "__cls": "SyncbackMetadataTask", "id": "task-correct", "aid": ACCOUNT_ID,
            "status": "local", "modelId": "t1", "modelClassName": "thread",
            "pluginId": "airpost-ai-triage", "value": corrected}}),
    )
    .unwrap();
    let deadline = Instant::now() + WAIT;
    while triage_value(&mut c, "t1").unwrap()["source"] != "user" {
        assert!(Instant::now() < deadline, "the correction was not stored");
        std::thread::sleep(Duration::from_millis(50));
    }
    let deadline = Instant::now() + WAIT;
    let changed = loop {
        assert!(Instant::now() < deadline, "no ai.triage.changed");
        let n = live.next_notification().unwrap();
        if n.method == method::AI_TRIAGE_CHANGED && n.params["subId"] == sub_id {
            break n.params;
        }
    };
    assert_eq!(
        (&changed["threadIds"], &changed["total"]),
        (&json!([]), &json!(0))
    );
    live.call(method::AI_TRIAGE_UNSUBSCRIBE, json!({"subId": sub_id}))
        .unwrap();
    run_job(&mut c, run);
    assert_eq!(server.count(), before);
    assert_eq!(triage_value(&mut c, "t1").unwrap()["category"], "important");
    assert_eq!(
        list(&mut c, json!({"category": "needs-reply", "limit": 10}))["total"],
        0
    );
}

#[test]
fn triage_pauses_on_rate_limits_and_resumes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let e = Serve::start();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let server = MockServer::start(move |_| {
        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::json(429, json!({"error": "slow down"})).header("Retry-After", "1")
        } else {
            stream_reply(ProviderKind::Ollama, &[TRIAGE_ANSWER])
        }
    });
    let mut c = e.client();
    c.call(method::SUBSCRIBE, json!({})).unwrap();
    store_settings(
        &mut c,
        stored(
            &server,
            json!({"triage": {"enabled": true, "provider": "local"}}),
        ),
    );
    let started = Instant::now();
    let (_, progress, capabilities) = run_job_seeing(
        &mut c,
        json!({"accountId": ACCOUNT_ID, "threadIds": ["t1", "t-missing"]}),
    );
    assert_eq!(progress.len(), 2);
    // Unavailable while paused, available again once it resumed.
    let reasons: Vec<&Value> = capabilities.iter().map(|t| &t["reason"]).collect();
    assert!(
        reasons.contains(&&json!("rate-limited")),
        "{capabilities:?}"
    );
    assert_eq!(
        capabilities.last().unwrap()["available"],
        true,
        "{capabilities:?}"
    );
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(server.count(), 2);
    assert_eq!(wait_for_triage(&mut c, "t1")["category"], "needs-reply");
    let hello = c.call(method::HELLO, json!({"protocol": 1})).unwrap();
    assert_eq!(hello["capabilities"]["ai"]["triage"]["available"], true);
}

#[test]
fn triage_jobs_are_cancelled_by_their_connection() {
    let e = Serve::start();
    // The first chunk after 1 s, then 7.5 s of chunks: a cancel shows as an abort at the next
    // write.
    let server = slow_ollama(Duration::from_secs(1));
    let mut c = e.client();
    store_settings(
        &mut c,
        stored(
            &server,
            json!({"triage": {"enabled": true, "provider": "local"}}),
        ),
    );
    let r = c
        .call(
            method::AI_TRIAGE_RUN,
            json!({"accountId": ACCOUNT_ID, "threadIds": ["t1"]}),
        )
        .unwrap();
    let job = r["jobId"].as_str().unwrap().to_owned();
    let deadline = Instant::now() + WAIT;
    while server.count() == 0 {
        assert!(Instant::now() < deadline, "the job never asked");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Another connection can't cancel it.
    let mut other = e.client();
    other
        .call(method::AI_CANCEL, json!({"jobId": job.clone()}))
        .unwrap();
    assert!(!server.saw_abort(Duration::from_millis(1500)));
    c.call(method::AI_CANCEL, json!({"jobId": job})).unwrap();
    assert!(server.saw_abort(Duration::from_secs(5)));
    assert!(triage_value(&mut c, "t1").is_none());
}

/// What `watch_job` hands on: the client, the job id, and the notification's method and
/// params (`capabilities.ai.triage` for `capabilities.changed`). True ends the watch.
type Seen<'a> = dyn FnMut(&mut Client, &str, &str, &Value) -> bool + 'a;

/// Sends `ai.triage.run` on `c` and reads until its answer, then hands each later
/// `ai.triage.progress` and `capabilities.changed` to `seen` until it returns true. Answers
/// to requests sent meanwhile with `send_raw` are skipped.
fn watch_job(c: &mut Client, params: Value, seen: &mut Seen<'_>) -> String {
    let req = Request::new(airpost_ipc::Id::Num(9002), method::AI_TRIAGE_RUN, params);
    c.send_raw(&airpost_ipc::encode(&req)).unwrap();
    let mut job = None;
    loop {
        match c.read_message().unwrap() {
            Message::Response(r) if r.id == req.id => {
                job = Some(
                    r.into_result().unwrap()["jobId"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
            Message::Notification(n)
                if n.method == method::AI_TRIAGE_PROGRESS
                    || n.method == method::CAPABILITIES_CHANGED =>
            {
                let params = if n.method == method::CAPABILITIES_CHANGED {
                    n.params["ai"]["triage"].clone()
                } else {
                    n.params
                };
                let id = job.clone().expect("answered before any notification");
                if seen(c, &id, &n.method, &params) {
                    return id;
                }
            }
            _ => {}
        }
    }
}

fn send_cancel(c: &mut Client, job: &str) {
    let req = Request::new(
        airpost_ipc::Id::Num(9003),
        method::AI_CANCEL,
        json!({ "jobId": job }),
    );
    c.send_raw(&airpost_ipc::encode(&req)).unwrap();
}

#[test]
fn triage_waits_for_the_daily_cap_until_it_is_raised() {
    let e = Serve::start();
    let server = ollama(&[TRIAGE_ANSWER]);
    let mut c = e.client();
    c.call(method::SUBSCRIBE, json!({})).unwrap();
    let mut values = stored(
        &server,
        json!({"triage": {"enabled": true, "provider": "local"}}),
    );
    values["core.ai.limits"] = json!({"dailyTokens": {"local": 1}});
    store_settings(&mut c, values);
    let mut other = e.client();
    let mut reasons = Vec::new();
    let mut last = Value::Null;
    watch_job(
        &mut c,
        json!({"accountId": ACCOUNT_ID, "threadIds": ["t1", "t-pgp"]}),
        &mut |_, _, method, params| {
            if method == method::CAPABILITIES_CHANGED {
                reasons.push(params["reason"].clone());
                if params["reason"] == "quota-exceeded" && generations(&server) == 1 {
                    // The first answer used the cap: the second thread waits, until the cap
                    // is raised.
                    std::thread::sleep(Duration::from_millis(300));
                    assert_eq!(generations(&server), 1);
                    store_settings(
                        &mut other,
                        json!({"core.ai.limits": {"dailyTokens": {"local": 1_000_000}}}),
                    );
                }
                return false;
            }
            last = params.clone();
            params["done"] == 2 || params["stopped"] == true
        },
    );
    assert_eq!(last["done"], 2, "{last}");
    assert!(last.get("stopped").is_none(), "{last}");
    assert_eq!(generations(&server), 2);
    assert!(reasons.contains(&json!("quota-exceeded")), "{reasons:?}");
    let hello = c.call(method::HELLO, json!({"protocol": 1})).unwrap();
    assert_eq!(hello["capabilities"]["ai"]["triage"]["available"], true);
}

/// The cancel goes out without waiting for its reply, which can arrive after the job's stop, so
/// the closing `hello` uses a fresh client. The 15-minute default still holds the provider then.
#[test]
fn a_quota_answer_pauses_triage_and_a_cancel_ends_the_job_at_once() {
    let e = Serve::start();
    let server = MockServer::start(|_| Reply::json(402, json!({"error": "no credit"})));
    let mut c = e.client();
    c.call(method::SUBSCRIBE, json!({})).unwrap();
    store_settings(
        &mut c,
        stored(
            &server,
            json!({"triage": {"enabled": true, "provider": "local"}}),
        ),
    );
    let mut cancelled = None;
    let mut last = Value::Null;
    let job = watch_job(
        &mut c,
        json!({"accountId": ACCOUNT_ID, "threadIds": ["t1"]}),
        &mut |c, job, method, params| {
            if method == method::CAPABILITIES_CHANGED {
                if params["reason"] == "quota-exceeded" && cancelled.is_none() {
                    cancelled = Some(Instant::now());
                    send_cancel(c, job);
                }
                return false;
            }
            last = params.clone();
            params["stopped"] == true || params["done"] == params["total"]
        },
    );
    let waited = cancelled.expect("paused").elapsed();
    assert!(waited < Duration::from_secs(5), "{waited:?}");
    assert_eq!(
        last,
        json!({"jobId": job, "done": 0, "total": 1, "stopped": true})
    );
    assert_eq!(server.count(), 1);
    let hello = e
        .client()
        .call(method::HELLO, json!({"protocol": 1}))
        .unwrap();
    assert_eq!(
        hello["capabilities"]["ai"]["triage"]["reason"],
        "quota-exceeded"
    );
}

#[test]
fn turning_triage_off_ends_a_waiting_job() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let e = Serve::start();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let server = MockServer::start(move |_| {
        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::json(429, json!({"error": "slow down"})).header("Retry-After", "60")
        } else {
            stream_reply(ProviderKind::Ollama, &[TRIAGE_ANSWER])
        }
    });
    let mut c = e.client();
    c.call(method::SUBSCRIBE, json!({})).unwrap();
    let features = json!({"triage": {"enabled": true, "provider": "local"}});
    store_settings(&mut c, stored(&server, features));
    let mut other = e.client();
    let mut turned_off = None;
    let mut last = Value::Null;
    watch_job(
        &mut c,
        json!({"accountId": ACCOUNT_ID, "threadIds": ["t1", "t-pgp"]}),
        &mut |_, _, method, params| {
            if method == method::CAPABILITIES_CHANGED {
                if params["reason"] == "rate-limited" && turned_off.is_none() {
                    turned_off = Some(Instant::now());
                    store_settings(
                        &mut other,
                        json!({"core.ai.features": {"triage": {"enabled": false}}}),
                    );
                }
                return false;
            }
            last = params.clone();
            params["stopped"] == true || params["done"] == params["total"]
        },
    );
    let waited = turned_off.expect("paused").elapsed();
    assert!(waited < Duration::from_secs(5), "{waited:?}");
    assert_eq!(last["stopped"], true, "{last}");
    assert_eq!(last["done"], 0, "{last}");
    assert_eq!(server.count(), 1);
}

#[test]
fn an_account_stopping_ends_its_job() {
    let mut e = Serve::start();
    let server = slow_ollama(Duration::from_secs(3));
    let mut c = e.client();
    store_settings(
        &mut c,
        stored(
            &server,
            json!({"triage": {"enabled": true, "provider": "local"}}),
        ),
    );
    let r = c
        .call(
            method::AI_TRIAGE_RUN,
            json!({"accountId": ACCOUNT_ID, "threadIds": ["t1", "t-pgp"]}),
        )
        .unwrap();
    let job = r["jobId"].as_str().unwrap().to_owned();
    let deadline = Instant::now() + WAIT;
    while server.count() == 0 {
        assert!(Instant::now() < deadline, "the job never asked");
        std::thread::sleep(Duration::from_millis(20));
    }
    e.parent(method::ACCOUNT_REMOVE, json!({ "accountId": ACCOUNT_ID }));
    let progress = loop {
        let n = c.next_notification().unwrap();
        if n.method == method::AI_TRIAGE_PROGRESS {
            break n.params;
        }
    };
    assert_eq!(
        progress,
        json!({"jobId": job, "done": 0, "total": 2, "stopped": true})
    );
}

#[test]
fn triage_judges_the_latest_message_not_from_an_alias() {
    let e = Serve::start();
    let server = ollama(&[TRIAGE_ANSWER]);
    let mut c = e.client();
    store_settings(
        &mut c,
        stored(
            &server,
            json!({"triage": {"enabled": true, "provider": "local"}}),
        ),
    );
    run_job(
        &mut c,
        json!({"accountId": ACCOUNT_ID, "threadIds": ["t-alias"]}),
    );
    assert_eq!(wait_for_triage(&mut c, "t-alias")["messageId"], "a1");
    let sent = server.last().json().to_string();
    assert!(sent.contains("Did the invoice arrive?"), "{sent}");
    assert!(!sent.contains("ALIAS REPLY"), "{sent}");
}

/// A connection that keeps every message the engine sent on it, to check what it carried.
struct Recorded {
    c: Client,
    seen: Vec<String>,
    /// Notifications read while waiting for a response.
    pending: std::collections::VecDeque<airpost_ipc::Notification>,
    next: i64,
}

impl Recorded {
    fn connect(e: &Serve, hello: Value) -> Recorded {
        let c = Client::connect(&e.socket).unwrap();
        c.set_read_timeout(Some(WAIT)).unwrap();
        let mut r = Recorded {
            c,
            seen: Vec::new(),
            pending: Default::default(),
            next: 0,
        };
        r.call(method::HELLO, hello).unwrap();
        r
    }

    fn read(&mut self) -> Message {
        let m = self
            .c
            .read_message()
            .unwrap_or_else(|e| panic!("{e:?} after {:#?}", self.seen));
        self.seen.push(format!("{m:?}"));
        m
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, airpost_ipc::ErrorObject> {
        self.next += 1;
        let req = Request::new(airpost_ipc::Id::Num(20_000 + self.next), method, params);
        self.c.send_raw(&airpost_ipc::encode(&req)).unwrap();
        loop {
            match self.read() {
                Message::Response(r) if r.id == req.id => return r.into_result(),
                Message::Notification(n) => self.pending.push_back(n),
                _ => {}
            }
        }
    }

    fn notification(&mut self) -> airpost_ipc::Notification {
        if let Some(n) = self.pending.pop_front() {
            return n;
        }
        loop {
            if let Message::Notification(n) = self.read() {
                return n;
            }
        }
    }

    /// Reads until a `capabilities.changed` whose `ai.triage` passes `want`; its `ai`.
    fn capabilities_until(&mut self, want: impl Fn(&Value) -> bool) -> Value {
        loop {
            let n = self.notification();
            if n.method == method::CAPABILITIES_CHANGED && want(&n.params["ai"]["triage"]) {
                return n.params["ai"].clone();
            }
        }
    }

    /// Runs `ai.triage.run` and reads its progress to the end; the last progress.
    fn triage(&mut self, params: Value) -> Value {
        self.call(method::AI_TRIAGE_RUN, params).unwrap();
        loop {
            let n = self.notification();
            if n.method == method::AI_TRIAGE_PROGRESS
                && (n.params["done"] == n.params["total"] || n.params["stopped"] == true)
            {
                return n.params;
            }
        }
    }

    #[track_caller]
    fn assert_never_carried(&self, secret: &str) {
        assert!(!self.seen.is_empty());
        for m in &self.seen {
            assert!(!m.contains(secret), "the socket carried the key: {m}");
        }
    }
}

/// The engine holds the key: background triage and relayed
/// calls run on a keyed provider, the key goes to that provider only, and no message from
/// the engine carries it. The key comes in on the parent's stdin, as the desktop imports it.
#[test]
fn engine_held_keys_serve_triage_and_relayed_calls() {
    let mut e = Serve::start();
    let server = provider(ProviderKind::Anthropic, &[TRIAGE_ANSWER]);
    let mut c = Recorded::connect(&e, json!({"protocol": 1, "client": "ai-test/1"}));
    c.call(method::SUBSCRIBE, json!({})).unwrap();
    let values = json!({
        "core.ai.providers": [{"id": "claude", "kind": "anthropic", "url": server.url(),
                               "model": "m-1"}],
        "core.ai.features": {"summary": {"enabled": true, "provider": "claude"},
                             "triage": {"enabled": true, "provider": "claude"}},
        "core.ai.accounts": [ACCOUNT_ID],
    });
    c.call(method::SETTINGS_SET, json!({ "values": values }))
        .unwrap();
    let hello = c.call(method::HELLO, json!({"protocol": 1})).unwrap();
    for f in ["summary", "triage"] {
        assert_eq!(
            hello["capabilities"]["ai"][f]["reason"], "needs-engine-key",
            "{f}"
        );
    }
    let run = json!({"accountId": ACCOUNT_ID, "threadIds": ["t1"]});
    let refused = c.call(method::AI_TRIAGE_RUN, run.clone()).unwrap_err();
    assert_eq!(refused.data.unwrap()["key"], "not-configured");
    assert_eq!(server.count(), 0);

    e.parent(
        method::CREDENTIALS_IMPORT,
        json!({"accounts": [], "secrets": {"integration-ai-claude-key": KEY}}),
    );
    let ai = c.capabilities_until(|t| t["available"] == true);
    assert_eq!(
        ai["triage"],
        json!({"available": true, "provider": "claude", "local": true})
    );
    assert_eq!(ai["summary"]["available"], true);
    let has = c
        .call(
            method::SECRETS_HAS,
            json!({"names": ["integration:ai-claude"]}),
        )
        .unwrap();
    assert_eq!(has["integration:ai-claude"], true);

    // Background triage: no config, the stored provider and its stored key.
    let done = c.triage(run.clone());
    assert_eq!((&done["done"], &done["total"]), (&json!(1), &json!(1)));
    let value = c
        .call(
            method::THREADS_GET,
            json!({"accountId": ACCOUNT_ID, "id": "t1"}),
        )
        .unwrap()["thread"]["metadata"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["pluginId"] == "airpost-ai-triage")
        .map(|m| m["value"].clone())
        .unwrap();
    assert_eq!(value["category"], "needs-reply");
    assert_eq!(server.count(), 1);
    assert_eq!(server.last().header("x-api-key"), Some(KEY));

    // A relayed device sends no config and gets the stored key's answer.
    let mut relayed = Recorded::connect(
        &e,
        json!({"protocol": 1, "client": "gateway", "relayed": true, "scopes": []}),
    );
    let r = relayed
        .call(
            method::AI_SUMMARIZE,
            json!({"accountId": ACCOUNT_ID, "threadId": "t1"}),
        )
        .unwrap();
    assert!(r["text"].is_string());
    assert_eq!(server.count(), 2);
    assert_eq!(server.last().header("x-api-key"), Some(KEY));

    // A local client's config for another server doesn't get the stored key.
    let elsewhere = provider(ProviderKind::Anthropic, &["x"]);
    let mut config = json!({"id": "claude", "kind": "anthropic", "url": elsewhere.url(),
                            "model": "m-1"});
    let other = c.call(method::AI_TEST, json!({ "config": config.clone() }));
    assert_eq!(other.unwrap_err().data.unwrap()["key"], "not-configured");
    assert_eq!(elsewhere.count(), 0);
    // The same server does: Preferences' Test with the form's model.
    config["url"] = json!(server.url());
    c.call(method::AI_TEST, json!({ "config": config }))
        .unwrap();
    assert_eq!(server.last().header("x-api-key"), Some(KEY));

    // secrets.delete and secrets.set of the key change the capabilities.
    c.call(
        method::SECRETS_DELETE,
        json!({"name": "integration:ai-claude"}),
    )
    .unwrap();
    let ai = c.capabilities_until(|t| t["available"] == false);
    assert_eq!(ai["triage"]["reason"], "needs-engine-key");
    assert_eq!(ai["summary"]["reason"], "needs-engine-key");
    c.call(
        method::SECRETS_SET,
        json!({"name": "integration:ai-claude", "value": "sk-set-later"}),
    )
    .unwrap();
    c.capabilities_until(|t| t["available"] == true);
    c.call(
        method::AI_SUMMARIZE,
        json!({"accountId": ACCOUNT_ID, "threadId": "t1", "refresh": true}),
    )
    .unwrap();
    assert_eq!(server.last().header("x-api-key"), Some("sk-set-later"));

    c.assert_never_carried(KEY);
    c.assert_never_carried("sk-set-later");
    relayed.assert_never_carried(KEY);
}

/// A stored key belongs to its provider's server: `settings.set` pointing the provider at
/// another server, or removing it, deletes the key first, so no client can send it elsewhere
/// by rewriting `core.ai.providers`.
#[test]
fn a_provider_moved_or_removed_loses_its_key() {
    let e = Serve::start();
    let server = provider(ProviderKind::Anthropic, &["x", "x"]);
    let listener = provider(ProviderKind::Anthropic, &["x"]);
    let mut c = Recorded::connect(&e, json!({"protocol": 1, "client": "ai-test/1"}));
    let at = |url: String| {
        json!({"core.ai.providers": [{"id": "claude", "kind": "anthropic", "url": url,
                                      "model": "m-1"}]})
    };
    let has = |c: &mut Recorded| {
        c.call(
            method::SECRETS_HAS,
            json!({"names": ["integration:ai-claude"]}),
        )
        .unwrap()["integration:ai-claude"]
            == true
    };
    c.call(method::SETTINGS_SET, json!({ "values": at(server.url()) }))
        .unwrap();
    c.call(
        method::SECRETS_SET,
        json!({"name": "integration:ai-claude", "value": KEY}),
    )
    .unwrap();
    c.call(method::AI_TEST, json!({"provider": "claude"}))
        .unwrap();
    assert_eq!(server.last().header("x-api-key"), Some(KEY));

    // Another model on the same server keeps it; a refused write deletes nothing.
    let mut same = at(format!("{}/", server.url()));
    same["core.ai.providers"][0]["model"] = json!("m-2");
    c.call(method::SETTINGS_SET, json!({ "values": same }))
        .unwrap();
    let mut keyed = at(listener.url());
    keyed["core.ai.providers"][0]["apiKey"] = json!("sk-inline");
    let refused = c.call(method::SETTINGS_SET, json!({ "values": keyed }));
    assert_eq!(refused.unwrap_err().code, code::INVALID_PARAMS);
    assert!(has(&mut c));

    c.call(
        method::SETTINGS_SET,
        json!({ "values": at(listener.url()) }),
    )
    .unwrap();
    assert!(!has(&mut c));
    let moved = c.call(method::AI_TEST, json!({"provider": "claude"}));
    assert_eq!(moved.unwrap_err().data.unwrap()["key"], "not-configured");
    assert_eq!(listener.count(), 0);

    c.call(
        method::SECRETS_SET,
        json!({"name": "integration:ai-claude", "value": KEY}),
    )
    .unwrap();
    c.call(
        method::SETTINGS_SET,
        json!({"values": {"core.ai.providers": []}}),
    )
    .unwrap();
    assert!(!has(&mut c));
    c.assert_never_carried(KEY);
}
