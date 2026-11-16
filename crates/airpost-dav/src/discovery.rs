// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! CalDAV/CardDAV service discovery (RFC 6764): the DAV host from DNS SRV/TXT or a table of
//! known providers, then `.well-known`, current-user-principal, the home set and the
//! collections. Derived from Mailspring-Sync `MailSync/DAVWorker.cpp`
//! (`resolveAddressBook`, `resolveCalendarHomeURL`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! The C++ engine asked the Mailspring identity server for the hosts
//! (`/api/resolve-dav-hosts` with the email domain and IMAP host). Here the SRV records of
//! the email domain are read directly, and the providers that publish none are
//! a table.

use crate::client::{AddressBookInfo, CalendarHome, DavClient};
use crate::href::{normalize_href, resolve};
use crate::requests;
use crate::xml::ns;
use airpost_core::{Result, SyncError};
use hickory_resolver::net::NetError;
use hickory_resolver::proto::rr::RData;
use hickory_resolver::TokioResolver;
use std::time::Duration;
use tracing::info;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DavService {
    CalDav,
    CardDav,
}

impl DavService {
    fn name(self) -> &'static str {
        match self {
            DavService::CalDav => "caldav",
            DavService::CardDav => "carddav",
        }
    }

    /// `/.well-known/caldav` or `/.well-known/carddav` (RFC 6764 §5).
    pub fn well_known_path(self) -> String {
        format!("/.well-known/{}", self.name())
    }
}

/// Where a DAV service lives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DavHost {
    pub host: String,
    /// `None` for the scheme's default port.
    pub port: Option<u16>,
    /// The context path from a TXT `path=` record (RFC 6764 §4), which replaces the
    /// `.well-known` probe.
    pub context_path: Option<String>,
}

impl DavHost {
    pub fn new(host: &str) -> DavHost {
        DavHost {
            host: host.trim_end_matches('.').to_ascii_lowercase(),
            ..Default::default()
        }
    }

    /// `host` or `host:port`.
    pub fn authority(&self) -> String {
        match self.port {
            Some(p) => format!("{}:{p}", self.host),
            None => self.host.clone(),
        }
    }
}

/// Finds the DAV host for an account, standing in for `/api/resolve-dav-hosts`.
pub trait HostResolver {
    fn dav_host(
        &self,
        service: DavService,
        email_domain: &str,
        imap_host: &str,
    ) -> Result<Option<DavHost>>;
}

/// The domain of an email address (everything after the first `@`, as the C++ took it).
pub fn email_domain(email: &str) -> &str {
    email.split_once('@').map_or("", |(_, d)| d)
}

/// Providers that serve DAV without SRV records on their mail domains, by IMAP host or
/// email domain, from each provider's published client settings.
const KNOWN_PROVIDERS: &[KnownProvider] = &[
    KnownProvider {
        imap_hosts: &["imap.mail.me.com"],
        domains: &["icloud.com", "me.com", "mac.com"],
        caldav: "caldav.icloud.com",
        carddav: "contacts.icloud.com",
        port: None,
    },
    KnownProvider {
        imap_hosts: &["imap.mail.yahoo.com", "export.imap.mail.yahoo.com"],
        domains: &["yahoo.com", "ymail.com", "rocketmail.com"],
        caldav: "caldav.calendar.yahoo.com",
        carddav: "carddav.address.yahoo.com",
        port: None,
    },
    KnownProvider {
        imap_hosts: &["imap.fastmail.com"],
        domains: &["fastmail.com", "fastmail.fm"],
        caldav: "caldav.fastmail.com",
        carddav: "carddav.fastmail.com",
        port: None,
    },
    KnownProvider {
        imap_hosts: &["imap.gmx.net", "imap.gmx.com"],
        domains: &["gmx.net", "gmx.de", "gmx.at", "gmx.ch", "gmx.com"],
        caldav: "caldav.gmx.net",
        carddav: "carddav.gmx.net",
        port: None,
    },
    KnownProvider {
        imap_hosts: &["posteo.de"],
        domains: &["posteo.de", "posteo.net"],
        caldav: "posteo.de",
        carddav: "posteo.de",
        port: Some(8443),
    },
    KnownProvider {
        imap_hosts: &["imap.mailbox.org"],
        domains: &["mailbox.org"],
        caldav: "dav.mailbox.org",
        carddav: "dav.mailbox.org",
        port: None,
    },
    KnownProvider {
        imap_hosts: &["imap.yandex.com", "imap.yandex.ru"],
        domains: &["yandex.com", "yandex.ru"],
        caldav: "caldav.yandex.ru",
        carddav: "carddav.yandex.ru",
        port: None,
    },
    KnownProvider {
        imap_hosts: &["imap.zoho.com", "imappro.zoho.com"],
        domains: &["zoho.com", "zohomail.com"],
        caldav: "calendar.zoho.com",
        carddav: "contacts.zoho.com",
        port: None,
    },
];

struct KnownProvider {
    imap_hosts: &'static [&'static str],
    domains: &'static [&'static str],
    caldav: &'static str,
    carddav: &'static str,
    port: Option<u16>,
}

/// The table lookup alone. Yahoo also matches its regional domains (`yahoo.co.uk`, …).
pub fn known_provider_host(
    service: DavService,
    email_domain: &str,
    imap_host: &str,
) -> Option<DavHost> {
    let domain = email_domain.to_ascii_lowercase();
    let imap = imap_host.to_ascii_lowercase();
    let p = KNOWN_PROVIDERS.iter().find(|p| {
        p.imap_hosts.contains(&imap.as_str())
            || p.domains.contains(&domain.as_str())
            || (p.caldav.ends_with("yahoo.com") && domain.starts_with("yahoo."))
    })?;
    let host = match service {
        DavService::CalDav => p.caldav,
        DavService::CardDav => p.carddav,
    };
    Some(DavHost {
        port: p.port,
        ..DavHost::new(host)
    })
}

/// SRV and TXT through the system resolver, then [`known_provider_host`].
///
/// Each lookup runs on a current-thread tokio runtime of its own, on a separate thread
/// when the caller is already inside one.
#[derive(Clone, Copy, Debug, Default)]
pub struct DnsResolver;

/// The whole budget for one DNS question.
pub const DNS_TIMEOUT: Duration = Duration::from_secs(10);

/// One SRV answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SrvRecord {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// The SRV record to use (RFC 2782): lowest priority, then highest weight. A target of
/// `.` means the service is decidedly not available.
pub fn pick_srv(records: &[SrvRecord]) -> Option<&SrvRecord> {
    let best = records
        .iter()
        .min_by(|a, b| a.priority.cmp(&b.priority).then(b.weight.cmp(&a.weight)))?;
    (best.target != "." && !best.target.is_empty()).then_some(best)
}

/// The context path in TXT strings (`path=/dav/`), RFC 6764 §4.
pub fn txt_context_path(strings: &[String]) -> Option<String> {
    strings
        .iter()
        .find_map(|s| s.strip_prefix("path="))
        .map(str::trim)
        .filter(|p| p.starts_with('/'))
        .map(String::from)
}

/// The budget for the MX lookup of account discovery, which the user waits on
/// (docs/tech-debt.md TD47).
pub const MX_TIMEOUT: Duration = Duration::from_secs(5);

impl DnsResolver {
    fn run<T: Send>(
        &self,
        name: &str,
        f: impl AsyncFnOnce(&TokioResolver) -> std::result::Result<T, NetError> + Send,
    ) -> Result<Option<T>> {
        self.run_within(name, DNS_TIMEOUT, f)
    }

    fn run_within<T: Send>(
        &self,
        name: &str,
        timeout: Duration,
        f: impl AsyncFnOnce(&TokioResolver) -> std::result::Result<T, NetError> + Send,
    ) -> Result<Option<T>> {
        let outcome = crate::runtime::outside_runtime(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(airpost_core::Error::from)?;
            Ok::<_, airpost_core::Error>(rt.block_on(async {
                let resolver = TokioResolver::builder_tokio()?.build()?;
                match tokio::time::timeout(timeout, f(&resolver)).await {
                    Ok(r) => r,
                    Err(_) => Err(NetError::Timeout),
                }
            }))
        })?;
        match outcome {
            Ok(v) => Ok(Some(v)),
            Err(NetError::Timeout | NetError::Io(_) | NetError::NoConnections) => {
                Err(SyncError::offline("Could not resolve hostname", name).into())
            }
            Err(_) => Ok(None),
        }
    }

    pub fn srv(&self, name: &str) -> Result<Vec<SrvRecord>> {
        let records = self.run(name, async |r| {
            let lookup = r.srv_lookup(name).await?;
            Ok(lookup
                .answers()
                .iter()
                .filter_map(|rec| match &rec.data {
                    RData::SRV(srv) => Some(SrvRecord {
                        priority: srv.priority,
                        weight: srv.weight,
                        port: srv.port,
                        target: srv.target.to_utf8(),
                    }),
                    _ => None,
                })
                .collect::<Vec<_>>())
        })?;
        Ok(records.unwrap_or_default())
    }

    /// The domain's MX hosts, lowercased and without the trailing dot, most preferred first;
    /// empty when there are none or the lookup failed within [`MX_TIMEOUT`].
    pub fn mx(&self, domain: &str) -> Vec<String> {
        let name = format!("{}.", domain.trim_end_matches('.'));
        let records = self.run_within(&name, MX_TIMEOUT, async |r| {
            let lookup = r.mx_lookup(name.as_str()).await?;
            let mut hosts: Vec<(u16, String)> = lookup
                .answers()
                .iter()
                .filter_map(|rec| match &rec.data {
                    RData::MX(mx) => Some((
                        mx.preference,
                        mx.exchange
                            .to_utf8()
                            .trim_end_matches('.')
                            .to_ascii_lowercase(),
                    )),
                    _ => None,
                })
                .collect();
            hosts.sort();
            Ok(hosts.into_iter().map(|(_, h)| h).collect::<Vec<_>>())
        });
        records.ok().flatten().unwrap_or_default()
    }

    pub fn txt(&self, name: &str) -> Result<Vec<String>> {
        let strings = self.run(name, async |r| {
            let lookup = r.txt_lookup(name).await?;
            Ok(lookup
                .answers()
                .iter()
                .filter_map(|rec| match &rec.data {
                    RData::TXT(txt) => Some(
                        txt.txt_data
                            .iter()
                            .map(|b| String::from_utf8_lossy(b).into_owned())
                            .collect::<String>(),
                    ),
                    _ => None,
                })
                .collect::<Vec<_>>())
        })?;
        Ok(strings.unwrap_or_default())
    }

    fn srv_host(&self, service: DavService, domain: &str) -> Result<Option<DavHost>> {
        // `_caldavs` first: RFC 6764 §3 prefers TLS.
        for (prefix, default_port) in [("s", 443), ("", 80)] {
            let name = format!("_{}{prefix}._tcp.{domain}.", service.name());
            let records = self.srv(&name)?;
            if let Some(best) = pick_srv(&records) {
                let mut host = DavHost::new(&best.target);
                if best.port != default_port && best.port != 443 {
                    host.port = Some(best.port);
                }
                host.context_path = txt_context_path(&self.txt(&name)?);
                return Ok(Some(host));
            }
        }
        Ok(None)
    }
}

/// Replace the SRV lookup with a fixed `host[:port]`; discovery then runs from
/// `.well-known`. The harness points these at Radicale and Baikal
/// (apps/engine/test/README.md, `harness/servers/dav.py`).
pub const ENV_CALDAV_HOST: &str = "AIRPOST_ENGINE_CALDAV_HOST";
pub const ENV_CARDDAV_HOST: &str = "AIRPOST_ENGINE_CARDDAV_HOST";

/// Parses `host[:port]`. A port that doesn't parse is left in the host, so the request
/// fails visibly rather than going to the default port.
pub fn parse_authority(authority: &str) -> DavHost {
    match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => match p.parse::<u16>() {
            Ok(port) => DavHost {
                port: Some(port),
                ..DavHost::new(h)
            },
            Err(_) => DavHost::new(authority),
        },
        _ => DavHost::new(authority),
    }
}

/// The `AIRPOST_ENGINE_CALDAV_HOST` / `AIRPOST_ENGINE_CARDDAV_HOST` override for `service`, when set.
pub fn env_host_override(service: DavService) -> Option<DavHost> {
    let key = match service {
        DavService::CalDav => ENV_CALDAV_HOST,
        DavService::CardDav => ENV_CARDDAV_HOST,
    };
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| parse_authority(v.trim()))
}

impl HostResolver for DnsResolver {
    fn dav_host(
        &self,
        service: DavService,
        email_domain: &str,
        imap_host: &str,
    ) -> Result<Option<DavHost>> {
        if let Some(h) = env_host_override(service) {
            return Ok(Some(h));
        }
        if !email_domain.is_empty() {
            if let Some(h) = self.srv_host(service, email_domain)? {
                return Ok(Some(h));
            }
        }
        Ok(known_provider_host(service, email_domain, imap_host))
    }
}

impl DavClient {
    /// The service root: the TXT context path when DNS gave one, else the target of the
    /// `.well-known` redirect over https, then http; when neither redirects, or the
    /// redirect points back at `.well-known` (a loop, or an upgrade to https), the host
    /// root over https.
    pub fn service_root(&mut self, service: DavService, host: &DavHost) -> String {
        let authority = host.authority();
        if let Some(path) = &host.context_path {
            return format!("https://{authority}{path}");
        }
        let well_known = service.well_known_path();
        let mut root = self.expected_redirect(&format!("https://{authority}{well_known}"));
        if root.is_empty() {
            root = self.expected_redirect(&format!("http://{authority}{well_known}"));
        }
        if root.is_empty() || root.contains("/.well-known") {
            root = format!("https://{authority}/");
        }
        root
    }

    fn principal_url(&mut self, root: &str) -> Result<String> {
        let doc = self.xml_request(
            root,
            "PROPFIND",
            requests::PROPFIND_PRINCIPAL,
            requests::DEPTH_0,
        )?;
        Ok(doc
            .last_prop_href(ns::DAV, "current-user-principal")
            .unwrap_or("")
            .to_string())
    }

    /// `resolveAddressBook` past the host lookup: the address books under the account's
    /// home set, or `None` when the chain ends on Yahoo's webmail. Errors with 404, 405 or
    /// 406 mean the server has no CardDAV.
    ///
    /// The C++ engine kept the last address book listed (`TODO: Pick the primary one`).
    pub fn discover_address_books(
        &mut self,
        host: &DavHost,
    ) -> Result<Option<Vec<AddressBookInfo>>> {
        let root = self.service_root(DavService::CardDav, host);
        if self.profile_for(&root).carddav_webmail_root == Some(root.as_str()) {
            return Ok(None);
        }
        // An empty principal resolves to the root, as the C++ did: some servers answer the
        // home-set query there.
        let principal = resolve(&root, &self.principal_url(&root)?);
        let home_doc = self.xml_request(
            &principal,
            "PROPFIND",
            requests::PROPFIND_ADDRESSBOOK_HOME_SET,
            requests::DEPTH_0,
        )?;
        let home = resolve(
            &root,
            home_doc
                .last_prop_href(ns::CARDDAV, "addressbook-home-set")
                .unwrap_or(""),
        );
        let books = self.xml_request(
            &home,
            "PROPFIND",
            requests::PROPFIND_ADDRESSBOOKS,
            requests::DEPTH_1,
        )?;
        Ok(Some(
            books
                .responses
                .iter()
                .filter(|r| r.is_address_book())
                .map(|r| AddressBookInfo {
                    url: resolve(&home, &r.href),
                    ctag: r.ctag().unwrap_or("").to_string(),
                    name: r.display_name().unwrap_or("").to_string(),
                })
                .collect(),
        ))
    }

    /// `resolveCalendarHomeURL` past the host lookup: the calendar home-set, or `None` when
    /// the server names no principal or home set. Errors with 404, 405 or 406 mean the
    /// server has no CalDAV.
    pub fn discover_calendar_home(&mut self, host: &DavHost) -> Result<Option<CalendarHome>> {
        let root = self.service_root(DavService::CalDav, host);
        let principal = self.principal_url(&root)?;
        if principal.is_empty() {
            info!("CalDAV: server returned no current-user-principal, skipping calendar discovery");
            return Ok(None);
        }
        let principal = resolve(&root, &principal);
        // Depth 0: at Depth 1 Nextcloud also answers for the principal's
        // calendar-proxy-read/-write children, and the last home set would win.
        let doc = self.xml_request(
            &principal,
            "PROPFIND",
            requests::PROPFIND_CALENDAR_HOME_SET,
            requests::DEPTH_0,
        )?;
        let home = doc
            .last_prop_href(ns::CALDAV, "calendar-home-set")
            .unwrap_or("");
        if home.is_empty() {
            info!("CalDAV: server returned no calendar-home-set, skipping calendar discovery");
            return Ok(None);
        }
        Ok(Some(CalendarHome {
            url: resolve(&root, home),
            principal_path: normalize_href(&principal),
            fixed_host: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_override() {
        let h = parse_authority("127.0.0.1:5232");
        assert_eq!((h.host.as_str(), h.port), ("127.0.0.1", Some(5232)));
        assert_eq!(h.authority(), "127.0.0.1:5232");
        let h = parse_authority("Dav.Example.com");
        assert_eq!((h.host.as_str(), h.port), ("dav.example.com", None));
        assert_eq!(parse_authority("h:x").port, None);
    }

    #[test]
    fn srv_selection() {
        let r = |p, w, t: &str| SrvRecord {
            priority: p,
            weight: w,
            port: 443,
            target: t.into(),
        };
        let recs = vec![r(10, 5, "b."), r(0, 1, "a."), r(0, 9, "c.")];
        assert_eq!(pick_srv(&recs).unwrap().target, "c.");
        assert_eq!(pick_srv(&[r(0, 0, ".")]), None);
        assert_eq!(pick_srv(&[]), None);
    }

    #[test]
    fn txt_paths() {
        assert_eq!(
            txt_context_path(&["v=1".into(), "path=/dav/".into()]),
            Some("/dav/".into())
        );
        assert_eq!(txt_context_path(&["path=relative".into()]), None);
    }

    #[test]
    fn known_hosts() {
        let h = known_provider_host(DavService::CardDav, "icloud.com", "imap.mail.me.com").unwrap();
        assert_eq!(h.authority(), "contacts.icloud.com");
        let h = known_provider_host(DavService::CalDav, "example.com", "imap.mail.me.com").unwrap();
        assert_eq!(h.authority(), "caldav.icloud.com");
        let h = known_provider_host(DavService::CalDav, "yahoo.co.uk", "").unwrap();
        assert_eq!(h.authority(), "caldav.calendar.yahoo.com");
        let h = known_provider_host(DavService::CalDav, "posteo.de", "").unwrap();
        assert_eq!(h.authority(), "posteo.de:8443");
        assert!(
            known_provider_host(DavService::CalDav, "example.com", "imap.example.com").is_none()
        );
        assert_eq!(email_domain("a@b.example"), "b.example");
        assert_eq!(email_domain("nobody"), "");
    }
}
