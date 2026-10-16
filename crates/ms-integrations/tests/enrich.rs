// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! People Data Labs against a mock server.

mod common;

use std::time::Duration;

use common::{MockServer, Reply};
use ms_integrations::{Integrations, Options, PeopleDataLabsConfig, ProviderConfig};
use serde_json::{json, Value};

fn config(url: &str, key: &str) -> PeopleDataLabsConfig {
    PeopleDataLabsConfig {
        api_key: key.into(),
        url: Some(url.into()),
    }
}

fn person_body() -> Value {
    json!({
        "status": 200,
        "likelihood": 9,
        "data": {
            "full_name": "Ann Example",
            "job_title": "Head Of Engineering",
            "job_company_name": "Acme",
            "job_company_website": "acme.io",
            "location_name": true,
            "linkedin_username": "annexample",
            "twitter_username": null,
            "facebook_username": "ann.example",
            "summary": "Builds things."
        }
    })
}

fn company_body() -> Value {
    json!({
        "status": 200,
        "likelihood": 8,
        "name": "acme",
        "display_name": "Acme",
        "website": "acme.io",
        "industry": "Computer Software",
        "summary": "Acme makes anvils.",
        "location": {"name": "San Francisco, California, United States", "locality": "San Francisco"},
        "linkedin_url": "linkedin.com/company/acme",
        "twitter_url": "twitter.com/acme",
        "facebook_url": null,
        "type": "private",
        "size": "51-200",
        "employee_count": 120,
        "total_funding_raised": 25000000.0
    })
}

fn pdl_server() -> MockServer {
    MockServer::start(|req| match req.path() {
        "/v5/person/enrich" => match req.query_value("email").as_deref() {
            Some("ann@acme.io") | Some("ann@gmail.com") => Reply::json(200, person_body()),
            _ => Reply::json(
                404,
                json!({"status": 404, "error": {"type": "not_found", "message": "No records were found matching your request"}}),
            ),
        },
        "/v5/company/enrich" => match req.query_value("website").as_deref() {
            Some("acme.io") => Reply::json(200, company_body()),
            _ => Reply::json(
                404,
                json!({"status": 404, "error": {"type": "not_found", "message": "No records were found matching your request"}}),
            ),
        },
        "/v5/autocomplete" => Reply::json(
            200,
            json!({"data": [], "fields": ["company"], "status": 200}),
        ),
        _ => Reply::json(405, json!({})),
    })
}

/// A server that isn't PDL, or a retired path: 404 for everything, without PDL's body.
fn wrong_server() -> MockServer {
    MockServer::start(|_| Reply::text(404, "Not Found"))
}

#[test]
fn person_mapping_and_request() {
    let server = pdl_server();
    let ix = Integrations::new().unwrap();
    let person = ix
        .enrich_person(&config(&server.url(), "k1"), " Ann@Acme.io ")
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&person).unwrap(),
        json!({
            "name": "Ann Example",
            "facebook": {"handle": "ann.example"},
            "linkedin": {"handle": "annexample"},
            "employment": {"title": "Head Of Engineering", "name": "Acme"},
            "bio": "Builds things."
        })
    );
    let r = &server.requests()[0];
    assert_eq!(r.method, "GET");
    assert_eq!(r.header("x-api-key"), Some("k1"));
    assert_eq!(r.query_value("email").as_deref(), Some("ann@acme.io"));
    assert_eq!(r.query_value("min_likelihood").as_deref(), Some("6"));
    assert_eq!(r.query_value("titlecase").as_deref(), Some("true"));
}

#[test]
fn company_mapping_and_request() {
    let server = pdl_server();
    let ix = Integrations::new().unwrap();
    let company = ix
        .enrich_company(&config(&server.url(), "k1"), "https://www.ACME.io/about")
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&company).unwrap(),
        json!({
            "name": "Acme",
            "domain": "acme.io",
            "category": {"industry": "Computer Software"},
            "description": "Acme makes anvils.",
            "location": "San Francisco, California, United States",
            "twitter": {"handle": "acme"},
            "linkedin": {"handle": "company/acme"},
            "type": "private",
            "metrics": {"raised": "25000000", "employees": "120", "employeesRange": "51-200"}
        })
    );
    assert_eq!(
        server.requests()[0].query_value("website").as_deref(),
        Some("acme.io")
    );
    let e = ix
        .enrich_company(&config(&server.url(), "k1"), "not a domain")
        .unwrap_err();
    assert_eq!(e.key(), "bad-request");
}

#[test]
fn contact_picks_company_domain() {
    let server = pdl_server();
    let ix = Integrations::new().unwrap();
    let cfg = config(&server.url(), "k1");

    // A webmail address with a match: the company comes from the person's employer.
    let profile = ix.enrich_contact(&cfg, "ann@gmail.com").unwrap();
    assert_eq!(profile.person.unwrap().name.as_deref(), Some("Ann Example"));
    assert_eq!(profile.company.unwrap().name, "Acme");
    let reqs = server.requests();
    assert_eq!(reqs[1].query_value("website").as_deref(), Some("acme.io"));

    // No person, webmail: no company lookup.
    let profile = ix.enrich_contact(&cfg, "nobody@gmail.com").unwrap();
    assert_eq!(serde_json::to_value(&profile).unwrap(), json!({}));
    assert_eq!(server.count(), 3);

    // No person, corporate domain: the company comes from the address.
    let profile = ix.enrich_contact(&cfg, "bob@acme.io").unwrap();
    assert!(profile.person.is_none());
    assert_eq!(profile.company.unwrap().domain.as_deref(), Some("acme.io"));
    // acme.io was cached by the first lookup.
    assert_eq!(server.count(), 4);
}

#[test]
fn negative_results_cached_shorter() {
    let server = pdl_server();
    let ix = Integrations::with_options(Options {
        enrichment_ttl: Duration::from_secs(3600),
        negative_enrichment_ttl: Duration::from_millis(150),
        ..Options::default()
    })
    .unwrap();
    let cfg = config(&server.url(), "k1");

    assert!(ix.enrich_person(&cfg, "ann@acme.io").unwrap().is_some());
    assert!(ix.enrich_person(&cfg, "nobody@acme.io").unwrap().is_none());
    assert!(ix.enrich_person(&cfg, "ANN@acme.io").unwrap().is_some());
    assert!(ix.enrich_person(&cfg, "nobody@acme.io").unwrap().is_none());
    assert_eq!(server.count(), 2);

    std::thread::sleep(Duration::from_millis(250));
    assert!(ix.enrich_person(&cfg, "ann@acme.io").unwrap().is_some());
    assert!(ix.enrich_person(&cfg, "nobody@acme.io").unwrap().is_none());
    assert_eq!(server.count(), 3);

    // Another key is another cache entry.
    ix.enrich_person(&config(&server.url(), "k2"), "ann@acme.io")
        .unwrap();
    assert_eq!(server.count(), 4);
}

#[test]
fn errors() {
    let server = MockServer::start(|req| match req.header("x-api-key") {
        Some("bad") => Reply::json(
            401,
            json!({"status": 401, "error": {"type": "authentication_error", "message": "Invalid API key"}}),
        ),
        Some("broke") => Reply::json(
            402,
            json!({"status": 402, "error": {"type": "payment_required", "message": "You have hit your account maximum"}}),
        ),
        Some("busy") => Reply::json(429, json!({"status": 429})).header("Retry-After", "12"),
        _ => Reply::json(500, json!({"status": 500})),
    });
    let ix = Integrations::new().unwrap();
    let call = |key: &str| {
        ix.enrich_person(&config(&server.url(), key), "ann@acme.io")
            .unwrap_err()
    };
    assert_eq!(call("bad").key(), "auth");
    let e = call("broke");
    assert_eq!((e.key(), e.is_retryable()), ("quota-exceeded", false));
    let e = call("busy");
    assert_eq!(e.key(), "rate-limited");
    assert_eq!(e.retry_after(), Some(Duration::from_secs(12)));
    assert_eq!(call("x").key(), "provider-error");

    let e = ix
        .enrich_person(&config(&server.url(), " "), "ann@acme.io")
        .unwrap_err();
    assert_eq!(e.key(), "not-configured");
    let e = ix
        .enrich_person(&config(&server.url(), "k"), "not-an-email")
        .unwrap_err();
    assert_eq!(e.key(), "bad-request");
}

#[test]
fn test_uses_free_autocomplete() {
    let server = pdl_server();
    let ix = Integrations::new().unwrap();
    let report = ix
        .test(&ProviderConfig::PeopleDataLabs(config(&server.url(), "k1")))
        .unwrap();
    assert!(report.host.starts_with("127.0.0.1:"));
    let r = &server.requests()[0];
    assert_eq!(r.path(), "/v5/autocomplete");
    assert_eq!(r.query_value("field").as_deref(), Some("company"));
    assert_eq!(r.header("x-api-key"), Some("k1"));
}

#[test]
fn bare_404_is_an_error_not_a_miss() {
    let server = wrong_server();
    let ix = Integrations::new().unwrap();
    let cfg = config(&server.url(), "k1");
    let e = ix.enrich_person(&cfg, "ann@acme.io").unwrap_err();
    assert_eq!((e.key(), e.status()), ("bad-request", Some(404)));
    // Not cached as a miss: the next call asks again.
    ix.enrich_person(&cfg, "ann@acme.io").unwrap_err();
    assert_eq!(server.count(), 2);
    assert!(ix.enrich_company(&cfg, "acme.io").is_err());
    let e = ix.test(&ProviderConfig::PeopleDataLabs(cfg)).unwrap_err();
    assert_eq!(e.key(), "bad-request");
}

#[test]
fn regional_webmail_gets_no_company_lookup() {
    let server = pdl_server();
    let ix = Integrations::new().unwrap();
    let cfg = config(&server.url(), "k1");
    for email in ["bob@yahoo.co.uk", "bob@hotmail.fr", "bob@comcast.net"] {
        let profile = ix.enrich_contact(&cfg, email).unwrap();
        assert_eq!(serde_json::to_value(&profile).unwrap(), json!({}));
    }
    assert!(server
        .requests()
        .iter()
        .all(|r| r.path() == "/v5/person/enrich"));
}

#[test]
fn blank_url_is_the_default() {
    for body in [
        json!({"provider": "peopledatalabs", "apiKey": "k", "url": ""}),
        json!({"provider": "peopledatalabs", "apiKey": "k", "url": null}),
        json!({"provider": "peopledatalabs", "apiKey": "k"}),
    ] {
        let ProviderConfig::PeopleDataLabs(cfg) = serde_json::from_value(body).unwrap() else {
            panic!("not a PDL config");
        };
        assert_eq!(
            cfg.base_url().unwrap(),
            ms_integrations::PEOPLE_DATA_LABS_URL
        );
    }
}
