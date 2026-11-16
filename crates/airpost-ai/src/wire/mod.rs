// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The three wire formats: request bodies, stream framing and decoding, the model list,
//! and how provider error responses map to [`AiError`].

mod anthropic;
mod ollama;
mod openai;

use std::fmt;
use std::time::Duration;

use reqwest::{Client, RequestBuilder, Url};
use serde_json::Value;

use crate::config::{ProviderKind, Target};
use crate::error::{AiError, Result};
use crate::ModelInfo;

/// One chat request: a system prompt and one user message.
pub(crate) struct ChatRequest {
    pub system: String,
    pub user: String,
    pub max_output_tokens: u32,
    /// Ask for a JSON object matching this schema (triage, suggestions).
    pub json_schema: Option<Value>,
}

/// What a decoded stream line or event carries.
#[derive(PartialEq)]
pub(crate) enum Event {
    Text(String),
    /// The model is reasoning (thinking or reasoning deltas): it has started, but has no
    /// answer text yet. The reasoning itself is dropped.
    Progress,
    /// The model that answered, when the provider says (it may resolve an alias).
    Model(String),
    InputTokens(u64),
    OutputTokens(u64),
    /// The model stopped generating; usage may still follow.
    Finished,
    /// The provider's end-of-stream marker.
    Done,
    Failed(AiError),
}

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Text(text) => write!(f, "Text({} chars)", text.chars().count()),
            Event::Progress => f.write_str("Progress"),
            Event::Model(model) => f.debug_tuple("Model").field(model).finish(),
            Event::InputTokens(n) => f.debug_tuple("InputTokens").field(n).finish(),
            Event::OutputTokens(n) => f.debug_tuple("OutputTokens").field(n).finish(),
            Event::Finished => f.write_str("Finished"),
            Event::Done => f.write_str("Done"),
            Event::Failed(e) => f.debug_tuple("Failed").field(e).finish(),
        }
    }
}

pub(crate) fn chat_request(
    client: &Client,
    target: &Target,
    req: &ChatRequest,
) -> Result<(Url, RequestBuilder)> {
    match target.kind {
        ProviderKind::OpenAi => openai::chat(client, target, req),
        ProviderKind::Anthropic => anthropic::chat(client, target, req),
        ProviderKind::Ollama => ollama::chat(client, target, req),
    }
}

/// The model list request; `after` continues a paged list (Anthropic).
pub(crate) fn models_request(
    client: &Client,
    target: &Target,
    after: Option<&str>,
) -> Result<(Url, RequestBuilder)> {
    match target.kind {
        ProviderKind::OpenAi => openai::models(client, target),
        ProviderKind::Anthropic => anthropic::models(client, target, after),
        ProviderKind::Ollama => ollama::models(client, target),
    }
}

/// The models in one page and the cursor of the next, if any.
pub(crate) fn parse_models(kind: ProviderKind, body: &Value) -> (Vec<ModelInfo>, Option<String>) {
    match kind {
        ProviderKind::OpenAi => (openai::parse_models(body), None),
        ProviderKind::Anthropic => anthropic::parse_models(body),
        ProviderKind::Ollama => (ollama::parse_models(body), None),
    }
}

pub(crate) fn url(base: &str, path: &str, query: &[(&str, &str)]) -> Result<Url> {
    let mut url = Url::parse(&format!("{base}{path}"))
        .map_err(|_| AiError::bad_request("the provider URL is not valid"))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Ok(url)
}

/// The longest stream line, and the longest SSE event, a provider may send. Real ones are a
/// few KiB; past this the stream fails instead of buffering without bound.
const MAX_LINE_BYTES: usize = 1 << 20;

/// Turns response bytes into [`Event`]s: server-sent events for OpenAI and Anthropic,
/// newline-delimited JSON for Ollama. Bytes are split on `\n` before decoding, so a UTF-8
/// sequence split across chunks arrives whole.
pub(crate) struct Decoder {
    kind: ProviderKind,
    pending: Vec<u8>,
    data: String,
}

impl Decoder {
    pub fn new(kind: ProviderKind) -> Self {
        Decoder {
            kind,
            pending: Vec::new(),
            data: String::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        self.pending.extend_from_slice(bytes);
        while let Some(i) = self.pending.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.pending.drain(..=i).collect();
            let line = String::from_utf8_lossy(&raw);
            self.line(line.trim_end_matches(['\n', '\r']), out);
        }
        if self.pending.len() > MAX_LINE_BYTES || self.data.len() > MAX_LINE_BYTES {
            self.pending = Vec::new();
            self.data = String::new();
            out.push(Event::Failed(AiError::provider(
                "the provider sent a stream line that is too long",
            )));
        }
    }

    /// The end of the body: a last line without a newline and an event without its blank
    /// line still count.
    pub fn finish(&mut self, out: &mut Vec<Event>) {
        if !self.pending.is_empty() {
            let raw = std::mem::take(&mut self.pending);
            let line = String::from_utf8_lossy(&raw).into_owned();
            self.line(line.trim_end_matches('\r'), out);
        }
        self.dispatch(out);
    }

    fn line(&mut self, line: &str, out: &mut Vec<Event>) {
        if self.kind == ProviderKind::Ollama {
            if !line.trim().is_empty() {
                ollama::decode(line, out);
            }
            return;
        }
        // SSE (html.spec.whatwg.org, "Interpreting an event stream"): `event:`, `id:`,
        // `retry:` and `:` comments (OpenRouter's keep-alives) carry nothing needed here;
        // both providers put the type inside the JSON.
        if line.is_empty() {
            self.dispatch(out);
        } else if let Some(data) = line.strip_prefix("data:") {
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
        }
    }

    fn dispatch(&mut self, out: &mut Vec<Event>) {
        if self.data.is_empty() {
            return;
        }
        let data = std::mem::take(&mut self.data);
        match self.kind {
            ProviderKind::OpenAi => openai::decode(&data, out),
            ProviderKind::Anthropic => anthropic::decode(&data, out),
            ProviderKind::Ollama => {}
        }
    }
}

/// An error status, or an error carried in a 200 stream (`status` `None`), as an
/// [`AiError`]. Providers name the failure in `error.code` (OpenAI), `error.type`
/// (Anthropic) or only in the message (Ollama, most compatible servers).
pub(crate) fn classify(status: Option<u16>, retry_after: Option<Duration>, body: &str) -> AiError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = [&v["error"]["code"], &v["error"]["type"], &v["code"]]
        .into_iter()
        .find_map(|c| c.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let message = tern_integrations::provider_message(body);
    let lower = message.to_lowercase();

    // Out of credit for good. A bare "quota" isn't enough: Gemini words its per-minute
    // limits as "Resource has been exhausted (e.g. check quota)".
    let billing = code == "insufficient_quota"
        || code.contains("billing")
        || lower.contains("billing")
        || lower.contains("credit");
    let quota = billing || lower.contains("quota");
    let context = code == "context_length_exceeded"
        || code == "request_too_large"
        || CONTEXT_PHRASES.iter().any(|p| lower.contains(p));
    let model_missing = code == "model_not_found"
        || (lower.contains("model")
            && (lower.contains("not found") || lower.contains("does not exist")));

    let status_code = match status {
        Some(s) => s,
        None => {
            return classify_stream_error(&code, message, billing, quota, context, model_missing)
        }
    };
    let status = Some(status_code);
    match status_code {
        401 | 403 => AiError::Auth {
            status: status_code,
        },
        402 => AiError::QuotaExceeded {
            status,
            retry_after: None,
        },
        429 if billing => AiError::QuotaExceeded {
            status,
            retry_after,
        },
        429 => AiError::RateLimited { retry_after },
        413 => AiError::ContextTooLong { status, message },
        404 if model_missing || code.contains("not_found") => {
            AiError::ModelNotFound { status, message }
        }
        404 => AiError::BadRequest {
            status,
            message: "the provider has no such endpoint; check the URL".into(),
        },
        300..=399 => AiError::BadRequest {
            status,
            message: "the provider redirected the request; check the URL".into(),
        },
        400..=499 if context => AiError::ContextTooLong { status, message },
        400..=499 if model_missing => AiError::ModelNotFound { status, message },
        400..=499 if quota => AiError::QuotaExceeded {
            status,
            retry_after: None,
        },
        400..=499 => AiError::BadRequest { status, message },
        _ => AiError::Provider { status, message },
    }
}

fn classify_stream_error(
    code: &str,
    message: String,
    billing: bool,
    quota: bool,
    context: bool,
    model_missing: bool,
) -> AiError {
    if billing {
        AiError::QuotaExceeded {
            status: None,
            retry_after: None,
        }
    } else if code.contains("rate_limit") || quota {
        AiError::RateLimited { retry_after: None }
    } else if context {
        AiError::ContextTooLong {
            status: None,
            message,
        }
    } else if model_missing {
        AiError::ModelNotFound {
            status: None,
            message,
        }
    } else {
        AiError::Provider {
            status: None,
            message,
        }
    }
}

/// How compatible servers word a prompt over the window (vLLM, llama.cpp, LM Studio,
/// Groq, Anthropic's `prompt is too long`).
const CONTEXT_PHRASES: &[&str] = &[
    "context length",
    "context_length",
    "context window",
    "maximum context",
    "prompt is too long",
    "too many tokens",
    "reduce the length",
    "exceeds the available context",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn events(kind: ProviderKind, chunks: &[&[u8]]) -> Vec<Event> {
        let mut d = Decoder::new(kind);
        let mut out = Vec::new();
        for c in chunks {
            d.feed(c, &mut out);
        }
        d.finish(&mut out);
        out
    }

    #[test]
    fn sse_lines_split_anywhere() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"héllo\"}}]}\n\n: keep-alive\n\ndata: [DONE]\n\n";
        let bytes = body.as_bytes();
        // Split inside the two-byte é.
        let split = body.find('é').unwrap() + 1;
        let out = events(ProviderKind::OpenAi, &[&bytes[..split], &bytes[split..]]);
        assert_eq!(out, [Event::Text("héllo".into()), Event::Done]);
    }

    #[test]
    fn crlf_and_unterminated_events() {
        let out = events(
            ProviderKind::OpenAi,
            &[b"data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\r\n\r\ndata: [DONE]"],
        );
        assert_eq!(out, [Event::Text("a".into()), Event::Done]);
        let out = events(
            ProviderKind::Ollama,
            &[b"{\"message\":{\"content\":\"x\"},\"done\":false}\n{\"done\":true}"],
        );
        assert_eq!(out, [Event::Text("x".into()), Event::Finished, Event::Done]);
    }

    #[test]
    fn status_classes() {
        let key = |status, body: &str| classify(Some(status), None, body).key();
        assert_eq!(key(401, ""), "auth");
        assert_eq!(key(402, ""), "quota-exceeded");
        assert_eq!(
            key(
                429,
                r#"{"error":{"code":"insufficient_quota","message":"You exceeded your current quota"}}"#
            ),
            "quota-exceeded"
        );
        assert_eq!(
            key(429, r#"{"error":{"type":"rate_limit_error"}}"#),
            "rate-limited"
        );
        assert_eq!(
            key(
                404,
                r#"{"error":{"code":"model_not_found","message":"The model `x` does not exist"}}"#
            ),
            "model-not-found"
        );
        assert_eq!(
            key(
                404,
                r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-x"}}"#
            ),
            "model-not-found"
        );
        assert_eq!(
            key(
                404,
                r#"{"error":"model 'llama9' not found, try pulling it first"}"#
            ),
            "model-not-found"
        );
        assert_eq!(key(404, "<html>Not Found</html>"), "bad-request");
        assert_eq!(
            key(
                400,
                r#"{"error":{"code":"context_length_exceeded","message":"too long"}}"#
            ),
            "context-too-long"
        );
        assert_eq!(
            key(
                400,
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 210000 tokens > 200000 maximum"}}"#
            ),
            "context-too-long"
        );
        assert_eq!(
            key(
                400,
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low"}}"#
            ),
            "quota-exceeded"
        );
        assert_eq!(key(413, ""), "context-too-long");
        assert_eq!(key(400, r#"{"error":"bad"}"#), "bad-request");
        assert_eq!(
            key(
                529,
                r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
            ),
            "provider-error"
        );
        assert_eq!(key(302, ""), "bad-request");
        let e = classify(Some(429), Some(Duration::from_secs(9)), "");
        assert_eq!(e.retry_after(), Some(Duration::from_secs(9)));
    }

    #[test]
    fn quota_wording_without_billing_is_a_rate_limit() {
        let after = Some(Duration::from_secs(20));
        let gemini = r#"[{"error":{"code":429,"message":"Resource has been exhausted (e.g. check quota).","status":"RESOURCE_EXHAUSTED"}}]"#;
        let e = classify(Some(429), after, gemini);
        assert_eq!(e, AiError::RateLimited { retry_after: after });
        let e = classify(
            Some(429),
            after,
            r#"{"error":{"message":"Quota exceeded for metric: requests per minute"}}"#,
        );
        assert_eq!(e, AiError::RateLimited { retry_after: after });
        let e = classify(
            Some(429),
            after,
            r#"{"error":{"code":"insufficient_quota","message":"check your plan and billing details"}}"#,
        );
        assert_eq!(
            e,
            AiError::QuotaExceeded {
                status: Some(429),
                retry_after: after
            }
        );
        assert_eq!(
            classify(
                None,
                None,
                r#"{"error":{"message":"quota exhausted, slow down"}}"#
            )
            .key(),
            "rate-limited"
        );
    }

    #[test]
    fn overlong_lines_fail() {
        let mut d = Decoder::new(ProviderKind::Ollama);
        let mut out = Vec::new();
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..(MAX_LINE_BYTES / chunk.len() + 1) {
            d.feed(&chunk, &mut out);
        }
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], Event::Failed(e) if e.to_string().contains("too long")));
        assert!(d.pending.capacity() <= chunk.len());
    }

    #[test]
    fn text_is_not_in_debug() {
        let d = format!("{:?}", Event::Text("secret plans".into()));
        assert_eq!(d, "Text(12 chars)");
    }

    #[test]
    fn stream_error_classes() {
        let key = |body: &str| classify(None, None, body).key();
        assert_eq!(
            key(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#),
            "provider-error"
        );
        assert_eq!(
            key(r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#),
            "rate-limited"
        );
        assert_eq!(key(r#"{"error":"something broke"}"#), "provider-error");
    }
}
