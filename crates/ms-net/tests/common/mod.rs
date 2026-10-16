// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A small HTTP/1.1 server for the ms-net tests. Each connection carries one request and
//! is closed after the reply; every request is recorded.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    /// Path plus query, as sent.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Decoded query parameters.
    pub fn query(&self) -> Vec<(String, String)> {
        pairs(self.target.split_once('?').map(|(_, q)| q).unwrap_or(""))
    }

    pub fn query_value(&self, key: &str) -> Option<String> {
        self.query()
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Decoded `application/x-www-form-urlencoded` body.
    pub fn form(&self) -> Vec<(String, String)> {
        pairs(&self.body)
    }

    pub fn form_value(&self, key: &str) -> Option<String> {
        self.form()
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("JSON body")
    }
}

fn pairs(encoded: &str) -> Vec<(String, String)> {
    let url = reqwest::Url::parse(&format!("http://x/?{encoded}")).unwrap();
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Wait this long before answering.
    pub delay: Duration,
}

impl Reply {
    pub fn json(status: u16, body: serde_json::Value) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    pub fn text(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "text/html".into())],
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    pub fn redirect(status: u16, location: &str) -> Reply {
        Reply {
            status,
            headers: vec![("Location".into(), location.into())],
            body: String::new(),
            delay: Duration::ZERO,
        }
    }

    pub fn delayed(mut self, delay: Duration) -> Reply {
        self.delay = delay;
        self
    }
}

type Handler = dyn Fn(&Request) -> Reply + Send + Sync;

pub struct MockServer {
    port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl MockServer {
    pub fn start(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let handler = handler.clone();
                let recorded = recorded.clone();
                std::thread::spawn(move || serve(stream, &*handler, &recorded));
            }
        });
        MockServer { port, requests }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

/// A port nothing listens on.
pub fn closed_port_url() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    format!("http://127.0.0.1:{port}")
}

fn serve(mut stream: TcpStream, handler: &Handler, recorded: &Mutex<Vec<Request>>) {
    let mut first = [0u8; 1];
    if stream.peek(&mut first).unwrap_or(0) == 0 {
        return;
    }
    // A TLS ClientHello: answer in plain text so the client's handshake fails.
    if first[0] == 0x16 {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
        return;
    }

    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("").to_owned();

    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);

    let req = Request {
        method,
        target,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    recorded.lock().unwrap().push(req.clone());
    let reply = handler(&req);
    if !reply.delay.is_zero() {
        std::thread::sleep(reply.delay);
    }

    let mut out = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (k, v) in &reply.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(&reply.body);
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.flush();
}
