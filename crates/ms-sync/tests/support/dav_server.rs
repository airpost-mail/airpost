//! A scripted CalDAV/CardDAV server for DavWorker tests, on ms-dav's `Fake` transport: one
//! account at `https://dav.test/dav/` with an address book and a calendar home, plus the
//! multistatus bodies the worker reads. The including crate declares the transport as
//! `mod fake` (`crates/ms-dav/tests/common/mod.rs`).

#![allow(dead_code)]

use std::sync::Arc;

use ms_core::{Account, ManualClock, Result};
use ms_dav::{DavClient, DavHost, DavService, HostResolver};
use ms_store::{DeltaStream, Store};
use serde_json::json;

use super::fake::Fake;

pub const ACCOUNT: &str = "acc1";
/// 2024-06-10T06:13:20Z: the store's and the client's clock.
pub const NOW: i64 = 1_718_000_000;

pub const ROOT: &str = "https://dav.test/dav/";
pub const PRINCIPAL: &str = "https://dav.test/dav/principals/me/";
pub const AB_HOME: &str = "https://dav.test/dav/ab/me/";
pub const BOOK: &str = "https://dav.test/dav/ab/me/contacts/";
pub const BOOK_PATH: &str = "/dav/ab/me/contacts/";
pub const CAL_HOME: &str = "https://dav.test/dav/cal/me/";

pub fn account() -> Arc<Account> {
    Arc::new(
        Account::from_json(json!({
            "id": ACCOUNT, "provider": "imap", "emailAddress": "me@dav.test",
            "settings": {
                "imap_host": "imap.dav.test", "imap_username": "me", "imap_password": "pw",
            },
        }))
        .unwrap(),
    )
}

pub fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    ms_store::migrate::migrate(dir.path(), &ManualClock::new(NOW), &mut Vec::new()).unwrap();
    let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    store.set_clock(Arc::new(ManualClock::new(NOW)));
    (dir, store)
}

/// Both services at `dav.test` with the `/dav/` context path, so discovery skips the
/// `.well-known` probes.
pub struct Resolver;

impl HostResolver for Resolver {
    fn dav_host(&self, _: DavService, _: &str, _: &str) -> Result<Option<DavHost>> {
        Ok(Some(DavHost {
            context_path: Some("/dav/".into()),
            ..DavHost::new("dav.test")
        }))
    }
}

pub fn client(f: &Fake) -> DavClient {
    f.client()
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn multistatus(responses: &[String], sync_token: Option<&str>) -> String {
    let token = sync_token
        .map(|t| format!("<d:sync-token>{}</d:sync-token>", escape(t)))
        .unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:multistatus xmlns:d=\"DAV:\" \
         xmlns:c=\"urn:ietf:params:xml:ns:caldav\" xmlns:card=\"urn:ietf:params:xml:ns:carddav\" \
         xmlns:cs=\"http://calendarserver.org/ns/\">{}{token}</d:multistatus>",
        responses.concat()
    )
}

fn ok_response(href: &str, props: &str) -> String {
    format!(
        "<d:response><d:href>{}</d:href><d:propstat><d:prop>{props}</d:prop>\
         <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
        escape(href)
    )
}

/// A listing entry: href and etag only.
pub fn etag_entry(href: &str, etag: &str) -> String {
    ok_response(href, &format!("<d:getetag>{}</d:getetag>", escape(etag)))
}

pub fn card_entry(href: &str, etag: &str, vcf: &str) -> String {
    ok_response(
        href,
        &format!(
            "<d:getetag>{}</d:getetag><card:address-data>{}</card:address-data>",
            escape(etag),
            escape(vcf)
        ),
    )
}

pub fn event_entry(href: &str, etag: &str, ics: &str) -> String {
    ok_response(
        href,
        &format!(
            "<d:getetag>{}</d:getetag><c:calendar-data>{}</c:calendar-data>",
            escape(etag),
            escape(ics)
        ),
    )
}

/// A sync-collection member removed since the token (RFC 6578 §3.5).
pub fn gone_entry(href: &str) -> String {
    format!(
        "<d:response><d:href>{}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>",
        escape(href)
    )
}

/// The in-body truncation marker of a sync-collection page (RFC 6578 §3.6).
pub fn truncated_entry(href: &str) -> String {
    format!(
        "<d:response><d:href>{}</d:href><d:status>HTTP/1.1 507 Insufficient Storage</d:status></d:response>",
        escape(href)
    )
}

pub fn vcard(uid: &str, name: &str, email: &str) -> String {
    format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nFN:{name}\r\nN:{name};;;;\r\nEMAIL:{email}\r\nEND:VCARD\r\n"
    )
}

pub fn group_vcard(uid: &str, name: &str, members: &[&str]) -> String {
    let members: String = members
        .iter()
        .map(|m| format!("X-ADDRESSBOOKSERVER-MEMBER:urn:uuid:{m}\r\n"))
        .collect();
    format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nFN:{name}\r\nN:{name};;;;\r\nX-ADDRESSBOOKSERVER-KIND:group\r\n{members}END:VCARD\r\n"
    )
}

/// One VEVENT, wrapped. `extra` holds further lines (RRULE, RECURRENCE-ID), CRLF-terminated.
pub fn vevent(uid: &str, start: &str, summary: &str, extra: &str) -> String {
    format!(
        "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20240601T000000Z\r\nDTSTART:{start}\r\nDURATION:PT1H\r\nSUMMARY:{summary}\r\n{extra}END:VEVENT\r\n"
    )
}

pub fn vcalendar(vevents: &[String]) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tern//test//EN\r\n{}END:VCALENDAR\r\n",
        vevents.concat()
    )
}

pub fn ics(uid: &str, start: &str, summary: &str) -> String {
    vcalendar(&[vevent(uid, start, summary, "")])
}

/// Principal discovery, shared by both services.
pub fn principal_routes(f: &Fake) {
    f.on(
        "PROPFIND",
        ROOT,
        207,
        &multistatus(
            &[ok_response(
                "/dav/",
                "<d:current-user-principal><d:href>/dav/principals/me/</d:href></d:current-user-principal>",
            )],
            None,
        ),
    );
}

/// The address book with `ctag`, both in the home-set listing and its Depth 0 check.
pub fn address_book_routes(f: &Fake, ctag: &str) {
    principal_routes(f);
    f.on_body(
        "PROPFIND",
        PRINCIPAL,
        "addressbook-home-set",
        207,
        &multistatus(
            &[ok_response(
                "/dav/principals/me/",
                "<card:addressbook-home-set><d:href>/dav/ab/me/</d:href></card:addressbook-home-set>",
            )],
            None,
        ),
    );
    let props = format!(
        "<d:resourcetype><d:collection/><card:addressbook/></d:resourcetype>\
         <d:displayname>Contacts</d:displayname><cs:getctag>{ctag}</cs:getctag>"
    );
    f.on(
        "PROPFIND",
        AB_HOME,
        207,
        &multistatus(&[ok_response(BOOK_PATH, &props)], None),
    );
    f.on(
        "PROPFIND",
        BOOK,
        207,
        &multistatus(
            &[ok_response(
                BOOK_PATH,
                &format!("<cs:getctag>{ctag}</cs:getctag>"),
            )],
            None,
        ),
    );
}

/// A calendar in the home-set listing: `(path, name, ctag)`.
pub fn calendar_entry(path: &str, name: &str, ctag: &str) -> String {
    ok_response(
        path,
        &format!(
            "<d:resourcetype><d:collection/><c:calendar/></d:resourcetype>\
             <d:displayname>{name}</d:displayname><cs:getctag>{ctag}</cs:getctag>\
             <c:supported-calendar-component-set><c:comp name=\"VEVENT\"/></c:supported-calendar-component-set>"
        ),
    )
}

/// The calendar home and its listing.
pub fn calendar_home_routes(f: &Fake, calendars: &[String]) {
    principal_routes(f);
    f.on_body(
        "PROPFIND",
        PRINCIPAL,
        "calendar-home-set",
        207,
        &multistatus(
            &[ok_response(
                "/dav/principals/me/",
                "<c:calendar-home-set><d:href>/dav/cal/me/</d:href></c:calendar-home-set>",
            )],
            None,
        ),
    );
    let mut listing = vec![ok_response(
        "/dav/cal/me/",
        "<d:resourcetype><d:collection/></d:resourcetype>",
    )];
    listing.extend(calendars.iter().cloned());
    f.on("PROPFIND", CAL_HOME, 207, &multistatus(&listing, None));
}

/// The bodies of the requests of `method` to `url` whose body contains `has`.
pub fn bodies(f: &Fake, method: &str, url: &str, has: &str) -> Vec<String> {
    f.requests()
        .into_iter()
        .filter(|r| r.method == method && r.url == url && r.body.contains(has))
        .map(|r| r.body)
        .collect()
}

pub fn check_invariants(store: &Store) {
    assert_eq!(
        ms_store::invariants::check(store.conn(), &[]).unwrap(),
        Vec::<String>::new()
    );
}
