// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The HTTP client, the runtime it runs on, and the streaming loop.
//!
//! reqwest's async client on a private tokio runtime that callers block on, rather than
//! its blocking client: a blocking body read can't be interrupted, and cancelling
//! (`ai.cancel`, a closed connection) has to drop the provider connection while the model
//! is still thinking. Engine threads block on the runtime at once (`Runtime::block_on`).
//! It has one worker thread, which runs hyper's connection tasks between calls: without
//! it a cancelled request's socket stays open, and the provider keeps generating, until
//! the next call.
//!
//! Each request logs one line with the provider kind, host, model, feature, status, token
//! counts and timing; never the path, the prompt or the answer.

use std::fmt;
use std::future::Future;
use std::time::{Duration, Instant};

use reqwest::header::RETRY_AFTER;
use reqwest::{Client, RequestBuilder, Response, Url};
use serde_json::Value;
use tokio::runtime::Runtime;
use tracing::{info, warn};

use crate::cancel::{wait_cancelled, CancelToken, Sink};
use crate::config::Target;
use crate::error::{AiError, Result};
use crate::wire::{self, ChatRequest, Decoder, Event};
use crate::Feature;

const FIRST_TOKEN_TIMEOUT: &str = "the model did not start answering in time";
const TOTAL_TIMEOUT: &str = "the answer took too long";
const REQUEST_TIMEOUT: &str = "timed out";

/// The answer may be this many bytes per token of `max_output_tokens`: four times what
/// any script needs. Past it the provider is misbehaving and the stream fails.
const ANSWER_BYTES_PER_TOKEN: usize = 16;
/// Error bodies are read this far; the message shown is a short excerpt anyway.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
/// OpenRouter's full model list is about 1 MiB.
const MAX_JSON_BODY_BYTES: usize = 8 << 20;

pub(crate) struct Http {
    client: Client,
    runtime: Runtime,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Timeouts {
    pub first_token: Duration,
    pub total: Duration,
}

/// A finished stream, or what arrived of a failed one.
#[derive(Default)]
pub(crate) struct ChatOutput {
    pub text: String,
    /// As the provider reported it.
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

impl fmt::Debug for ChatOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatOutput")
            .field("text_chars", &self.text.chars().count())
            .field("model", &self.model)
            .field("input_tokens", &self.input_tokens)
            .field("output_tokens", &self.output_tokens)
            .finish()
    }
}

/// A failed chat request. `partial` is set once the provider accepted the request (a 2xx
/// status), since it may bill for it from then on: what streamed before the failure, the
/// cut, the cancel or the timeout.
#[derive(Debug)]
pub(crate) struct ChatFailure {
    pub error: AiError,
    pub partial: Option<ChatOutput>,
}

impl Http {
    pub fn new(connect_timeout: Duration) -> Result<Self> {
        airpost_integrations::ensure_crypto_provider();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            // The engine's AI worker lane name, so a panic in a connection task is
            // contained there too.
            .thread_name("ai")
            .enable_all()
            .build()
            .map_err(|_| AiError::Offline {
                reason: "HTTP client unavailable",
            })?;
        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_rustls()
            .user_agent(concat!("Airpost/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| AiError::Offline {
                reason: "HTTP client unavailable",
            })?;
        Ok(Http { client, runtime })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Sends a chat request and streams its answer into `sink`.
    pub fn chat(
        &self,
        target: &Target,
        feature: Feature,
        req: &ChatRequest,
        sink: &mut dyn Sink,
        timeouts: Timeouts,
    ) -> std::result::Result<ChatOutput, ChatFailure> {
        let cancel = sink.cancel_token().cloned();
        let started = Instant::now();
        let mut progress = Progress::default();
        let mut out = ChatOutput::default();
        let result = self.runtime.block_on(stream(
            &self.client,
            target,
            req,
            sink,
            cancel.as_ref(),
            timeouts,
            started,
            &mut progress,
            &mut out,
        ));
        let ms = started.elapsed().as_millis();
        let (kind, host, model) = (target.kind.as_str(), &target.host, &target.model);
        let feature = feature.as_str();
        match result {
            Ok(()) => {
                let first = progress
                    .first_token
                    .map_or("none".to_owned(), |d: Duration| {
                        format!("{} ms", d.as_millis())
                    });
                info!(
                    "- ai: {kind} {host} {model} {feature} HTTP {} in {ms} ms, first token {first}, {} in / {} out tokens",
                    progress.status.unwrap_or(0),
                    count(out.input_tokens),
                    count(out.output_tokens),
                );
                Ok(out)
            }
            Err(error) => {
                if error == AiError::Cancelled {
                    info!("- ai: {kind} {host} {model} {feature} cancelled after {ms} ms");
                } else {
                    warn!("-X ai: {kind} {host} {model} {feature} failed after {ms} ms: {error}");
                }
                let accepted = progress.status.is_some_and(|s| (200..300).contains(&s));
                Err(ChatFailure {
                    error,
                    partial: accepted.then_some(out),
                })
            }
        }
    }

    /// A GET whose 2xx body is JSON (the model list).
    pub fn get_json(
        &self,
        target: &Target,
        url: &Url,
        req: RequestBuilder,
        timeout: Duration,
    ) -> Result<Value> {
        let started = Instant::now();
        let deadline = tokio::time::Instant::from_std(started + timeout);
        let mut status = 0;
        let result = self.runtime.block_on(async {
            let resp = race(req.send(), None, deadline, REQUEST_TIMEOUT)
                .await?
                .map_err(transport)?;
            status = resp.status().as_u16();
            if !(200..300).contains(&status) {
                let (body, retry_after) = read_body(resp, deadline, MAX_ERROR_BODY_BYTES).await?;
                return Err(wire::classify(Some(status), retry_after, &body));
            }
            let (body, _) = read_body(resp, deadline, MAX_JSON_BODY_BYTES + 1).await?;
            if body.len() > MAX_JSON_BODY_BYTES {
                return Err(AiError::Provider {
                    status: Some(status),
                    message: "the response is too large".into(),
                });
            }
            serde_json::from_str(&body).map_err(|_| AiError::Provider {
                status: Some(status),
                message: "unexpected response body".into(),
            })
        });
        let ms = started.elapsed().as_millis();
        let (kind, host) = (target.kind.as_str(), url.host_str().unwrap_or(""));
        match &result {
            Ok(_) => info!("- ai: {kind} {host} models HTTP {status} in {ms} ms"),
            Err(e) => warn!("-X ai: {kind} {host} models failed after {ms} ms: {e}"),
        }
        result
    }
}

fn count(n: Option<u64>) -> String {
    n.map_or("?".to_owned(), |n| n.to_string())
}

/// How far a request got, for the log line and the usage of a failed one.
#[derive(Default)]
struct Progress {
    status: Option<u16>,
    /// The first answer or reasoning token: from then on the total deadline applies.
    first_token: Option<Duration>,
}

/// Streams the answer into `out` and `sink`. On failure `out` holds what arrived.
#[allow(clippy::too_many_arguments)]
async fn stream(
    client: &Client,
    target: &Target,
    req: &ChatRequest,
    sink: &mut dyn Sink,
    cancel: Option<&CancelToken>,
    timeouts: Timeouts,
    started: Instant,
    progress: &mut Progress,
    out: &mut ChatOutput,
) -> Result<()> {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(AiError::Cancelled);
    }
    let deadline = tokio::time::Instant::from_std(started + timeouts.total);
    let first_deadline = deadline.min(tokio::time::Instant::from_std(
        started + timeouts.first_token,
    ));
    let (_, builder) = wire::chat_request(client, target, req)?;

    let mut resp = race(builder.send(), cancel, first_deadline, FIRST_TOKEN_TIMEOUT)
        .await?
        .map_err(transport)?;
    let status = resp.status().as_u16();
    progress.status = Some(status);
    if !(200..300).contains(&status) {
        let (body, retry_after) = race(
            read_body(resp, deadline, MAX_ERROR_BODY_BYTES),
            cancel,
            deadline,
            TOTAL_TIMEOUT,
        )
        .await??;
        return Err(wire::classify(Some(status), retry_after, &body));
    }

    let max_answer = (req.max_output_tokens as usize).saturating_mul(ANSWER_BYTES_PER_TOKEN);
    let mut decoder = Decoder::new(target.kind);
    let mut events = Vec::new();
    let mut finished = false;
    let mut done = false;
    while !done {
        let (limit, reason) = if progress.first_token.is_some() {
            (deadline, TOTAL_TIMEOUT)
        } else {
            (first_deadline, FIRST_TOKEN_TIMEOUT)
        };
        let chunk = race(resp.chunk(), cancel, limit, reason)
            .await?
            .map_err(transport)?;
        let eof = chunk.is_none();
        match chunk {
            Some(bytes) => decoder.feed(&bytes, &mut events),
            None => decoder.finish(&mut events),
        }
        for event in events.drain(..) {
            match event {
                Event::Text(text) => {
                    progress
                        .first_token
                        .get_or_insert_with(|| started.elapsed());
                    if out.text.len() + text.len() > max_answer {
                        return Err(AiError::provider("the answer was too long"));
                    }
                    out.text.push_str(&text);
                    if sink.delta(&text).is_break() {
                        return Err(AiError::Cancelled);
                    }
                }
                Event::Progress => {
                    progress
                        .first_token
                        .get_or_insert_with(|| started.elapsed());
                }
                Event::Model(model) => out.model = Some(model),
                Event::InputTokens(n) => out.input_tokens = Some(n),
                Event::OutputTokens(n) => out.output_tokens = Some(n),
                Event::Finished => finished = true,
                Event::Done => done = true,
                Event::Failed(e) => return Err(e),
            }
        }
        if eof {
            if !done && !finished {
                return Err(AiError::provider("the answer ended early"));
            }
            break;
        }
    }
    Ok(())
}

/// `fut`, unless the token fires or the deadline passes first. Dropping `fut` drops the
/// request and with it the provider connection.
async fn race<F: Future>(
    fut: F,
    cancel: Option<&CancelToken>,
    deadline: tokio::time::Instant,
    timeout_reason: &'static str,
) -> Result<F::Output> {
    tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => Err(AiError::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(AiError::Offline { reason: timeout_reason }),
        out = fut => Ok(out),
    }
}

/// The body of an error or JSON response, up to `limit` bytes, and its Retry-After.
async fn read_body(
    mut resp: Response,
    deadline: tokio::time::Instant,
    limit: usize,
) -> Result<(String, Option<Duration>)> {
    let retry_after = resp
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let mut body = Vec::new();
    while body.len() < limit {
        match race(resp.chunk(), None, deadline, REQUEST_TIMEOUT)
            .await?
            .map_err(transport)?
        {
            Some(chunk) => body.extend_from_slice(&chunk[..chunk.len().min(limit - body.len())]),
            None => break,
        }
    }
    Ok((String::from_utf8_lossy(&body).into_owned(), retry_after))
}

fn transport(e: reqwest::Error) -> AiError {
    airpost_integrations::transport_error(&e).into()
}
