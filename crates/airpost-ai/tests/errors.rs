// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Error statuses and stream failures of each provider, as `AiError` keys; the one retry of
//! non-streamed calls; timeouts, cut streams and cancellation.

mod common;

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use airpost_ai::{
    Ai, AiError, CancelToken, Feature, Options, ProviderKind, ReplyOpts, Sink, SummaryOpts,
    UsageRecord,
};
use common::*;
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

        // A per-minute limit worded as a quota (Gemini) is still a rate limit.
        let e = summarize(
            kind,
            Reply::json(
                429,
                json!({"error": {"code": 429, "message": "Resource has been exhausted (e.g. check quota).", "status": "RESOURCE_EXHAUSTED"}}),
            )
            .header("Retry-After", "30"),
        );
        assert_eq!(
            e,
            AiError::RateLimited {
                retry_after: Some(Duration::from_secs(30))
            },
            "{kind:?}"
        );
        let e = summarize(
            kind,
            Reply::json(
                429,
                json!({"error": {"code": "insufficient_quota", "message": "check your plan and billing details"}}),
            )
            .header("Retry-After", "60"),
        );
        assert_eq!(e.key(), "quota-exceeded");
        assert_eq!(e.retry_after(), Some(Duration::from_secs(60)));
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
    let texts: Vec<String> = (0..10).map(|i| format!("{i}")).collect();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let chunks = openai_stream("m-1", &texts, None);
    let server =
        MockServer::start(move |_| Reply::stream(SSE, chunks.clone(), Duration::from_millis(250)));
    let ai = timeouts(Duration::from_millis(1500), Duration::from_millis(1500));
    let e = summarize_with(&ai, ProviderKind::OpenAi, &server).unwrap_err();
    assert_eq!(
        e,
        AiError::Offline {
            reason: "the answer took too long"
        }
    );

    let ai = timeouts(Duration::from_secs(5), Duration::from_secs(30));
    assert_eq!(
        summarize_with(&ai, ProviderKind::OpenAi, &server).unwrap(),
        "0123456789"
    );
}

/// A stream that reasons for at least 1.25 s before it answers "Done.": every chunk
/// before the answer comes 250 ms after the last.
fn thinking_reply(kind: ProviderKind) -> Reply {
    let gap = Duration::from_millis(250);
    let (content_type, chunks): (&str, Vec<(Duration, String)>) = match kind {
        ProviderKind::OpenAi => {
            let mut full = openai_stream("m-1", &["Done."], Some((100, 10)));
            let thinking = (0..5).map(|_| {
                sse_data(&json!({"model": "m-1", "choices": [{"index": 0,
                    "delta": {"reasoning_content": "hmm "}, "finish_reason": null}]}))
            });
            full.splice(1..1, thinking);
            (SSE, timed(full, gap, 1..6))
        }
        ProviderKind::Anthropic => {
            let mut full = anthropic_stream("m-1", &["Done."], (100, 10));
            let thinking = (0..4).map(|_| {
                sse_event(
                    "content_block_delta",
                    &json!({"type": "content_block_delta", "index": 0,
                        "delta": {"type": "thinking_delta", "thinking": "hmm "}}),
                )
            });
            full.splice(3..3, thinking);
            (SSE, timed(full, gap, 1..7))
        }
        ProviderKind::Ollama => {
            let mut full = ollama_stream("m-1", &["Done."], Some((100, 10)));
            let thinking = (0..5).map(|_| {
                format!(
                    "{}\n",
                    json!({"model": "m-1", "message": {"role": "assistant", "content": "",
                        "thinking": "hmm "}, "done": false})
                )
            });
            full.splice(0..0, thinking);
            (NDJSON, timed(full, gap, 0..5))
        }
    };
    Reply {
        status: 200,
        headers: vec![("Content-Type".into(), content_type.into())],
        body: common::Body::Stream { chunks, cut: false },
        delay: Duration::ZERO,
    }
}

/// `chunks` with `gap` before each one in `slow`, none before the rest.
fn timed(
    chunks: Vec<String>,
    gap: Duration,
    slow: std::ops::Range<usize>,
) -> Vec<(Duration, String)> {
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            (
                if slow.contains(&i) {
                    gap
                } else {
                    Duration::ZERO
                },
                c,
            )
        })
        .collect()
}

#[test]
fn reasoning_counts_as_the_first_token() {
    for kind in ALL_KINDS {
        let reply = thinking_reply(kind);
        let server = MockServer::start(move |_| reply.clone());
        let ai = timeouts(Duration::from_secs(1), Duration::from_secs(10));
        let started = Instant::now();
        let text = summarize_with(&ai, kind, &server);
        assert_eq!(text.as_deref(), Ok("Done."), "{kind:?}");
        assert!(started.elapsed() >= Duration::from_millis(1250), "{kind:?}");
    }
    // Keep-alives are not progress.
    let pings: Vec<String> = (0..6)
        .map(|_| sse_event("ping", &json!({"type": "ping"})))
        .collect();
    let server =
        MockServer::start(move |_| Reply::stream(SSE, pings.clone(), Duration::from_millis(200)));
    let ai = timeouts(Duration::from_millis(500), Duration::from_secs(10));
    let e = summarize_with(&ai, ProviderKind::Anthropic, &server).unwrap_err();
    assert_eq!(
        e,
        AiError::Offline {
            reason: "the model did not start answering in time"
        }
    );
}

#[test]
fn overlong_answers_fail() {
    let texts: Vec<String> = (0..40).map(|_| "x".repeat(100)).collect();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let reply = stream_reply(ProviderKind::OpenAi, &texts);
    let server = MockServer::start(move |_| reply.clone());
    let mut c = config(ProviderKind::OpenAi, &server);
    // 128 tokens at most 2 KiB.
    c.max_output_tokens = Some(128);
    let e = ai()
        .draft_reply(
            &c,
            &thread(),
            &Default::default(),
            &mut collect(&mut Vec::new()),
        )
        .unwrap_err();
    assert_eq!(e.key(), "provider-error");
    assert!(e.to_string().contains("too long"), "{e}");
}

#[test]
fn oversized_bodies_are_not_read_whole() {
    let huge = format!("{{\"data\": [{}]}}", "{\"id\": \"m\"},".repeat(800_000));
    let server = MockServer::start(move |_| Reply::text(200, &huge));
    let mut c = config(ProviderKind::OpenAi, &server);
    c.model = String::new();
    let e = ai().models(&c).unwrap_err();
    assert_eq!(e.key(), "provider-error");
    assert!(e.to_string().contains("too large"), "{e}");

    // A huge error body is still classified from its start.
    let body = format!(
        "{{\"error\": {{\"message\": \"Invalid API key\"}}}}{}",
        " ".repeat(4 << 20)
    );
    let server = MockServer::start(move |_| Reply::text(401, &body));
    c.url = Some(format!("{}/v1", server.url()));
    assert_eq!(ai().models(&c).unwrap_err().key(), "auth");
}

#[test]
fn plain_http_to_remote_hosts_is_refused() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::OpenAi, &["x"]));
    let mut c = config(ProviderKind::OpenAi, &server);
    c.url = Some("http://8.8.8.8/v1".into());
    let started = Instant::now();
    let e = ai()
        .summarize(
            &c,
            &thread(),
            &SummaryOpts::default(),
            &mut collect(&mut Vec::new()),
        )
        .unwrap_err();
    assert_eq!(e.key(), "bad-request");
    assert!(e.to_string().contains("use https"), "{e}");
    assert_eq!(ai().models(&c).unwrap_err().key(), "bad-request");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "nothing was sent"
    );
    assert_eq!(server.count(), 0);
}

fn observed() -> (Ai, Arc<Mutex<Vec<UsageRecord>>>) {
    let seen: Arc<Mutex<Vec<UsageRecord>>> = Arc::default();
    let sink = seen.clone();
    let ai = Ai::with_options(Options {
        retry_delay: Duration::from_millis(10),
        usage_observer: Some(Arc::new(move |r: &UsageRecord| {
            sink.lock().unwrap().push(r.clone())
        })),
        ..Options::default()
    })
    .unwrap();
    (ai, seen)
}

#[test]
fn failed_streams_still_count_usage() {
    // Cancelled after two deltas: the input tokens came in message_start.
    let reply = slow_stream(ProviderKind::Anthropic);
    let server = MockServer::start(move |_| reply.clone());
    let (ai, seen) = observed();
    let mut sink = StopAfter { after: 2, seen: 0 };
    let e = ai
        .draft_reply(
            &config(ProviderKind::Anthropic, &server),
            &thread(),
            &Default::default(),
            &mut sink,
        )
        .unwrap_err();
    assert_eq!(e, AiError::Cancelled);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].feature, Feature::Reply);
        let usage = seen[0].usage;
        assert_eq!(usage.input_tokens, 100);
        assert!(usage.estimated && usage.output_tokens >= 1, "{usage:?}");
    }

    // Cut mid-stream, nothing reported: both counts estimated.
    let full = stream_reply(ProviderKind::OpenAi, &["one ", "two ", "three"]);
    let server = MockServer::start(move |_| full.clone().cut_after(3));
    let (ai, seen) = observed();
    summarize_with(&ai, ProviderKind::OpenAi, &server).unwrap_err();
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].usage.estimated && seen[0].usage.input_tokens > 50);
    }

    // A refused request bills nothing; the retried triage counts once.
    let calls = Arc::new(AtomicUsize::new(0));
    let n = calls.clone();
    let server = MockServer::start(move |_| {
        if n.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::json(500, json!({"error": "boom"}))
        } else {
            stream_reply(
                ProviderKind::OpenAi,
                &[r#"{"category":"other","needsReply":false,"reason":"x"}"#],
            )
        }
    });
    let (ai, seen) = observed();
    ai.triage(&config(ProviderKind::OpenAi, &server), &thread())
        .unwrap();
    assert_eq!(server.count(), 2);
    assert_eq!(seen.lock().unwrap().len(), 1);
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
    Reply::stream(content_type, chunks, Duration::from_millis(100))
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
        assert!(started.elapsed() < Duration::from_millis(2500));
        assert!(
            server.saw_abort(Duration::from_secs(10)),
            "{kind:?}: provider kept streaming"
        );
    }
}

/// Collects deltas and carries a token, which it cancels itself after `cancel_after`
/// deltas.
struct Tokened {
    token: CancelToken,
    deltas: usize,
    cancel_after: Option<usize>,
}

impl Tokened {
    fn new(token: CancelToken) -> Tokened {
        Tokened {
            token,
            deltas: 0,
            cancel_after: None,
        }
    }
}

impl Sink for Tokened {
    fn delta(&mut self, _: &str) -> ControlFlow<()> {
        self.deltas += 1;
        if self.cancel_after == Some(self.deltas) {
            self.token.cancel();
        }
        ControlFlow::Continue(())
    }

    fn cancel_token(&self) -> Option<&CancelToken> {
        Some(&self.token)
    }
}

#[test]
fn token_cancels_while_waiting_for_the_first_token() {
    for kind in ALL_KINDS {
        let reply = slow_stream(kind).first_chunk_after(Duration::from_secs(4));
        let server = MockServer::start(move |_| reply.clone());
        let token = CancelToken::new();
        let canceller = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            canceller.cancel();
        });
        let mut sink = Tokened::new(token);
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
            took >= Duration::from_millis(200) && took < Duration::from_secs(3),
            "{took:?}"
        );
        assert!(server.saw_abort(Duration::from_secs(15)), "{kind:?}");
    }
}

#[test]
fn token_cancels_mid_stream() {
    let reply = slow_stream(ProviderKind::Anthropic);
    let server = MockServer::start(move |_| reply.clone());
    let mut sink = Tokened::new(CancelToken::new());
    sink.cancel_after = Some(3);
    let e = ai()
        .summarize(
            &config(ProviderKind::Anthropic, &server),
            &thread(),
            &SummaryOpts::default(),
            &mut sink,
        )
        .unwrap_err();
    assert_eq!(e, AiError::Cancelled);
    assert!((3..50).contains(&sink.deltas), "{}", sink.deltas);
    assert!(server.saw_abort(Duration::from_secs(10)));
}

#[test]
fn cancelled_token_sends_nothing() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::OpenAi, &["x"]));
    let token = CancelToken::new();
    token.cancel();
    let mut sink = Tokened::new(token);
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
fn cancelled_triage_sends_nothing() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::Ollama, &["{}"]));
    let token = CancelToken::new();
    token.cancel();
    let e = ai()
        .triage_until(
            &config(ProviderKind::Ollama, &server),
            &thread(),
            Some(&token),
        )
        .unwrap_err();
    assert_eq!(e, AiError::Cancelled);
    assert_eq!(server.count(), 0);
}

#[test]
fn token_cancels_suggestions() {
    let reply = slow_stream(ProviderKind::Ollama).first_chunk_after(Duration::from_secs(4));
    let server = MockServer::start(move |_| reply.clone());
    let token = CancelToken::new();
    let canceller = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        canceller.cancel();
    });
    let started = Instant::now();
    let e = ai()
        .suggest_replies_until(
            &config(ProviderKind::Ollama, &server),
            &thread(),
            &ReplyOpts::default(),
            Some(&token),
        )
        .unwrap_err();
    assert_eq!(e, AiError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
    // Cancelled isn't retried.
    assert_eq!(server.count(), 1);
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
    let arrived = Arc::new((Mutex::new(0usize), std::sync::Condvar::new()));
    let server = Arc::new(MockServer::start(move |_| {
        let (count, all_in) = &*arrived;
        let mut count = count.lock().unwrap();
        *count += 1;
        all_in.notify_all();
        let (count, _) = all_in
            .wait_timeout_while(count, Duration::from_secs(20), |n| *n < 4)
            .unwrap();
        if *count < 4 {
            return Reply::text(500, "requests were not concurrent");
        }
        Reply::stream(SSE, chunks.clone(), Duration::ZERO)
    }));
    let ai = Arc::new(ai());
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let (ai, server) = (ai.clone(), server.clone());
            std::thread::spawn(move || summarize_with(&ai, ProviderKind::OpenAi, &server))
        })
        .collect();
    for w in workers {
        assert_eq!(w.join().unwrap().unwrap(), "abcdef");
    }
}
