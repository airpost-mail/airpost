// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The three wire formats: request bodies, stream framing and decoding, the model list,
//! and how provider error responses map to [`AiError`].

mod anthropic;
mod ollama;
mod openai;

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
#[derive(Debug, PartialEq)]
pub(crate) enum Event {
    Text(String),
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
    let message = ms_integrations::provider_message(body);
    let lower = message.to_lowercase();

    let quota = code == "insufficient_quota"
        || code.contains("billing")
        || lower.contains("quota")
        || lower.contains("credit balance");
    let context = code == "context_length_exceeded"
        || code == "request_too_large"
        || CONTEXT_PHRASES.iter().any(|p| lower.contains(p));
    let model_missing = code == "model_not_found"
        || (lower.contains("model")
            && (lower.contains("not found") || lower.contains("does not exist")));

    let status_code = match status {
        Some(s) => s,
        None => return classify_stream_error(&code, message, quota, context, model_missing),
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
        429 if quota => AiError::QuotaExceeded {
            status,
            retry_after: None,
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
    quota: bool,
    context: bool,
    model_missing: bool,
) -> AiError {
    if code.contains("rate_limit") {
        AiError::RateLimited { retry_after: None }
    } else if quota {
        AiError::QuotaExceeded {
            status: None,
            retry_after: None,
        }
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
