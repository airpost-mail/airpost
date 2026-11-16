// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Keys, usernames, message text and email addresses stay out of `Debug` output, error
//! text and logs.

mod common;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use airpost_integrations::{
    DeepLConfig, GrammarRequest, Integrations, LanguageToolConfig, LibreTranslateConfig,
    PeopleDataLabsConfig, ProviderConfig, Secret, TextFormat, TranslateRequest,
    TranslationProvider,
};
use common::{MockServer, Reply};
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

const KEY: &str = "sk-very-secret-123";
const USER: &str = "owner@example.com";
const EMAIL: &str = "ann@acme.io";
const TEXT: &str = "Confidential merger plans";
const PROXY_USER: &str = "alice";
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
        if !event
            .metadata()
            .target()
            .starts_with("airpost_integrations")
        {
            return;
        }
        let mut line = String::new();
        event.record(&mut Fields(&mut line));
        line.push('\n');
        self.0.lock().unwrap().push_str(&line);
    }
}

fn assert_clean(what: &str, text: &str) {
    for secret in [KEY, USER, EMAIL, TEXT, PROXY_USER, PROXY_PASSWORD] {
        assert!(!text.contains(secret), "{what} leaks {secret:?}: {text}");
    }
}

#[test]
fn debug_output_redacts() {
    let configs = [
        ProviderConfig::LibreTranslate(LibreTranslateConfig {
            url: "https://lt.example.org".into(),
            api_key: Some(KEY.into()),
        }),
        ProviderConfig::DeepL(DeepLConfig {
            api_key: KEY.into(),
            url: None,
        }),
        ProviderConfig::LibreTranslate(LibreTranslateConfig {
            url: format!("https://{PROXY_USER}:{PROXY_PASSWORD}@lt.example.org"),
            api_key: None,
        }),
        ProviderConfig::DeepL(DeepLConfig {
            api_key: KEY.into(),
            url: Some(format!(
                "https://{PROXY_USER}:{PROXY_PASSWORD}@deepl.example.org"
            )),
        }),
        ProviderConfig::LanguageTool(LanguageToolConfig {
            url: Some(format!("{PROXY_USER}:{PROXY_PASSWORD}@lt.example.org")),
            username: None,
            api_key: None,
        }),
        ProviderConfig::PeopleDataLabs(PeopleDataLabsConfig {
            api_key: KEY.into(),
            url: Some(format!("http://{PROXY_PASSWORD}@pdl.example.org")),
        }),
        ProviderConfig::LanguageTool(LanguageToolConfig {
            url: Some("https://api.languagetool.org".into()),
            username: Some(USER.into()),
            api_key: Some(KEY.into()),
        }),
        ProviderConfig::PeopleDataLabs(PeopleDataLabsConfig {
            api_key: KEY.into(),
            url: None,
        }),
    ];
    for c in &configs {
        assert_clean("config Debug", &format!("{c:?} {c:#?}"));
    }
    let from_json: ProviderConfig =
        serde_json::from_value(json!({"provider": "deepl", "apiKey": KEY})).unwrap();
    assert_clean("deserialized config Debug", &format!("{from_json:?}"));
    assert_eq!(format!("{:?}", Secret::from(KEY)), "Secret(***)");

    let t = TranslateRequest {
        text: TEXT.into(),
        source: None,
        target: "es".into(),
        format: TextFormat::Text,
    };
    assert_clean("TranslateRequest Debug", &format!("{t:?}"));
    let g = GrammarRequest {
        text: TEXT.into(),
        ..GrammarRequest::default()
    };
    assert_clean("GrammarRequest Debug", &format!("{g:?}"));
}

#[test]
fn logs_and_errors_carry_no_secrets() {
    let server = MockServer::start(|req| match req.path() {
        "/translate" => Reply::json(500, json!({"error": "upstream failed"})),
        "/v2/check" => Reply::text(403, "AuthException: invalid username or API key"),
        "/v5/person/enrich" => {
            Reply::json(200, json!({"status": 200, "data": {"full_name": "Ann"}}))
        }
        "/v5/company/enrich" => Reply::json(401, json!({"error": {"message": "Invalid API key"}})),
        _ => Reply::json(404, json!({})),
    });
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let ix = Integrations::new().unwrap();
    let mut errors = String::new();

    tracing::subscriber::with_default(subscriber, || {
        let libre = TranslationProvider::LibreTranslate(LibreTranslateConfig {
            url: server.url(),
            api_key: Some(KEY.into()),
        });
        let req = TranslateRequest {
            text: TEXT.into(),
            source: None,
            target: "es".into(),
            format: TextFormat::Text,
        };
        let e = ix.translate(&libre, &req).unwrap_err();
        errors.push_str(&format!(
            "{e} {e:?} {}\n",
            serde_json::to_string(&e).unwrap()
        ));

        let lt = LanguageToolConfig {
            url: Some(server.url()),
            username: Some(USER.into()),
            api_key: Some(KEY.into()),
        };
        let g = GrammarRequest {
            text: TEXT.into(),
            ..GrammarRequest::default()
        };
        let e = ix.check_grammar(&lt, &g).unwrap_err();
        errors.push_str(&format!("{e} {e:?}\n"));

        let pdl = PeopleDataLabsConfig {
            api_key: KEY.into(),
            url: Some(server.url()),
        };
        let e = ix.enrich_contact(&pdl, EMAIL).unwrap_err();
        errors.push_str(&format!("{e} {e:?}\n"));
    });

    let logs = capture.0.lock().unwrap().clone();
    assert_clean("logs", &logs);
    assert_clean("errors", &errors);
    let host = server.url().trim_start_matches("http://").to_owned();
    let host = host.split(':').next().unwrap();
    assert!(
        logs.contains(&format!("libretranslate {host} HTTP 500")),
        "{logs}"
    );
    assert!(
        logs.contains(&format!("peopledatalabs {host} HTTP 200")),
        "{logs}"
    );
    assert!(
        logs.contains(&format!("peopledatalabs {host} HTTP 401")),
        "{logs}"
    );
    assert_eq!(logs.lines().count(), 4, "{logs}");
    assert_clean("Integrations Debug", &format!("{ix:?}"));
}
