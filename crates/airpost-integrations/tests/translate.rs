// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! LibreTranslate and DeepL against a mock server: request shape, mapping, errors, cache.

mod common;

use std::time::Duration;

use airpost_integrations::{
    DeepLConfig, IntegrationError, Integrations, LibreTranslateConfig, Options, ProviderConfig,
    Secret, TextFormat, TranslateRequest, TranslationProvider, DEEPL_FREE_URL, DEEPL_PRO_URL,
};
use common::{closed_port_url, MockServer, Reply};
use serde_json::json;

fn libre(url: &str, key: Option<&str>) -> TranslationProvider {
    TranslationProvider::LibreTranslate(LibreTranslateConfig {
        url: url.into(),
        api_key: key.map(Secret::from),
    })
}

fn deepl(url: &str, key: &str) -> TranslationProvider {
    TranslationProvider::DeepL(DeepLConfig {
        api_key: key.into(),
        url: Some(url.into()),
    })
}

fn request(text: &str, target: &str) -> TranslateRequest {
    TranslateRequest {
        text: text.into(),
        source: None,
        target: target.into(),
        format: TextFormat::Html,
    }
}

fn failure(r: airpost_integrations::Result<impl std::fmt::Debug>) -> IntegrationError {
    r.expect_err("an error")
}

#[test]
fn libretranslate_translate_shape() {
    let server = MockServer::start(|req| match req.path() {
        "/translate" => Reply::json(
            200,
            json!({"translatedText": "<b>Hola</b>", "detectedLanguage": {"confidence": 90.0, "language": "en"}}),
        ),
        _ => Reply::json(404, json!({})),
    });
    let ix = Integrations::new().unwrap();
    let t = ix
        .translate(
            &libre(&format!("{}/", server.url()), Some("k1")),
            &request("<b>Hello</b>", "es"),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&t).unwrap(),
        json!({"translatedText": "<b>Hola</b>", "detectedLanguage": {"language": "en", "confidence": 90.0}})
    );
    let reqs = server.requests();
    assert_eq!(reqs[0].method, "POST");
    assert_eq!(
        reqs[0].json(),
        json!({"q": "<b>Hello</b>", "source": "auto", "target": "es", "format": "html", "api_key": "k1"})
    );
}

#[test]
fn libretranslate_without_key_or_source_auto() {
    let server = MockServer::start(|_| Reply::json(200, json!({"translatedText": "Hallo"})));
    let ix = Integrations::new().unwrap();
    let mut req = request("Hello", "de");
    req.source = Some("EN".into());
    req.format = TextFormat::Text;
    let t = ix
        .translate(&libre(&server.url(), Some("  ")), &req)
        .unwrap();
    assert_eq!(t.translated_text, "Hallo");
    assert_eq!(t.detected_language, None);
    assert_eq!(
        server.requests()[0].json(),
        json!({"q": "Hello", "source": "EN", "target": "de", "format": "text"})
    );
}

#[test]
fn libretranslate_detect_and_languages() {
    let server = MockServer::start(|req| match req.path() {
        "/detect" => Reply::json(200, json!([{"confidence": 88.5, "language": "fr"}])),
        "/languages" => Reply::json(
            200,
            json!([{"code": "en", "name": "English", "targets": ["es", "fr"]}, {"code": "fr", "name": "French"}]),
        ),
        _ => Reply::json(404, json!({})),
    });
    let ix = Integrations::new().unwrap();
    let p = libre(&server.url(), Some("k"));
    let detected = ix.detect_language(&p, "Bonjour").unwrap();
    assert_eq!(detected[0].language, "fr");
    assert_eq!(detected[0].confidence, Some(88.5));
    assert_eq!(
        server.requests()[0].json(),
        json!({"q": "Bonjour", "api_key": "k"})
    );

    let langs = ix.translation_languages(&p).unwrap();
    assert_eq!(
        serde_json::to_value(&langs).unwrap(),
        json!([{"code": "en", "name": "English", "targets": ["es", "fr"]}, {"code": "fr", "name": "French"}])
    );
}

#[test]
fn libretranslate_errors() {
    let server = MockServer::start(|req| match req.json()["target"].as_str().unwrap_or("") {
        "need-key" => Reply::json(
            400,
            json!({"error": "Visit https://portal.libretranslate.com to get an API key"}),
        ),
        "bad-key" => Reply::json(403, json!({"error": "Invalid API key"})),
        "banned" => Reply::json(403, json!({"error": "Too many request limits violations"})),
        "slow" => Reply::json(429, json!({"error": "Slowdown: 20 per 1 minute"}))
            .header("Retry-After", "30"),
        "boom" => Reply::json(500, json!({"error": "Cannot translate text: oops"})),
        "garbage" => Reply::text(200, "<html>portal</html>"),
        _ => Reply::json(400, json!({"error": "xx is not supported"})),
    });
    let ix = Integrations::new().unwrap();
    let p = libre(&server.url(), None);
    let call = |target: &str| ix.translate(&p, &request("Hello", target));

    let e = failure(call("need-key"));
    assert_eq!(
        (e.key(), e.status(), e.is_retryable()),
        ("auth", Some(400), false)
    );
    let e = failure(call("bad-key"));
    assert_eq!((e.key(), e.status()), ("auth", Some(403)));
    let e = failure(call("banned"));
    assert_eq!(e.key(), "rate-limited");
    let e = failure(call("slow"));
    assert_eq!(e.key(), "rate-limited");
    assert_eq!(e.retry_after(), Some(Duration::from_secs(30)));
    assert!(e.is_retryable());
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({"key": "rate-limited", "message": "the provider is rate limiting requests", "retryable": true, "retryAfter": 30})
    );
    let e = failure(call("boom"));
    assert_eq!(
        (e.key(), e.status(), e.is_retryable()),
        ("provider-error", Some(500), true)
    );
    assert!(e.to_string().contains("Cannot translate text: oops"));
    let e = failure(call("garbage"));
    assert_eq!((e.key(), e.status()), ("provider-error", Some(200)));
    let e = failure(call("xx"));
    assert_eq!(
        (e.key(), e.status(), e.is_retryable()),
        ("bad-request", Some(400), false)
    );
    assert!(e.to_string().contains("xx is not supported"));

    // Failures are not cached.
    let before = server.count();
    let _ = call("boom");
    assert_eq!(server.count(), before + 1);
}

#[test]
fn offline_and_timeout() {
    let ix = Integrations::with_options(Options {
        request_timeout: Duration::from_millis(300),
        ..Options::default()
    })
    .unwrap();
    let e = failure(ix.translate(&libre(&closed_port_url(), None), &request("Hi", "es")));
    assert_eq!(e.key(), "offline");
    assert!(e.is_retryable());

    let server = MockServer::start(|_| {
        Reply::json(200, json!({"translatedText": "late"})).delayed(Duration::from_secs(2))
    });
    let e = failure(ix.translate(&libre(&server.url(), None), &request("Hi", "es")));
    assert_eq!(
        e,
        IntegrationError::Offline {
            reason: "timed out"
        }
    );
}

#[test]
fn not_configured_and_bad_url() {
    let ix = Integrations::new().unwrap();
    let e = failure(ix.translate(&libre("  ", None), &request("Hi", "es")));
    assert_eq!(e.key(), "not-configured");
    assert!(!e.is_retryable());
    let e = failure(ix.translate(&deepl("", ""), &request("Hi", "es")));
    assert_eq!(e.key(), "not-configured");
    let e = failure(ix.translate(&libre("ftp://example.com", None), &request("Hi", "es")));
    assert_eq!(e.key(), "bad-request");
    let e = failure(ix.translate(&libre("http://localhost:1", None), &request("Hi", " ")));
    assert_eq!(e.key(), "bad-request");
}

#[test]
fn translation_cache() {
    let server = MockServer::start(|req| {
        let q = req.json()["q"].as_str().unwrap_or("").to_uppercase();
        Reply::json(200, json!({ "translatedText": q }))
    });
    let ix = Integrations::with_options(Options {
        translation_ttl: Duration::from_millis(150),
        ..Options::default()
    })
    .unwrap();
    let p = libre(&server.url(), Some("k1"));

    assert_eq!(
        ix.translate(&p, &request("a", "es"))
            .unwrap()
            .translated_text,
        "A"
    );
    assert_eq!(
        ix.translate(&p, &request("a", "es"))
            .unwrap()
            .translated_text,
        "A"
    );
    assert_eq!(server.count(), 1);
    // Target case and an explicit "auto" source normalise to the same entry.
    let mut same = request("a", "ES");
    same.source = Some("auto".into());
    ix.translate(&p, &same).unwrap();
    assert_eq!(server.count(), 1);

    ix.translate(&p, &request("b", "es")).unwrap();
    ix.translate(&p, &request("a", "fr")).unwrap();
    ix.translate(&libre(&server.url(), Some("k2")), &request("a", "es"))
        .unwrap();
    let mut text = request("a", "es");
    text.format = TextFormat::Text;
    ix.translate(&p, &text).unwrap();
    assert_eq!(server.count(), 5);

    std::thread::sleep(Duration::from_millis(250));
    ix.translate(&p, &request("a", "es")).unwrap();
    assert_eq!(server.count(), 6);

    ix.clear_cache();
    ix.translate(&p, &request("a", "es")).unwrap();
    assert_eq!(server.count(), 7);

    // Empty text never reaches the provider.
    ix.translate(&p, &request("  ", "es")).unwrap();
    assert_eq!(server.count(), 7);
}

#[test]
fn deepl_endpoint_from_key() {
    let config = |key: &str, url: Option<&str>| DeepLConfig {
        api_key: key.into(),
        url: url.map(String::from),
    };
    assert_eq!(config("abc:fx", None).base_url().unwrap(), DEEPL_FREE_URL);
    assert_eq!(config(" abc:fx ", None).base_url().unwrap(), DEEPL_FREE_URL);
    assert_eq!(config("abc", None).base_url().unwrap(), DEEPL_PRO_URL);
    assert_eq!(config("abc:fxy", None).base_url().unwrap(), DEEPL_PRO_URL);
    assert_eq!(
        config("abc:fx", Some("http://127.0.0.1:9/"))
            .base_url()
            .unwrap(),
        "http://127.0.0.1:9"
    );
    assert_eq!(config("abc", Some(" ")).base_url().unwrap(), DEEPL_PRO_URL);
    assert_eq!(
        config("", None).base_url().unwrap_err().key(),
        "not-configured"
    );
}

#[test]
fn deepl_translate_shape() {
    let server = MockServer::start(|req| match req.path() {
        "/v2/translate" => Reply::json(
            200,
            json!({"translations": [{"detected_source_language": "DE", "text": "<b>Hello</b>"}]}),
        ),
        _ => Reply::json(404, json!({})),
    });
    let ix = Integrations::new().unwrap();
    let mut req = request("<b>Hallo</b>", "en");
    req.source = Some("de".into());
    let t = ix.translate(&deepl(&server.url(), "abc:fx"), &req).unwrap();
    assert_eq!(
        serde_json::to_value(&t).unwrap(),
        json!({"translatedText": "<b>Hello</b>", "detectedLanguage": {"language": "de"}})
    );
    let r = &server.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.header("authorization"), Some("DeepL-Auth-Key abc:fx"));
    assert_eq!(
        r.json(),
        json!({"text": ["<b>Hallo</b>"], "target_lang": "EN-US", "source_lang": "DE", "tag_handling": "html"})
    );

    let mut plain = request("Hallo", "fr");
    plain.format = TextFormat::Text;
    ix.translate(&deepl(&server.url(), "abc:fx"), &plain)
        .unwrap();
    assert_eq!(
        server.requests()[1].json(),
        json!({"text": ["Hallo"], "target_lang": "FR"})
    );
}

#[test]
fn deepl_errors() {
    let server = MockServer::start(
        |req| match req.json()["target_lang"].as_str().unwrap_or("") {
            "AA" => Reply::json(403, json!({"message": "Authorization failed."})),
            "BB" => Reply::json(456, json!({"message": "Quota exceeded"})),
            "CC" => Reply::json(429, json!({"message": "Too many requests"})),
            "DD" => Reply::json(529, json!({"message": "Too many requests, high load"})),
            "EE" => Reply::json(200, json!({"translations": []})),
            _ => Reply::json(
                400,
                json!({"message": "Value for 'target_lang' not supported."}),
            ),
        },
    );
    let ix = Integrations::new().unwrap();
    let p = deepl(&server.url(), "k");
    let call = |target: &str| ix.translate(&p, &request("x", target));
    assert_eq!(failure(call("aa")).key(), "auth");
    let e = failure(call("bb"));
    assert_eq!(
        (e.key(), e.status(), e.is_retryable()),
        ("quota-exceeded", Some(456), false)
    );
    assert_eq!(failure(call("cc")).key(), "rate-limited");
    let e = failure(call("dd"));
    assert_eq!((e.key(), e.is_retryable()), ("provider-error", true));
    assert_eq!(failure(call("ee")).key(), "provider-error");
    let e = failure(call("zz"));
    assert_eq!(e.key(), "bad-request");
    assert!(e.to_string().contains("not supported"));
}

#[test]
fn deepl_languages_usage_and_detect() {
    let server = MockServer::start(|req| match req.path() {
        "/v2/languages" => Reply::json(
            200,
            json!([{"language": "EN-US", "name": "English (American)", "supports_formality": false}]),
        ),
        "/v2/usage" if req.header("authorization") == Some("DeepL-Auth-Key unlimited") => {
            Reply::json(
                200,
                json!({"character_count": 5, "character_limit": 1_000_000_000_000u64}),
            )
        }
        "/v2/usage" => Reply::json(
            200,
            json!({"character_count": 1234, "character_limit": 500000}),
        ),
        _ => Reply::json(404, json!({})),
    });
    let ix = Integrations::new().unwrap();
    let p = deepl(&server.url(), "k");
    let langs = ix.translation_languages(&p).unwrap();
    assert_eq!(
        serde_json::to_value(&langs).unwrap(),
        json!([{"code": "en-us", "name": "English (American)"}])
    );
    assert_eq!(
        server.requests()[0].query_value("type").as_deref(),
        Some("target")
    );

    let report = ix.test(&ProviderConfig::from(p.clone())).unwrap();
    let host = server.url().trim_start_matches("http://").to_owned();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        json!({"host": host, "usage": {"used": 1234, "limit": 500000}})
    );
    let report = ix
        .test(&ProviderConfig::from(deepl(&server.url(), "unlimited")))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&report.usage).unwrap(),
        json!({"used": 5})
    );

    let e = failure(ix.detect_language(&p, "Hallo"));
    assert_eq!(e.key(), "unsupported");
}

#[test]
fn libretranslate_test_uses_detect() {
    let server = MockServer::start(|req| match req.path() {
        "/detect" if req.json()["api_key"] == "good" => {
            Reply::json(200, json!([{"confidence": 50, "language": "en"}]))
        }
        "/detect" => Reply::json(403, json!({"error": "Invalid API key"})),
        _ => Reply::json(404, json!({})),
    });
    let ix = Integrations::new().unwrap();
    let ok = ix.test(&ProviderConfig::from(libre(&server.url(), Some("good"))));
    assert!(ok.unwrap().host.starts_with("127.0.0.1:"));
    let e = ix
        .test(&ProviderConfig::from(libre(&server.url(), Some("bad"))))
        .unwrap_err();
    assert_eq!(e.key(), "auth");
}

#[test]
fn provider_config_json() {
    let p: TranslationProvider =
        serde_json::from_value(json!({"provider": "deepl", "apiKey": "abc:fx"})).unwrap();
    let TranslationProvider::DeepL(c) = &p else {
        panic!("deepl")
    };
    assert_eq!(c.base_url().unwrap(), DEEPL_FREE_URL);
    let p: ProviderConfig = serde_json::from_value(
        json!({"provider": "libretranslate", "url": "https://lt.example.org", "apiKey": null}),
    )
    .unwrap();
    assert!(matches!(p, ProviderConfig::LibreTranslate(_)));
    let req: TranslateRequest =
        serde_json::from_value(json!({"text": "<b>x</b>", "target": "es", "format": "html"}))
            .unwrap();
    assert_eq!(req.format, TextFormat::Html);
    assert_eq!(req.source, None);
}

#[test]
fn libretranslate_language_codes_match_the_desktop() {
    let server = MockServer::start(|req| match req.path() {
        "/translate" => Reply::json(
            200,
            json!({"translatedText": "Hei", "detectedLanguage": {"confidence": 80.0, "language": "zh-Hans"}}),
        ),
        "/detect" => Reply::json(200, json!([{"confidence": 70.0, "language": "nb"}])),
        "/languages" => Reply::json(
            200,
            json!([{"code": "zh-Hans", "name": "Chinese", "targets": ["nb", "pt-BR"]}, {"code": "nb", "name": "Norwegian"}]),
        ),
        _ => Reply::json(404, json!({})),
    });
    let ix = Integrations::new().unwrap();
    let p = libre(&server.url(), None);
    let t = ix.translate(&p, &request("你好", "no")).unwrap();
    assert_eq!(t.detected_language.unwrap().language, "zh");
    assert_eq!(server.requests()[0].json()["target"], "nb");
    assert_eq!(ix.detect_language(&p, "Hei").unwrap()[0].language, "no");
    assert_eq!(
        serde_json::to_value(ix.translation_languages(&p).unwrap()).unwrap(),
        json!([{"code": "zh", "name": "Chinese", "targets": ["no", "pt-br"]}, {"code": "no", "name": "Norwegian"}])
    );
}
