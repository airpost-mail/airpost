// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The CalDAV/CardDAV client the workers (P5) drive. Derived from the protocol half of
//! Mailspring-Sync `MailSync/DAVWorker.cpp` (`performXMLRequest`, `performVCardRequest`,
//! `performICSRequest`, `getAuthorizationHeader`, the sync-collection and multiget loops,
//! the calendar listing), Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! The client never touches the store: each call is one protocol step and returns what the
//! server said, and the worker decides what to save. Errors keep the C++ shapes:
//! `Invalid Response Code: <status>` with `"<url> RETURNED <body>"` as debuginfo, so task
//! errors reach the client unchanged; [`http_status`] reads the status back.

use crate::href::{host_of, normalize_href, with_scheme};
use crate::profile::{DavProfile, ServerHints};
use crate::ratelimit::RateLimiter;
use crate::requests::{self, CalendarSyncRange, Collection, MAX_SYNC_PAGES};
use crate::transport::{HttpRequest, HttpResponse, Transport, REDIRECT_PROBE_TIMEOUT};
use crate::xml::{ns, DavResponse, Multistatus};
use base64::Engine;
use ms_core::{Account, Clock, Error, Result, SyncError};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// The error key for a write refused with 412 Precondition Failed.
pub const ETAG_CONFLICT: &str = "etag-conflict";

/// The HTTP status of an `Invalid Response Code: <status>` error.
pub fn http_status(err: &Error) -> Option<u16> {
    err.as_sync()?
        .key
        .strip_prefix("Invalid Response Code: ")?
        .trim()
        .parse()
        .ok()
}

fn status_error(resp: &HttpResponse) -> Result<()> {
    ms_net::http::check_status(resp.status, &resp.url, &resp.body)
}

/// Supplies a current OAuth access token. Called before every request, as
/// `getAuthorizationHeader` asked the token manager each time; the provider caches the token
/// and refreshes it near expiry.
pub type TokenProvider = Arc<dyn Fn() -> Result<String> + Send + Sync>;

/// How requests authenticate. Credentials go with every request rather than after a 401
/// challenge: servers format `WWW-Authenticate` inconsistently, and it saves a round trip.
#[derive(Clone)]
pub enum DavAuth {
    Bearer(TokenProvider),
    Basic { username: String, password: String },
    None,
}

impl std::fmt::Debug for DavAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DavAuth::Bearer(_) => f.write_str("Bearer(…)"),
            DavAuth::Basic { username, .. } => write!(f, "Basic({username}, …)"),
            DavAuth::None => f.write_str("None"),
        }
    }
}

impl DavAuth {
    /// `getAuthorizationHeader`: OAuth Bearer when the account has a refresh token,
    /// otherwise Basic with the IMAP credentials. `tokens` is only kept in the first case.
    pub fn for_account(
        account: &Account,
        tokens: impl Fn() -> Result<String> + Send + Sync + 'static,
    ) -> DavAuth {
        if !account.refresh_token().is_empty() {
            return DavAuth::bearer(tokens);
        }
        DavAuth::Basic {
            username: account.imap_username().to_string(),
            password: account.imap_password().to_string(),
        }
    }

    pub fn bearer(tokens: impl Fn() -> Result<String> + Send + Sync + 'static) -> DavAuth {
        DavAuth::Bearer(Arc::new(tokens))
    }

    /// A Bearer token that never changes, for tests and short-lived clients.
    pub fn fixed_bearer(token: impl Into<String>) -> DavAuth {
        let token = token.into();
        DavAuth::bearer(move || Ok(token.clone()))
    }

    /// The `Authorization` value; fails when the token provider does.
    pub fn header_value(&self) -> Result<Option<String>> {
        Ok(match self {
            DavAuth::Bearer(tokens) => Some(format!("Bearer {}", tokens()?)),
            DavAuth::Basic { username, password } => Some(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
            )),
            DavAuth::None => None,
        })
    }
}

/// One member of a collection listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HrefEtag {
    pub href: String,
    pub etag: String,
}

/// A resource returned with its data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DavObject {
    pub href: String,
    pub etag: String,
    pub data: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MultigetResult {
    pub objects: Vec<DavObject>,
    /// `<response>` elements seen, including those without data. Zero for a non-empty
    /// request means the server's answer was malformed or empty.
    pub responses: usize,
}

/// One entry of a sync-collection report (RFC 6578 §3.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncChange {
    /// A response-level 404: the member was removed.
    Deleted { href: String },
    /// Changed, with its data (incremental syncs ask for it).
    Data {
        href: String,
        etag: String,
        data: String,
    },
    /// Changed, etag only (the initial sync); fetch it by multiget.
    Needed { href: String, etag: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncCollectionResult {
    /// The newest token returned, else the one sent.
    pub sync_token: String,
    pub pages: usize,
    /// [`MAX_SYNC_PAGES`] pages were read and the server still reported truncation.
    pub hit_page_limit: bool,
    pub changes: Vec<SyncChange>,
}

impl SyncCollectionResult {
    pub fn deleted_hrefs(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter_map(|c| match c {
                SyncChange::Deleted { href } => Some(href.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn needed_hrefs(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter_map(|c| match c {
                SyncChange::Needed { href, .. } => Some(href.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn data(&self) -> Vec<DavObject> {
        self.changes
            .iter()
            .filter_map(|c| match c {
                SyncChange::Data { href, etag, data } => Some(DavObject {
                    href: href.clone(),
                    etag: etag.clone(),
                    data: data.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// No changes and no token: Yahoo CalDAV never returns a `<sync-token>`, so an empty
    /// answer says nothing and the caller falls back to the etag listing.
    pub fn is_empty_without_token(&self) -> bool {
        self.changes.is_empty() && self.sync_token.is_empty()
    }
}

/// How a sync-collection attempt ended.
#[derive(Debug)]
pub enum SyncCollection {
    Complete(SyncCollectionResult),
    /// 403, 409 or 410, or a `valid-sync-token` precondition: the token is no longer valid.
    /// The caller clears it and retries once from scratch, then falls back to the etag
    /// listing.
    TokenRejected(Error),
    /// Any other failure, or a server known to lack sync-collection (`None`): use the etag
    /// listing (Robur and GMX lack it; Zimbra and Posteo refuse it with HTTP errors).
    Unsupported(Option<Error>),
}

/// `DELETE` outcome. A 404 or 410 means the resource is already gone, which the caller
/// treats as success when the href was the server's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    AlreadyGone(u16),
}

/// What a PUT returned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteResult {
    pub status: u16,
    /// The new ETag when the server sent one (RFC 4791 §5.3.4 lets it withhold it).
    pub etag: Option<String>,
    pub location: Option<String>,
}

/// Resource bodies the client writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    VCard,
    ICalendar,
}

/// Whether a calendar is the account's own (`calendarOwnership`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ownership {
    Mine,
    Other,
    #[default]
    Unknown,
}

impl Ownership {
    /// The `Calendar.owner` JSON value: `"mine"`, `"other"` or `""`.
    pub fn as_str(self) -> &'static str {
        match self {
            Ownership::Mine => "mine",
            Ownership::Other => "other",
            Ownership::Unknown => "",
        }
    }
}

/// A VEVENT calendar from the home-set listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalendarInfo {
    /// The href as the server wrote it; the calendar id is derived from it.
    pub path: String,
    pub name: String,
    pub ctag: String,
    /// `ical:calendar-color`, empty when absent.
    pub color: String,
    pub description: String,
    /// `ical:calendar-order` read as `stoi` did, `None` when absent or non-numeric.
    pub order: Option<i64>,
    pub read_only: bool,
    /// The server supplied `current-user-privilege-set`.
    pub privileges_advertised: bool,
    pub ownership: Ownership,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalendarListing {
    /// Calendars (resourcetype `caldav:calendar`) supporting VEVENT, in server order.
    pub calendars: Vec<CalendarInfo>,
    /// Every href the listing answered for except the home itself and members that are
    /// positively not calendars. A 207 is per resource (RFC 4918 §9.1), so a member with an
    /// error propstat still counts as listed. The worker prunes local calendars missing here.
    pub listed_paths: Vec<String>,
}

/// The calendar home-set for an account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalendarHome {
    /// The home-set URL (scheme-less for Gmail).
    pub url: String,
    /// The account principal, normalised; empty when the server named none, which leaves
    /// calendar ownership unknown.
    pub principal_path: String,
    /// Set for Gmail: calendar paths are appended to this host.
    pub fixed_host: Option<String>,
}

impl CalendarHome {
    /// Gmail's fixed CalDAV home, `apidata.googleusercontent.com/caldav/v2/<email>`.
    /// The host comes from [`gmail_caldav_host`](crate::profile::gmail_caldav_host).
    pub fn gmail(email: &str) -> CalendarHome {
        let principal = format!("/caldav/v2/{email}");
        let host = crate::profile::gmail_caldav_host();
        CalendarHome {
            url: format!("{host}{principal}"),
            principal_path: normalize_href(&principal),
            fixed_host: Some(host),
        }
    }

    /// The URL of a calendar collection path (`resolvedCalendarURL`).
    pub fn calendar_url(&self, path: &str) -> String {
        if let Some(host) = &self.fixed_host {
            return format!("{host}{path}");
        }
        crate::href::resolve(&self.url, path)
    }
}

/// An address book from discovery.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressBookInfo {
    pub url: String,
    pub ctag: String,
    pub name: String,
}

/// The DAV client for one account, owned by one thread.
pub struct DavClient {
    transport: Box<dyn Transport + Send>,
    auth: DavAuth,
    clock: Arc<dyn Clock>,
    sleep: Box<dyn Fn(Duration) + Send>,
    rate: RateLimiter,
    verbose: bool,
    hints: HashMap<String, ServerHints>,
}

impl DavClient {
    pub fn new(transport: Box<dyn Transport + Send>, auth: DavAuth) -> DavClient {
        DavClient {
            transport,
            auth,
            clock: ms_core::clock::system(),
            sleep: Box::new(std::thread::sleep),
            rate: RateLimiter::default(),
            verbose: false,
            hints: HashMap::new(),
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replaces the rate-limit sleep, for tests.
    pub fn with_sleep(mut self, sleep: Box<dyn Fn(Duration) + Send>) -> Self {
        self.sleep = sleep;
        self
    }

    /// Verbose logging writes request bodies (vCards, ICS) to the log, which otherwise only
    /// gets their sizes: they carry names, attendees and descriptions.
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// Replaces the credentials, e.g. after the user re-enters a password.
    pub fn set_auth(&mut self, auth: DavAuth) {
        self.auth = auth;
    }

    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.rate
    }

    /// The quirk profile for a URL, using what that host has said about itself so far.
    pub fn profile_for(&self, url: &str) -> DavProfile {
        let hints = self.hints.get(&host_of(url)).cloned().unwrap_or_default();
        DavProfile::detect(url, &hints)
    }

    fn send(&mut self, mut req: HttpRequest) -> Result<HttpResponse> {
        req.http1_only = self.profile_for(&req.url).http1_only;
        if let Some(auth) = self.auth.header_value()? {
            req.headers.insert(0, ("Authorization".into(), auth));
        }
        let resp = self.transport.send(&req)?;
        let server = resp.header("Server").map(String::from);
        let dav = resp.header("DAV").map(String::from);
        if server.is_some() || dav.is_some() {
            let entry = self.hints.entry(host_of(&req.url)).or_default();
            entry.server = server.or(entry.server.take());
            entry.dav = dav.or(entry.dav.take());
        }
        Ok(resp)
    }

    fn apply_rate_limit_delay(&mut self) {
        let delay = self.rate.delay(self.clock.now());
        if let Some(d) = delay.retry_after {
            (self.sleep)(d);
        }
        if let Some(d) = delay.backoff {
            (self.sleep)(d);
        }
    }

    /// `performXMLRequest` up to the status check: waits out any rate limit, sends
    /// `Prefer: return-minimal` and the Depth, and records 429/503 for the backoff. No
    /// `Accept` header: `text/vcard` there makes some servers answer raw vCards.
    pub fn xml_exchange(
        &mut self,
        url: &str,
        method: &str,
        body: &str,
        depth: &str,
    ) -> Result<HttpResponse> {
        self.apply_rate_limit_delay();
        let req = HttpRequest {
            method: method.to_string(),
            url: with_scheme(url),
            headers: vec![
                ("Prefer".into(), "return-minimal".into()),
                (
                    "Content-Type".into(),
                    "application/xml; charset=utf-8".into(),
                ),
                ("Depth".into(), depth.to_string()),
            ],
            body: body.to_string(),
            ..Default::default()
        };
        let resp = self.send(req)?;
        if (200..=209).contains(&resp.status) {
            self.rate.record_success();
        } else if resp.status == 429 || resp.status == 503 {
            let now = self.clock.now();
            self.rate
                .record_rate_limited(resp.status, resp.header("Retry-After"), now);
        }
        Ok(resp)
    }

    /// `performXMLRequest`: a PROPFIND or REPORT whose 2xx body is parsed as a multistatus.
    pub fn xml_request(
        &mut self,
        url: &str,
        method: &str,
        body: &str,
        depth: &str,
    ) -> Result<Multistatus> {
        let resp = self.xml_exchange(url, method, body, depth)?;
        status_error(&resp)?;
        Multistatus::parse(&resp.body)
    }

    /// `PerformExpectedRedirect`: the absolute target of a 301/302/303/307/308 answer to a
    /// GET, or `""` for any other answer or failure. RFC 6764 §5 requires following
    /// redirects on `.well-known`; 308 postdates it.
    ///
    /// Sent without credentials, as in the C++: the probe also goes over plain http and to
    /// whatever host answers for the domain.
    pub fn expected_redirect(&mut self, url: &str) -> String {
        let req = HttpRequest {
            method: "GET".into(),
            url: url.to_string(),
            timeout: Some(REDIRECT_PROBE_TIMEOUT),
            http1_only: self.profile_for(url).http1_only,
            ..Default::default()
        };
        let Ok(resp) = self.transport.send(&req) else {
            return String::new();
        };
        if !matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
            return String::new();
        }
        resp.header("Location")
            .and_then(|loc| reqwest::Url::parse(url).ok()?.join(loc).ok())
            .map(String::from)
            .unwrap_or_default()
    }

    /// `performVCardRequest` / `performICSRequest`: a write with an optional `If-Match`
    /// (`If-None-Match: *` when `create_only`). Logs the body only when verbose. The status
    /// is not checked here.
    pub fn write_exchange(
        &mut self,
        kind: ResourceKind,
        url: &str,
        method: &str,
        body: &str,
        etag: &str,
        create_only: bool,
    ) -> Result<HttpResponse> {
        let mut headers = Vec::new();
        match kind {
            ResourceKind::VCard => {
                headers.push(("Content-Type".into(), "text/vcard; charset=utf-8".into()));
                headers.push(("Accept".into(), "text/vcard; version=4.0".into()));
                if self.verbose {
                    info!("{url}: {body} {etag}");
                } else {
                    info!("{url}: {} bytes etag:{etag}", body.len());
                }
            }
            ResourceKind::ICalendar => {
                headers.push(("Content-Type".into(), "text/calendar; charset=utf-8".into()));
                if self.verbose {
                    info!("performICSRequest: {method} {url} etag:{etag}, data: {body}");
                } else {
                    info!(
                        "performICSRequest: {method} {url} etag:{etag}, {} bytes",
                        body.len()
                    );
                }
            }
        }
        if !etag.is_empty() {
            headers.push(("If-Match".into(), etag.to_string()));
        } else if create_only {
            headers.push(("If-None-Match".into(), "*".into()));
        }
        self.send(HttpRequest {
            method: method.to_string(),
            url: with_scheme(url),
            headers,
            body: body.to_string(),
            ..Default::default()
        })
    }

    fn put(
        &mut self,
        kind: ResourceKind,
        url: &str,
        body: &str,
        etag: &str,
        create_only: bool,
    ) -> Result<WriteResult> {
        let resp = self.write_exchange(kind, url, "PUT", body, etag, create_only)?;
        if resp.status == 412 {
            return Err(etag_conflict(kind).into());
        }
        status_error(&resp)?;
        Ok(WriteResult {
            status: resp.status,
            etag: resp.header("ETag").map(String::from),
            location: resp.header("Location").map(String::from),
        })
    }

    /// PUTs an event resource, with `If-Match` when `etag` is set. A 412 is an
    /// `etag-conflict` error. The caller re-reads the resource with
    /// [`DavClient::multiget`] to pick up server changes and the new etag.
    pub fn put_ics(&mut self, url: &str, ics: &str, etag: &str) -> Result<WriteResult> {
        self.put(ResourceKind::ICalendar, url, ics, etag, false)
    }

    /// PUTs a new event resource with `If-None-Match: *`, so a resource the engine has not
    /// synced yet (an invitation the server's scheduling delivered) is never overwritten
    /// (RFC 4791 §5.3.2). A 412 is an `etag-conflict` error. The C++ engine sent no
    /// precondition on creates.
    pub fn create_ics(&mut self, url: &str, ics: &str) -> Result<WriteResult> {
        self.put(ResourceKind::ICalendar, url, ics, "", true)
    }

    /// PUTs an existing card, with `If-Match` when `etag` is set. A 412 is an
    /// `etag-conflict` error.
    pub fn put_vcard(&mut self, url: &str, vcf: &str, etag: &str) -> Result<WriteResult> {
        self.put(ResourceKind::VCard, url, vcf, etag, false)
    }

    /// Creates a card at `href` (from [`new_card_href`]) with `If-None-Match: *`, so an
    /// existing resource is never overwritten (RFC 6352 §6.3.2). The C++ engine POSTed to
    /// the collection, which RFC 6352 doesn't define and sabre/dav (Baikal, Nextcloud) and
    /// Radicale refuse.
    pub fn create_vcard(
        &mut self,
        address_book_url: &str,
        href: &str,
        vcf: &str,
    ) -> Result<WriteResult> {
        let url = crate::href::resolve(address_book_url, href);
        self.put(ResourceKind::VCard, &url, vcf, "", true)
    }

    /// DELETE with `If-Match` when `etag` is set. 404 and 410 are
    /// [`DeleteOutcome::AlreadyGone`]; 412 is `etag-conflict` unless the resource turns out
    /// to be gone.
    pub fn delete(&mut self, kind: ResourceKind, url: &str, etag: &str) -> Result<DeleteOutcome> {
        let resp = self.write_exchange(kind, url, "DELETE", "", etag, false)?;
        match resp.status {
            404 | 410 => Ok(DeleteOutcome::AlreadyGone(resp.status)),
            412 => {
                // If-Match fails when there is no current representation (RFC 9110
                // §13.1.1), so sabre/dav (Baikal, Nextcloud) answer 412 for a resource
                // another client already deleted. Ask whether it still exists.
                if !etag.is_empty() {
                    if let Some(gone @ (404 | 410)) = self.resource_status(url) {
                        info!("{url}: DELETE got 412 but the resource is gone ({gone})");
                        return Ok(DeleteOutcome::AlreadyGone(gone));
                    }
                }
                Err(etag_conflict(kind).into())
            }
            _ => {
                status_error(&resp)?;
                Ok(DeleteOutcome::Deleted)
            }
        }
    }

    /// The status of a Depth 0 getetag PROPFIND on `url`, `None` when nothing answered.
    fn resource_status(&mut self, url: &str) -> Option<u16> {
        self.xml_exchange(
            url,
            "PROPFIND",
            requests::PROPFIND_GETETAG,
            requests::DEPTH_0,
        )
        .ok()
        .map(|r| r.status)
    }

    /// The address book's ctag by Depth 0 PROPFIND (`validateCachedAddressBook`). A 404,
    /// 406 or 410 means the cached URL is gone; the caller rediscovers.
    pub fn address_book_ctag(&mut self, url: &str) -> Result<Option<String>> {
        let ms = self.xml_request(url, "PROPFIND", requests::PROPFIND_CTAG, requests::DEPTH_0)?;
        Ok(ms
            .root
            .last_text(ns::CALENDARSERVER, "getctag")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    /// Every card's href and etag (`runForAddressBook`'s addressbook-query).
    pub fn address_book_etags(&mut self, url: &str) -> Result<Vec<HrefEtag>> {
        let ms = self.xml_request(
            url,
            "REPORT",
            requests::ADDRESSBOOK_QUERY_ETAGS,
            requests::DEPTH_1,
        )?;
        Ok(href_etags(&ms))
    }

    /// Every event's href and etag in `range` (`runForCalendar`'s calendar-query). The
    /// server returns any event with an instance in the range.
    pub fn calendar_etags(
        &mut self,
        url: &str,
        range: &CalendarSyncRange,
    ) -> Result<Vec<HrefEtag>> {
        let ms = self.xml_request(
            url,
            "REPORT",
            &requests::calendar_query_etags(range),
            requests::DEPTH_1,
        )?;
        Ok(href_etags(&ms))
    }

    /// One multiget request for `hrefs` (at most [`requests::MULTIGET_CHUNK`]; the caller
    /// chunks so each chunk gets its own transaction). Responses without data are logged
    /// and skipped; for calendars, so are those without an etag.
    pub fn multiget(
        &mut self,
        url: &str,
        kind: Collection,
        hrefs: &[String],
    ) -> Result<MultigetResult> {
        let ms = self.xml_request(
            url,
            "REPORT",
            &requests::multiget(kind, hrefs),
            requests::DEPTH_1,
        )?;
        let mut out = MultigetResult {
            responses: ms.responses.len(),
            ..Default::default()
        };
        for r in &ms.responses {
            let etag = r.etag().unwrap_or("").to_string();
            let data = match kind {
                Collection::AddressBook => r.address_data(),
                Collection::Calendar => r.calendar_data(),
            }
            .unwrap_or("");
            if data.is_empty() || (kind == Collection::Calendar && etag.is_empty()) {
                match kind {
                    Collection::AddressBook => {
                        info!("Received addressbook entry {etag} with an empty body")
                    }
                    Collection::Calendar if !etag.is_empty() => {
                        info!("Received calendar event {etag} with an empty body")
                    }
                    Collection::Calendar => {}
                }
                continue;
            }
            out.objects.push(DavObject {
                href: r.href.clone(),
                etag,
                data: data.to_string(),
            });
        }
        if out.responses == 0 && !hrefs.is_empty() {
            warn!(
                "Multiget for {} hrefs returned 0 D:response nodes - server response may be malformed or empty",
                hrefs.len()
            );
        }
        Ok(out)
    }

    /// RFC 6578 sync-collection from `token` (empty for an initial sync), following 507
    /// truncation over at most [`MAX_SYNC_PAGES`] pages. Each page continues from the token
    /// the previous one returned; every page asks for the same properties as the first.
    ///
    /// A 507 arrives either inside the 207 (a response-level status) or as the HTTP status,
    /// with the partial multistatus as its body; both mean "more pages".
    pub fn sync_collection(&mut self, url: &str, kind: Collection, token: &str) -> SyncCollection {
        if self.profile_for(url).no_sync_collection {
            return SyncCollection::Unsupported(None);
        }
        let initial = token.is_empty();
        let label = match kind {
            Collection::AddressBook => " for contacts",
            Collection::Calendar => "",
        };
        let mut result = SyncCollectionResult {
            sync_token: token.to_string(),
            ..Default::default()
        };
        let mut more = true;
        while more && result.pages < MAX_SYNC_PAGES {
            result.pages += 1;
            let body = requests::sync_collection(kind, &result.sync_token, initial);
            let ms = match self.sync_page(url, &body, label) {
                Ok(page) => page,
                Err(failure) => return failure,
            };
            more = ms.http_507;
            for r in &ms.doc.responses {
                match r.status {
                    Some(507) => {
                        more = true;
                        info!(
                            "sync-collection page {} truncated (507 in response), continuing...",
                            result.pages
                        );
                    }
                    Some(404) => result.changes.push(SyncChange::Deleted {
                        href: r.href.clone(),
                    }),
                    _ => {
                        let etag = r.etag().unwrap_or("").to_string();
                        let data = match kind {
                            Collection::AddressBook => r.address_data(),
                            Collection::Calendar => r.calendar_data(),
                        }
                        .unwrap_or("");
                        if !data.is_empty() {
                            result.changes.push(SyncChange::Data {
                                href: r.href.clone(),
                                etag,
                                data: data.to_string(),
                            });
                        } else if !etag.is_empty() {
                            result.changes.push(SyncChange::Needed {
                                href: r.href.clone(),
                                etag,
                            });
                        }
                    }
                }
            }
            if let Some(t) = &ms.doc.sync_token {
                result.sync_token = t.clone();
            }
        }
        if more {
            result.hit_page_limit = true;
            warn!("sync-collection hit max pages limit ({MAX_SYNC_PAGES}), sync may be incomplete");
        }
        match kind {
            Collection::AddressBook => info!(
                "sync-collection complete ({} pages) for contacts: {} needed, {} deleted",
                result.pages,
                result.needed_hrefs().len(),
                result.deleted_hrefs().len()
            ),
            Collection::Calendar => info!(
                "sync-collection complete ({} pages): {} direct, {} needed, {} deleted, token={}",
                result.pages,
                result.data().len(),
                result.needed_hrefs().len(),
                result.deleted_hrefs().len(),
                result.sync_token
            ),
        }
        SyncCollection::Complete(result)
    }

    fn sync_page(
        &mut self,
        url: &str,
        body: &str,
        label: &str,
    ) -> std::result::Result<SyncPage, SyncCollection> {
        let resp = match self.xml_exchange(url, "REPORT", body, requests::DEPTH_0) {
            Ok(r) => r,
            Err(e) => {
                info!("sync-collection not supported or failed{label}, using legacy sync");
                return Err(SyncCollection::Unsupported(Some(e)));
            }
        };
        if resp.status == 507 {
            match Multistatus::parse(&resp.body) {
                Ok(doc) => {
                    info!("Received HTTP 507, parsing truncated response");
                    return Ok(SyncPage {
                        doc,
                        http_507: true,
                    });
                }
                Err(_) => warn!("Failed to parse HTTP 507 response body"),
            }
        }
        let checked = status_error(&resp).and_then(|_| Multistatus::parse(&resp.body));
        match checked {
            Ok(doc) => Ok(SyncPage {
                doc,
                http_507: false,
            }),
            Err(e) => {
                let token_rejected = matches!(http_status(&e), Some(403 | 409 | 410))
                    || e.as_sync()
                        .is_some_and(|s| s.debuginfo.contains("valid-sync-token"));
                if token_rejected {
                    Err(SyncCollection::TokenRejected(e))
                } else {
                    info!("sync-collection not supported or failed{label}, using legacy sync");
                    Err(SyncCollection::Unsupported(Some(e)))
                }
            }
        }
    }

    /// The calendars in a home set with their metadata (`runCalendars`). A 404, 405 or 406
    /// means the server has no CalDAV; the caller skips calendar sync.
    pub fn list_calendars(&mut self, home: &CalendarHome) -> Result<CalendarListing> {
        let ms = self.xml_request(
            &home.url,
            "PROPFIND",
            requests::PROPFIND_CALENDARS,
            requests::DEPTH_1,
        )?;
        Ok(calendar_listing(&ms, home))
    }
}

struct SyncPage {
    doc: Multistatus,
    http_507: bool,
}

fn etag_conflict(kind: ResourceKind) -> SyncError {
    let what = match kind {
        ResourceKind::ICalendar => "Event",
        ResourceKind::VCard => "Contact",
    };
    SyncError::new(
        ETAG_CONFLICT,
        format!("{what} was modified by another client. Please refresh and try again."),
        false,
        false,
    )
}

fn href_etags(ms: &Multistatus) -> Vec<HrefEtag> {
    ms.responses
        .iter()
        .filter(|r| !r.href.is_empty())
        .map(|r| HrefEtag {
            href: r.href.clone(),
            etag: r.etag().unwrap_or("").to_string(),
        })
        .collect()
}

/// The resource href for a new card: `{uid}.vcf` when the UID is safe in a path, else
/// `{contact_id}.vcf`, under the address book's path. Same rule as
/// [`href_for_new_event`](crate::href::href_for_new_event).
pub fn new_card_href(address_book_url: &str, uid: &str, contact_id: &str) -> String {
    let path = crate::href::path_of(address_book_url);
    let base = if path.ends_with('/') {
        path
    } else {
        format!("{path}/")
    };
    let ics = crate::href::href_for_new_event(&base, uid, contact_id);
    format!("{}.vcf", ics.trim_end_matches(".ics"))
}

/// `stoi`: optional whitespace and sign, then leading digits.
fn leading_int(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<i64>().ok().map(|v| sign * v)
}

/// `calendarIsWritable`: no privilege set means writable (the property is optional, and
/// refusing locally would make every calendar on such servers uneditable). `write` is an
/// aggregate and servers may list what it contains instead, so `write`, `write-content`,
/// `bind` and `all` each count. Only a 2xx propstat is read: the 404 propstat for missing
/// properties carries them as empty elements (RFC 4918 §9.1).
fn writability(r: &DavResponse) -> (bool, bool) {
    match r.privileges() {
        None => (true, false),
        Some(privs) => (
            privs.iter().any(|(_, name)| {
                matches!(name.as_str(), "write" | "write-content" | "bind" | "all")
            }),
            true,
        ),
    }
}

/// `calendarOwnership`: DAV:owner (RFC 3744 §5.1) compared with the principal. Google names
/// the owner `<principal>/user`. Case-folded, since the principal keeps the case the user
/// typed. The display name is not used: any sharer chooses it, and ownership decides which
/// copy of an invitation an RSVP is written onto.
fn ownership(r: &DavResponse, principal_path: &str) -> Ownership {
    if principal_path.is_empty() {
        return Ownership::Unknown;
    }
    let Some(owner) = r
        .ok_prop(ns::DAV, "owner")
        .and_then(|o| o.child(ns::DAV, "href"))
    else {
        return Ownership::Unknown;
    };
    let owner = owner.text.trim();
    if owner.is_empty() {
        return Ownership::Unknown;
    }
    let normalized = normalize_href(owner).to_lowercase();
    let principal = principal_path.to_lowercase();
    if normalized == principal || normalized == format!("{principal}/user") {
        Ownership::Mine
    } else {
        Ownership::Other
    }
}

fn calendar_listing(ms: &Multistatus, home: &CalendarHome) -> CalendarListing {
    let home_path = normalize_href(&with_scheme(&home.url));
    let mut listing = CalendarListing::default();
    for r in &ms.responses {
        if !r.href.is_empty()
            && normalize_href(&r.href) != home_path
            && !r.is_positively_not_calendar()
        {
            listing.listed_paths.push(r.href.clone());
        }
    }

    let vevent: Vec<&DavResponse> = ms
        .responses
        .iter()
        .filter(|r| r.supports_component("VEVENT"))
        .collect();
    let mut owners: HashMap<&str, Ownership> = HashMap::new();
    let mut any_mine = false;
    let mut any_named = false;
    for r in &vevent {
        let o = ownership(r, &home.principal_path);
        any_mine |= o == Ownership::Mine;
        any_named |= o != Ownership::Unknown;
        owners.insert(r.href.as_str(), o);
    }
    // Owners named and none ours means the principal is wrong (an alias, or a Workspace
    // address the server echoes as the primary): ownership stays unknown for the pass.
    if any_named && !any_mine {
        warn!(
            "Server named an owner for every calendar and none is principal '{}'; leaving ownership unknown",
            home.principal_path
        );
        owners.clear();
    }

    for r in vevent.into_iter().filter(|r| r.is_calendar()) {
        let name = r.display_name().unwrap_or("").to_string();
        let order_raw = r.prop_text(ns::APPLE_ICAL, "calendar-order").unwrap_or("");
        let order = leading_int(order_raw);
        if !order_raw.is_empty() && order.is_none() {
            warn!("Calendar '{name}': ignoring non-numeric calendar-order value '{order_raw}'");
        }
        let (writable, advertised) = writability(r);
        listing.calendars.push(CalendarInfo {
            path: r.href.clone(),
            ctag: r.ctag().unwrap_or("").to_string(),
            color: r
                .prop_text(ns::APPLE_ICAL, "calendar-color")
                .unwrap_or("")
                .to_string(),
            description: r
                .prop_text(ns::CALDAV, "calendar-description")
                .unwrap_or("")
                .to_string(),
            order,
            read_only: !writable,
            privileges_advertised: advertised,
            ownership: owners.get(r.href.as_str()).copied().unwrap_or_default(),
            name,
        });
    }
    listing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_of_errors() {
        let e: Error =
            SyncError::new("Invalid Response Code: 404", "u RETURNED x", true, false).into();
        assert_eq!(http_status(&e), Some(404));
        let e: Error = SyncError::new("Could not connect to server", "u", true, true).into();
        assert_eq!(http_status(&e), None);
    }

    #[test]
    fn stoi_like() {
        assert_eq!(leading_int("3"), Some(3));
        assert_eq!(leading_int(" -2x"), Some(-2));
        assert_eq!(leading_int("1.5"), Some(1));
        assert_eq!(leading_int("abc"), None);
    }

    #[test]
    fn card_hrefs() {
        assert_eq!(
            new_card_href("https://h/dav/ab/", "uid-1", "row"),
            "/dav/ab/uid-1.vcf"
        );
        assert_eq!(
            new_card_href("https://h/dav/ab", "a/b", "row"),
            "/dav/ab/row.vcf"
        );
    }

    #[test]
    fn gmail_home() {
        let h = CalendarHome::gmail("a@gmail.com");
        assert_eq!(h.url, "apidata.googleusercontent.com/caldav/v2/a@gmail.com");
        assert_eq!(h.principal_path, "/caldav/v2/a@gmail.com");
        assert_eq!(
            h.calendar_url("/caldav/v2/a%40gmail.com/events/"),
            "apidata.googleusercontent.com/caldav/v2/a%40gmail.com/events/"
        );
        let other = CalendarHome {
            url: "https://dav.example.com/cal/u/".into(),
            ..Default::default()
        };
        assert_eq!(
            other.calendar_url("/cal/u/work/"),
            "https://dav.example.com/cal/u/work/"
        );
        assert_eq!(
            other.calendar_url("https://x.example/c/"),
            "https://x.example/c/"
        );
    }
}
