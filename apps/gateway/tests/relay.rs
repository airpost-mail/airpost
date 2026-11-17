// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The relay against a real `airpost-engine --mode serve`:
//! one account whose servers refuse connections, added on the engine's stdin, and a browser
//! played by a WebSocket client with the pairing cookie.
//!
//! The engine binary is `AIRPOST_ENGINE_BIN`, else `target/debug/airpost-engine`; without one the test
//! skips, unless `CI` or `REQUIRE_AIRPOST` is set.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use airpost_gateway::config::{Config, ConfigInput};
use airpost_gateway::http::{close, now_ms, AppState, Server};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const ACCOUNT_ID: &str = "c0ffee-gateway";
const WAIT: Duration = Duration::from_secs(20);

fn engine_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AIRPOST_ENGINE_BIN") {
        return Some(PathBuf::from(p));
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/airpost-engine");
    if p.is_file() {
        return Some(p);
    }
    let required =
        std::env::var_os("CI").is_some() || std::env::var_os("REQUIRE_AIRPOST").is_some();
    assert!(
        !required,
        "no airpost-engine binary: cargo build -p airpost-engine, or set AIRPOST_ENGINE_BIN"
    );
    eprintln!(
        "skipping: no airpost-engine binary (cargo build -p airpost-engine, or set AIRPOST_ENGINE_BIN)"
    );
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
        let bin = engine_binary()?;
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
            json!({ "protocol": 1, "client": "airpost-web/test" }),
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

    assert!(ws
        .call(
            3,
            "subscribe",
            json!({ "accounts": null, "classes": ["Task"] })
        )
        .await["result"]["seq"]
        .is_u64());

    let unknown = json!({ "accountId": ACCOUNT_ID,
                        "task": { "id": "t-unknown", "__cls": "NoSuchTask", "aid": ACCOUNT_ID } });
    let refused = ws.call(4, "task.queue", unknown).await;
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
    let up = s
        .http("GET", "/gateway/v1/session", &[("Cookie", &cookie)], "")
        .await;
    assert!(up.contains(r#""state":"up""#), "{up}");
    s.engine.child.kill().unwrap();
    s.engine.child.wait().unwrap();
    assert_eq!(ws.close_code().await, close::ENGINE_UNAVAILABLE);
    // /session says so at once, not after the probe cache's TTL.
    let down = s
        .http("GET", "/gateway/v1/session", &[("Cookie", &cookie)], "")
        .await;
    assert!(down.contains(r#""state":"down""#), "{down}");

    // With the engine gone a new socket closes at once.
    let mut ws = s.ws(&cookie).await;
    assert_eq!(ws.close_code().await, close::ENGINE_UNAVAILABLE);
}

/// A raw HTTP/1.1 exchange: status code, lowercased headers and the body bytes.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

impl Setup {
    /// Sends `head` and `body` in one write. The server may answer before it reads the
    /// whole body, and must then read the rest before it closes: bytes left unread make the
    /// close a reset, which can cost the client the answer.
    async fn raw(&self, head: String, body: Vec<u8>) -> Reply {
        let mut s = TcpStream::connect(self.addr).await.unwrap();
        let (mut r, mut w) = s.split();
        let write = async {
            let mut bytes = head.into_bytes();
            bytes.extend_from_slice(&body);
            let _ = w.write_all(&bytes).await;
        };
        let mut out = Vec::new();
        let (_, read) = tokio::join!(write, r.read_to_end(&mut out));
        read.unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let text = String::from_utf8_lossy(&out[..split]).into_owned();
        let mut lines = text.lines();
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|l| l.split_once(": "))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.to_owned()))
            .collect();
        Reply {
            status,
            headers,
            body: out[split + 4..].to_vec(),
        }
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Reply {
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.addr,
            body.len()
        );
        for (k, v) in headers {
            head += &format!("{k}: {v}\r\n");
        }
        head += "\r\n";
        self.raw(head, body.to_vec()).await
    }

    async fn upload(&self, cookie: &str, name: &str, content_type: &str, body: &[u8]) -> Reply {
        let origin = self.origin();
        self.request(
            "POST",
            &format!("/gateway/v1/files/{ACCOUNT_ID}?name={name}"),
            &[
                ("Cookie", cookie),
                ("Origin", &origin),
                ("Content-Type", content_type),
            ],
            body,
        )
        .await
    }
}

/// Uploads become Files through `files.put`, downloads are
/// attachments in a sandbox, and only sniffed raster images are served inline.
#[tokio::test(flavor = "multi_thread")]
async fn file_routes() {
    let Some(s) = Setup::start().await else {
        return;
    };
    let cookie = s.pair().await;
    let png: Vec<u8> = [b"\x89PNG\r\n\x1a\n".as_slice(), &[1, 2, 3, 4, 5, 6, 7, 8]].concat();

    // The sender's type is ignored: an svg label on PNG bytes.
    let up = s
        .upload(&cookie, "pixel%20one.png", "image/svg+xml", &png)
        .await;
    assert_eq!(up.status, 200, "{}", String::from_utf8_lossy(&up.body));
    let put: Value = serde_json::from_slice(&up.body).unwrap();
    let id = put["fileId"].as_str().unwrap().to_owned();
    assert_eq!(put["file"]["filename"], "pixel one.png");
    assert_eq!(put["file"]["size"], png.len());
    let incoming = s._dir.path().join("gateway").join("incoming");
    assert_eq!(
        std::fs::read_dir(&incoming).unwrap().count(),
        0,
        "temp files removed"
    );

    let url = format!("/gateway/v1/files/{ACCOUNT_ID}/{id}");
    let got = s.request("GET", &url, &[("Cookie", &cookie)], b"").await;
    assert_eq!(got.status, 200);
    assert_eq!(got.body, png);
    assert_eq!(got.header("content-type"), Some("application/octet-stream"));
    assert_eq!(got.header("content-security-policy"), Some("sandbox"));
    assert_eq!(got.header("x-content-type-options"), Some("nosniff"));
    assert!(got
        .header("content-disposition")
        .unwrap()
        .starts_with("attachment; filename=\"pixel one.png\""));

    let inline = s
        .request(
            "GET",
            &format!("{url}?inline=1"),
            &[("Cookie", &cookie)],
            b"",
        )
        .await;
    assert_eq!(inline.header("content-type"), Some("image/png"));
    assert!(inline
        .header("content-disposition")
        .unwrap()
        .starts_with("inline;"));

    let range = s
        .request(
            "GET",
            &url,
            &[("Cookie", &cookie), ("Range", "bytes=2-5")],
            b"",
        )
        .await;
    assert_eq!(range.status, 206);
    assert_eq!(range.body, &png[2..6]);

    let head = s.request("HEAD", &url, &[("Cookie", &cookie)], b"").await;
    assert_eq!(head.status, 200);
    assert_eq!(head.header("content-length"), Some("16"));
    assert!(head.body.is_empty());

    // HTML labelled as PNG is still an attachment.
    let up = s
        .upload(&cookie, "x.png", "image/png", b"<html><script>x</script>")
        .await;
    let html_id = serde_json::from_slice::<Value>(&up.body).unwrap()["fileId"]
        .as_str()
        .unwrap()
        .to_owned();
    let html = s
        .request(
            "GET",
            &format!("/gateway/v1/files/{ACCOUNT_ID}/{html_id}?inline=1"),
            &[("Cookie", &cookie)],
            b"",
        )
        .await;
    assert_eq!(
        html.header("content-type"),
        Some("application/octet-stream")
    );
    assert!(html
        .header("content-disposition")
        .unwrap()
        .starts_with("attachment;"));

    // Refusals.
    let missing = s
        .request(
            "GET",
            &format!("/gateway/v1/files/{ACCOUNT_ID}/nope"),
            &[("Cookie", &cookie)],
            b"",
        )
        .await;
    assert_eq!(missing.status, 404);
    let other = s
        .request(
            "GET",
            &format!("/gateway/v1/files/other/{id}"),
            &[("Cookie", &cookie)],
            b"",
        )
        .await;
    assert_eq!(other.status, 404);
    assert_eq!(s.request("GET", &url, &[], b"").await.status, 401);
    let no_origin = s
        .request(
            "POST",
            &format!("/gateway/v1/files/{ACCOUNT_ID}?name=a"),
            &[("Cookie", &cookie)],
            b"x",
        )
        .await;
    assert_eq!(no_origin.status, 403);
    assert_eq!(s.upload(&cookie, "", "text/plain", b"x").await.status, 400);

    // Over the cap, declared or streamed.
    let origin = s.origin();
    let declared = s
        .raw(
            format!(
                "POST /gateway/v1/files/{ACCOUNT_ID}?name=big HTTP/1.1\r\nHost: {}\r\n\
                 Connection: close\r\nCookie: {cookie}\r\nOrigin: {origin}\r\n\
                 Content-Length: {}\r\n\r\n",
                s.addr,
                (25 << 20) + 1
            ),
            Vec::new(),
        )
        .await;
    assert_eq!(declared.status, 413);
    // Well past the cap, so that much of it is still unread when the gateway refuses it.
    let big = vec![0u8; (29 << 20) + 1];
    let mut chunked = format!("{:x}\r\n", big.len()).into_bytes();
    chunked.extend_from_slice(&big);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    let streamed = s
        .raw(
            format!(
                "POST /gateway/v1/files/{ACCOUNT_ID}?name=big HTTP/1.1\r\nHost: {}\r\n\
                 Connection: close\r\nCookie: {cookie}\r\nOrigin: {origin}\r\n\
                 Transfer-Encoding: chunked\r\n\r\n",
                s.addr
            ),
            chunked,
        )
        .await;
    assert_eq!(streamed.status, 413);
    assert_eq!(
        std::fs::read_dir(&incoming).unwrap().count(),
        0,
        "temp files removed"
    );

    // A browser's relayed connection can't call files.* itself.
    let mut ws = s.ws(&cookie).await;
    ws.hello().await;
    let refused = ws
        .call(
            2,
            "files.get",
            json!({ "accountId": ACCOUNT_ID, "fileId": id }),
        )
        .await;
    assert_eq!(refused["error"]["code"], -32008);
}

/// A browser that closes gets the close frame back (RFC 6455 5.5.1), so its `onclose` says
/// 1000 and `wasClean`, not 1006.
#[tokio::test(flavor = "multi_thread")]
async fn a_close_from_the_browser_is_answered() {
    let Some(s) = Setup::start().await else {
        return;
    };
    let cookie = s.pair().await;
    let mut ws = s.ws(&cookie).await;
    ws.hello().await;
    ws.0.close(None).await.unwrap();
    let got = tokio::time::timeout(WAIT, ws.0.next()).await.unwrap();
    assert!(matches!(got, Some(Ok(Message::Close(_)))), "{got:?}");
}
