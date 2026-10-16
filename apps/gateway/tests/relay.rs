// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The relay against a real `mailsync --mode serve` (docs/web-gateway-plan.md 2.4, 2.6):
//! one account whose servers refuse connections, added on the engine's stdin, and a browser
//! played by a WebSocket client with the pairing cookie.
//!
//! The engine binary is `MAILSYNC_BIN`, else `target/debug/mailsync`; without one the test
//! skips, unless `CI` or `REQUIRE_TERN` is set.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tern_gateway::config::{Config, ConfigInput};
use tern_gateway::http::{close, now_ms, AppState, Server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const ACCOUNT_ID: &str = "c0ffee-gateway";
const WAIT: Duration = Duration::from_secs(20);

fn mailsync_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MAILSYNC_BIN") {
        return Some(PathBuf::from(p));
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/mailsync");
    if p.is_file() {
        return Some(p);
    }
    let required = std::env::var_os("CI").is_some() || std::env::var_os("REQUIRE_TERN").is_some();
    assert!(
        !required,
        "no mailsync binary: cargo build -p mailsync, or set MAILSYNC_BIN"
    );
    eprintln!("skipping: no mailsync binary (cargo build -p mailsync, or set MAILSYNC_BIN)");
    None
}

struct Engine {
    child: Child,
}

impl Engine {
    fn start(bin: &Path, config: &Path) -> Engine {
        let run = |args: &[&str]| {
            Command::new(bin)
                .env("CONFIG_DIR_PATH", config)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        };
        assert!(run(&["--mode", "migrate"]).wait().unwrap().success());
        let socket = config.join("s").join("e.sock");
        let mut child = run(&["--mode", "serve", "--socket", socket.to_str().unwrap()]);
        let account = json!({
            "id": ACCOUNT_ID, "emailAddress": "me@example.com", "provider": "imap",
            "settings": {
                "imap_host": "127.0.0.1", "imap_port": 1, "imap_username": "me",
                "imap_password": "s3cret-imap", "imap_security": "none",
                "smtp_host": "127.0.0.1", "smtp_port": 1, "smtp_username": "me",
                "smtp_password": "s3cret-smtp", "smtp_security": "none",
                "imap_allow_insecure_ssl": false, "smtp_allow_insecure_ssl": false
            }
        });
        let add = json!({ "jsonrpc": "2.0", "id": 1, "method": "account.add",
                          "params": { "account": account, "identity": null } });
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(format!("{add}\n").as_bytes()).unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert!(out.read_line(&mut line).unwrap() > 0, "serve exited");
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == 1 {
                assert!(v.get("error").is_none(), "{v}");
                break;
            }
        }
        // Keep draining stdout so the engine never blocks on it.
        std::thread::spawn(move || {
            let mut sink = String::new();
            while out.read_line(&mut sink).unwrap_or(0) > 0 {
                sink.clear();
            }
        });
        let start = Instant::now();
        while !config.join("run").join("engine.json").exists() {
            assert!(start.elapsed() < WAIT, "no run/engine.json");
            std::thread::sleep(Duration::from_millis(50));
        }
        Engine { child }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Setup {
    addr: SocketAddr,
    state: AppState,
    engine: Engine,
    _dir: tempfile::TempDir,
}

impl Setup {
    async fn start() -> Option<Setup> {
        let bin = mailsync_binary()?;
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::start(&bin, dir.path());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = Config::new(ConfigInput {
            bind: addr,
            public_url: None,
            insecure_public_url: false,
            allow_origins: &[],
            trusted_proxies: &[],
            web_root: None,
            config_dir: dir.path().to_owned(),
        })
        .unwrap();
        let state = AppState::new(config).unwrap();
        tokio::spawn(Server::with_listener(listener, state.clone()).run(std::future::pending()));
        Some(Setup {
            addr,
            state,
            engine,
            _dir: dir,
        })
    }

    fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn http(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
        let mut s = TcpStream::connect(self.addr).await.unwrap();
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            self.addr
        );
        for (k, v) in headers {
            req += &format!("{k}: {v}\r\n");
        }
        req += &format!("Content-Length: {}\r\n\r\n{body}", body.len());
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    /// Pairs a device and returns its `Cookie` header.
    async fn pair(&self) -> String {
        let code = self
            .state
            .devices()
            .mint_code(None, false, 600_000, now_ms())
            .unwrap();
        self.state.reload().unwrap();
        let res = self
            .http(
                "POST",
                "/gateway/v1/pair",
                &[("Origin", &self.origin())],
                &format!(r#"{{"code":"{code}"}}"#),
            )
            .await;
        let set = res
            .lines()
            .find_map(|l| l.strip_prefix("set-cookie: "))
            .unwrap_or_else(|| panic!("{res}"));
        set.split(';').next().unwrap().to_owned()
    }

    async fn ws(&self, cookie: &str) -> Ws {
        let mut req = format!("ws://{}/gateway/v1/ws", self.addr)
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("Origin", self.origin().parse().unwrap());
        req.headers_mut().insert("Cookie", cookie.parse().unwrap());
        Ws(tokio_tungstenite::connect_async(req).await.unwrap().0)
    }
}

struct Ws(tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>);

impl Ws {
    async fn send(&mut self, text: &str) {
        self.0.send(Message::Text(text.into())).await.unwrap();
    }

    async fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send(&req.to_string()).await;
        self.response(id).await
    }

    /// The next message, or None on a timeout. Panics on a close.
    async fn next(&mut self, wait: Duration) -> Option<Value> {
        loop {
            match tokio::time::timeout(wait, self.0.next()).await {
                Err(_) => return None,
                Ok(Some(Ok(Message::Text(t)))) => return Some(serde_json::from_str(&t).unwrap()),
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
                other => panic!("expected a message, got {other:?}"),
            }
        }
    }

    async fn response(&mut self, id: i64) -> Value {
        let start = Instant::now();
        loop {
            let v = self.next(WAIT).await.expect("no response");
            if v["id"] == id {
                return v;
            }
            assert!(start.elapsed() < WAIT, "no response {id}");
        }
    }

    async fn close_code(&mut self) -> u16 {
        loop {
            match tokio::time::timeout(WAIT, self.0.next()).await.unwrap() {
                Some(Ok(Message::Close(Some(f)))) => return f.code.into(),
                Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) => {}
                other => panic!("expected a close frame, got {other:?}"),
            }
        }
    }

    async fn hello(&mut self) -> Value {
        self.call(
            1,
            "hello",
            json!({ "protocol": 1, "client": "tern-web/test" }),
        )
        .await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn relays_with_policy() {
    let Some(s) = Setup::start().await else {
        return;
    };
    let session = s
        .http(
            "GET",
            "/gateway/v1/session",
            &[("Cookie", &s.pair().await)],
            "",
        )
        .await;
    assert!(session.contains(r#""state":"up""#), "{session}");

    let cookie = s.pair().await;
    let mut ws = s.ws(&cookie).await;
    let hello = ws.hello().await;
    let result = &hello["result"];
    assert!(result["features"]
        .as_array()
        .unwrap()
        .contains(&json!("relayed")));
    assert_eq!(result["gateway"]["scopes"], json!([]));
    assert_eq!(result["accounts"][0]["accountId"], ACCOUNT_ID);

    let list = ws.call(2, "account.list", Value::Null).await;
    let text = list.to_string();
    assert!(!text.contains("s3cret"), "{text}");
    assert_eq!(
        list["result"]["accounts"][0]["account"]["settings"]["imap_host"],
        "127.0.0.1"
    );

    assert_eq!(
        ws.call(
            3,
            "subscribe",
            json!({ "accounts": null, "classes": ["Task"] })
        )
        .await["result"],
        json!({})
    );

    let usage = json!({ "accountId": ACCOUNT_ID,
                        "task": { "id": "t-usage", "__cls": "SendFeatureUsageEventTask", "aid": ACCOUNT_ID } });
    let refused = ws.call(4, "task.queue", usage).await;
    assert_eq!(refused["error"]["code"], -32008);
    assert_eq!(
        ws.call(5, "secrets.set", json!({})).await["error"]["code"],
        -32008
    );

    let star = json!({ "accountId": ACCOUNT_ID,
                       "task": { "id": "t-star", "__cls": "ChangeStarredTask", "aid": ACCOUNT_ID,
                                 "threadIds": [], "messageIds": [], "starred": true } });
    let queued = ws.call(6, "task.queue", star).await;
    assert_ne!(queued["error"]["code"], -32008, "{queued}");

    // A second request hidden behind raw newlines inside the first reaches the engine as
    // part of one line, never as its own.
    ws.send(
        "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"engine.status\",\"params\":{\"a\":\n\
         {\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"secrets.set\",\"params\":{}}\n}}",
    )
    .await;
    let status = ws.response(7).await;
    assert!(status.get("result").is_some(), "{status}");
    while let Some(v) = ws.next(Duration::from_millis(500)).await {
        assert_ne!(v["id"], 8, "the smuggled request was answered: {v}");
        assert!(v["id"].is_null() || v.get("method").is_some(), "{v}");
    }

    ws.send(r#"{"jsonrpc":"2.0","id":9,"method":"hello","params":{"protocol":1}}"#)
        .await;
    assert_eq!(ws.close_code().await, close::PROTOCOL);
}

#[tokio::test(flavor = "multi_thread")]
async fn revoke_and_engine_exit_close_the_socket() {
    let Some(mut s) = Setup::start().await else {
        return;
    };
    let cookie = s.pair().await;
    let mut ws = s.ws(&cookie).await;
    ws.hello().await;
    let id = s.state.devices().list(now_ms()).unwrap()[0].id.clone();
    s.state.devices().revoke(&id, now_ms()).unwrap();
    s.state.reload().unwrap();
    assert_eq!(ws.close_code().await, close::UNPAIRED);

    let cookie = s.pair().await;
    let mut ws = s.ws(&cookie).await;
    ws.hello().await;
    s.engine.child.kill().unwrap();
    s.engine.child.wait().unwrap();
    assert_eq!(ws.close_code().await, close::ENGINE_UNAVAILABLE);

    // With the engine gone a new socket closes at once, and /session says so.
    let mut ws = s.ws(&cookie).await;
    assert_eq!(ws.close_code().await, close::ENGINE_UNAVAILABLE);
    let session = s
        .http("GET", "/gateway/v1/session", &[("Cookie", &cookie)], "")
        .await;
    assert!(session.contains(r#""state":"down""#), "{session}");
}
