// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Contact and company enrichment on People Data Labs (`GET /v5/person/enrich`,
//! `GET /v5/company/enrich`, `GET /v5/autocomplete`; docs.peopledatalabs.com). Auth is the
//! `X-Api-Key` header. A match is 200 and billed; no match is 404 and not billed.
//!
//! Results are mapped to the participant-profile sidebar's `IPerson` and `ICompany`
//! (`participant-profile/lib/sidebar-participant-profile.tsx`). Fields are read as strings
//! only: PDL's free plan returns `true` in place of some values it holds but won't show.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::cache::KeyBuilder;
use crate::config::PeopleDataLabsConfig;
use crate::error::{IntegrationError, Result};
use crate::http::{self, is_success, parse, status_error, Http, Reply};
use crate::translate::host_of;
use crate::{Integrations, TestReport};

const PDL: &str = "peopledatalabs";

/// PDL's likelihood is 1-10. Below 6 a person match is too often someone else with a similar
/// profile, and showing a stranger's job in the sidebar is worse than showing nothing.
const PERSON_MIN_LIKELIHOOD: &str = "6";

/// Webmail and ISP mail domains: the address says nothing about the person's company, and
/// a company lookup on one returns the mail provider as their employer.
const WEBMAIL_DOMAINS: &[&str] = &[
    "126.com",
    "139.com",
    "163.com",
    "21cn.com",
    "abv.bg",
    "aliyun.com",
    "arcor.de",
    "att.net",
    "bellsouth.net",
    "bigpond.com",
    "bigpond.net.au",
    "bk.ru",
    "blueyonder.co.uk",
    "bol.com.br",
    "btinternet.com",
    "btopenworld.com",
    "centurylink.net",
    "charter.net",
    "comcast.net",
    "cox.net",
    "daum.net",
    "duck.com",
    "earthlink.net",
    "email.cz",
    "fastmail.com",
    "fastmail.fm",
    "foxmail.com",
    "free.fr",
    "freenet.de",
    "frontier.com",
    "googlemail.com",
    "gmail.com",
    "hanmail.net",
    "hey.com",
    "hushmail.com",
    "icloud.com",
    "ig.com.br",
    "inbox.ru",
    "interia.pl",
    "juno.com",
    "laposte.net",
    "list.ru",
    "mac.com",
    "mail.com",
    "mail.ru",
    "mailbox.org",
    "me.com",
    "nate.com",
    "neuf.fr",
    "netzero.net",
    "ntlworld.com",
    "o2.pl",
    "onet.pl",
    "optonline.net",
    "optusnet.com.au",
    "pm.me",
    "posteo.de",
    "posteo.net",
    "proton.me",
    "protonmail.ch",
    "protonmail.com",
    "rambler.ru",
    "rediffmail.com",
    "rogers.com",
    "sbcglobal.net",
    "seznam.cz",
    "sfr.fr",
    "shaw.ca",
    "sina.cn",
    "sina.com",
    "sky.com",
    "skynet.be",
    "sohu.com",
    "sympatico.ca",
    "talktalk.net",
    "telenet.be",
    "telus.net",
    "terra.com.br",
    "tuta.io",
    "tutamail.com",
    "tutanota.com",
    "tutanota.de",
    "uol.com.br",
    "ukr.net",
    "verizon.net",
    "virginmedia.com",
    "web.de",
    "windstream.net",
    "wp.pl",
    "xtra.co.nz",
    "yeah.net",
    "zohomail.com",
];

/// Providers with a domain per country (`yahoo.co.uk`, `hotmail.fr`, `gmx.at`): matched on
/// the first label when at most two labels follow it.
const WEBMAIL_FAMILIES: &[&str] = &[
    "aim",
    "aol",
    "gmx",
    "hotmail",
    "libero",
    "live",
    "msn",
    "naver",
    "orange",
    "outlook",
    "qq",
    "rocketmail",
    "t-online",
    "virgilio",
    "wanadoo",
    "windowslive",
    "yahoo",
    "yandex",
    "ymail",
    "zoho",
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Handle {
    pub handle: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Employment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The sidebar's `IPerson`, plus `name`. `linkedin.handle` is the bare username; the sidebar
/// adds `in/`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facebook: Option<Handle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitter: Option<Handle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkedin: Option<Handle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub employment: Option<Employment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CompanyCategory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub industry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sector: Option<String>,
}

/// Numbers as strings, as the sidebar reads them (`Number(metrics.raised)`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raised: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_cap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub employees: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub employees_range: Option<String>,
}

/// The sidebar's `ICompany`. PDL has no logo, time zone, phone or Crunchbase handle, so
/// those stay empty; `type` is PDL's (`public`, `private`, `public_subsidiary`, `nonprofit`,
/// …), of which the sidebar shows `public` and `private`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CompanyCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facebook: Option<Handle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitter: Option<Handle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkedin: Option<Handle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crunchbase: Option<Handle>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<CompanyMetrics>,
}

/// What the sidebar merges into its state: `{person?, company?}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ContactProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person: Option<Person>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<Company>,
}

/// A person and their employer's website, which picks the company lookup.
#[derive(Clone, Debug)]
pub(crate) struct PersonMatch {
    pub person: Person,
    pub company_domain: Option<String>,
}

pub(crate) fn person(
    ix: &Integrations,
    config: &PeopleDataLabsConfig,
    email: &str,
) -> Result<Option<PersonMatch>> {
    let base = config.base_url()?;
    let key = config.key()?;
    let email = normalise_email(email)?;
    let cache_key = KeyBuilder::new("person")
        .part(&base)
        .part(key)
        .part(&email)
        .finish();
    if let Some(hit) = ix.people.get(&cache_key) {
        return Ok(hit);
    }
    let found = enrich_get(
        &ix.http,
        config,
        &base,
        "/v5/person/enrich",
        &[
            ("email", &email),
            ("min_likelihood", PERSON_MIN_LIKELIHOOD),
            ("titlecase", "true"),
        ],
    )?
    .map(|body| map_person(body.get("data").and_then(Value::as_object)));
    let ttl = match found {
        Some(_) => ix.options.enrichment_ttl,
        None => ix.options.negative_enrichment_ttl,
    };
    ix.people.insert(cache_key, found.clone(), ttl);
    Ok(found)
}

pub(crate) fn company(
    ix: &Integrations,
    config: &PeopleDataLabsConfig,
    domain: &str,
) -> Result<Option<Company>> {
    let base = config.base_url()?;
    let key = config.key()?;
    let domain = normalise_domain(domain)
        .ok_or_else(|| IntegrationError::bad_request("not a domain name"))?;
    let cache_key = KeyBuilder::new("company")
        .part(&base)
        .part(key)
        .part(&domain)
        .finish();
    if let Some(hit) = ix.companies.get(&cache_key) {
        return Ok(hit);
    }
    let found = enrich_get(
        &ix.http,
        config,
        &base,
        "/v5/company/enrich",
        &[("website", &domain), ("titlecase", "true")],
    )?
    .and_then(|body| map_company(&body));
    let ttl = match found {
        Some(_) => ix.options.enrichment_ttl,
        None => ix.options.negative_enrichment_ttl,
    };
    ix.companies.insert(cache_key, found.clone(), ttl);
    Ok(found)
}

pub(crate) fn contact(
    ix: &Integrations,
    config: &PeopleDataLabsConfig,
    email: &str,
) -> Result<ContactProfile> {
    let email = normalise_email(email)?;
    let found = person(ix, config, &email)?;
    let domain = found
        .as_ref()
        .and_then(|m| m.company_domain.clone())
        .or_else(|| {
            let domain = email.rsplit('@').next()?;
            (!is_webmail(domain)).then(|| domain.to_owned())
        });
    let company = match domain {
        Some(d) => company(ix, config, &d)?,
        None => None,
    };
    Ok(ContactProfile {
        person: found.map(|m| m.person),
        company,
    })
}

pub(crate) fn test(http: &Http, config: &PeopleDataLabsConfig) -> Result<TestReport> {
    let base = config.base_url()?;
    // Free for active keys, unlike enrichment, which bills per match. "a" always has
    // suggestions, so anything but a 200 means the URL or key is wrong.
    let reply = pdl_get(
        http,
        config,
        &base,
        "/v5/autocomplete",
        &[("field", "company"), ("text", "a"), ("size", "1")],
    )?;
    if reply.status != 200 {
        return Err(status_error(&reply));
    }
    Ok(TestReport {
        host: host_of(&base),
        usage: None,
    })
}

fn pdl_get(
    http: &Http,
    config: &PeopleDataLabsConfig,
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Reply> {
    let url = http::url(base, path, query)?;
    let req = http
        .client()
        .get(url.clone())
        .header("X-Api-Key", config.key()?);
    http.send(PDL, &url, req)
}

/// The body of a 200, `None` for no match. PDL's no-match is a 404 with `error.type`
/// `not_found`; any other 404 (a wrong URL or path) is an error, so it isn't cached as a
/// miss.
fn enrich_get(
    http: &Http,
    config: &PeopleDataLabsConfig,
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Option<Value>> {
    let reply = pdl_get(http, config, base, path, query)?;
    if reply.status == 404 && is_not_found(&reply.body) {
        return Ok(None);
    }
    if !is_success(&reply) {
        return Err(status_error(&reply));
    }
    parse(&reply).map(Some)
}

fn is_not_found(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .is_ok_and(|v| v["error"]["type"].as_str() == Some("not_found"))
}

fn is_webmail(domain: &str) -> bool {
    if WEBMAIL_DOMAINS.contains(&domain) {
        return true;
    }
    let mut labels = domain.split('.');
    let first = labels.next().unwrap_or("");
    let rest = labels.count();
    WEBMAIL_FAMILIES.contains(&first) && (1..=2).contains(&rest)
}

fn map_person(data: Option<&Map<String, Value>>) -> PersonMatch {
    let empty = Map::new();
    let d = data.unwrap_or(&empty);
    let title = string(d, "job_title");
    let employer = string(d, "job_company_name");
    PersonMatch {
        person: Person {
            name: string(d, "full_name"),
            facebook: string(d, "facebook_username").map(|handle| Handle { handle }),
            twitter: string(d, "twitter_username").map(|handle| Handle { handle }),
            linkedin: string(d, "linkedin_username").map(|handle| Handle { handle }),
            employment: (title.is_some() || employer.is_some()).then_some(Employment {
                title,
                name: employer,
            }),
            location: string(d, "location_name"),
            bio: string(d, "summary"),
        },
        company_domain: string(d, "job_company_website").and_then(|w| normalise_domain(&w)),
    }
}

/// `None` when the record has no name to show.
fn map_company(body: &Value) -> Option<Company> {
    let d = body.as_object()?;
    let name = string(d, "display_name").or_else(|| string(d, "name"))?;
    let metrics = CompanyMetrics {
        raised: number(d, "total_funding_raised"),
        market_cap: None,
        employees: number(d, "employee_count"),
        employees_range: string(d, "size"),
    };
    let has_metrics = metrics != CompanyMetrics::default();
    Some(Company {
        name,
        domain: string(d, "website"),
        category: string(d, "industry").map(|industry| CompanyCategory {
            industry: Some(industry),
            sector: None,
        }),
        description: string(d, "summary").or_else(|| string(d, "headline")),
        location: d
            .get("location")
            .and_then(Value::as_object)
            .and_then(|l| string(l, "name")),
        facebook: social(d, "facebook_url", "facebook.com"),
        twitter: social(d, "twitter_url", "twitter.com"),
        linkedin: social(d, "linkedin_url", "linkedin.com"),
        kind: string(d, "type"),
        ticker: string(d, "ticker"),
        metrics: has_metrics.then_some(metrics),
        ..Company::default()
    })
}

fn string(d: &Map<String, Value>, field: &str) -> Option<String> {
    d.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn number(d: &Map<String, Value>, field: &str) -> Option<String> {
    let n = d.get(field)?;
    if let Some(i) = n.as_u64() {
        return Some(i.to_string());
    }
    n.as_f64().map(|f| format!("{}", f.round() as u64))
}

/// The path after `host` in a PDL profile URL (`linkedin.com/company/acme` →
/// `company/acme`), which the sidebar appends to `https://www.<host>/`.
fn social(d: &Map<String, Value>, field: &str, host: &str) -> Option<Handle> {
    let url = string(d, field)?;
    let rest = strip_scheme(&url);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.strip_prefix(host).unwrap_or(rest);
    let handle = rest.trim_matches('/');
    (!handle.is_empty()).then(|| Handle {
        handle: handle.to_owned(),
    })
}

fn strip_scheme(url: &str) -> &str {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
}

fn normalise_email(email: &str) -> Result<String> {
    let email = email.trim().to_lowercase();
    match email.split_once('@') {
        Some((local, domain)) if !local.is_empty() && domain.contains('.') => Ok(email),
        _ => Err(IntegrationError::bad_request("not an email address")),
    }
}

/// `https://www.Acme.com/about` → `acme.com`.
fn normalise_domain(input: &str) -> Option<String> {
    let lower = input.trim().to_lowercase();
    let rest = strip_scheme(&lower);
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or("");
    let host = host.strip_prefix("www.").unwrap_or(host);
    (host.contains('.') && !host.starts_with('.') && !host.ends_with('.')).then(|| host.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains() {
        assert_eq!(
            normalise_domain("https://www.Acme.com/about").as_deref(),
            Some("acme.com")
        );
        assert_eq!(
            normalise_domain("acme.co.uk").as_deref(),
            Some("acme.co.uk")
        );
        assert_eq!(
            normalise_domain("acme.com:8080").as_deref(),
            Some("acme.com")
        );
        assert_eq!(normalise_domain("localhost"), None);
        assert_eq!(normalise_domain(""), None);
    }

    #[test]
    fn webmail() {
        for d in [
            "gmail.com",
            "yahoo.co.uk",
            "hotmail.fr",
            "outlook.de",
            "live.co.uk",
            "ymail.com",
            "126.com",
            "comcast.net",
            "btinternet.com",
            "orange.fr",
            "libero.it",
            "naver.com",
            "gmx.at",
            "yandex.com.tr",
        ] {
            assert!(is_webmail(d), "{d}");
        }
        for d in [
            "acme.io",
            "yahoo.example.co.uk",
            "mail.acme.com",
            "liveperson.com",
        ] {
            assert!(!is_webmail(d), "{d}");
        }
    }

    #[test]
    fn not_found_needs_pdl_shape() {
        assert!(is_not_found(
            r#"{"status":404,"error":{"type":"not_found","message":"No records"}}"#
        ));
        assert!(!is_not_found(r#"{"status":404}"#));
        assert!(!is_not_found("<html>Not Found</html>"));
    }

    #[test]
    fn emails() {
        assert_eq!(normalise_email(" Ann@Acme.COM ").unwrap(), "ann@acme.com");
        assert!(normalise_email("ann").is_err());
        assert!(normalise_email("@acme.com").is_err());
    }

    #[test]
    fn social_handles() {
        let d = serde_json::json!({
            "linkedin_url": "linkedin.com/company/acme",
            "facebook_url": "https://www.facebook.com/acme/",
            "twitter_url": "",
        });
        let d = d.as_object().unwrap();
        assert_eq!(
            social(d, "linkedin_url", "linkedin.com").unwrap().handle,
            "company/acme"
        );
        assert_eq!(
            social(d, "facebook_url", "facebook.com").unwrap().handle,
            "acme"
        );
        assert_eq!(social(d, "twitter_url", "twitter.com"), None);
    }
}
