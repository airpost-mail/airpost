// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Error statuses and stream failures of each provider, as `AiError` keys; the one retry of
//! non-streamed calls; timeouts, cut streams and cancellation.

mod common;

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use ms_ai::{Ai, AiError, CancelToken, Options, ProviderKind, ReplyOpts, Sink, SummaryOpts};
use serde_json::json;

fn ai() -> Ai {
    Ai::with_options(Options {
        retry_delay: Duration::from_millis(10),
        ..Options::default()
    })
    .unwrap()
}

fn summarize_with(ai: &Ai, kind: ProviderKind, server: &MockServer) -> Result<String, AiError> {
    ai.summarize(
        &config(kind, server),
        &thread(),
        &SummaryOpts { refresh: true },
        &mut collect(&mut Vec::new()),
    )
    .map(|c| c.text)
}

fn summarize(kind: ProviderKind, reply: Reply) -> AiError {
    let server = MockServer::start(move |_| reply.clone());
    summarize_with(&ai(), kind, &server).unwrap_err()
}

#[test]
fn statuses_in_every_format() {
    for kind in ALL_KINDS {
        let cases: Vec<(Reply, &str)> = vec![
            (
                Reply::json(401, json!({"error": {"message": "Invalid API key"}})),
                "auth",
            ),
            (
                Reply::json(403, json!({"error": {"message": "forbidden"}})),
                "auth",
            ),
            (Reply::json(402, json!({"error": "pay"})), "quota-exceeded"),
            (
                Reply::json(
                    429,
                    json!({"error": {"type": "rate_limit_error", "message": "slow"}}),
                ),
                "rate-limited",
            ),
            (
                Reply::json(
                    429,
                    json!({"error": {"code": "insufficient_quota", "message": "You exceeded your current quota"}}),
                ),
                "quota-exceeded",
            ),
            (
                Reply::json(404, json!({"error": "model 'm-1' not found"})),
                "model-not-found",
            ),
            (Reply::text(404, "Not Found"), "bad-request"),
            (
                Reply::json(
                    400,
                    json!({"error": {"code": "context_length_exceeded", "message": "This model's maximum context length is 8192 tokens"}}),
                ),
                "context-too-long",
            ),
            (
                Reply::json(413, json!({"error": {"type": "request_too_large"}})),
                "context-too-long",
            ),
            (
                Reply::json(400, json!({"error": {"message": "temperature is invalid"}})),
                "bad-request",
            ),
            (Reply::json(500, json!({"error": "boom"})), "provider-error"),
            (
                Reply::json(
                    529,
                    json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
                ),
                "provider-error",
            ),
        ];
        for (reply, key) in cases {
            let status = reply.status;
            let e = summarize(kind, reply);
            assert_eq!(e.key(), key, "{kind:?} HTTP {status}: {e}");
            if key != "rate-limited" {
                assert_eq!(e.status(), Some(status), "{kind:?}");
            }
        }
    }
}

#[test]
fn retry_after_is_carried() {
    for kind in ALL_KINDS {
        let e = summarize(
            kind,
            Reply::json(429, json!({"error": "slow down"})).header("Retry-After", "17"),
        );
        assert_eq!(e.retry_after(), Some(Duration::from_secs(17)));
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"key": "rate-limited", "message": "the provider is rate limiting requests", "retryable": true, "retryAfter": 17})
        );
    }
}

#[test]
fn provider_messages_are_carried_truncated() {
    let long = "x".repeat(1000);
    let e = summarize(
        ProviderKind::OpenAi,
        Reply::json(400, json!({"error": {"message": long}})),
    );
    let AiError::BadRequest { message, .. } = &e else {
        panic!("{e:?}");
    };
    assert!(message.chars().count() <= 201);
}

#[test]
fn stream_errors() {
    let openai = Reply::stream(
        SSE,
        vec![
            sse_data(&json!({"choices": [{"delta": {"content": "Hal"}}]})),
            sse_data(&json!({"error": {"message": "upstream failed", "type": "server_error"}})),
        ],
        Duration::ZERO,
    );
    assert_eq!(
        summarize(ProviderKind::OpenAi, openai).key(),
        "provider-error"
    );

    let anthropic = Reply::stream(
        SSE,
        vec![sse_event(
            "error",
            &json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
        )],
        Duration::ZERO,
    );
    let e = summarize(ProviderKind::Anthropic, anthropic);
    assert_eq!(e.key(), "provider-error");
    assert!(e.is_retryable());

    let mut refusal = anthropic_stream("m-1", &[], (10, 0));
    let at = refusal.len() - 2;
    refusal[at] = sse_event(
        "message_delta",
        &json!({"type": "message_delta", "delta": {"stop_reason": "refusal"}, "usage": {"output_tokens": 0}}),
    );
    assert_eq!(
        summarize(
            ProviderKind::Anthropic,
            Reply::stream(SSE, refusal, Duration::ZERO)
        )
        .key(),
        "bad-request"
    );

    let ollama = Reply::stream(
        NDJSON,
        vec!["{\"error\":\"model runner has unexpectedly stopped\"}\n".into()],
        Duration::ZERO,
    );
    assert_eq!(
        summarize(ProviderKind::Ollama, ollama).key(),
        "provider-error"
    );
}

#[test]
fn empty_answers_fail() {
    for kind in ALL_KINDS {
        let e = summarize(kind, stream_reply(kind, &[]));
        assert_eq!(e.key(), "provider-error", "{kind:?}");
    }
}

#[test]
fn cut_streams_fail_after_streaming_what_arrived() {
    for kind in ALL_KINDS {
        let full = stream_reply(kind, &["one ", "two ", "three"]);
        // Up to and including the second text chunk.
        let keep = match kind {
            ProviderKind::OpenAi => 3,
            ProviderKind::Anthropic => 8,
            ProviderKind::Ollama => 2,
        };
        let server = MockServer::start(move |_| full.clone().cut_after(keep));
        let mut deltas = Vec::new();
        let e = ai()
            .summarize(
                &config(kind, &server),
                &thread(),
                &SummaryOpts::default(),
                &mut collect(&mut deltas),
            )
            .unwrap_err();
        assert_eq!(deltas, ["one ", "two "], "{kind:?}");
        assert!(
            matches!(e.key(), "offline" | "provider-error"),
            "{kind:?}: {e}"
        );
        assert!(e.is_retryable());
        assert_eq!(server.count(), 1, "streamed calls are never retried");
    }
}

#[test]
fn clean_eof_without_end_marker_is_an_error() {
    let chunks = vec![sse_data(
        &json!({"choices": [{"delta": {"content": "partial"}}]}),
    )];
    let e = summarize(
        ProviderKind::OpenAi,
        Reply::stream(SSE, chunks, Duration::ZERO),
    );
    assert_eq!(e.key(), "provider-error");
    assert!(e.to_string().contains("ended early"), "{e}");
}

#[test]
fn non_streamed_calls_retry_once() {
    for (first, retried) in [
        (Reply::json(500, json!({"error": "boom"})), true),
        (Reply::json(503, json!({"error": "down"})), true),
        (Reply::json(401, json!({"error": "no"})), false),
        (Reply::json(429, json!({"error": "slow"})), false),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let n = calls.clone();
        let server = MockServer::start(move |_| {
            if n.fetch_add(1, Ordering::SeqCst) == 0 {
                first.clone()
            } else {
                stream_reply(
                    ProviderKind::OpenAi,
                    &[r#"{"suggestions":[{"label":"Yes","text":"Yes."}]}"#],
                )
            }
        });
        let config = config(ProviderKind::OpenAi, &server);
        let result = ai().suggest_replies(&config, &thread(), &ReplyOpts::default());
        assert_eq!(result.is_ok(), retried);
        assert_eq!(server.count(), if retried { 2 } else { 1 });
    }

    // Twice in a row: the second error is returned.
    let server = MockServer::start(|_| Reply::json(500, json!({"error": "boom"})));
    let e = ai()
        .triage(&config(ProviderKind::Ollama, &server), &thread())
        .unwrap_err();
    assert_eq!(e.key(), "provider-error");
    assert_eq!(server.count(), 2);

    // Streamed calls are not retried.
    let server = MockServer::start(|_| Reply::json(500, json!({"error": "boom"})));
    summarize_with(&ai(), ProviderKind::Anthropic, &server).unwrap_err();
    assert_eq!(server.count(), 1);
}

#[test]
fn errors_are_not_cached() {
    let calls = Arc::new(AtomicUsize::new(0));
    let n = calls.clone();
    let server = MockServer::start(move |_| {
        if n.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::json(500, json!({"error": "boom"}))
        } else {
            stream_reply(ProviderKind::Ollama, &["fine"])
        }
    });
    let ai = ai();
    let config = config(ProviderKind::Ollama, &server);
    let opts = SummaryOpts::default();
    assert!(ai
        .summarize(&config, &thread(), &opts, &mut collect(&mut Vec::new()))
        .is_err());
    assert!(ai
        .summarize(&config, &thread(), &opts, &mut collect(&mut Vec::new()))
        .is_ok());
    assert_eq!(server.count(), 2);
}

fn timeouts(first_token: Duration, total: Duration) -> Ai {
    Ai::with_options(Options {
        first_token_timeout: first_token,
        total_timeout: total,
        retry_delay: Duration::from_millis(10),
        ..Options::default()
    })
    .unwrap()
}

#[test]
fn slow_first_token_times_out() {
    for kind in ALL_KINDS {
        // Headers arrive at once, the first chunk late.
        let reply = stream_reply(kind, &["late"]).first_chunk_after(Duration::from_secs(3));
        let server = MockServer::start(move |_| reply.clone());
        let ai = timeouts(Duration::from_millis(300), Duration::from_secs(10));
        let started = Instant::now();
        let e = summarize_with(&ai, kind, &server).unwrap_err();
        assert_eq!(
            e,
            AiError::Offline {
                reason: "the model did not start answering in time"
            },
            "{kind:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    // Before the response headers, too.
    let server = MockServer::start(|_| {
        stream_reply(ProviderKind::OpenAi, &["late"]).delayed(Duration::from_secs(3))
    });
    let ai = timeouts(Duration::from_millis(300), Duration::from_secs(10));
    let e = summarize_with(&ai, ProviderKind::OpenAi, &server).unwrap_err();
    assert_eq!(e.key(), "offline");
}

#[test]
fn slow_tokens_after_the_first_use_the_total_limit() {
    let chunks = openai_stream("m-1", &["a", "b", "c", "d"], None);
    let server =
        MockServer::start(move |_| Reply::stream(SSE, chunks.clone(), Duration::from_millis(150)));
    // Each chunk is within the first-token limit, but the whole answer isn't.
    let ai = timeouts(Duration::from_millis(400), Duration::from_millis(500));
    let e = summarize_with(&ai, ProviderKind::OpenAi, &server).unwrap_err();
    assert_eq!(
        e,
        AiError::Offline {
            reason: "the answer took too long"
        }
    );

    let ai = timeouts(Duration::from_millis(400), Duration::from_secs(5));
    assert_eq!(
        summarize_with(&ai, ProviderKind::OpenAi, &server).unwrap(),
        "abcd"
    );
}

#[test]
fn unreachable_provider_is_offline() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut c = config(
        ProviderKind::Ollama,
        &MockServer::start(|_| Reply::text(200, "")),
    );
    c.url = Some(format!("http://127.0.0.1:{port}"));
    let e = ai()
        .summarize(
            &c,
            &thread(),
            &SummaryOpts::default(),
            &mut collect(&mut Vec::new()),
        )
        .unwrap_err();
    assert_eq!(
        e,
        AiError::Offline {
            reason: "connection refused"
        }
    );
}

/// Stops after `after` deltas.
struct StopAfter {
    after: usize,
    seen: usize,
}

impl Sink for StopAfter {
    fn delta(&mut self, _: &str) -> ControlFlow<()> {
        self.seen += 1;
        if self.seen >= self.after {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

fn slow_stream(kind: ProviderKind) -> Reply {
    let texts: Vec<String> = (0..50).map(|i| format!("w{i} ")).collect();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let full = stream_reply(kind, &texts);
    let common::Body::Stream { chunks, .. } = full.body else {
        unreachable!()
    };
    let chunks = chunks.into_iter().map(|(_, c)| c).collect();
    let content_type = if kind == ProviderKind::Ollama {
        NDJSON
    } else {
        SSE
    };
    Reply::stream(content_type, chunks, Duration::from_millis(40))
}

#[test]
fn sink_break_cancels_and_drops_the_connection() {
    for kind in ALL_KINDS {
        let reply = slow_stream(kind);
        let server = MockServer::start(move |_| reply.clone());
        let mut sink = StopAfter { after: 2, seen: 0 };
        let started = Instant::now();
        let e = ai()
            .draft_reply(
                &config(kind, &server),
                &thread(),
                &Default::default(),
                &mut sink,
            )
            .unwrap_err();
        assert_eq!(e, AiError::Cancelled, "{kind:?}");
        assert_eq!(sink.seen, 2);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            server.saw_abort(Duration::from_secs(2)),
            "{kind:?}: provider kept streaming"
        );
    }
}

/// Collects deltas and carries a token.
struct Tokened {
    token: CancelToken,
    deltas: usize,
}

impl Sink for Tokened {
    fn delta(&mut self, _: &str) -> ControlFlow<()> {
        self.deltas += 1;
        ControlFlow::Continue(())
    }

    fn cancel_token(&self) -> Option<&CancelToken> {
        Some(&self.token)
    }
}

#[test]
fn token_cancels_while_waiting_for_the_first_token() {
    for kind in ALL_KINDS {
        let reply = slow_stream(kind).first_chunk_after(Duration::from_secs(1));
        let server = MockServer::start(move |_| reply.clone());
        let token = CancelToken::new();
        let canceller = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            canceller.cancel();
        });
        let mut sink = Tokened { token, deltas: 0 };
        let started = Instant::now();
        let e = ai()
            .summarize(
                &config(kind, &server),
                &thread(),
                &SummaryOpts::default(),
                &mut sink,
            )
            .unwrap_err();
        assert_eq!(e, AiError::Cancelled, "{kind:?}");
        assert_eq!(sink.deltas, 0);
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(150) && took < Duration::from_secs(1),
            "{took:?}"
        );
        assert!(server.saw_abort(Duration::from_secs(3)), "{kind:?}");
    }
}

#[test]
fn token_cancels_mid_stream() {
    let reply = slow_stream(ProviderKind::Anthropic);
    let server = MockServer::start(move |_| reply.clone());
    let token = CancelToken::new();
    let canceller = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        canceller.cancel();
    });
    let mut sink = Tokened { token, deltas: 0 };
    let e = ai()
        .summarize(
            &config(ProviderKind::Anthropic, &server),
            &thread(),
            &SummaryOpts::default(),
            &mut sink,
        )
        .unwrap_err();
    assert_eq!(e, AiError::Cancelled);
    assert!(sink.deltas > 0 && sink.deltas < 50, "{}", sink.deltas);
    assert!(server.saw_abort(Duration::from_secs(2)));
}

#[test]
fn cancelled_token_sends_nothing() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::OpenAi, &["x"]));
    let token = CancelToken::new();
    token.cancel();
    let mut sink = Tokened { token, deltas: 0 };
    let e = ai()
        .summarize(
            &config(ProviderKind::OpenAi, &server),
            &thread(),
            &SummaryOpts::default(),
            &mut sink,
        )
        .unwrap_err();
    assert_eq!(e, AiError::Cancelled);
    assert_eq!(server.count(), 0);
}

#[test]
fn missing_settings() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::OpenAi, &["x"]));
    let mut c = config(ProviderKind::Anthropic, &server);
    c.api_key = Some("  ".into());
    let e = ai().triage(&c, &thread()).unwrap_err();
    assert_eq!(
        e,
        AiError::NotConfigured {
            what: "the API key"
        }
    );
    let mut c = config(ProviderKind::Ollama, &server);
    c.model = String::new();
    assert_eq!(ai().test(&c).unwrap_err().key(), "not-configured");
    assert_eq!(server.count(), 0);
}

#[test]
fn threads_share_one_ai() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Ai>();

    let chunks = openai_stream("m-1", &["a", "b", "c", "d", "e", "f"], None);
    let server = Arc::new(MockServer::start(move |_| {
        Reply::stream(SSE, chunks.clone(), Duration::from_millis(60))
    }));
    let ai = Arc::new(ai());
    let started = Instant::now();
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let (ai, server) = (ai.clone(), server.clone());
            std::thread::spawn(move || summarize_with(&ai, ProviderKind::OpenAi, &server))
        })
        .collect();
    for w in workers {
        assert_eq!(w.join().unwrap().unwrap(), "abcdef");
    }
    // Each stream takes about 0.5 s; run one after another they would take 2 s.
    let took = started.elapsed();
    assert!(took < Duration::from_millis(1200), "{took:?}");
}
