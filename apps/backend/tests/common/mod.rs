// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A server on an ephemeral port with a temp data dir, and a plain HTTP/1.1 client.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::HeaderValue;
use hyper::{HeaderMap, Method, Request, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};

use tern_backend::config::{Config, FeatureSwitches, IpNet, Signup};
use tern_backend::http::{AppState, Server};
use tern_backend::repo::{Repo, SqliteRepo, User};

pub const PUBLIC_URL: &str = "https://tern.test";

pub struct TestServer {
    pub addr: SocketAddr,
    pub repo: Arc<SqliteRepo>,
    pub state: AppState,
    pub data_dir: PathBuf,
    client: Client<HttpConnector, Full<Bytes>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    _dir: tempfile::TempDir,
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.body))
    }
}

impl TestServer {
    pub async fn start(signup: Signup) -> TestServer {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            data_dir: dir.path().to_owned(),
            bind: "127.0.0.1:0".parse().unwrap(),
            public_url: Some(PUBLIC_URL.to_owned()),
            signup,
            features: FeatureSwitches::default(),
            trusted_proxies: vec![IpNet::parse("127.0.0.0/8").unwrap()],
        };
        let repo = Arc::new(SqliteRepo::open(&config.data_dir).unwrap());
        let server = Server::bind(&config, repo.clone()).await.unwrap();
        let addr = server.local_addr().unwrap();
        let state = server.state().clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(server.run(async {
            let _ = rx.await;
        }));
        TestServer {
            addr,
            repo,
            state,
            data_dir: dir.path().to_owned(),
            client: Client::builder(TokioExecutor::new()).build_http(),
            shutdown: Some(tx),
            task: Some(task),
            _dir: dir,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: impl Into<Bytes>,
    ) -> Reply {
        let mut req = Request::builder().method(method).uri(self.url(path));
        for (k, v) in headers {
            req = req.header(*k, HeaderValue::from_str(v).unwrap());
        }
        let res = self
            .client
            .request(req.body(Full::new(body.into())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    /// Sends a request and returns the response with its body unread, for streams.
    pub async fn open(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: impl Into<Bytes>,
    ) -> hyper::Response<Incoming> {
        let mut req = Request::builder().method(method).uri(self.url(path));
        for (k, v) in headers {
            req = req.header(*k, HeaderValue::from_str(v).unwrap());
        }
        self.client
            .request(req.body(Full::new(body.into())).unwrap())
            .await
            .unwrap()
    }

    /// Like [`TestServer::open`], over an in-memory connection served by the same state. Paused
    /// tokio time auto-advances whenever the runtime waits, and loopback TCP data is not always
    /// visible yet when it does, so timing assertions use this path.
    pub async fn open_in_memory(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: impl Into<Bytes>,
    ) -> hyper::Response<Incoming> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let svc = hyper_util::service::TowerToHyperService::new(tern_backend::http::router(
            self.state.clone(),
        ));
        tokio::spawn(async move {
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(server_io), svc)
                .await;
        });
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(client_io))
            .await
            .unwrap();
        tokio::spawn(conn);
        let mut req = Request::builder().method(method).uri(path);
        for (k, v) in headers {
            req = req.header(*k, HeaderValue::from_str(v).unwrap());
        }
        sender
            .send_request(req.body(Full::new(body.into())).unwrap())
            .await
            .unwrap()
    }

    pub async fn post_json(&self, path: &str, auth: &str, body: &serde_json::Value) -> Reply {
        self.request(
            Method::POST,
            path,
            &[
                ("authorization", auth),
                ("content-type", "application/json"),
            ],
            body.to_string(),
        )
        .await
    }

    pub async fn get(&self, path: &str, headers: &[(&str, &str)]) -> Reply {
        self.request(Method::GET, path, headers, Bytes::new()).await
    }

    pub async fn post_form(
        &self,
        path: &str,
        headers: &[(&str, &str)],
        fields: &[(&str, &str)],
    ) -> Reply {
        let body = fields
            .iter()
            .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let mut h = vec![("content-type", "application/x-www-form-urlencoded")];
        h.extend_from_slice(headers);
        self.request(Method::POST, path, &h, body).await
    }

    /// Creates a user directly in the repo.
    pub fn add_user(&self, email: &str, password: &str) -> User {
        let new = tern_backend::auth::new_user(email, "Ada", "Lovelace", password).unwrap();
        self.repo
            .create_user(&new, tern_backend::ids::now_ms())
            .unwrap()
    }

    /// Creates a device token directly in the repo.
    pub fn add_token(&self, user: &User) -> String {
        tern_backend::auth::issue_token(self.repo.as_ref(), user, "test", 1).unwrap()
    }

    pub async fn stop(mut self) {
        let _ = self.shutdown.take().unwrap().send(());
        self.task.take().unwrap().await.unwrap().unwrap();
    }
}

pub fn basic(token: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{token}:"))
    )
}

fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The decoded `#tern-login-result` of a login page: base64url without padding, decoding to
/// ASCII JSON.
pub fn login_result(html: &str) -> Option<serde_json::Value> {
    use base64::Engine;
    let start = html.find("id=\"tern-login-result\"")?;
    let open_end = start + html[start..].find('>')? + 1;
    let close = open_end + html[open_end..].find("</div>")?;
    let text = html[open_end..close].trim();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .expect("base64url");
    assert!(bytes.is_ascii(), "tern-login-result must decode to ASCII");
    Some(serde_json::from_slice(&bytes).expect("login result JSON"))
}
