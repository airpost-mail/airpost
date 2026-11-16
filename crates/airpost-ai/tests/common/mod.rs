// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A fake model provider: the small HTTP/1.1 server of the tern-integrations tests, plus
//! streamed replies (chunked, with per-chunk delays, optionally cut off) and stream bodies
//! in the three wire formats. Each connection carries one request; every request is
//! recorded, and so is every stream the client hung up on.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tern_ai::{ContextMessage, MailContext, Person, ProviderConfig, ProviderKind};

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
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

    pub fn query_value(&self, key: &str) -> Option<String> {
        let query = self.target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let url = reqwest::Url::parse(&format!("http://x/?{query}")).unwrap();
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("JSON body")
    }
}

#[derive(Clone, Debug)]
pub enum Body {
    Whole(String),
    /// Sent with chunked encoding, each piece after its delay.
    Stream {
        chunks: Vec<(Duration, String)>,
        /// Close the connection without the final chunk.
        cut: bool,
    },
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Body,
    /// Wait this long before sending the status line.
    pub delay: Duration,
}

impl Reply {
    pub fn json(status: u16, body: Value) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Body::Whole(body.to_string()),
            delay: Duration::ZERO,
        }
    }

    pub fn text(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "text/html".into())],
            body: Body::Whole(body.into()),
            delay: Duration::ZERO,
        }
    }

    /// `chunks` sent one by one, `gap` apart.
    pub fn stream(content_type: &str, chunks: Vec<String>, gap: Duration) -> Reply {
        Reply {
            status: 200,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: Body::Stream {
                chunks: chunks.into_iter().map(|c| (gap, c)).collect(),
                cut: false,
            },
            delay: Duration::ZERO,
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn delayed(mut self, delay: Duration) -> Reply {
        self.delay = delay;
        self
    }

    /// Waits `delay` before the first chunk (headers go out at once).
    pub fn first_chunk_after(mut self, delay: Duration) -> Reply {
        if let Body::Stream { chunks, .. } = &mut self.body {
            if let Some(first) = chunks.first_mut() {
                first.0 = delay;
            }
        }
        self
    }

    /// Only the first `keep` chunks, then the connection closes mid-body.
    pub fn cut_after(mut self, keep: usize) -> Reply {
        if let Body::Stream { chunks, cut } = &mut self.body {
            chunks.truncate(keep);
            *cut = true;
        }
        self
    }
}

type Handler = dyn Fn(&Request) -> Reply + Send + Sync;

pub struct MockServer {
    port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    aborted: Arc<AtomicUsize>,
}

impl MockServer {
    pub fn start(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let aborted = Arc::new(AtomicUsize::new(0));
        let handler: Arc<Handler> = Arc::new(handler);
        let (recorded, hung_up) = (requests.clone(), aborted.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let handler = handler.clone();
                let recorded = recorded.clone();
                let hung_up = hung_up.clone();
                std::thread::spawn(move || serve(stream, &*handler, &recorded, &hung_up));
            }
        });
        MockServer {
            port,
            requests,
            aborted,
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub fn last(&self) -> Request {
        self.requests().pop().expect("a request")
    }

    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Waits up to `within` for a stream the client hung up on.
    pub fn saw_abort(&self, within: Duration) -> bool {
        let until = Instant::now() + within;
        while Instant::now() < until {
            if self.aborted.load(Ordering::SeqCst) > 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

fn serve(
    mut stream: TcpStream,
    handler: &Handler,
    recorded: &Mutex<Vec<Request>>,
    aborted: &AtomicUsize,
) {
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

    let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
    match &reply.body {
        Body::Whole(b) => head.push_str(&format!("Content-Length: {}\r\n", b.len())),
        Body::Stream { .. } => head.push_str("Transfer-Encoding: chunked\r\n"),
    }
    for (k, v) in &reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    match reply.body {
        Body::Whole(b) => {
            let _ = stream.write_all(b.as_bytes());
        }
        Body::Stream { chunks, cut } => {
            for (delay, chunk) in chunks {
                std::thread::sleep(delay);
                let frame = format!("{:x}\r\n{chunk}\r\n", chunk.len());
                // A hung-up client shows as a failed write, or as EOF on the read side.
                if stream.write_all(frame.as_bytes()).is_err()
                    || stream.flush().is_err()
                    || peer_closed(&stream)
                {
                    aborted.fetch_add(1, Ordering::SeqCst);
                    return;
                }
            }
            if !cut {
                let _ = stream.write_all(b"0\r\n\r\n");
            }
        }
    }
    let _ = stream.flush();
}

fn peer_closed(stream: &TcpStream) -> bool {
    let _ = stream.set_nonblocking(true);
    let mut buf = [0u8; 1];
    let closed = matches!(stream.peek(&mut buf), Ok(0));
    let _ = stream.set_nonblocking(false);
    closed
}

// Stream bodies in each wire format.

pub fn openai_stream(model: &str, texts: &[&str], usage: Option<(u64, u64)>) -> Vec<String> {
    let mut out = vec![sse_data(&json!({
        "id": "c1", "object": "chat.completion.chunk", "model": model,
        "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": null}],
    }))];
    for t in texts {
        out.push(sse_data(&json!({
            "id": "c1", "object": "chat.completion.chunk", "model": model,
            "choices": [{"index": 0, "delta": {"content": t}, "finish_reason": null}],
        })));
    }
    out.push(sse_data(&json!({
        "id": "c1", "object": "chat.completion.chunk", "model": model,
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
    })));
    if let Some((input, output)) = usage {
        out.push(sse_data(&json!({
            "id": "c1", "object": "chat.completion.chunk", "model": model, "choices": [],
            "usage": {"prompt_tokens": input, "completion_tokens": output, "total_tokens": input + output},
        })));
    }
    out.push("data: [DONE]\n\n".into());
    out
}

pub fn anthropic_stream(model: &str, texts: &[&str], usage: (u64, u64)) -> Vec<String> {
    let mut out = vec![
        sse_event(
            "message_start",
            &json!({
                "type": "message_start",
                "message": {"id": "msg_1", "type": "message", "role": "assistant", "model": model,
                    "content": [], "stop_reason": null,
                    "usage": {"input_tokens": usage.0, "cache_read_input_tokens": 0, "output_tokens": 1}},
            }),
        ),
        sse_event(
            "content_block_start",
            &json!({
                "type": "content_block_start", "index": 0,
                "content_block": {"type": "thinking", "thinking": ""},
            }),
        ),
        sse_event(
            "content_block_delta",
            &json!({
                "type": "content_block_delta", "index": 0,
                "delta": {"type": "thinking_delta", "thinking": "Let me think"},
            }),
        ),
        sse_event(
            "content_block_stop",
            &json!({"type": "content_block_stop", "index": 0}),
        ),
        sse_event("ping", &json!({"type": "ping"})),
        sse_event(
            "content_block_start",
            &json!({
                "type": "content_block_start", "index": 1,
                "content_block": {"type": "text", "text": ""},
            }),
        ),
    ];
    for t in texts {
        out.push(sse_event(
            "content_block_delta",
            &json!({
                "type": "content_block_delta", "index": 1,
                "delta": {"type": "text_delta", "text": t},
            }),
        ));
    }
    out.push(sse_event(
        "content_block_stop",
        &json!({"type": "content_block_stop", "index": 1}),
    ));
    out.push(sse_event(
        "message_delta",
        &json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn", "stop_sequence": null},
            "usage": {"output_tokens": usage.1},
        }),
    ));
    out.push(sse_event("message_stop", &json!({"type": "message_stop"})));
    out
}

pub fn ollama_stream(model: &str, texts: &[&str], usage: Option<(u64, u64)>) -> Vec<String> {
    let mut out: Vec<String> = texts
        .iter()
        .map(|t| {
            format!(
                "{}\n",
                json!({"model": model, "created_at": "2026-10-18T10:00:00Z",
                    "message": {"role": "assistant", "content": t}, "done": false})
            )
        })
        .collect();
    let mut last = json!({"model": model, "created_at": "2026-10-18T10:00:01Z",
        "message": {"role": "assistant", "content": ""}, "done": true, "done_reason": "stop"});
    if let Some((input, output)) = usage {
        last["prompt_eval_count"] = json!(input);
        last["eval_count"] = json!(output);
    }
    out.push(format!("{last}\n"));
    out
}

pub fn sse_data(v: &Value) -> String {
    format!("data: {v}\n\n")
}

pub fn sse_event(name: &str, v: &Value) -> String {
    format!("event: {name}\ndata: {v}\n\n")
}

pub const SSE: &str = "text/event-stream";
pub const NDJSON: &str = "application/x-ndjson";

/// Replies with the stream for `kind`.
pub fn stream_reply(kind: ProviderKind, texts: &[&str]) -> Reply {
    let chunks = match kind {
        ProviderKind::OpenAi => openai_stream("m-1", texts, Some((100, 10))),
        ProviderKind::Anthropic => anthropic_stream("m-1", texts, (100, 10)),
        ProviderKind::Ollama => ollama_stream("m-1", texts, Some((100, 10))),
    };
    let content_type = if kind == ProviderKind::Ollama {
        NDJSON
    } else {
        SSE
    };
    Reply::stream(content_type, chunks, Duration::ZERO)
}

pub const ALL_KINDS: [ProviderKind; 3] = [
    ProviderKind::OpenAi,
    ProviderKind::Anthropic,
    ProviderKind::Ollama,
];

pub const KEY: &str = "sk-very-secret-123";

/// A config pointing at `server`, with the base URL each kind expects.
pub fn config(kind: ProviderKind, server: &MockServer) -> ProviderConfig {
    let url = match kind {
        ProviderKind::OpenAi => format!("{}/v1", server.url()),
        _ => server.url(),
    };
    ProviderConfig {
        id: "test".into(),
        kind,
        url: Some(url),
        api_key: Some(KEY.into()),
        model: "m-1".into(),
        context_tokens: None,
        max_output_tokens: None,
    }
}

pub const OWNER_EMAIL: &str = "bo@example.com";
pub const OTHER_EMAIL: &str = "ann@acme.io";
pub const BODY: &str = "Confidential merger plans for Tuesday";

pub fn thread() -> MailContext {
    let ann = Person::new("Ann Lee", OTHER_EMAIL);
    let bo = Person::new("Bo Chen", OWNER_EMAIL);
    MailContext {
        subject: "Offsite".into(),
        participants: vec![ann.clone(), bo.clone()],
        owner: bo.clone(),
        messages: vec![
            ContextMessage {
                id: "m1".into(),
                date: "2026-10-16 09:00".into(),
                from: ann,
                subject: None,
                body: Some(format!("{BODY}. Can you make it?")),
                body_omitted: false,
                attachments: vec!["agenda.pdf".into()],
            },
            ContextMessage {
                id: "m2".into(),
                date: "2026-10-16 10:00".into(),
                from: bo,
                subject: None,
                body: Some("Probably, let me check.".into()),
                body_omitted: false,
                attachments: Vec::new(),
            },
        ],
        include_addresses: false,
    }
}

/// A sink that keeps every delta.
pub fn collect(into: &mut Vec<String>) -> impl FnMut(&str) -> std::ops::ControlFlow<()> + '_ {
    move |t| {
        into.push(t.to_owned());
        std::ops::ControlFlow::Continue(())
    }
}
