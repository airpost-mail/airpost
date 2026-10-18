// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The HTTP client, the runtime it runs on, and the streaming loop.
//!
//! reqwest's async client on a private current-thread tokio runtime that callers block on,
//! rather than its blocking client: a blocking body read can't be interrupted, and
//! cancelling (`ai.cancel`, a closed connection) has to drop the provider connection while
//! the model is still thinking. Tokio lets several threads block on one current-thread
//! runtime at once (`Runtime::block_on`), so engine threads share it.
//!
//! Each request logs one line with the provider kind, host, model, feature, status, token
//! counts and timing; never the path, the prompt or the answer.

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

pub(crate) struct Http {
    client: Client,
    runtime: Runtime,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Timeouts {
    pub first_token: Duration,
    pub total: Duration,
}

/// A finished stream.
#[derive(Debug, Default)]
pub(crate) struct ChatOutput {
    pub text: String,
    /// As the provider reported it.
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

impl Http {
    pub fn new(connect_timeout: Duration) -> Result<Self> {
        ms_integrations::ensure_crypto_provider();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| AiError::Offline {
                reason: "HTTP client unavailable",
            })?;
        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_rustls()
            .user_agent(concat!("Tern/", env!("CARGO_PKG_VERSION")))
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
    ) -> Result<ChatOutput> {
        let cancel = sink.cancel_token().cloned();
        let started = Instant::now();
        let mut status = None;
        let mut first_token = None;
        let result = self.runtime.block_on(stream(
            &self.client,
            target,
            req,
            sink,
            cancel.as_ref(),
            timeouts,
            started,
            &mut status,
            &mut first_token,
        ));
        let ms = started.elapsed().as_millis();
        let (kind, host, model) = (target.kind.as_str(), &target.host, &target.model);
        let feature = feature.as_str();
        match &result {
            Ok(out) => {
                let first = first_token.map_or("none".to_owned(), |d: Duration| {
                    format!("{} ms", d.as_millis())
                });
                info!(
                    "- ai: {kind} {host} {model} {feature} HTTP {} in {ms} ms, first token {first}, {} in / {} out tokens",
                    status.unwrap_or(0),
                    count(out.input_tokens),
                    count(out.output_tokens),
                );
            }
            Err(AiError::Cancelled) => {
                info!("- ai: {kind} {host} {model} {feature} cancelled after {ms} ms")
            }
            Err(e) => warn!("-X ai: {kind} {host} {model} {feature} failed after {ms} ms: {e}"),
        }
        result
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
            let body = read_body(resp, deadline).await?;
            if !(200..300).contains(&status) {
                return Err(wire::classify(Some(status), body.1, &body.0));
            }
            serde_json::from_str(&body.0).map_err(|_| AiError::Provider {
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

#[allow(clippy::too_many_arguments)]
async fn stream(
    client: &Client,
    target: &Target,
    req: &ChatRequest,
    sink: &mut dyn Sink,
    cancel: Option<&CancelToken>,
    timeouts: Timeouts,
    started: Instant,
    status_out: &mut Option<u16>,
    first_token: &mut Option<Duration>,
) -> Result<ChatOutput> {
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
    *status_out = Some(status);
    if !(200..300).contains(&status) {
        let (body, retry_after) =
            race(read_body(resp, deadline), cancel, deadline, TOTAL_TIMEOUT).await??;
        return Err(wire::classify(Some(status), retry_after, &body));
    }

    let mut decoder = Decoder::new(target.kind);
    let mut events = Vec::new();
    let mut out = ChatOutput::default();
    let mut finished = false;
    let mut done = false;
    while !done {
        let (limit, reason) = if first_token.is_some() {
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
                    first_token.get_or_insert_with(|| started.elapsed());
                    out.text.push_str(&text);
                    if sink.delta(&text).is_break() {
                        return Err(AiError::Cancelled);
                    }
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
    Ok(out)
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

/// The whole body of an error or JSON response, and its Retry-After.
async fn read_body(
    resp: Response,
    deadline: tokio::time::Instant,
) -> Result<(String, Option<Duration>)> {
    let retry_after = resp
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = race(resp.text(), None, deadline, REQUEST_TIMEOUT)
        .await?
        .map_err(transport)?;
    Ok((body, retry_after))
}

fn transport(e: reqwest::Error) -> AiError {
    ms_integrations::transport_error(&e).into()
}
