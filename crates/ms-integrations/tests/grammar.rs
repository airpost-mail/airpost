// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! LanguageTool against a mock server.

mod common;

use common::{MockServer, Reply};
use ms_integrations::{
    GrammarRequest, Integrations, LanguageToolConfig, ProviderConfig, Secret,
    LANGUAGETOOL_PREMIUM_URL, LANGUAGETOOL_URL,
};
use serde_json::{json, Value};

fn config(url: &str, creds: Option<(&str, &str)>) -> LanguageToolConfig {
    LanguageToolConfig {
        url: Some(url.into()),
        username: creds.map(|(u, _)| Secret::from(u)),
        api_key: creds.map(|(_, k)| Secret::from(k)),
    }
}

fn check_response() -> Value {
    json!({
        "software": {"name": "LanguageTool", "version": "6.6", "apiVersion": 1, "premium": false},
        "language": {
            "name": "English (US)", "code": "en-US",
            "detectedLanguage": {"name": "English (US)", "code": "en-US", "confidence": 0.99}
        },
        "matches": [
            {
                "message": "Possible agreement error.",
                "shortMessage": "Grammar",
                "offset": 5, "length": 3,
                "replacements": [{"value": "are"}, {"value": "were"}],
                "context": {"text": "They is here.", "offset": 5, "length": 3},
                "sentence": "They is here.",
                "rule": {
                    "id": "AGREEMENT", "subId": "1", "description": "Subject-verb agreement",
                    "issueType": "grammar",
                    "category": {"id": "GRAMMAR", "name": "Grammar"}
                }
            },
            {
                "message": "Possible spelling mistake found.",
                "offset": 0, "length": 4,
                "replacements": [],
                "context": {"text": "Thye", "offset": 0, "length": 4},
                "sentence": "Thye",
                "rule": {
                    "id": "MORFOLOGIK_RULE_EN_US", "description": "Possible spelling mistake",
                    "issueType": "misspelling",
                    "category": {"id": "TYPOS", "name": "Possible Typo"}
                }
            }
        ]
    })
}

#[test]
fn check_maps_matches_and_sends_form() {
    let server = MockServer::start(|req| match req.path() {
        "/v2/check" => Reply::json(200, check_response()),
        _ => Reply::json(404, json!({})),
    });
    let ix = Integrations::new().unwrap();
    let cfg = config(
        &format!("{}/v2/", server.url()),
        Some(("me@example.com", "k1")),
    );
    let req = GrammarRequest {
        text: "They is here.".into(),
        language: None,
        disabled_rules: vec!["A".into(), "B".into()],
        preferred_variants: vec!["en-GB".into()],
        include_misspellings: false,
    };
    let result = ix.check_grammar(&cfg, &req).unwrap();
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        json!({
            "matches": [{
                "offset": 5, "length": 3,
                "message": "Possible agreement error.", "shortMessage": "Grammar",
                "replacements": ["are", "were"],
                "ruleId": "AGREEMENT", "ruleDescription": "Subject-verb agreement",
                "category": "GRAMMAR", "categoryName": "Grammar", "issueType": "grammar",
                "sentence": "They is here.",
                "contextText": "They is here.", "contextOffset": 5, "contextLength": 3
            }],
            "language": {"code": "en-US", "name": "English (US)"},
            "detectedLanguage": {"code": "en-US", "name": "English (US)"}
        })
    );
    let r = &server.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.form_value("text").as_deref(), Some("They is here."));
    assert_eq!(r.form_value("language").as_deref(), Some("auto"));
    assert_eq!(r.form_value("disabledRules").as_deref(), Some("A,B"));
    assert_eq!(r.form_value("preferredVariants").as_deref(), Some("en-GB"));
    assert_eq!(r.form_value("username").as_deref(), Some("me@example.com"));
    assert_eq!(r.form_value("apiKey").as_deref(), Some("k1"));

    let all = ix
        .check_grammar(
            &config(&server.url(), None),
            &GrammarRequest {
                text: "Thye is here.".into(),
                language: Some("en-US".into()),
                preferred_variants: vec!["en-GB".into()],
                include_misspellings: true,
                ..GrammarRequest::default()
            },
        )
        .unwrap();
    assert_eq!(all.matches.len(), 2);
    assert_eq!(all.matches[1].short_message, "");
    let r = &server.requests()[1];
    assert_eq!(r.form_value("language").as_deref(), Some("en-US"));
    // Variants only apply to auto-detection; no credentials, no auth fields.
    assert_eq!(r.form_value("preferredVariants"), None);
    assert_eq!(r.form_value("disabledRules"), None);
    assert_eq!(r.form_value("username"), None);
    assert_eq!(r.form_value("apiKey"), None);
}

#[test]
fn empty_text_is_not_sent() {
    let server = MockServer::start(|_| Reply::json(200, check_response()));
    let ix = Integrations::new().unwrap();
    let r = ix
        .check_grammar(
            &config(&server.url(), None),
            &GrammarRequest {
                text: " \n".into(),
                ..GrammarRequest::default()
            },
        )
        .unwrap();
    assert!(r.matches.is_empty());
    assert_eq!(server.count(), 0);
}

#[test]
fn premium_endpoint() {
    let creds = Some(("me@example.com", "k"));
    assert_eq!(
        config(LANGUAGETOOL_URL, None).base_url().unwrap(),
        LANGUAGETOOL_URL
    );
    assert_eq!(
        config("https://api.languagetool.org/v2/", creds)
            .base_url()
            .unwrap(),
        LANGUAGETOOL_PREMIUM_URL
    );
    assert_eq!(
        config("https://lt.example.org/", creds).base_url().unwrap(),
        "https://lt.example.org"
    );
    // Half the credentials is an error, not a silent fall back to the free tier.
    let half = |username: Option<&str>, api_key: Option<&str>| LanguageToolConfig {
        url: None,
        username: username.map(Secret::from),
        api_key: api_key.map(Secret::from),
    };
    for (cfg, what) in [
        (half(Some("me@example.com"), None), "API key"),
        (half(Some("me@example.com"), Some(" ")), "API key"),
        (half(None, Some("k")), "username"),
    ] {
        let e = cfg.base_url().unwrap_err();
        assert_eq!(e.key(), "not-configured");
        assert!(e.to_string().contains(what), "{e}");
        let e = Integrations::new()
            .unwrap()
            .test(&ProviderConfig::LanguageTool(cfg))
            .unwrap_err();
        assert_eq!(e.key(), "not-configured");
    }
    // An absent, null or blank URL is the public server.
    for body in [
        json!({}),
        json!({"url": null}),
        json!({"url": " "}),
        json!({"url": "", "username": "", "apiKey": ""}),
    ] {
        let cfg: LanguageToolConfig = serde_json::from_value(body).unwrap();
        assert_eq!(cfg.base_url().unwrap(), LANGUAGETOOL_URL);
    }
}

#[test]
fn languages_use_long_codes() {
    let server = MockServer::start(|_| {
        Reply::json(
            200,
            json!([{"name": "German (Austria)", "code": "de", "longCode": "de-AT"}]),
        )
    });
    let ix = Integrations::new().unwrap();
    let langs = ix.grammar_languages(&config(&server.url(), None)).unwrap();
    assert_eq!(
        serde_json::to_value(&langs).unwrap(),
        json!([{"code": "de-AT", "name": "German (Austria)"}])
    );
    assert_eq!(server.requests()[0].path(), "/v2/languages");
}

#[test]
fn errors() {
    let server = MockServer::start(|req| match req.form_value("language").as_deref() {
        Some("big") => Reply::text(
            413,
            "Error: Your text exceeds the limit of 20000 characters",
        ),
        Some("auth") => Reply::text(403, "AuthException: invalid username or API key"),
        Some("slow") => Reply::text(429, "Error: too many requests").header("Retry-After", "60"),
        _ => Reply::text(500, "Error: Internal Error"),
    });
    let ix = Integrations::new().unwrap();
    let cfg = config(&server.url(), None);
    let call = |language: &str| {
        ix.check_grammar(
            &cfg,
            &GrammarRequest {
                text: "x".into(),
                language: Some(language.into()),
                ..GrammarRequest::default()
            },
        )
        .unwrap_err()
    };
    let e = call("big");
    assert_eq!((e.key(), e.status()), ("bad-request", Some(413)));
    assert!(e.to_string().contains("exceeds the limit"));
    assert_eq!(call("auth").key(), "auth");
    let e = call("slow");
    assert_eq!(e.key(), "rate-limited");
    assert_eq!(e.retry_after().map(|d| d.as_secs()), Some(60));
    assert_eq!(call("other").key(), "provider-error");
}

#[test]
fn test_checks_a_sentence() {
    let server = MockServer::start(|_| Reply::json(200, json!({"matches": []})));
    let ix = Integrations::new().unwrap();
    let report = ix
        .test(&ProviderConfig::LanguageTool(config(
            &server.url(),
            Some(("u@example.com", "k")),
        )))
        .unwrap();
    assert!(report.host.starts_with("127.0.0.1:"));
    let r = &server.requests()[0];
    assert_eq!(r.path(), "/v2/check");
    assert_eq!(r.form_value("language").as_deref(), Some("en-US"));
    assert_eq!(r.form_value("apiKey").as_deref(), Some("k"));
}
