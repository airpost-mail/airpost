// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Anthropic messages (`POST /v1/messages`, `GET /v1/models`; platform.claude.com/docs,
//! "Messages" and "Streaming messages").

use reqwest::{Client, RequestBuilder, Url};
use serde_json::{json, Value};

use super::{classify, url, ChatRequest, Event};
use crate::config::Target;
use crate::error::{AiError, Result};
use crate::ModelInfo;

const VERSION: &str = "2023-06-01";
/// The API's largest page.
const MODELS_PAGE: &str = "1000";

pub(super) fn chat(
    client: &Client,
    target: &Target,
    req: &ChatRequest,
) -> Result<(Url, RequestBuilder)> {
    let url = url(&target.base, "/v1/messages", &[])?;
    let mut body = json!({
        "model": target.model,
        "max_tokens": req.max_output_tokens,
        "system": req.system,
        "messages": [{"role": "user", "content": req.user}],
        "stream": true,
    });
    // Structured outputs instead of a prefilled `{`: current models reject a final
    // assistant turn with a 400.
    if let Some(schema) = &req.json_schema {
        body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
    }
    Ok((url.clone(), headers(client.post(url), target).json(&body)))
}

fn headers(builder: RequestBuilder, target: &Target) -> RequestBuilder {
    builder
        .header("x-api-key", target.key().unwrap_or(""))
        .header("anthropic-version", VERSION)
}

/// One SSE `data` payload. Thinking blocks (on by default on current models) arrive as
/// `thinking_delta` and are skipped; only `text_delta` is answer text.
pub(super) fn decode(data: &str, out: &mut Vec<Event>) {
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return;
    };
    match v["type"].as_str().unwrap_or("") {
        "message_start" => {
            let message = &v["message"];
            if let Some(model) = message["model"].as_str() {
                out.push(Event::Model(model.to_owned()));
            }
            if let Some(n) = input_tokens(&message["usage"]) {
                out.push(Event::InputTokens(n));
            }
        }
        "content_block_delta" => {
            let delta = &v["delta"];
            if delta["type"] == "text_delta" {
                if let Some(text) = delta["text"].as_str().filter(|t| !t.is_empty()) {
                    out.push(Event::Text(text.to_owned()));
                }
            }
        }
        "message_delta" => {
            if let Some(n) = v["usage"]["output_tokens"].as_u64() {
                out.push(Event::OutputTokens(n));
            }
            match v["delta"]["stop_reason"].as_str() {
                Some("refusal") => out.push(Event::Failed(AiError::bad_request(
                    "the model declined to answer",
                ))),
                Some(_) => out.push(Event::Finished),
                None => {}
            }
        }
        "message_stop" => out.push(Event::Done),
        "error" => out.push(Event::Failed(classify(None, None, data))),
        _ => {}
    }
}

/// Uncached, cache-write and cache-read input tokens are reported separately; all count.
fn input_tokens(usage: &Value) -> Option<u64> {
    let base = usage["input_tokens"].as_u64()?;
    let cached = ["cache_creation_input_tokens", "cache_read_input_tokens"]
        .iter()
        .filter_map(|k| usage[*k].as_u64())
        .sum::<u64>();
    Some(base + cached)
}

pub(super) fn models(
    client: &Client,
    target: &Target,
    after: Option<&str>,
) -> Result<(Url, RequestBuilder)> {
    let mut query = vec![("limit", MODELS_PAGE)];
    if let Some(after) = after {
        query.push(("after_id", after));
    }
    let url = url(&target.base, "/v1/models", &query)?;
    Ok((url.clone(), headers(client.get(url), target)))
}

/// `{data: [{id, display_name, max_input_tokens?}], has_more, last_id}`.
pub(super) fn parse_models(body: &Value) -> (Vec<ModelInfo>, Option<String>) {
    let models = body["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|m| {
                    let id = m["id"].as_str()?.to_owned();
                    Some(ModelInfo {
                        name: m["display_name"].as_str().unwrap_or(&id).to_owned(),
                        context_tokens: m["max_input_tokens"]
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok()),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let next = match body["has_more"].as_bool() {
        Some(true) => body["last_id"].as_str().map(str::to_owned),
        _ => None,
    };
    (models, next)
}
