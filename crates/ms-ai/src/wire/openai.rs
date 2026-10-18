// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! OpenAI-compatible chat completions (`POST {base}/chat/completions`, `GET {base}/models`;
//! platform.openai.com/docs/api-reference/chat). The base URL carries the version
//! (`https://api.openai.com/v1`, `https://openrouter.ai/api/v1`).

use reqwest::header::AUTHORIZATION;
use reqwest::{Client, RequestBuilder, Url};
use serde_json::{json, Value};

use super::{classify, url, ChatRequest, Event};
use crate::config::{Target, OPENAI_URL};
use crate::error::Result;
use crate::ModelInfo;

pub(super) fn chat(
    client: &Client,
    target: &Target,
    req: &ChatRequest,
) -> Result<(Url, RequestBuilder)> {
    let url = url(&target.base, "/chat/completions", &[])?;
    let mut body = json!({
        "model": target.model,
        "messages": [
            {"role": "system", "content": req.system},
            {"role": "user", "content": req.user},
        ],
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    // OpenAI's reasoning models reject `max_tokens` and require `max_completion_tokens`;
    // most compatible servers know only `max_tokens`.
    let limit_field = if target.base == OPENAI_URL {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    body[limit_field] = json!(req.max_output_tokens);
    if req.json_schema.is_some() {
        // JSON mode rather than `json_schema`: compatible servers support it far more
        // widely, and the answer is validated here anyway.
        body["response_format"] = json!({"type": "json_object"});
    }
    let mut builder = client.post(url.clone()).json(&body);
    if let Some(key) = target.key() {
        builder = builder.header(AUTHORIZATION, format!("Bearer {key}"));
    }
    Ok((url, builder))
}

/// One SSE `data` payload: a `chat.completion.chunk`, `[DONE]`, or an error object some
/// servers send mid-stream.
pub(super) fn decode(data: &str, out: &mut Vec<Event>) {
    if data.trim() == "[DONE]" {
        out.push(Event::Done);
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return;
    };
    if v.get("error").is_some_and(|e| !e.is_null()) {
        out.push(Event::Failed(classify(None, None, data)));
        return;
    }
    if let Some(model) = v["model"].as_str().filter(|m| !m.is_empty()) {
        out.push(Event::Model(model.to_owned()));
    }
    if let Some(choice) = v["choices"].get(0) {
        let delta = &choice["delta"];
        if let Some(text) = delta["content"].as_str() {
            if !text.is_empty() {
                out.push(Event::Text(text.to_owned()));
            }
        }
        // Reasoning models on compatible servers stream their reasoning first:
        // `reasoning_content` (DeepSeek, vLLM, llama.cpp) or `reasoning` (OpenRouter).
        let reasoning = ["reasoning_content", "reasoning"]
            .iter()
            .any(|k| delta[*k].as_str().is_some_and(|t| !t.is_empty()));
        if reasoning {
            out.push(Event::Progress);
        }
        if choice["finish_reason"].is_string() {
            out.push(Event::Finished);
        }
    }
    let usage = &v["usage"];
    if let Some(n) = usage["prompt_tokens"].as_u64() {
        out.push(Event::InputTokens(n));
    }
    if let Some(n) = usage["completion_tokens"].as_u64() {
        out.push(Event::OutputTokens(n));
    }
}

pub(super) fn models(client: &Client, target: &Target) -> Result<(Url, RequestBuilder)> {
    let url = url(&target.base, "/models", &[])?;
    let mut builder = client.get(url.clone());
    if let Some(key) = target.key() {
        builder = builder.header(AUTHORIZATION, format!("Bearer {key}"));
    }
    Ok((url, builder))
}

/// `{data: [{id, …}]}`. The window is `context_length` on OpenRouter, `max_model_len` on
/// vLLM and `context_window` on Groq; OpenAI doesn't say.
pub(super) fn parse_models(body: &Value) -> Vec<ModelInfo> {
    let Some(items) = body["data"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_owned();
            let name = m["name"].as_str().unwrap_or(&id).to_owned();
            let context_tokens = ["context_length", "max_model_len", "context_window"]
                .iter()
                .find_map(|k| m[*k].as_u64())
                .and_then(|n| u32::try_from(n).ok());
            Some(ModelInfo {
                id,
                name,
                context_tokens,
            })
        })
        .collect()
}
