// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Ollama's native API (`POST /api/chat`, `GET /api/tags`; github.com/ollama/ollama,
//! docs/api.md). Used instead of its OpenAI endpoint, which ignores `num_ctx` and
//! truncates long prompts to the model's default window without an error.

use reqwest::header::AUTHORIZATION;
use reqwest::{Client, RequestBuilder, Url};
use serde_json::{json, Value};

use super::{classify, url, ChatRequest, Event};
use crate::config::Target;
use crate::error::Result;
use crate::ModelInfo;

pub(super) fn chat(
    client: &Client,
    target: &Target,
    req: &ChatRequest,
) -> Result<(Url, RequestBuilder)> {
    let url = url(&target.base, "/api/chat", &[])?;
    let mut body = json!({
        "model": target.model,
        "messages": [
            {"role": "system", "content": req.system},
            {"role": "user", "content": req.user},
        ],
        "stream": true,
        "options": {
            "num_ctx": target.context_tokens,
            "num_predict": req.max_output_tokens,
        },
    });
    if req.json_schema.is_some() {
        // `"json"` rather than a schema object: versions before 0.5 accept only that.
        body["format"] = json!("json");
    }
    Ok((url.clone(), auth(client.post(url), target).json(&body)))
}

/// A key is optional: only an Ollama behind an authenticating proxy needs one.
fn auth(builder: RequestBuilder, target: &Target) -> RequestBuilder {
    match target.key() {
        Some(key) => builder.header(AUTHORIZATION, format!("Bearer {key}")),
        None => builder,
    }
}

/// One NDJSON line: `{model, message: {content}, done: false}` per chunk, then
/// `{done: true, prompt_eval_count, eval_count}`, or `{error}`. `message.thinking` (reasoning
/// models) counts only as [`Event::Progress`].
pub(super) fn decode(line: &str, out: &mut Vec<Event>) {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if v.get("error").is_some_and(|e| !e.is_null()) {
        out.push(Event::Failed(classify(None, None, line)));
        return;
    }
    if let Some(model) = v["model"].as_str().filter(|m| !m.is_empty()) {
        out.push(Event::Model(model.to_owned()));
    }
    if let Some(text) = v["message"]["content"].as_str().filter(|t| !t.is_empty()) {
        out.push(Event::Text(text.to_owned()));
    }
    if v["message"]["thinking"]
        .as_str()
        .is_some_and(|t| !t.is_empty())
    {
        out.push(Event::Progress);
    }
    if v["done"].as_bool() == Some(true) {
        if let Some(n) = v["prompt_eval_count"].as_u64() {
            out.push(Event::InputTokens(n));
        }
        if let Some(n) = v["eval_count"].as_u64() {
            out.push(Event::OutputTokens(n));
        }
        out.push(Event::Finished);
        out.push(Event::Done);
    }
}

pub(super) fn models(client: &Client, target: &Target) -> Result<(Url, RequestBuilder)> {
    let url = url(&target.base, "/api/tags", &[])?;
    Ok((url.clone(), auth(client.get(url), target)))
}

/// `{models: [{name, model, details}]}`; the window needs `/api/show` per model, so it is
/// left unknown.
pub(super) fn parse_models(body: &Value) -> Vec<ModelInfo> {
    let Some(items) = body["models"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|m| {
            let id = m["model"].as_str().or(m["name"].as_str())?.to_owned();
            Some(ModelInfo {
                name: m["name"].as_str().unwrap_or(&id).to_owned(),
                id,
                context_tokens: None,
            })
        })
        .collect()
}
