// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! IMAP and SMTP settings for a new account (`account.discover`, docs/desktop-client-plan.md
//! 3.6). Ports the desktop's `expandAccountWithCommonSettings` (onboarding-helpers.ts, from
//! Mailspring, Copyright © 2017 Foundry 376, GPL-3.0), and keeps its order:
//!
//! 1. the [`Matcher`]s, by the address's domain, then by its MX hosts (mailcore2's
//!    `providers.json`, Copyright © 2001-2013 MailCore team, BSD-3-Clause; NOTICE.md
//!    "mailcore2");
//! 2. for password accounts only, the domain's Thunderbird autoconfig file
//!    (`autoconfig.<domain>`, then `<domain>/.well-known`): an OAuth token is accepted only by
//!    its provider's servers, which the file may not name;
//! 3. the provider table by domain, then by the account's provider preset (`yahoo`, `gmail`, …),
//!    derived from Thunderbird's ISPDB (MPL-2.0);
//! 4. `imap.<domain>` and `smtp.<domain>`.
//!
//! Both tables are `data/providers.json`, one copy for every client. Discovery never picks
//! cleartext (docs/tech-debt.md TD4): ports 143 and 25 and an unknown security mean STARTTLS,
//! an autoconfig `plain` server is tried with STARTTLS, and the data holds no `none`. The
//! network lookups (MX, autoconfig) are the caller's, so this module stays pure.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// `SSL / TLS`, as the settings store it.
pub const SECURITY_TLS: &str = "SSL / TLS";
pub const SECURITY_STARTTLS: &str = "STARTTLS";

/// The most of an autoconfig document [`autoconfig_settings`] reads.
pub const AUTOCONFIG_MAX_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
struct Data {
    matchers: Vec<RawMatcher>,
    domains: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
struct RawMatcher {
    id: String,
    #[serde(default)]
    domains: Vec<String>,
    #[serde(default)]
    mx: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    imap: Server,
    smtp: Server,
}

/// One server of a matcher. `host` may hold `{domain}`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: Option<u16>,
    pub security: String,
}

/// A provider recognised by the address's domain or its MX hosts.
pub struct Matcher {
    pub id: String,
    domains: Vec<Regex>,
    mx: Vec<Regex>,
    exclude: Vec<Regex>,
    pub imap: Server,
    pub smtp: Server,
}

impl Matcher {
    fn matches(&self, domain: &str, mx: &[String]) -> bool {
        if self.exclude.iter().any(|r| r.is_match(domain)) {
            return false;
        }
        self.domains.iter().any(|r| r.is_match(domain))
            || self
                .mx
                .iter()
                .any(|r| mx.iter().any(|host| r.is_match(host)))
    }
}

struct Tables {
    matchers: Vec<Matcher>,
    domains: Map<String, Value>,
}

/// The whole pattern, as the desktop's `new RegExp(`^${test}$`)` matched it.
fn anchored(patterns: &[String]) -> Vec<Regex> {
    patterns
        .iter()
        .filter_map(|p| Regex::new(&format!("^(?:{p})$")).ok())
        .collect()
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let data: Data = serde_json::from_str(include_str!("../data/providers.json"))
            .expect("data/providers.json is valid");
        Tables {
            matchers: data
                .matchers
                .into_iter()
                .map(|m| Matcher {
                    id: m.id,
                    domains: anchored(&m.domains),
                    mx: anchored(&m.mx),
                    exclude: anchored(&m.exclude),
                    imap: m.imap,
                    smtp: m.smtp,
                })
                .collect(),
            domains: data.domains,
        }
    })
}

/// Where [`discover`]'s settings came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Matcher,
    Autoconfig,
    ProviderTable,
    Fallback,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Matcher => "matcher",
            Source::Autoconfig => "autoconfig",
            Source::ProviderTable => "provider-table",
            Source::Fallback => "fallback",
        }
    }
}

/// The settings found for an address: hosts, ports, usernames, security, `*_allow_insecure_ssl`
/// and `container_folder`, as the account JSON stores them. Never secrets.
#[derive(Clone, Debug, PartialEq)]
pub struct Discovered {
    pub source: Source,
    /// The matcher's id or the provider table's key, when one was used.
    pub template: Option<String>,
    pub settings: Map<String, Value>,
}

/// The lookups discovery needs from the network.
pub trait Lookups {
    /// The domain's MX hosts, lowercased; empty when there are none or the lookup failed.
    fn mx(&mut self, domain: &str) -> Vec<String>;
    /// The body of an HTTPS GET, `None` unless it answered 2xx.
    fn fetch(&mut self, url: &str) -> Option<String>;
}

/// The part of `email` after the last `@`, lowercased.
pub fn email_domain(email: &str) -> String {
    email.rsplit('@').next().unwrap_or("").to_ascii_lowercase()
}

/// The security the client's menu stores when `port` is picked, or `None` for another port.
pub fn security_for_port(protocol: &str, port: u16) -> Option<&'static str> {
    match (protocol, port) {
        ("imap", 993) | ("smtp", 465) => Some(SECURITY_TLS),
        ("imap", 143) | ("smtp", 587) | ("smtp", 25) => Some(SECURITY_STARTTLS),
        _ => None,
    }
}

/// `discoveredSecurity`: `value` when it names TLS, else what the port implies, else
/// STARTTLS. Never `none`.
pub fn discovered_security(protocol: &str, value: Option<&str>, port: u16) -> &'static str {
    match value {
        Some(SECURITY_TLS) => SECURITY_TLS,
        Some(SECURITY_STARTTLS) => SECURITY_STARTTLS,
        _ => security_for_port(protocol, port).unwrap_or(SECURITY_STARTTLS),
    }
}

/// The first matcher for the domain or one of its MX hosts.
pub fn matcher_for(domain: &str, mx: &[String]) -> Option<&'static Matcher> {
    tables().matchers.iter().find(|m| m.matches(domain, mx))
}

/// The matchers, for listing them.
pub fn matchers() -> &'static [Matcher] {
    &tables().matchers
}

/// The provider table's entry for the domain, else for the provider preset, aliases followed
/// once; with its key.
pub fn provider_entry(
    domain: &str,
    provider: &str,
) -> Option<(String, &'static Map<String, Value>)> {
    let domains = &tables().domains;
    let (key, entry) = [domain, provider]
        .iter()
        .find_map(|k| domains.get_key_value(*k))?;
    match entry.get("alias").and_then(Value::as_str) {
        Some(alias) => domains
            .get(alias)
            .and_then(Value::as_object)
            .map(|e| (alias.to_owned(), e)),
        None => entry.as_object().map(|e| (key.clone(), e)),
    }
}

fn username(email: &str, format: Option<&str>) -> Option<String> {
    match format {
        Some("email") => Some(email.to_owned()),
        Some("email-without-domain") => email.split('@').next().map(str::to_owned),
        _ => None,
    }
}

fn insert_server(
    out: &mut Map<String, Value>,
    protocol: &str,
    host: String,
    port: Option<u16>,
    username: Option<String>,
    security: &str,
    allow_insecure_ssl: bool,
) {
    out.insert(format!("{protocol}_host"), json!(host.trim()));
    if let Some(p) = port {
        out.insert(format!("{protocol}_port"), json!(p));
    }
    if let Some(u) = username {
        out.insert(format!("{protocol}_username"), json!(u));
    }
    out.insert(format!("{protocol}_security"), json!(security));
    out.insert(
        format!("{protocol}_allow_insecure_ssl"),
        json!(allow_insecure_ssl),
    );
}

/// The settings a matcher gives `email`.
pub fn matcher_settings(m: &Matcher, email: &str) -> Map<String, Value> {
    let domain = email_domain(email);
    let mut out = Map::new();
    for (protocol, server) in [("imap", &m.imap), ("smtp", &m.smtp)] {
        let port = server.port.unwrap_or(0);
        insert_server(
            &mut out,
            protocol,
            server.host.replace("{domain}", &domain),
            server.port,
            Some(email.to_owned()),
            discovered_security(protocol, Some(&server.security), port),
            false,
        );
    }
    out.insert("container_folder".into(), json!(""));
    out
}

/// The settings a provider table entry gives `email`. An entry without an IMAP host (a
/// POP-only provider) gives an empty one, and the client asks for the servers.
pub fn provider_settings(entry: &Map<String, Value>, email: &str) -> Map<String, Value> {
    let domain = email_domain(email);
    let mut out = Map::new();
    for protocol in ["imap", "smtp"] {
        let text = |k: &str| {
            entry
                .get(&format!("{protocol}_{k}"))
                .and_then(Value::as_str)
        };
        let port = entry
            .get(&format!("{protocol}_port"))
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok());
        let insecure = entry
            .get(&format!("{protocol}_allow_insecure_ssl"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        insert_server(
            &mut out,
            protocol,
            text("host").unwrap_or("").replace("%EMAILDOMAIN%", &domain),
            port,
            username(email, text("user_format")),
            discovered_security(protocol, text("security"), port.unwrap_or(0)),
            insecure,
        );
    }
    if let Some(folder) = entry.get("container_folder") {
        out.insert("container_folder".into(), folder.clone());
    }
    out
}

/// The guess for a domain nothing knows: `imap.<domain>` on 993 and `smtp.<domain>` on 465.
pub fn fallback_settings(email: &str) -> Map<String, Value> {
    let domain = email_domain(email);
    let mut out = Map::new();
    insert_server(
        &mut out,
        "imap",
        format!("imap.{domain}"),
        Some(993),
        Some(email.to_owned()),
        SECURITY_TLS,
        false,
    );
    insert_server(
        &mut out,
        "smtp",
        format!("smtp.{domain}"),
        Some(465),
        Some(email.to_owned()),
        SECURITY_TLS,
        false,
    );
    out.insert("container_folder".into(), json!(""));
    out
}

/// The autoconfig URLs tried for a domain, in order.
pub fn autoconfig_urls(domain: &str) -> [String; 2] {
    [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ]
}

/// The settings in a Thunderbird autoconfig document (`TryThunderbirdAutoconfig`): its first
/// IMAP incoming and SMTP outgoing server, of the `emailProvider` whose id is the domain when
/// there are several. `None` when the document has no such provider or lacks either kind.
pub fn autoconfig_settings(xml: &str, email: &str) -> Option<Map<String, Value>> {
    if xml.len() > AUTOCONFIG_MAX_BYTES {
        return None;
    }
    let domain = email_domain(email);
    let doc = roxmltree::Document::parse(xml).ok()?;
    let providers: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("emailProvider"))
        .collect();
    let provider = match providers.as_slice() {
        [] => return None,
        [only] => *only,
        several => *several
            .iter()
            .find(|p| p.attribute("id") == Some(domain.as_str()))?,
    };
    let server = |tag: &str, kind: &str| {
        provider
            .children()
            .find(|n| n.has_tag_name(tag) && n.attribute("type") == Some(kind))
    };
    let (incoming, outgoing) = (
        server("incomingServer", "imap")?,
        server("outgoingServer", "smtp")?,
    );
    let child = |n: roxmltree::Node, tag: &str| {
        n.children()
            .find(|c| c.has_tag_name(tag))
            .and_then(|c| c.text())
            .map(|t| t.trim().to_owned())
    };
    let local = email.split('@').next().unwrap_or("").to_owned();
    let mut out = Map::new();
    for (protocol, node) in [("imap", incoming), ("smtp", outgoing)] {
        let host = child(node, "hostname")
            .filter(|h| !h.is_empty())
            .map(|h| h.replace("%EMAILDOMAIN%", &domain))
            .unwrap_or_else(|| format!("{protocol}.{domain}"));
        let port = child(node, "port").and_then(|p| p.parse::<u16>().ok());
        let username = match child(node, "username").as_deref() {
            Some("%EMAILLOCALPART%") => local.clone(),
            _ => email.to_owned(),
        };
        // A `plain` server is tried with STARTTLS: discovery never picks cleartext.
        let security = match child(node, "socketType").as_deref() {
            Some("SSL") => SECURITY_TLS,
            _ => SECURITY_STARTTLS,
        };
        insert_server(
            &mut out,
            protocol,
            host,
            port,
            Some(username),
            security,
            false,
        );
    }
    Some(out)
}

/// Whether the account authenticates with OAuth (`Account.usesOAuth` on the desktop).
pub fn uses_oauth(provider: &str) -> bool {
    matches!(provider, "gmail" | "office365" | "outlook")
}

/// The settings for a new account with this address and provider preset (`imap`, `gmail`,
/// `yahoo`, …), from the first source that knows it.
pub fn discover(email: &str, provider: &str, net: &mut dyn Lookups) -> Discovered {
    let domain = email_domain(email);
    // Domains first, so a known one costs no DNS lookup; the desktop checked each matcher's
    // domains and MX hosts together, which differs only when one provider's MX pattern
    // matches the hosts of a domain another provider lists.
    let matched = matcher_for(&domain, &[]).or_else(|| matcher_for(&domain, &net.mx(&domain)));
    if let Some(m) = matched {
        return Discovered {
            source: Source::Matcher,
            template: Some(m.id.clone()),
            settings: matcher_settings(m, email),
        };
    }
    let entry = provider_entry(&domain, provider);
    if !uses_oauth(provider) {
        for url in autoconfig_urls(&domain) {
            let Some(body) = net.fetch(&url) else {
                continue;
            };
            // A file that names no IMAP and SMTP server ends the autoconfig step, as on
            // the desktop: the second URL is tried only when the first doesn't answer.
            if let Some(mut settings) = autoconfig_settings(&body, email) {
                // Autoconfig describes servers only; a Proton Bridge account still needs the
                // table's container folder.
                let folder = entry
                    .and_then(|(_, e)| e.get("container_folder").cloned())
                    .unwrap_or_else(|| json!(""));
                settings.insert("container_folder".into(), folder);
                return Discovered {
                    source: Source::Autoconfig,
                    template: None,
                    settings,
                };
            }
            break;
        }
    }
    match entry {
        Some((key, e)) => Discovered {
            source: Source::ProviderTable,
            template: Some(key),
            settings: provider_settings(e, email),
        },
        None => Discovered {
            source: Source::Fallback,
            template: None,
            settings: fallback_settings(email),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Net {
        mx: Vec<String>,
        pages: HashMap<String, String>,
        fetched: Vec<String>,
    }

    impl Lookups for Net {
        fn mx(&mut self, _domain: &str) -> Vec<String> {
            self.mx.clone()
        }
        fn fetch(&mut self, url: &str) -> Option<String> {
            self.fetched.push(url.to_owned());
            self.pages.get(url).cloned()
        }
    }

    fn s(d: &Discovered, k: &str) -> Value {
        d.settings.get(k).cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn the_data_holds_no_cleartext() {
        let text = include_str!("../data/providers.json");
        assert!(!text.to_ascii_lowercase().contains("\"none\""));
        for m in matchers() {
            for server in [&m.imap, &m.smtp] {
                assert!(
                    [SECURITY_TLS, SECURITY_STARTTLS].contains(&server.security.as_str()),
                    "{}",
                    m.id
                );
            }
        }
        for (key, entry) in &tables().domains {
            for p in ["imap", "smtp"] {
                if let Some(v) = entry.get(format!("{p}_security")) {
                    assert!(v == SECURITY_TLS || v == SECURITY_STARTTLS, "{key}");
                }
            }
        }
    }

    #[test]
    fn dead_entries_are_gone() {
        for id in [
            "facebook",
            "openmailbox",
            "mobileme",
            "default",
            "insecure-default",
        ] {
            assert!(matchers().iter().all(|m| m.id != id), "{id}");
        }
        assert!(provider_entry("openmailbox.org", "imap").is_none());
        let mut net = Net::default();
        assert_eq!(
            discover("x@fb.com", "imap", &mut net).source,
            Source::Fallback
        );
    }

    #[test]
    fn gmail_by_domain_and_by_mx() {
        let mut net = Net::default();
        let d = discover("ben@gmail.com", "gmail", &mut net);
        assert_eq!(d.source, Source::Matcher);
        assert_eq!(d.template.as_deref(), Some("gmail"));
        assert_eq!(s(&d, "imap_host"), "imap.gmail.com");
        assert_eq!(s(&d, "smtp_port"), 587);
        assert_eq!(s(&d, "smtp_security"), SECURITY_STARTTLS);
        assert_eq!(s(&d, "imap_username"), "ben@gmail.com");
        let mut net = Net {
            mx: vec!["aspmx.l.google.com".into()],
            ..Default::default()
        };
        let d = discover("me@foundry376.com", "gmail", &mut net);
        assert_eq!(d.template.as_deref(), Some("gmail"));
        assert!(net.fetched.is_empty(), "no autoconfig for OAuth");
    }

    #[test]
    fn mx_patterns_are_anchored() {
        let mut net = Net {
            mx: vec!["evilaspmx.l.google.com.example".into()],
            ..Default::default()
        };
        assert_eq!(
            discover("a@corp.example", "gmail", &mut net).source,
            Source::ProviderTable,
            "falls through to the gmail preset"
        );
    }

    #[test]
    fn excluded_domains_skip_the_matcher() {
        let m = matcher_for("yahoo.co.jp", &[]).unwrap();
        assert_eq!(m.id, "yahoo.co.jp");
        assert_eq!(matcher_for("yahoo.de", &[]).unwrap().id, "yahoo");
    }

    #[test]
    fn icloud_and_proton_presets() {
        let mut net = Net::default();
        let d = discover("jane@icloud.com", "imap", &mut net);
        assert_eq!(d.template.as_deref(), Some("icloud"));
        assert_eq!(s(&d, "imap_host"), "imap.mail.me.com");
        let d = discover("me@proton.me", "protonmail", &mut net);
        assert_eq!(d.source, Source::ProviderTable);
        assert_eq!(s(&d, "imap_host"), "127.0.0.1");
        assert_eq!(s(&d, "imap_security"), SECURITY_STARTTLS);
        assert_eq!(s(&d, "imap_allow_insecure_ssl"), true);
        assert_eq!(s(&d, "container_folder"), "Folders");
    }

    #[test]
    fn provider_table_formats_usernames_and_ports() {
        let mut net = Net::default();
        let d = discover("ivan@alumnos.upm.es", "imap", &mut net);
        assert_eq!(d.template.as_deref(), Some("alumnos.upm.es"));
        assert_eq!(s(&d, "imap_host"), "correo.alumnos.upm.es");
        assert_eq!(s(&d, "imap_username"), "ivan");
        assert!(s(&d, "imap_port").is_u64());
        // A POP-only provider has no IMAP host; the client asks for it.
        let d = discover("x@abc.plala.or.jp", "imap", &mut net);
        assert_eq!(s(&d, "imap_host"), "");
        assert_eq!(s(&d, "smtp_security"), SECURITY_STARTTLS);
    }

    const AUTOCONFIG: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="other.test"><incomingServer type="imap"><hostname>wrong</hostname></incomingServer></emailProvider>
  <emailProvider id="corp.test">
    <incomingServer type="pop3"><hostname>pop.corp.test</hostname><port>995</port></incomingServer>
    <incomingServer type="imap">
      <hostname>mail.%EMAILDOMAIN%</hostname><port>143</port><socketType>plain</socketType>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.corp.test</hostname><port>465</port><socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn autoconfig_never_picks_cleartext() {
        let mut net = Net::default();
        net.pages.insert(
            "https://corp.test/.well-known/autoconfig/mail/config-v1.1.xml".into(),
            AUTOCONFIG.into(),
        );
        let d = discover("ann@corp.test", "imap", &mut net);
        assert_eq!(d.source, Source::Autoconfig);
        assert_eq!(s(&d, "imap_host"), "mail.corp.test");
        assert_eq!(s(&d, "imap_port"), 143);
        assert_eq!(s(&d, "imap_security"), SECURITY_STARTTLS);
        assert_eq!(s(&d, "imap_username"), "ann");
        assert_eq!(s(&d, "smtp_security"), SECURITY_TLS);
        assert_eq!(s(&d, "smtp_username"), "ann@corp.test");
        assert_eq!(s(&d, "container_folder"), "");
        assert_eq!(net.fetched.len(), 2);
    }

    #[test]
    fn a_file_without_imap_ends_autoconfig() {
        let mut net = Net::default();
        net.pages.insert(
            "https://autoconfig.corp.test/mail/config-v1.1.xml".into(),
            "<clientConfig><emailProvider id=\"corp.test\"/></clientConfig>".into(),
        );
        let d = discover("ann@corp.test", "imap", &mut net);
        assert_eq!(d.source, Source::Fallback);
        assert_eq!(net.fetched.len(), 1);
        assert_eq!(s(&d, "imap_host"), "imap.corp.test");
        assert_eq!(s(&d, "imap_security"), SECURITY_TLS);
        assert!(autoconfig_settings("not xml", "a@b.c").is_none());
    }

    #[test]
    fn securities() {
        assert_eq!(discovered_security("imap", None, 143), SECURITY_STARTTLS);
        assert_eq!(
            discovered_security("smtp", Some("none"), 25),
            SECURITY_STARTTLS
        );
        assert_eq!(
            discovered_security("imap", Some("bogus"), 993),
            SECURITY_TLS
        );
        assert_eq!(discovered_security("imap", None, 1143), SECURITY_STARTTLS);
        assert_eq!(security_for_port("smtp", 2525), None);
    }
}
