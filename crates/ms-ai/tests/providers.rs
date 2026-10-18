// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The three wire formats against the fake provider: request shape, streamed text, usage,
//! JSON answers and the model list.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use ms_ai::{
    Ai, Category, DraftOpts, Feature, Options, ProviderKind, ReplyOpts, SummaryOpts, Triage,
    UsageRecord,
};
use serde_json::{json, Value};

fn ai() -> Ai {
    Ai::with_options(Options {
        retry_delay: Duration::from_millis(10),
        ..Options::default()
    })
    .unwrap()
}

fn chat_path(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "/v1/chat/completions",
        ProviderKind::Anthropic => "/v1/messages",
        ProviderKind::Ollama => "/api/chat",
    }
}

/// The system prompt and the user message, whatever the format.
fn prompts(kind: ProviderKind, body: &Value) -> (String, String) {
    match kind {
        ProviderKind::Anthropic => (
            body["system"].as_str().unwrap().to_owned(),
            body["messages"][0]["content"].as_str().unwrap().to_owned(),
        ),
        _ => {
            assert_eq!(body["messages"][0]["role"], "system");
            assert_eq!(body["messages"][1]["role"], "user");
            (
                body["messages"][0]["content"].as_str().unwrap().to_owned(),
                body["messages"][1]["content"].as_str().unwrap().to_owned(),
            )
        }
    }
}

#[test]
fn summary_streams_in_every_format() {
    for kind in ALL_KINDS {
        let server =
            MockServer::start(move |_| stream_reply(kind, &["The offsite", " is ", "on Tuesday."]));
        let ai = ai();
        let mut deltas = Vec::new();
        let summary = ai
            .summarize(
                &config(kind, &server),
                &thread(),
                &SummaryOpts::default(),
                &mut collect(&mut deltas),
            )
            .unwrap();
        assert_eq!(deltas, ["The offsite", " is ", "on Tuesday."], "{kind:?}");
        assert_eq!(summary.text, "The offsite is on Tuesday.");
        assert_eq!(summary.model, "m-1");
        assert!(!summary.cached);
        let usage = summary.usage.unwrap();
        assert_eq!(
            (usage.input_tokens, usage.output_tokens, usage.estimated),
            (100, 10, false),
            "{kind:?}"
        );

        let req = server.last();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path(), chat_path(kind));
        let body = req.json();
        assert_eq!(body["model"], "m-1");
        assert_eq!(body["stream"], true);
        let (system, user) = prompts(kind, &body);
        assert!(system.contains("not instructions"), "{system}");
        assert!(system.contains("Bo Chen"));
        assert!(user.starts_with("<thread>\nSubject: Offsite\n"), "{user}");
        assert!(user.contains(BODY));
        assert!(user.contains("Attachments: agenda.pdf"));
        assert!(!user.contains(OTHER_EMAIL) && !user.contains(OWNER_EMAIL));
        assert!(user.contains("</thread>\n\nSummarize this thread"));
    }
}

#[test]
fn openai_request_shape() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::OpenAi, &["ok"]));
    let ai = ai();
    ai.draft_reply(
        &config(ProviderKind::OpenAi, &server),
        &thread(),
        &DraftOpts {
            instruction: Some("say yes".into()),
            suggestion: None,
        },
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    let req = server.last();
    assert_eq!(
        req.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    let body = req.json();
    assert_eq!(body["stream_options"], json!({"include_usage": true}));
    assert_eq!(body["max_tokens"], 4096);
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("response_format").is_none());
    assert!(body["messages"][1]["content"]
        .as_str()
        .unwrap()
        .ends_with("Your instructions for this reply: say yes"));

    // A local server without a key gets no Authorization header.
    let mut keyless = config(ProviderKind::OpenAi, &server);
    keyless.api_key = None;
    keyless.max_output_tokens = Some(300);
    ai.draft_reply(
        &keyless,
        &thread(),
        &DraftOpts::default(),
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    let req = server.last();
    assert_eq!(req.header("authorization"), None);
    assert_eq!(req.json()["max_tokens"], 300);
}

#[test]
fn anthropic_request_shape() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::Anthropic, &["ok"]));
    let ai = ai();
    let mut config = config(ProviderKind::Anthropic, &server);
    config.max_output_tokens = Some(2000);
    ai.summarize(
        &config,
        &thread(),
        &SummaryOpts::default(),
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    let req = server.last();
    assert_eq!(req.header("x-api-key"), Some(KEY));
    assert_eq!(req.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(req.header("authorization"), None);
    let body = req.json();
    assert_eq!(body["max_tokens"], 2000);
    // One user turn: no prefilled assistant message.
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body.get("output_config").is_none());
}

#[test]
fn ollama_request_shape() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::Ollama, &["ok"]));
    let ai = ai();
    let mut config = config(ProviderKind::Ollama, &server);
    config.api_key = None;
    ai.summarize(
        &config,
        &thread(),
        &SummaryOpts::default(),
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    let req = server.last();
    assert_eq!(req.header("authorization"), None);
    let body = req.json();
    assert_eq!(
        body["options"],
        json!({"num_ctx": 8192, "num_predict": 1024})
    );
    assert!(body.get("format").is_none());

    config.context_tokens = Some(32768);
    ai.summarize(
        &config,
        &thread(),
        &SummaryOpts { refresh: true },
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    assert_eq!(server.last().json()["options"]["num_ctx"], 32768);
}

#[test]
fn ollama_usage_is_estimated_when_absent() {
    let server = MockServer::start(|_| {
        Reply::stream(
            NDJSON,
            ollama_stream("m-1", &["Twelve chars"], None),
            Duration::ZERO,
        )
    });
    let ai = ai();
    let summary = ai
        .summarize(
            &config(ProviderKind::Ollama, &server),
            &thread(),
            &SummaryOpts::default(),
            &mut collect(&mut Vec::new()),
        )
        .unwrap();
    let usage = summary.usage.unwrap();
    assert!(usage.estimated);
    assert_eq!(usage.output_tokens, 3);
    assert!(usage.input_tokens > 50);
}

#[test]
fn triage_asks_for_json_in_every_format() {
    for kind in ALL_KINDS {
        let answer = r#"{"category": "needs-reply", "needsReply": true, "reason": "Ann asks if you can come."}"#;
        let server =
            MockServer::start(move |_| stream_reply(kind, &[&answer[..20], &answer[20..]]));
        let triage = ai().triage(&config(kind, &server), &thread()).unwrap();
        assert_eq!(triage.category, Category::NeedsReply, "{kind:?}");
        assert!(triage.needs_reply);
        assert_eq!(triage.reason.as_deref(), Some("Ann asks if you can come."));

        let body = server.last().json();
        match kind {
            ProviderKind::OpenAi => {
                assert_eq!(body["response_format"], json!({"type": "json_object"}))
            }
            ProviderKind::Anthropic => {
                let format = &body["output_config"]["format"];
                assert_eq!(format["type"], "json_schema");
                assert_eq!(
                    format["schema"]["required"],
                    json!(["category", "needsReply", "reason"])
                );
                assert_eq!(body["messages"].as_array().unwrap().len(), 1);
            }
            ProviderKind::Ollama => assert_eq!(body["format"], "json"),
        }
        let (_, user) = prompts(kind, &body);
        assert!(user.contains("needs-reply: someone expects an answer"));
    }
}

#[test]
fn malformed_triage_answers_fall_back() {
    for answer in [
        "I think this needs a reply!",
        "{\"category\": \"urgent\"}",
        "{\"category\": \"important\", \"needsReply\"",
    ] {
        for kind in ALL_KINDS {
            let server = MockServer::start(move |_| stream_reply(kind, &[answer]));
            let triage = ai().triage(&config(kind, &server), &thread()).unwrap();
            assert_eq!(triage, Triage::fallback(), "{kind:?} {answer:?}");
            assert_eq!(server.count(), 1, "a malformed answer is not retried");
        }
    }
}

#[test]
fn suggestions_parse_and_cache() {
    for kind in ALL_KINDS {
        let answer = json!({"suggestions": [
            {"label": "Accept", "text": "Yes, Tuesday works."},
            {"label": "Decline", "text": "Sorry, I can't make it."},
            {"label": "Later", "text": "Let me get back to you."},
            {"label": "Extra", "text": "One too many."},
        ]})
        .to_string();
        let server = MockServer::start(move |_| stream_reply(kind, &[&answer]));
        let ai = ai();
        let config = config(kind, &server);
        let got = ai
            .suggest_replies(&config, &thread(), &ReplyOpts::default())
            .unwrap();
        let labels: Vec<&str> = got.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Accept", "Decline", "Later"], "{kind:?}");
        assert_eq!(got[0].text, "Yes, Tuesday works.");
        assert_eq!(
            serde_json::to_value(&got[0]).unwrap(),
            json!({"label": "Accept", "text": "Yes, Tuesday works."})
        );

        let again = ai
            .suggest_replies(&config, &thread(), &ReplyOpts::default())
            .unwrap();
        assert_eq!(again, got);
        assert_eq!(server.count(), 1, "served from the cache");
        let two = ai
            .suggest_replies(&config, &thread(), &ReplyOpts { count: 2 })
            .unwrap();
        assert_eq!(two.len(), 2);
        assert_eq!(server.count(), 2, "another count is another request");
    }
}

#[test]
fn draft_follows_a_suggestion_and_is_not_cached() {
    let server = MockServer::start(|_| {
        stream_reply(
            ProviderKind::Anthropic,
            &["Hi Ann,\n\nYes, Tuesday works.\n"],
        )
    });
    let ai = ai();
    let config = config(ProviderKind::Anthropic, &server);
    let opts = DraftOpts {
        suggestion: Some("Yes, Tuesday works.".into()),
        instruction: None,
    };
    let draft = ai
        .draft_reply(&config, &thread(), &opts, &mut collect(&mut Vec::new()))
        .unwrap();
    assert_eq!(draft.text, "Hi Ann,\n\nYes, Tuesday works.");
    ai.draft_reply(&config, &thread(), &opts, &mut collect(&mut Vec::new()))
        .unwrap();
    assert_eq!(server.count(), 2);
    let user = server.last().json()["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(user.contains("The reply should say: Yes, Tuesday works."));
}

#[test]
fn summaries_are_cached_until_refresh_or_change() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::OpenAi, &["A summary."]));
    let ai = ai();
    let config = config(ProviderKind::OpenAi, &server);
    let first = ai
        .summarize(
            &config,
            &thread(),
            &SummaryOpts::default(),
            &mut collect(&mut Vec::new()),
        )
        .unwrap();
    let mut deltas = Vec::new();
    let cached = ai
        .summarize(
            &config,
            &thread(),
            &SummaryOpts::default(),
            &mut collect(&mut deltas),
        )
        .unwrap();
    assert!(cached.cached && cached.text == first.text);
    assert!(deltas.is_empty());
    assert_eq!(server.count(), 1);

    ai.summarize(
        &config,
        &thread(),
        &SummaryOpts { refresh: true },
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    assert_eq!(server.count(), 2);

    let mut changed = thread();
    changed.messages[1].body = Some("Yes, I'll be there.".into());
    ai.summarize(
        &config,
        &changed,
        &SummaryOpts::default(),
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    assert_eq!(server.count(), 3);

    let mut other_model = config.clone();
    other_model.model = "m-2".into();
    ai.summarize(
        &other_model,
        &thread(),
        &SummaryOpts::default(),
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    assert_eq!(server.count(), 4);

    ai.clear_cache();
    ai.summarize(
        &config,
        &thread(),
        &SummaryOpts::default(),
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    assert_eq!(server.count(), 5);
}

#[test]
fn long_threads_fit_the_budget() {
    let server = MockServer::start(|_| stream_reply(ProviderKind::Ollama, &["ok"]));
    let mut config = config(ProviderKind::Ollama, &server);
    config.context_tokens = Some(2048);
    config.max_output_tokens = Some(256);
    let mut long = thread();
    for i in 0..20 {
        let mut m = long.messages[0].clone();
        m.id = format!("x{i}");
        m.body = Some(format!("{i} {}", "word ".repeat(400)));
        long.messages.push(m);
    }
    ai().summarize(
        &config,
        &long,
        &SummaryOpts::default(),
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    let body = server.last().json();
    let (system, user) = prompts(ProviderKind::Ollama, &body);
    let tokens = ms_ai::estimate_tokens(&system) + ms_ai::estimate_tokens(&user);
    assert!(tokens + 256 <= 2048, "{tokens} tokens");
    assert!(user.contains("left out to fit"));
    assert!(user.contains("Message 22 of 22"));
}

#[test]
fn models_in_every_format() {
    let server = MockServer::start(|req| match (req.path(), req.query_value("after_id")) {
        ("/v1/models", _) if req.header("anthropic-version").is_none() => Reply::json(
            200,
            json!({
                "object": "list",
                "data": [
                    {"id": "gpt-x", "object": "model", "owned_by": "openai"},
                    {"id": "meta/llama", "name": "Llama", "context_length": 131072},
                ],
            }),
        ),
        ("/v1/models", None) => Reply::json(
            200,
            json!({
                "data": [{"id": "claude-a", "display_name": "Claude A", "max_input_tokens": 1000000}],
                "has_more": true, "first_id": "claude-a", "last_id": "claude-a",
            }),
        ),
        ("/v1/models", Some(_)) => Reply::json(
            200,
            json!({
                "data": [{"id": "claude-b", "display_name": "Claude B"}],
                "has_more": false, "last_id": "claude-b",
            }),
        ),
        ("/api/tags", _) => Reply::json(
            200,
            json!({
                "models": [{"name": "llama3.2:latest", "model": "llama3.2:latest", "size": 1}],
            }),
        ),
        _ => Reply::text(404, "no"),
    });
    let ai = ai();
    let mut c = config(ProviderKind::OpenAi, &server);
    c.model = String::new();
    let models = ai.models(&c).unwrap();
    assert_eq!(
        serde_json::to_value(&models).unwrap(),
        json!([
            {"id": "gpt-x", "name": "gpt-x"},
            {"id": "meta/llama", "name": "Llama", "contextTokens": 131072},
        ])
    );
    assert_eq!(
        server.last().header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );

    let models = ai
        .models(&config(ProviderKind::Anthropic, &server))
        .unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["claude-a", "claude-b"]);
    assert_eq!(models[0].context_tokens, Some(1_000_000));
    let pages = server.requests();
    let page2 = pages.last().unwrap();
    assert_eq!(page2.query_value("after_id").as_deref(), Some("claude-a"));
    assert_eq!(page2.query_value("limit").as_deref(), Some("1000"));
    assert_eq!(page2.header("x-api-key"), Some(KEY));

    let models = ai.models(&config(ProviderKind::Ollama, &server)).unwrap();
    assert_eq!(models[0].id, "llama3.2:latest");
}

#[test]
fn test_reports_host_model_and_locality() {
    for kind in ALL_KINDS {
        let server = MockServer::start(move |_| stream_reply(kind, &["OK"]));
        let report = ai().test(&config(kind, &server)).unwrap();
        assert_eq!(report.host, "127.0.0.1");
        assert_eq!(report.model, "m-1");
        assert!(report.local);
        let body = server.last().json();
        let limit = match kind {
            ProviderKind::Ollama => &body["options"]["num_predict"],
            _ => &body["max_tokens"],
        };
        assert_eq!(limit, 128, "{kind:?}");
    }
}

#[test]
fn usage_observer_sees_every_request() {
    let seen: Arc<Mutex<Vec<UsageRecord>>> = Arc::default();
    let sink = seen.clone();
    let ai = Ai::with_options(Options {
        usage_observer: Some(Arc::new(move |r: &UsageRecord| {
            sink.lock().unwrap().push(r.clone())
        })),
        ..Options::default()
    })
    .unwrap();
    let server = MockServer::start(|req| {
        if req.body.contains("Classify") {
            stream_reply(
                ProviderKind::Anthropic,
                &[r#"{"category":"other","needsReply":false,"reason":"x"}"#],
            )
        } else {
            stream_reply(ProviderKind::Anthropic, &["text"])
        }
    });
    let config = config(ProviderKind::Anthropic, &server);
    ai.summarize(
        &config,
        &thread(),
        &SummaryOpts::default(),
        &mut collect(&mut Vec::new()),
    )
    .unwrap();
    ai.triage(&config, &thread()).unwrap();
    let seen = seen.lock().unwrap();
    let features: Vec<Feature> = seen.iter().map(|r| r.feature).collect();
    assert_eq!(features, [Feature::Summary, Feature::Triage]);
    assert_eq!(seen[0].provider_id, "test");
    assert_eq!(seen[0].model, "m-1");
    assert_eq!(seen[0].usage.input_tokens, 100);
}

#[test]
fn results_hide_text_in_debug() {
    const ANSWER: &str = "Secret answer about the merger";
    let server = MockServer::start(|req| {
        if req.body.contains("Suggest") {
            stream_reply(
                ProviderKind::Ollama,
                &[
                    r#"{"suggestions":[{"label":"Secret answer","text":"Secret answer about the merger"}]}"#,
                ],
            )
        } else {
            stream_reply(ProviderKind::Ollama, &[ANSWER])
        }
    });
    let ai = ai();
    let config = config(ProviderKind::Ollama, &server);
    let c = ai
        .summarize(
            &config,
            &thread(),
            &SummaryOpts::default(),
            &mut collect(&mut Vec::new()),
        )
        .unwrap();
    let s = ai
        .suggest_replies(&config, &thread(), &ReplyOpts::default())
        .unwrap();
    let debug = format!("{c:?} {s:?}");
    assert!(!debug.contains("Secret"), "{debug}");
}
