// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `integrations.*` and `capabilities` on `serve`'s socket, against airpost-integrations' mock
//! provider server.

#![cfg(unix)]

#[path = "../../../crates/airpost-integrations/tests/common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use airpost_ipc::{code, feature, method, Client, Message, Request};
use common::{MockServer, Reply};
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(30);

struct Serve {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
    _stdin: ChildStdin,
}

impl Serve {
    fn start() -> Serve {
        let dir = tempfile::tempdir().unwrap();
        let airpost_engine = || {
            let mut c = Command::new(env!("CARGO_BIN_EXE_airpost-engine"));
            c.env("CONFIG_DIR_PATH", dir.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            c
        };
        let migrated = airpost_engine()
            .args(["--mode", "migrate"])
            .output()
            .unwrap();
        assert!(migrated.status.success());
        let socket = dir.path().join("s").join("e.sock");
        let mut child = airpost_engine()
            .args(["--mode", "serve", "--socket"])
            .arg(&socket)
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        // The parent's hello is answered once the socket is bound.
        use std::io::Write;
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": 1, "method": "hello", "params": {"protocol": 1}})
        )
        .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert!(line.contains("\"result\""), "{line}");
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
        c.hello("integrations-test/1").unwrap();
        c
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn next_matching(c: &mut Client, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        assert!(Instant::now() < deadline, "no matching notification");
        let n = c.next_notification().unwrap();
        let v = serde_json::to_value(&n).unwrap();
        if pred(&v) {
            return v;
        }
    }
}

fn libretranslate() -> MockServer {
    MockServer::start(|req| match req.path() {
        "/translate" if req.json()["q"] == "slow" => {
            Reply::json(200, json!({"translatedText": "lento"})).delayed(Duration::from_secs(2))
        }
        "/translate" => Reply::json(
            200,
            json!({"translatedText": "Hola", "detectedLanguage": {"confidence": 90.0, "language": "en"}}),
        ),
        "/detect" if req.json()["api_key"] == "bad" => {
            Reply::json(403, json!({"error": "Invalid API key"}))
        }
        "/detect" => Reply::json(200, json!([{"confidence": 80.0, "language": "en"}])),
        "/languages" => Reply::json(
            200,
            json!([{"code": "en", "name": "English", "targets": ["es"]}]),
        ),
        _ => Reply::json(404, json!({})),
    })
}

#[test]
fn hello_announces_integrations_and_capabilities_follow_the_settings() {
    let e = Serve::start();
    let mut c = e.client();
    let hello = c.call(method::HELLO, json!({"protocol": 1})).unwrap();
    for f in [feature::INTEGRATIONS, feature::CAPABILITIES] {
        assert!(hello["features"].as_array().unwrap().contains(&json!(f)));
    }
    assert_eq!(
        hello["capabilities"]["integrations"]["translation"],
        json!({"available": false, "reason": "not-set-up"})
    );
    assert_eq!(
        hello["capabilities"]["server"]["readReceipts"]["reason"],
        json!("needs-airpost-server")
    );

    let mut watcher = e.client();
    for x in [&mut c, &mut watcher] {
        x.call(method::SUBSCRIBE, json!({"accounts": []})).unwrap();
    }
    c.call(
        method::SETTINGS_SET,
        json!({"values": {"core.integrations.translation":
            {"provider": "deepl", "enabled": true}}}),
    )
    .unwrap();
    // Both get it, the writer too, whatever their filter.
    for x in [&mut c, &mut watcher] {
        let n = next_matching(x, |v| v["method"] == method::CAPABILITIES_CHANGED);
        assert_eq!(
            n["params"]["integrations"]["translation"],
            json!({"available": true, "provider": "deepl", "credentials": "request"})
        );
    }
    let status = watcher.call(method::ENGINE_STATUS, json!({})).unwrap();
    assert_eq!(
        status["capabilities"]["integrations"]["translation"]["available"],
        json!(true)
    );

    // A change that leaves the capabilities as they were sends none.
    c.call(
        method::SETTINGS_SET,
        json!({"values": {"core.integrations.translation":
            {"provider": "deepl", "enabled": true, "username": "x"}}}),
    )
    .unwrap();
    c.call(
        method::SETTINGS_SET,
        json!({"values": {"core.integrations.translation": null}}),
    )
    .unwrap();
    let n = next_matching(&mut watcher, |v| {
        v["method"] == method::CAPABILITIES_CHANGED || v["method"] == method::SETTINGS_CHANGED
    });
    assert_eq!(n["method"], method::SETTINGS_CHANGED, "{n}");
    let n = next_matching(&mut watcher, |v| {
        v["method"] == method::CAPABILITIES_CHANGED || v["method"] == method::SETTINGS_CHANGED
    });
    assert_eq!(n["method"], method::SETTINGS_CHANGED, "{n}");
    let n = next_matching(&mut watcher, |v| {
        v["method"] == method::CAPABILITIES_CHANGED
    });
    assert_eq!(
        n["params"]["integrations"]["translation"]["reason"],
        json!("not-set-up")
    );
}

#[test]
fn integrations_call_the_provider_and_map_its_errors() {
    let e = Serve::start();
    let server = libretranslate();
    let mut c = e.client();
    let config = json!({"provider": "libretranslate", "url": server.url(), "apiKey": "k1"});

    let t = c
        .call(
            method::INTEGRATIONS_TRANSLATE,
            json!({"config": config, "text": "<b>Hello</b>", "target": "es", "format": "html"}),
        )
        .unwrap();
    assert_eq!(t["translatedText"], "Hola");
    let sent = server.requests().pop().unwrap().json();
    assert_eq!(sent["q"], "<b>Hello</b>");
    assert_eq!(sent["format"], "html");
    assert_eq!(sent["api_key"], "k1");

    let d = c
        .call(
            method::INTEGRATIONS_DETECT,
            json!({"config": config, "text": "Hello"}),
        )
        .unwrap();
    assert_eq!(d[0]["language"], "en");
    let l = c
        .call(method::INTEGRATIONS_LANGUAGES, json!({"config": config}))
        .unwrap();
    assert_eq!(l[0]["code"], "en");
    let report = c
        .call(method::INTEGRATIONS_TEST, json!({"config": config}))
        .unwrap();
    assert_eq!(report["host"], server.url().trim_start_matches("http://"));

    let bad = json!({"provider": "libretranslate", "url": server.url(), "apiKey": "bad"});
    let err = c
        .call(method::INTEGRATIONS_TEST, json!({"config": bad}))
        .unwrap_err();
    let airpost_ipc::ClientError::Rpc(err) = err else {
        panic!("{err:?}")
    };
    assert_eq!(err.code, code::INTEGRATION_FAILED);
    let data = err.data.unwrap();
    assert_eq!(data["key"], "auth");
    assert_eq!(data["retryable"], false);
    assert!(!err.message.contains("bad"), "{}", err.message);

    let err = c
        .call(method::INTEGRATIONS_TRANSLATE, json!({"config": config}))
        .unwrap_err();
    assert_eq!(err.code(), Some(code::INVALID_PARAMS));
}

/// A slow provider call doesn't hold back the connection's later requests (T21).
#[test]
fn a_slow_integration_is_answered_after_later_requests() {
    let e = Serve::start();
    let server = libretranslate();
    let mut c = e.client();
    let config = json!({"provider": "libretranslate", "url": server.url()});
    let slow = Request::new(
        100,
        method::INTEGRATIONS_TRANSLATE,
        json!({"config": config, "text": "slow", "target": "es"}),
    );
    c.send_raw(&airpost_ipc::encode(&slow)).unwrap();
    let status = Request::new(101, method::ENGINE_STATUS, json!({}));
    c.send_raw(&airpost_ipc::encode(&status)).unwrap();
    let mut order = Vec::new();
    while order.len() < 2 {
        if let Message::Response(r) = c.read_message().unwrap() {
            order.push(serde_json::to_value(&r.id).unwrap());
            if r.id == Some(airpost_ipc::Id::Num(100)) {
                assert_eq!(r.result.unwrap()["translatedText"], "lento");
            }
        }
    }
    assert_eq!(order, vec![json!(101), json!(100)]);
}

/// A relayed device can't make the engine call a URL it names.
#[test]
fn relayed_connections_without_admin_may_not_call_integrations() {
    let e = Serve::start();
    let mut c = Client::connect(&e.socket).unwrap();
    c.set_read_timeout(Some(WAIT)).unwrap();
    c.call(
        method::HELLO,
        json!({"protocol": 1, "client": "gateway", "relayed": true, "scopes": []}),
    )
    .unwrap();
    let err = c
        .call(
            method::INTEGRATIONS_TEST,
            json!({"config": {"provider": "libretranslate", "url": "http://127.0.0.1:9"}}),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some(code::FORBIDDEN_RELAYED));
}

/// A stored key goes only to the url the settings name for its provider: a caller that
/// names another host gets no key from the store (T13: secrets are write-only).
#[test]
fn a_stored_key_goes_only_to_the_stored_url() {
    let e = Serve::start();
    let evil = libretranslate();
    let mine = libretranslate();
    let mut c = e.client();
    for provider in ["deepl", "libretranslate"] {
        c.call(
            method::SECRETS_SET,
            json!({"name": format!("integration:{provider}"), "value": "st0red"}),
        )
        .unwrap();
    }
    c.call(
        method::SETTINGS_SET,
        json!({"values": {"core.integrations.translation":
            {"provider": "libretranslate", "enabled": true, "url": mine.url()}}}),
    )
    .unwrap();

    for provider in ["deepl", "libretranslate"] {
        let _ = c.call(
            method::INTEGRATIONS_TEST,
            json!({"config": {"provider": provider, "url": evil.url()}}),
        );
    }
    for r in evil.requests() {
        let seen = format!("{} {:?} {}", r.target, r.headers, r.body);
        assert!(!seen.contains("st0red"), "{seen}");
    }

    c.call(
        method::INTEGRATIONS_TEST,
        json!({"config": {"provider": "libretranslate", "url": mine.url()}}),
    )
    .unwrap();
    assert!(
        mine.requests().iter().any(|r| r.body.contains("st0red")),
        "{:?}",
        mine.requests()
    );
}
