// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The gateway's HTTP surface over a real socket: static app, Host and Origin checks,
//! pairing, the cookie, unpair and revoke, and WebSocket close codes.

use std::net::SocketAddr;
use std::path::PathBuf;

use futures::StreamExt;
use tern_gateway::config::{Config, ConfigInput};
use tern_gateway::devices::DeviceFile;
use tern_gateway::http::{close, now_ms, AppState, Server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

struct Gateway {
    addr: SocketAddr,
    state: AppState,
    _dir: tempfile::TempDir,
}

impl Gateway {
    async fn start() -> Gateway {
        let dir = tempfile::tempdir().unwrap();
        let web = dir.path().join("web");
        std::fs::create_dir_all(web.join("assets")).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><title>Tern</title>").unwrap();
        std::fs::write(web.join("assets/app-1234.js"), "console.log(1)").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = Config::new(ConfigInput {
            bind: addr,
            public_url: None,
            insecure_public_url: false,
            allow_origins: &["http://localhost:5173".to_owned()],
            trusted_proxies: &[],
            web_root: Some(web),
            config_dir: PathBuf::from(dir.path()),
        })
        .unwrap();
        let state = AppState::new(config).unwrap();
        let server = Server::with_listener(listener, state.clone());
        tokio::spawn(server.run(std::future::pending()));
        Gateway {
            addr,
            state,
            _dir: dir,
        }
    }

    fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn devices(&self) -> &DeviceFile {
        self.state.devices()
    }

    fn mint(&self, admin: bool) -> String {
        let code = self
            .devices()
            .mint_code(None, admin, 600_000, now_ms())
            .unwrap();
        self.state.reload().unwrap();
        code
    }

    async fn request(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Res {
        let mut s = TcpStream::connect(self.addr).await.unwrap();
        let host = self.addr.to_string();
        let has_host = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host"));
        let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
        if !has_host {
            req += &format!("Host: {host}\r\n");
        }
        for (k, v) in headers {
            req += &format!("{k}: {v}\r\n");
        }
        if !body.is_empty() {
            req += &format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            );
        }
        req += "\r\n";
        req += body;
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        Res::parse(&String::from_utf8_lossy(&out))
    }

    async fn pair(&self, code: &str) -> Res {
        let origin = self.origin();
        let body = format!(r#"{{"code":"{code}","deviceName":"Test browser"}}"#);
        self.request("POST", "/gateway/v1/pair", &[("Origin", &origin)], &body)
            .await
    }

    /// Pairs and returns the `Cookie` header value.
    async fn paired_cookie(&self, admin: bool) -> String {
        let res = self.pair(&self.mint(admin)).await;
        assert_eq!(res.status, 200, "{}", res.body);
        res.cookie()
    }

    async fn ws_close_code(&self, origin: Option<&str>, cookie: Option<&str>) -> u16 {
        let mut req = format!("ws://{}/gateway/v1/ws", self.addr)
            .into_client_request()
            .unwrap();
        if let Some(o) = origin {
            req.headers_mut().insert("Origin", o.parse().unwrap());
        }
        if let Some(c) = cookie {
            req.headers_mut().insert("Cookie", c.parse().unwrap());
        }
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        match ws.next().await {
            Some(Ok(Message::Close(Some(frame)))) => frame.code.into(),
            other => panic!("expected a close frame, got {other:?}"),
        }
    }
}

struct Res {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Res {
    fn parse(raw: &str) -> Res {
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        let mut lines = head.lines();
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        Res {
            status,
            headers,
            body: body.to_owned(),
        }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> serde_json::Value {
        // Bodies may be chunked; the JSON is the one line that starts with `{`.
        let line = self
            .body
            .lines()
            .find(|l| l.starts_with('{'))
            .unwrap_or("null");
        serde_json::from_str(line).unwrap()
    }

    /// `name=value` from Set-Cookie.
    fn cookie(&self) -> String {
        let set = self.header("set-cookie").expect("set-cookie");
        set.split(';').next().unwrap().to_owned()
    }
}

#[tokio::test]
async fn serves_the_app_with_headers() {
    let g = Gateway::start().await;
    let res = g.request("GET", "/", &[], "").await;
    assert_eq!(res.status, 200);
    assert!(res.body.contains("<title>Tern</title>"));
    let csp = res.header("content-security-policy").unwrap();
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains(&format!("ws://{}", g.addr)));
    assert_eq!(res.header("cache-control"), Some("no-cache"));
    assert_eq!(res.header("x-content-type-options"), Some("nosniff"));

    let res = g.request("GET", "/assets/app-1234.js", &[], "").await;
    assert_eq!(res.status, 200);
    assert!(res.header("cache-control").unwrap().contains("immutable"));

    // SPA routes get the app.
    let res = g.request("GET", "/pair", &[], "").await;
    assert_eq!(res.status, 200);
    assert!(res.body.contains("<title>Tern</title>"));

    let res = g.request("GET", "/gateway/v1/nope", &[], "").await;
    assert_eq!(res.status, 404);
    assert_eq!(res.json()["error"], "not-found");
    assert_eq!(res.header("cache-control"), Some("no-store"));
}

#[tokio::test]
async fn healthz_skips_the_host_check() {
    let g = Gateway::start().await;
    let res = g
        .request("GET", "/healthz", &[("Host", "anything")], "")
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["ok"], true);
}

#[tokio::test]
async fn host_and_origin_checks() {
    let g = Gateway::start().await;
    let res = g
        .request("GET", "/", &[("Host", "rebound.example")], "")
        .await;
    assert_eq!(res.status, 403);
    let host = format!("localhost:{}", g.addr.port());
    let res = g.request("GET", "/", &[("Host", &host)], "").await;
    assert_eq!(res.status, 200);

    let code = g.mint(false);
    let body = format!(r#"{{"code":"{code}"}}"#);
    let res = g.request("POST", "/gateway/v1/pair", &[], &body).await;
    assert_eq!(res.status, 403, "no Origin");
    let res = g
        .request(
            "POST",
            "/gateway/v1/pair",
            &[("Origin", "https://evil.example")],
            &body,
        )
        .await;
    assert_eq!(res.status, 403);
    // The dev server's origin was allowed with --allow-origin.
    let res = g
        .request(
            "POST",
            "/gateway/v1/pair",
            &[
                ("Origin", "http://localhost:5173"),
                ("Host", "localhost:5173"),
            ],
            &body,
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.body);
}

#[tokio::test]
async fn pairing_flow() {
    let g = Gateway::start().await;
    let res = g.request("GET", "/gateway/v1/session", &[], "").await;
    assert_eq!(res.status, 401);
    assert_eq!(res.json()["error"], "unpaired");

    let code = g.mint(true);
    let res = g.pair("AAAA-AAAA-AAAA").await;
    assert_eq!(res.status, 401);

    let res = g.pair(&code.to_lowercase()).await;
    assert_eq!(res.status, 200, "{}", res.body);
    let set = res.header("set-cookie").unwrap().to_owned();
    assert!(set.starts_with("tern_device=tern_gw_"));
    assert!(set.contains("HttpOnly"));
    assert!(set.contains("SameSite=Strict"));
    assert!(!set.contains("Secure"), "plain http on loopback");
    let token = res.cookie().split_once('=').unwrap().1.to_owned();
    assert!(!res.body.contains(&token), "the token never goes in a body");
    let device = res.json()["device"].clone();
    assert_eq!(device["name"], "Test browser");
    assert_eq!(device["scopes"], serde_json::json!(["admin"]));

    let res = g.pair(&code).await;
    assert_eq!(res.status, 401, "codes are one-shot");

    let cookie = format!("tern_device={token}");
    let res = g
        .request("GET", "/gateway/v1/session", &[("Cookie", &cookie)], "")
        .await;
    assert_eq!(res.status, 200);
    let s = res.json();
    assert_eq!(s["device"]["id"], device["id"]);
    assert_eq!(s["engine"]["state"], "down");
    assert!(res
        .header("set-cookie")
        .unwrap()
        .contains("Max-Age=34560000"));
}

#[tokio::test]
async fn pairing_is_rate_limited() {
    let g = Gateway::start().await;
    for _ in 0..5 {
        assert_eq!(g.pair("AAAA-AAAA-AAAA").await.status, 401);
    }
    let res = g.pair("AAAA-AAAA-AAAA").await;
    assert_eq!(res.status, 429);
    assert!(res.header("retry-after").is_some());
}

#[tokio::test]
async fn unpair_and_revoke() {
    let g = Gateway::start().await;
    let origin = g.origin();
    let cookie = g.paired_cookie(false).await;
    let res = g
        .request(
            "POST",
            "/gateway/v1/unpair",
            &[("Cookie", &cookie), ("Origin", &origin)],
            "",
        )
        .await;
    assert_eq!(res.status, 204);
    assert!(res.header("set-cookie").unwrap().contains("Max-Age=0"));
    let res = g
        .request("GET", "/gateway/v1/session", &[("Cookie", &cookie)], "")
        .await;
    assert_eq!(res.status, 401);

    let cookie = g.paired_cookie(false).await;
    let id = g.devices().list(now_ms()).unwrap()[0].id.clone();
    assert!(g.devices().revoke(&id, now_ms()).unwrap());
    g.state.reload().unwrap();
    let res = g
        .request("GET", "/gateway/v1/session", &[("Cookie", &cookie)], "")
        .await;
    assert_eq!(res.status, 401);
}

#[tokio::test]
async fn websocket_close_codes() {
    let g = Gateway::start().await;
    let origin = g.origin();
    assert_eq!(g.ws_close_code(Some(&origin), None).await, close::UNPAIRED);
    let cookie = g.paired_cookie(false).await;
    assert_eq!(
        g.ws_close_code(Some("https://evil.example"), Some(&cookie))
            .await,
        close::FORBIDDEN
    );
    assert_eq!(g.ws_close_code(None, Some(&cookie)).await, close::FORBIDDEN);
    // Ten upgrades a minute per device; the relay isn't there yet, so each closes 4502.
    for _ in 0..10 {
        assert_eq!(
            g.ws_close_code(Some(&origin), Some(&cookie)).await,
            close::ENGINE_UNAVAILABLE
        );
    }
    assert_eq!(
        g.ws_close_code(Some(&origin), Some(&cookie)).await,
        close::LIMITED
    );
}
