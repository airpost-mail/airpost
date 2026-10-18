// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Keys, addresses, mail text and answers stay out of logs, `Debug` output and error text
//! (docs/ai-plan.md A8).
//!
//! Only `logs_carry_no_content` may make requests in this binary: tracing caches a
//! callsite's interest when another thread hits it first without the capturing subscriber.

mod common;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use common::*;
use ms_ai::{Ai, Category, DraftOpts, ProviderConfig, ProviderKind, SummaryOpts, Triage};
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

const ANSWER: &str = "Secret answer about the merger";
const INSTRUCTION: &str = "mention the acquisition price";
const PROXY_PASSWORD: &str = "hunter2";

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<String>>);

struct Fields<'a>(&'a mut String);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
}

impl<S: tracing::Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if !event.metadata().target().starts_with("ms_ai") {
            return;
        }
        let mut line = String::new();
        event.record(&mut Fields(&mut line));
        line.push('\n');
        self.0.lock().unwrap().push_str(&line);
    }
}

fn assert_clean(what: &str, text: &str) {
    for secret in [
        KEY,
        OWNER_EMAIL,
        OTHER_EMAIL,
        BODY,
        ANSWER,
        INSTRUCTION,
        PROXY_PASSWORD,
        "Offsite",
    ] {
        assert!(!text.contains(secret), "{what} leaks {secret:?}: {text}");
    }
}

#[test]
fn logs_carry_no_content() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    tracing::subscriber::with_default(subscriber, || {
        let server = MockServer::start(|req| {
            if req.body.contains("fail") {
                Reply::json(400, json!({"error": {"message": "bad request"}}))
            } else {
                stream_reply(ProviderKind::Anthropic, &[ANSWER])
            }
        });
        let ai = Ai::new().unwrap();
        let config = config(ProviderKind::Anthropic, &server);
        let opts = DraftOpts {
            instruction: Some(INSTRUCTION.into()),
            suggestion: None,
        };
        let draft = ai
            .draft_reply(&config, &thread(), &opts, &mut collect(&mut Vec::new()))
            .unwrap();
        assert_eq!(draft.text, ANSWER);
        ai.summarize(
            &config,
            &thread(),
            &SummaryOpts::default(),
            &mut collect(&mut Vec::new()),
        )
        .unwrap();

        let failing = DraftOpts {
            instruction: Some("fail".into()),
            suggestion: None,
        };
        let e = ai
            .draft_reply(&config, &thread(), &failing, &mut collect(&mut Vec::new()))
            .unwrap_err();
        assert_clean("error text", &e.to_string());
        assert_clean("error JSON", &serde_json::to_string(&e).unwrap());
        assert_clean("error Debug", &format!("{e:?}"));
    });
    let logs = capture.0.lock().unwrap().clone();
    assert_clean("logs", &logs);
    assert!(
        logs.contains("- ai: anthropic 127.0.0.1 m-1 reply HTTP 200"),
        "{logs}"
    );
    assert!(logs.contains("100 in / 10 out tokens"), "{logs}");
    assert!(
        logs.contains("-X ai: anthropic 127.0.0.1 m-1 reply failed"),
        "{logs}"
    );
}

#[test]
fn debug_output_redacts() {
    let mut config: ProviderConfig = config(
        ProviderKind::OpenAi,
        &MockServer::start(|_| Reply::text(200, "")),
    );
    config.url = Some(format!("https://alice:{PROXY_PASSWORD}@llm.example.org/v1"));
    let context = thread();
    let opts = DraftOpts {
        instruction: Some(INSTRUCTION.into()),
        suggestion: Some(ANSWER.into()),
    };
    let ai = Ai::new().unwrap();
    let rendered = context.render(4000);
    assert!(rendered.text.contains(BODY));
    let triage = Triage {
        category: Category::Important,
        needs_reply: false,
        reason: Some(ANSWER.into()),
    };
    for (what, text) in [
        ("rendered", format!("{rendered:?}")),
        ("triage", format!("{triage:?}")),
        ("config", format!("{config:?}")),
        ("context", format!("{context:?}")),
        ("messages", format!("{:?}", context.messages)),
        ("opts", format!("{opts:?}")),
        ("ai", format!("{ai:?}")),
    ] {
        assert_clean(what, &text);
    }
}
