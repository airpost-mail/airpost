// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The client against captured-format responses from each server family. The fixtures
//! follow each server's documented and observed response shapes (sabre/dav for Baikal and
//! Nextcloud, Radicale's xmlutils, iCloud's default-namespace style with CDATA bodies,
//! Google's split propstats, Cyrus's in-body 507) and the quirks the C++ comments in
//! `DAVWorker.cpp` describe.

mod common;

use airpost_dav::ical::{parse_events_in, EventFields, FloatingZone};
use airpost_dav::requests::CalendarSyncRange;
use airpost_dav::{
    http_status, CalendarHome, Collection, ContactFields, DavAuth, DavHost, DavServer,
    DeleteOutcome, Ownership, SyncChange, SyncCollection, ETAG_CONFLICT,
};
use common::{fixture, Fake};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn events(ics: &str) -> Vec<EventFields> {
    parse_events_in(ics, FloatingZone::Zone(chrono_tz::UTC))
}

fn complete(s: SyncCollection) -> airpost_dav::SyncCollectionResult {
    match s {
        SyncCollection::Complete(r) => r,
        other => panic!("expected a complete sync, got {other:?}"),
    }
}

fn header<'a>(req: &'a airpost_dav::HttpRequest, name: &str) -> Option<&'a str> {
    req.header(name)
}

mod radicale {
    use super::*;

    const ROOT: &str = "http://localhost:5232/";
    const HOME: &str = "http://localhost:5232/alice/";
    const CAL: &str = "http://localhost:5232/alice/7f3c4a52-36a9-4e0c-a3a5-7a0b9c1d2e3f/";

    fn host() -> DavHost {
        DavHost {
            port: Some(5232),
            ..DavHost::new("localhost")
        }
    }

    fn fake() -> Fake {
        let f = Fake::new();
        f.redirect("http://localhost:5232/.well-known/caldav", "/");
        f.redirect("http://localhost:5232/.well-known/carddav", "/");
        f.once(
            "PROPFIND",
            ROOT,
            Some("current-user-principal"),
            207,
            &[],
            &fixture("radicale/principal.xml"),
        );
        f.once(
            "PROPFIND",
            HOME,
            Some("calendar-home-set"),
            207,
            &[],
            &fixture("radicale/calendar_home_set.xml"),
        );
        f.once(
            "PROPFIND",
            HOME,
            Some("calendar-order"),
            207,
            &[],
            &fixture("radicale/calendars.xml"),
        );
        f
    }

    #[test]
    fn discovery_and_listing() {
        let f = fake();
        let mut c = f.client();
        let home = c.discover_calendar_home(&host()).unwrap().unwrap();
        assert_eq!(home.url, HOME);
        assert_eq!(home.principal_path, "/alice");

        let reqs = f.requests();
        // https first, then http, then the root it redirected to.
        assert_eq!(reqs[0].url, "https://localhost:5232/.well-known/caldav");
        assert_eq!(reqs[1].url, "http://localhost:5232/.well-known/caldav");
        // The probes carry no credentials: the second one is plain http.
        assert_eq!(header(&reqs[0], "Authorization"), None);
        assert_eq!(header(&reqs[1], "Authorization"), None);
        assert_eq!(reqs[2].method, "PROPFIND");
        assert_eq!(header(&reqs[2], "Depth"), Some("0"));
        assert_eq!(header(&reqs[2], "Prefer"), Some("return-minimal"));
        assert_eq!(
            header(&reqs[2], "Authorization"),
            Some("Basic YWxpY2U6c2VjcmV0")
        );
        assert_eq!(header(&reqs[3], "Depth"), Some("0"));

        let listing = c.list_calendars(&home).unwrap();
        assert_eq!(listing.calendars.len(), 1);
        let cal = &listing.calendars[0];
        assert_eq!(cal.path, "/alice/7f3c4a52-36a9-4e0c-a3a5-7a0b9c1d2e3f/");
        assert_eq!(cal.name, "Personal");
        assert_eq!(cal.ctag, "\"0d1ee9b9ef3c1bc9b4b4d8f8d6b4a6e1\"");
        assert_eq!(cal.color, "#ff0000ff");
        assert_eq!(cal.description, "Home things");
        assert_eq!(cal.order, None);
        assert!(!cal.read_only && cal.privileges_advertised);
        assert_eq!(cal.ownership, Ownership::Mine);
        // The VTODO list still counts as listed; the address book and the home do not.
        assert_eq!(
            listing.listed_paths,
            vec![
                "/alice/7f3c4a52-36a9-4e0c-a3a5-7a0b9c1d2e3f/",
                "/alice/tasks/"
            ]
        );
        assert_eq!(home.calendar_url(&cal.path), CAL);
        assert_eq!(header(f.requests().last().unwrap(), "Depth"), Some("1"));
    }

    #[test]
    fn initial_sync_then_multiget() {
        let f = Fake::new();
        f.once(
            "REPORT",
            CAL,
            Some("<D:sync-token/>"),
            207,
            &[],
            &fixture("radicale/sync_initial.xml"),
        );
        f.once(
            "REPORT",
            CAL,
            Some("calendar-multiget"),
            207,
            &[],
            &fixture("radicale/multiget.xml"),
        );
        let mut c = f.client();
        let r = complete(c.sync_collection(CAL, Collection::Calendar, ""));
        assert_eq!(r.pages, 1);
        assert_eq!(
            r.sync_token,
            "http://radicale.org/ns/sync/4c5ad3c3a4d1c0e2a8d3"
        );
        let needed = r.needed_hrefs();
        assert_eq!(needed.len(), 2);
        assert!(r.data().is_empty() && r.deleted_hrefs().is_empty());
        let sync_req = &f.requests()[0];
        assert_eq!(header(sync_req, "Depth"), Some("0"));
        assert!(sync_req.body.contains("<D:prop><D:getetag/></D:prop>"));

        let got = c.multiget(CAL, Collection::Calendar, &needed).unwrap();
        assert_eq!(got.responses, 2);
        assert_eq!(got.objects.len(), 2);
        assert!(f.requests()[1]
            .body
            .contains("<d:href>/alice/7f3c4a52-36a9-4e0c-a3a5-7a0b9c1d2e3f/meeting.ics</d:href>"));

        let meeting = events(&got.objects[0].data);
        assert_eq!(meeting.len(), 1);
        assert_eq!(meeting[0].uid, "meeting-1@radicale");
        assert_eq!(meeting[0].summary, "Planning & review");
        assert_eq!(meeting[0].location, "Room 1, 2nd floor");
        assert_eq!(meeting[0].start, Some(1_718_002_800));
        assert_eq!(meeting[0].recurrence_end, 1_718_006_400);
        assert_eq!(got.objects[0].etag, "\"e1a\"");

        let standup = events(&got.objects[1].data);
        assert_eq!(standup.len(), 2);
        assert_eq!(standup[0].recurrence_id, "");
        assert!(standup[0].is_recurring);
        assert_eq!(standup[0].recurrence_start, 1_717_403_400);
        assert_eq!(standup[0].recurrence_end, 1_727_685_000);
        assert_eq!(standup[1].recurrence_id, "20240617T083000Z");
        assert_eq!(standup[1].status, "TENTATIVE");
        assert_eq!(standup[1].recurrence_start, 1_718_614_800);
        assert_eq!(standup[1].recurrence_end, 1_718_615_700);
        assert_eq!(
            airpost_dav::ical::count_with_uid(&standup, "standup@radicale"),
            2
        );
        assert_eq!(
            airpost_dav::ical::matching_event(&standup, "20240617T083000Z")
                .unwrap()
                .summary,
            "Standup (moved)"
        );
    }

    #[test]
    fn incremental_sync_has_deletions_and_data() {
        let f = Fake::new();
        f.once(
            "REPORT",
            CAL,
            Some("<D:sync-token>http://radicale.org/ns/sync/4c5ad3c3a4d1c0e2a8d3</D:sync-token>"),
            207,
            &[],
            &fixture("radicale/sync_incremental.xml"),
        );
        let mut c = f.client();
        let r = complete(c.sync_collection(
            CAL,
            Collection::Calendar,
            "http://radicale.org/ns/sync/4c5ad3c3a4d1c0e2a8d3",
        ));
        assert!(f.requests()[0]
            .body
            .contains("<D:prop><D:getetag/><C:calendar-data/></D:prop>"));
        assert_eq!(
            r.deleted_hrefs(),
            vec!["/alice/7f3c4a52-36a9-4e0c-a3a5-7a0b9c1d2e3f/meeting.ics"]
        );
        let data = r.data();
        assert_eq!(data.len(), 1);
        let holiday = &events(&data[0].data)[0];
        assert!(holiday.all_day);
        assert_eq!(holiday.recurrence_start, 1_720_051_200);
        assert_eq!(holiday.recurrence_end, 1_720_137_600);
        assert_eq!(r.sync_token, "http://radicale.org/ns/sync/9e8d7c6b5a49");
    }

    #[test]
    fn address_book_discovery_and_etags() {
        let f = fake();
        f.once(
            "PROPFIND",
            HOME,
            Some("addressbook-home-set"),
            207,
            &[],
            &fixture("radicale/addressbook_home_set.xml"),
        );
        f.once(
            "PROPFIND",
            HOME,
            Some("getctag"),
            207,
            &[],
            &fixture("radicale/addressbooks.xml"),
        );
        let mut c = f.client();
        let books = c.discover_address_books(&host()).unwrap().unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].url, "http://localhost:5232/alice/contacts/");
        assert_eq!(books[0].ctag, "\"b2\"");
        assert_eq!(books[0].name, "Contacts");

        f.once(
            "REPORT",
            CAL,
            Some("time-range"),
            207,
            &[],
            &fixture("radicale/sync_initial.xml"),
        );
        let range = CalendarSyncRange::around(1_718_000_000);
        let etags = c.calendar_etags(CAL, &range).unwrap();
        assert_eq!(etags.len(), 2);
        assert_eq!(etags[1].etag, "\"e2a\"");
        let body = f.requests().last().unwrap().body.clone();
        assert!(body.contains("<c:comp-filter name=\"VEVENT\"><c:time-range start=\""));
    }
}

mod baikal {
    use super::*;

    const ROOT: &str = "https://baikal.example/dav.php";
    const PRINCIPAL: &str = "https://baikal.example/dav.php/principals/alice/";
    const HOME: &str = "https://baikal.example/dav.php/calendars/alice/";
    const CAL: &str = "https://baikal.example/dav.php/calendars/alice/default/";

    #[test]
    fn discovery_listing_and_http1() {
        let f = Fake::new();
        f.redirect("https://baikal.example/.well-known/caldav", "/dav.php");
        f.on("PROPFIND", ROOT, 207, &fixture("baikal/principal.xml"));
        f.on(
            "PROPFIND",
            PRINCIPAL,
            207,
            &fixture("baikal/calendar_home_set.xml"),
        );
        f.on("PROPFIND", HOME, 207, &fixture("baikal/calendars.xml"));
        let mut c = f.client();
        let home = c
            .discover_calendar_home(&DavHost::new("baikal.example"))
            .unwrap()
            .unwrap();
        assert_eq!(home.url, HOME);
        assert_eq!(home.principal_path, "/dav.php/principals/alice");
        assert_eq!(c.profile_for(HOME).server, DavServer::Baikal);

        let listing = c.list_calendars(&home).unwrap();
        assert_eq!(
            listing.listed_paths,
            vec!["/dav.php/calendars/alice/default/"]
        );
        let cal = &listing.calendars[0];
        assert_eq!(cal.name, "Default calendar");
        assert_eq!(cal.ctag, "http://sabre.io/ns/sync/12");
        assert_eq!(cal.color, "#2D7F94FF");
        assert_eq!(cal.order, Some(1));
        assert_eq!(cal.description, "");
        assert!(!cal.read_only);
        assert_eq!(cal.ownership, Ownership::Mine);

        // Baikal's nginx image fails auth under HTTP/2 (DAVWorker.cpp:2335); the host name
        // identifies it from the first probe on.
        assert!(f.requests().iter().all(|r| r.http1_only));
    }

    #[test]
    fn expired_token_is_rejected_not_unsupported() {
        let f = Fake::new();
        f.once(
            "REPORT",
            CAL,
            None,
            403,
            &[],
            &fixture("baikal/invalid_sync_token.xml"),
        );
        let mut c = f.client();
        match c.sync_collection(CAL, Collection::Calendar, "http://sabre.io/ns/sync/5") {
            SyncCollection::TokenRejected(e) => {
                assert_eq!(http_status(&e), Some(403));
                assert!(e.as_sync().unwrap().debuginfo.contains("valid-sync-token"));
            }
            other => panic!("{other:?}"),
        }
        // A 507 from no-token sync, a 500 and anything else fall back to the etag listing.
        f.once("REPORT", CAL, None, 500, &[], "boom");
        assert!(matches!(
            c.sync_collection(CAL, Collection::Calendar, ""),
            SyncCollection::Unsupported(Some(_))
        ));
        f.once(
            "REPORT",
            CAL,
            None,
            207,
            &[],
            &fixture("baikal/sync_initial.xml"),
        );
        let r = complete(c.sync_collection(CAL, Collection::Calendar, ""));
        assert_eq!(
            r.needed_hrefs(),
            vec!["/dav.php/calendars/alice/default/0a1b2c3d.ics"]
        );
        assert_eq!(
            r.changes[0],
            SyncChange::Needed {
                href: "/dav.php/calendars/alice/default/0a1b2c3d.ics".into(),
                etag: "\"8f54bd5a9e4d1bb1d4a1e6c0f8a72e3d\"".into()
            }
        );
    }

    #[test]
    fn put_conflict_and_group_cards() {
        let f = Fake::new();
        let url = "https://baikal.example/dav.php/calendars/alice/default/x.ics";
        f.once("PUT", url, None, 412, &[], "");
        let mut c = f.client();
        let err = c
            .put_ics(url, "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", "\"old\"")
            .unwrap_err();
        let s = err.as_sync().unwrap();
        assert_eq!(s.key, ETAG_CONFLICT);
        assert!(!s.retryable);
        let put = &f.requests()[0];
        assert_eq!(header(put, "If-Match"), Some("\"old\""));
        assert_eq!(
            header(put, "Content-Type"),
            Some("text/calendar; charset=utf-8")
        );

        let ab = "https://baikal.example/dav.php/addressbooks/alice/default/";
        f.once(
            "REPORT",
            ab,
            Some("addressbook-multiget"),
            207,
            &[],
            &fixture("baikal/addressbook_multiget.xml"),
        );
        let got = c
            .multiget(
                ab,
                Collection::AddressBook,
                &[
                    "/dav.php/addressbooks/alice/default/team.vcf".into(),
                    "/dav.php/addressbooks/alice/default/missing.vcf".into(),
                ],
            )
            .unwrap();
        assert_eq!(got.responses, 2);
        assert_eq!(got.objects.len(), 1);
        assert!(got.objects[0].data.contains("\r\n"));
        let fields = ContactFields::from_vcard(&got.objects[0].data);
        assert!(fields.is_group);
        assert_eq!(fields.uid, "team-group-uid");
        assert_eq!(fields.name, "Team");
        assert_eq!(fields.member_ids, vec!["c-ann", "c-bob"]);
    }
}

mod icloud {
    use super::*;

    const CAL_HOME: &str = "https://p52-caldav.icloud.com:443/123456789/calendars/";

    #[test]
    fn calendars_ownership_and_cdata() {
        let f = Fake::new();
        f.redirect(
            "https://caldav.icloud.com/.well-known/caldav",
            "https://caldav.icloud.com/",
        );
        f.on(
            "PROPFIND",
            "https://caldav.icloud.com/",
            207,
            &fixture("icloud/principal.xml"),
        );
        f.on(
            "PROPFIND",
            "https://caldav.icloud.com/123456789/principal/",
            207,
            &fixture("icloud/calendar_home_set.xml"),
        );
        f.on("PROPFIND", CAL_HOME, 207, &fixture("icloud/calendars.xml"));
        let mut c = f.client();
        let home = c
            .discover_calendar_home(&DavHost::new("caldav.icloud.com"))
            .unwrap()
            .unwrap();
        assert_eq!(home.url, CAL_HOME);
        assert_eq!(home.principal_path, "/123456789/principal");

        let listing = c.list_calendars(&home).unwrap();
        let names: Vec<&str> = listing.calendars.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Home", "alice@icloud.com"]);
        let own = &listing.calendars[0];
        assert_eq!(own.color, "#FF2968FF");
        assert_eq!(own.order, Some(1));
        assert_eq!(own.ownership, Ownership::Mine);
        assert!(!own.read_only);
        // Named after the user, but owned by someone else: the display name is not trusted.
        let shared = &listing.calendars[1];
        assert_eq!(shared.ownership, Ownership::Other);
        assert!(shared.read_only);
        assert_eq!(
            listing.listed_paths,
            vec![
                "/123456789/calendars/home/",
                "/123456789/calendars/tasks/",
                "/123456789/calendars/8E7D5F04-2A0C-4B5B-9C32-0D7A9B3C7F11/"
            ]
        );
        assert_eq!(
            home.calendar_url(&own.path),
            "https://p52-caldav.icloud.com:443/123456789/calendars/home/"
        );
        assert_eq!(c.profile_for(CAL_HOME).server, DavServer::ICloud);

        let cal = home.calendar_url(&own.path);
        f.once(
            "REPORT",
            &cal,
            None,
            207,
            &[],
            &fixture("icloud/multiget.xml"),
        );
        let got = c
            .multiget(
                &cal,
                Collection::Calendar,
                &["/123456789/calendars/home/5D7C8B2E-0F0A-4C6B-A0B4-AD13D84E7B57.ics".into()],
            )
            .unwrap();
        let ev = &events(&got.objects[0].data)[0];
        assert_eq!(ev.uid, "5D7C8B2E-0F0A-4C6B-A0B4-AD13D84E7B57");
        assert_eq!(ev.start, Some(1_717_614_000));
        assert_eq!(ev.recurrence_end, 1_717_617_600);
        assert_eq!(ev.organizer, "Alice Appleseed <alice@icloud.com>");
        assert_eq!(
            ev.attendees,
            vec!["Bob Builder <bob@example.com>", "alice@icloud.com"]
        );
        // The VALARM's DESCRIPTION belongs to the alarm.
        assert_eq!(ev.description, "");
        assert_eq!(ev.location, "Cafe");
    }

    #[test]
    fn contacts_and_vcard3_groups() {
        let f = Fake::new();
        f.redirect(
            "https://contacts.icloud.com/.well-known/carddav",
            "https://contacts.icloud.com/",
        );
        f.on(
            "PROPFIND",
            "https://contacts.icloud.com/",
            207,
            &fixture("icloud/card_principal.xml"),
        );
        f.on(
            "PROPFIND",
            "https://contacts.icloud.com/123456789/principal/",
            207,
            &fixture("icloud/addressbook_home_set.xml"),
        );
        f.on(
            "PROPFIND",
            "https://p52-contacts.icloud.com:443/123456789/carddavhome/",
            207,
            &fixture("icloud/addressbooks.xml"),
        );
        let mut c = f.client();
        let books = c
            .discover_address_books(&DavHost::new("contacts.icloud.com"))
            .unwrap()
            .unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(
            books[0].url,
            "https://p52-contacts.icloud.com:443/123456789/carddavhome/card/"
        );
        assert_eq!(books[0].ctag, "HwoQEgwAAAh0p0x3AAAAAA");
        assert!(c.profile_for(&books[0].url).vcard3_contact_groups);

        f.once(
            "REPORT",
            &books[0].url,
            None,
            207,
            &[],
            &fixture("icloud/card_multiget.xml"),
        );
        let got = c
            .multiget(
                &books[0].url,
                Collection::AddressBook,
                &["/a.vcf".into(), "/b.vcf".into()],
            )
            .unwrap();
        let katie = ContactFields::from_vcard(&got.objects[0].data);
        // `item1.EMAIL` is grouped; the C++ line model missed it.
        assert_eq!(katie.email, "katie@example.com");
        assert_eq!(katie.name, "Katie Appleseed");
        assert!(!katie.is_group);
        let group = ContactFields::from_vcard(&got.objects[1].data);
        assert!(group.is_group);
        assert_eq!(
            group.member_ids,
            vec!["6a5c4f29-1a0e-4e43-a4b5-0f1e5ad0f5e1"]
        );
    }
}

mod google {
    use super::*;

    const HOME: &str = "https://apidata.googleusercontent.com/caldav/v2/alice@gmail.com";
    const EVENTS: &str = "apidata.googleusercontent.com/caldav/v2/alice%40gmail.com/events/";

    #[test]
    fn fixed_home_owner_suffix_and_bearer() {
        let f = Fake::new();
        f.on("PROPFIND", HOME, 207, &fixture("google/calendars.xml"));
        let mut c = f.client();
        // The provider is asked before every request, so a refreshed token is picked up.
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        c.set_auth(DavAuth::bearer(move || {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            Ok(if n == 0 { "ya29.a0Af" } else { "ya29.a0Bg" }.to_string())
        }));
        let home = CalendarHome::gmail("alice@gmail.com");
        let listing = c.list_calendars(&home).unwrap();
        assert_eq!(
            listing.listed_paths,
            vec!["/caldav/v2/alice%40gmail.com/events/"]
        );
        let cal = &listing.calendars[0];
        assert_eq!(cal.name, "alice@gmail.com");
        assert_eq!(cal.ctag, "63860000000");
        assert_eq!(cal.color, "#9fe1e7");
        // Google names the owner `<principal>/user`.
        assert_eq!(cal.ownership, Ownership::Mine);
        assert!(!cal.read_only);
        assert_eq!(home.calendar_url(&cal.path), EVENTS);
        assert_eq!(
            header(&f.requests()[0], "Authorization"),
            Some("Bearer ya29.a0Af")
        );
        c.list_calendars(&home).unwrap();
        assert_eq!(
            header(&f.requests()[1], "Authorization"),
            Some("Bearer ya29.a0Bg")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // A provider failure fails the request without sending it.
        c.set_auth(DavAuth::bearer(|| {
            Err(airpost_core::SyncError::new("Token refresh failed", "", false, false).into())
        }));
        let e = c.list_calendars(&home).unwrap_err();
        assert_eq!(e.as_sync().unwrap().key, "Token refresh failed");
        assert_eq!(f.requests().len(), 2);
        let p = c.profile_for(HOME);
        assert_eq!(p.server, DavServer::Google);
        assert!(p.contacts_via_people_api);
    }

    #[test]
    fn propstat_404_is_not_a_deletion() {
        let f = Fake::new();
        let url = format!("https://{EVENTS}");
        f.once(
            "REPORT",
            &url,
            Some("<D:sync-token/>"),
            207,
            &[],
            &fixture("google/sync_initial.xml"),
        );
        f.once(
            "REPORT",
            &url,
            Some("sync/CPjM9Lj5z4cDEAAYASDCzKzFAijCzKzFAg=="),
            207,
            &[],
            &fixture("google/sync_incremental.xml"),
        );
        let mut c = f.client();
        let r = complete(c.sync_collection(EVENTS, Collection::Calendar, ""));
        assert_eq!(
            r.needed_hrefs(),
            vec!["/caldav/v2/alice%40gmail.com/events/0f4ab1qv3kgh6bvn3k9j1b2c3d%40google.com.ics"]
        );
        assert_eq!(
            r.deleted_hrefs(),
            vec!["/caldav/v2/alice%40gmail.com/events/7h2k9vq0p4l5m6n7o8p9q0r1s2%40google.com.ics"]
        );

        let r = complete(c.sync_collection(EVENTS, Collection::Calendar, &r.sync_token));
        let data = r.data();
        assert_eq!(data[0].etag, "\"63860000002\"");
        let ev = &events(&data[0].data)[0];
        assert_eq!(ev.uid, "0f4ab1qv3kgh6bvn3k9j1b2c3d@google.com");
        assert_eq!(ev.description, "Agenda:\n1. Budget\n2. Hiring");
        assert_eq!(ev.attendees, vec!["alice@gmail.com <alice@gmail.com>"]);
        assert_eq!(ev.status, "CONFIRMED");
        assert_eq!(ev.recurrence_start, 1_718_204_400);
        assert_eq!(ev.recurrence_end, 1_718_208_000);
        assert!(ev.has_dtstamp);
    }
}

mod nextcloud {
    use super::*;

    const ROOT: &str = "https://cloud.example.com/remote.php/dav/";
    const PRINCIPAL: &str = "https://cloud.example.com/remote.php/dav/principals/users/alice/";
    const HOME: &str = "https://cloud.example.com/remote.php/dav/calendars/alice/";

    #[test]
    fn deleted_calendars_shares_and_depth0_home_set() {
        let f = Fake::new();
        f.redirect(
            "https://cloud.example.com/.well-known/caldav",
            "/remote.php/dav/",
        );
        f.on("PROPFIND", ROOT, 207, &fixture("nextcloud/principal.xml"));
        f.on(
            "PROPFIND",
            PRINCIPAL,
            207,
            &fixture("nextcloud/calendar_home_set.xml"),
        );
        f.on("PROPFIND", HOME, 207, &fixture("nextcloud/calendars.xml"));
        let mut c = f.client();
        let home = c
            .discover_calendar_home(&DavHost::new("cloud.example.com"))
            .unwrap()
            .unwrap();
        let home_set_req = f
            .requests()
            .into_iter()
            .find(|r| r.url == PRINCIPAL)
            .unwrap();
        // Depth 1 would add the calendar-proxy children's home sets.
        assert_eq!(header(&home_set_req, "Depth"), Some("0"));
        assert_eq!(
            home.principal_path,
            "/remote.php/dav/principals/users/alice"
        );

        let listing = c.list_calendars(&home).unwrap();
        let names: Vec<&str> = listing.calendars.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Personal", "Contact birthdays", "Work (Bob)"]);
        assert_eq!(
            listing.listed_paths,
            vec![
                "/remote.php/dav/calendars/alice/personal/",
                "/remote.php/dav/calendars/alice/contact_birthdays/",
                "/remote.php/dav/calendars/alice/work_shared_by_bob/"
            ]
        );
        let [personal, birthdays, bob] = &listing.calendars[..] else {
            panic!()
        };
        assert_eq!(
            (personal.order, personal.read_only, personal.ownership),
            (Some(0), false, Ownership::Mine)
        );
        assert_eq!(
            (birthdays.read_only, birthdays.ownership),
            (true, Ownership::Mine)
        );
        assert_eq!(
            (bob.order, bob.read_only, bob.ownership),
            (None, true, Ownership::Other)
        );
        let p = c.profile_for(HOME);
        assert_eq!(p.server, DavServer::Nextcloud);
        assert!(p.unreliable_sync_token);
    }

    #[test]
    fn incremental_with_duration_in_ambiguous_hour() {
        let f = Fake::new();
        let url = format!("{HOME}personal/");
        f.once(
            "REPORT",
            &url,
            None,
            207,
            &[],
            &fixture("nextcloud/sync_incremental.xml"),
        );
        let mut c = f.client();
        let r =
            complete(c.sync_collection(&url, Collection::Calendar, "http://sabre.io/ns/sync/42"));
        assert_eq!(
            r.deleted_hrefs(),
            vec!["/remote.php/dav/calendars/alice/personal/DEADBEEF-1.ics"]
        );
        assert_eq!(r.sync_token, "http://sabre.io/ns/sync/43");
        let ev = &events(&r.data()[0].data)[0];
        // 02:30 happens twice on 2024-10-27 in Berlin; the earlier (CEST) instant is used.
        assert_eq!(ev.recurrence_start, 1_729_989_000);
        assert_eq!(ev.recurrence_end, 1_729_992_600);
    }

    #[test]
    fn address_books() {
        let f = Fake::new();
        f.redirect(
            "https://cloud.example.com/.well-known/carddav",
            "/remote.php/dav/",
        );
        f.on("PROPFIND", ROOT, 207, &fixture("nextcloud/principal.xml"));
        f.on(
            "PROPFIND",
            PRINCIPAL,
            207,
            &fixture("nextcloud/addressbook_home_set.xml"),
        );
        f.on(
            "PROPFIND",
            "https://cloud.example.com/remote.php/dav/addressbooks/users/alice/",
            207,
            &fixture("nextcloud/addressbooks.xml"),
        );
        let mut c = f.client();
        let books = c
            .discover_address_books(&DavHost::new("cloud.example.com"))
            .unwrap()
            .unwrap();
        let urls: Vec<&str> = books.iter().map(|b| b.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://cloud.example.com/remote.php/dav/addressbooks/users/alice/contacts/",
                "https://cloud.example.com/remote.php/dav/addressbooks/users/alice/z-app-generated--contactsinteraction--recent/"
            ]
        );
        assert_eq!(books[0].ctag, "http://sabre.io/ns/sync/9");
    }
}

mod fastmail {
    use super::*;

    const AB: &str =
        "https://carddav.fastmail.com/dav/addressbooks/user/alice@fastmail.com/Default/";

    #[test]
    fn truncated_sync_pages_through_507() {
        let f = Fake::new();
        f.once(
            "REPORT",
            AB,
            Some("<D:sync-token/>"),
            207,
            &[],
            &fixture("fastmail/sync_page1.xml"),
        );
        f.once(
            "REPORT",
            AB,
            Some("ns/sync/1001"),
            207,
            &[],
            &fixture("fastmail/sync_page2.xml"),
        );
        let mut c = f.client();
        let r = complete(c.sync_collection(AB, Collection::AddressBook, ""));
        assert_eq!(r.pages, 2);
        assert!(!r.hit_page_limit);
        assert_eq!(r.sync_token, "https://www.fastmail.com/ns/sync/1002");
        assert_eq!(r.needed_hrefs().len(), 3);
        let reqs = f.requests();
        assert_eq!(reqs.len(), 2);
        // Every page of an initial sync asks for etags only.
        for req in &reqs {
            assert!(req.body.contains("<D:prop><D:getetag/></D:prop>"));
            assert!(req.body.contains("urn:ietf:params:xml:ns:carddav"));
        }
        assert!(reqs[1]
            .body
            .contains("<D:sync-token>https://www.fastmail.com/ns/sync/1001</D:sync-token>"));
    }

    #[test]
    fn cards_groups_and_writes() {
        let f = Fake::new();
        f.once(
            "REPORT",
            AB,
            None,
            207,
            &[],
            &fixture("fastmail/multiget.xml"),
        );
        let mut c = f.client();
        let got = c
            .multiget(AB, Collection::AddressBook, &["/x".into()])
            .unwrap();
        assert_eq!(got.objects.len(), 3);
        let sean = ContactFields::from_vcard(&got.objects[0].data);
        assert_eq!(sean.name, "Seán O'Brien");
        assert_eq!(sean.email, "sean@example.ie");
        assert_eq!(sean.emails, vec!["sean@example.ie", "sean.home@example.ie"]);
        let nameless = ContactFields::from_vcard(&got.objects[1].data);
        assert_eq!(nameless.name, "");
        assert_eq!(nameless.email, "nobody@example.org");
        let group = ContactFields::from_vcard(&got.objects[2].data);
        assert!(group.is_group);
        assert_eq!(group.member_ids, vec!["0b5c2f2a-1", "0b5c2f2a-2"]);
        let p = c.profile_for(AB);
        assert!(p.reassigns_contact_uid && p.vcard3_contact_groups);

        let href = airpost_dav::client::new_card_href(AB, "new-uid", "row-id");
        assert_eq!(
            href,
            "/dav/addressbooks/user/alice@fastmail.com/Default/new-uid.vcf"
        );
        let put_url = format!("{AB}new-uid.vcf");
        f.once("PUT", &put_url, None, 201, &[("ETag", "\"n1\"")], "");
        let w = c
            .create_vcard(AB, &href, "BEGIN:VCARD\r\nEND:VCARD\r\n")
            .unwrap();
        assert_eq!(w.etag.as_deref(), Some("\"n1\""));
        let put = f.requests().last().unwrap().clone();
        assert_eq!(header(&put, "If-None-Match"), Some("*"));
        assert_eq!(
            header(&put, "Content-Type"),
            Some("text/vcard; charset=utf-8")
        );
        assert_eq!(header(&put, "Accept"), Some("text/vcard; version=4.0"));

        f.once("DELETE", &put_url, None, 404, &[], "");
        assert_eq!(
            c.delete(airpost_dav::ResourceKind::VCard, &put_url, "\"n1\"")
                .unwrap(),
            DeleteOutcome::AlreadyGone(404)
        );
        assert_eq!(
            header(f.requests().last().unwrap(), "If-Match"),
            Some("\"n1\"")
        );
        f.once("DELETE", &put_url, None, 410, &[], "");
        assert_eq!(
            c.delete(airpost_dav::ResourceKind::VCard, &put_url, "")
                .unwrap(),
            DeleteOutcome::AlreadyGone(410)
        );
        f.once("DELETE", &put_url, None, 204, &[], "");
        assert_eq!(
            c.delete(airpost_dav::ResourceKind::VCard, &put_url, "")
                .unwrap(),
            DeleteOutcome::Deleted
        );
        // 412 while the resource still exists (with another etag) is a real conflict.
        f.once("DELETE", &put_url, None, 412, &[], "");
        f.once(
            "PROPFIND",
            &put_url,
            Some("getetag"),
            207,
            &[],
            &format!(
                "<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>{href}</d:href><d:propstat><d:prop><d:getetag>\"n2\"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"
            ),
        );
        let e = c
            .delete(airpost_dav::ResourceKind::VCard, &put_url, "\"stale\"")
            .unwrap_err();
        assert_eq!(e.as_sync().unwrap().key, ETAG_CONFLICT);
        let reqs = f.requests();
        let check = &reqs[reqs.len() - 1];
        assert_eq!(check.method, "PROPFIND");
        assert_eq!(header(check, "Depth"), Some("0"));
        assert_eq!(header(check, "If-Match"), None);
    }

    /// sabre/dav answers 412 to an If-Match DELETE of a resource that no longer exists
    /// ("An If-Match header was specified and the resource did not exist").
    #[test]
    fn delete_412_on_a_gone_resource_is_already_gone() {
        const URL: &str =
            "https://nextcloud.example.com/remote.php/dav/addressbooks/users/alice/contacts/a.vcf";
        let f = Fake::new();
        f.once(
            "DELETE",
            URL,
            None,
            412,
            &[],
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<d:error xmlns:d=\"DAV:\" xmlns:s=\"http://sabredav.org/ns\"><s:exception>Sabre\\DAV\\Exception\\PreconditionFailed</s:exception><s:message>An If-Match header was specified and the resource did not exist</s:message><s:header>If-Match</s:header></d:error>",
        );
        f.once("PROPFIND", URL, Some("getetag"), 404, &[], "");
        let mut c = f.client();
        assert_eq!(
            c.delete(airpost_dav::ResourceKind::VCard, URL, "\"e1\"")
                .unwrap(),
            DeleteOutcome::AlreadyGone(404)
        );
        // Without an etag there was no If-Match, so a 412 is not re-checked.
        f.once("DELETE", URL, None, 412, &[], "");
        let e = c
            .delete(airpost_dav::ResourceKind::VCard, URL, "")
            .unwrap_err();
        assert_eq!(e.as_sync().unwrap().key, ETAG_CONFLICT);
        assert_eq!(f.requests().len(), 3);
    }
}

mod generic {
    use super::*;

    const CAL: &str = "https://dav.example.com/cal/";

    #[test]
    fn http_507_is_a_truncated_page() {
        let f = Fake::new();
        f.once(
            "REPORT",
            CAL,
            Some("<D:sync-token/>"),
            507,
            &[],
            &fixture("generic/sync_507.xml"),
        );
        f.once(
            "REPORT",
            CAL,
            Some("tok-1"),
            207,
            &[],
            &fixture("generic/sync_after_507.xml"),
        );
        let mut c = f.client();
        let r = complete(c.sync_collection(CAL, Collection::Calendar, ""));
        assert_eq!(r.pages, 2);
        assert_eq!(r.needed_hrefs(), vec!["/cal/a.ics", "/cal/b.ics"]);
        assert_eq!(r.sync_token, "tok-2");
    }

    #[test]
    fn empty_answer_without_token_falls_back() {
        // Yahoo CalDAV never returns a sync-token (DAVWorker.cpp:2112).
        let f = Fake::new();
        f.once(
            "REPORT",
            CAL,
            None,
            207,
            &[],
            &fixture("generic/sync_no_token.xml"),
        );
        let mut c = f.client();
        let r = complete(c.sync_collection(CAL, Collection::Calendar, ""));
        assert!(r.is_empty_without_token());
    }

    #[test]
    fn rate_limits_wait_before_the_next_request() {
        let f = Fake::new();
        f.once(
            "REPORT",
            CAL,
            None,
            429,
            &[("Retry-After", "30")],
            "slow down",
        );
        f.once("REPORT", CAL, None, 503, &[], "busy");
        f.on("REPORT", CAL, 207, &fixture("generic/sync_no_token.xml"));
        let mut c = f.client();
        let range = CalendarSyncRange::around(1_718_000_000);
        let e = c.calendar_etags(CAL, &range).unwrap_err();
        assert_eq!(http_status(&e), Some(429));
        assert_eq!(
            e.as_sync().unwrap().debuginfo,
            format!("{CAL} RETURNED slow down")
        );
        assert!(f.sleeps.lock().unwrap().is_empty());

        let e = c.calendar_etags(CAL, &range).unwrap_err();
        assert_eq!(http_status(&e), Some(503));
        assert_eq!(*f.sleeps.lock().unwrap(), vec![Duration::from_secs(30)]);

        c.calendar_etags(CAL, &range).unwrap();
        assert_eq!(
            *f.sleeps.lock().unwrap(),
            vec![
                Duration::from_secs(30),
                Duration::from_secs(30),
                Duration::from_millis(100)
            ]
        );
        assert_eq!(c.rate_limiter().backoff_ms(), 100);
    }

    #[test]
    fn gmx_skips_sync_collection() {
        let f = Fake::new();
        let mut c = f.client();
        assert!(matches!(
            c.sync_collection(
                "https://caldav.gmx.net/begenda/dav/u/calendar/",
                Collection::Calendar,
                ""
            ),
            SyncCollection::Unsupported(None)
        ));
        assert!(f.requests().is_empty());
    }

    #[test]
    fn yahoo_carddav_ends_on_webmail() {
        let f = Fake::new();
        f.redirect(
            "https://carddav.address.yahoo.com/.well-known/carddav",
            "https://mail.yahoo.com/",
        );
        let mut c = f.client();
        assert_eq!(
            c.discover_address_books(&DavHost::new("carddav.address.yahoo.com"))
                .unwrap(),
            None
        );
    }

    #[test]
    fn errors_keep_the_cpp_shapes() {
        let f = Fake::new();
        f.once("PROPFIND", CAL, None, 207, &[], "<html>not xml");
        f.once("PROPFIND", CAL, None, 405, &[], "Method Not Allowed");
        let mut c = f.client();
        let home = CalendarHome {
            url: CAL.into(),
            ..Default::default()
        };
        let e = c.list_calendars(&home).unwrap_err();
        assert_eq!(e.as_sync().unwrap().key, "Unable to parse CalDav XML");
        let e = c.list_calendars(&home).unwrap_err();
        let s = e.as_sync().unwrap();
        assert_eq!(s.key, "Invalid Response Code: 405");
        assert_eq!(s.debuginfo, format!("{CAL} RETURNED Method Not Allowed"));
        assert!(s.retryable && !s.offline);
    }

    #[test]
    fn missing_principal_or_home_set_means_no_caldav() {
        let f = Fake::new();
        f.on(
            "PROPFIND",
            "https://dav.example.com/",
            207,
            "<multistatus xmlns='DAV:'/>",
        );
        let mut c = f.client();
        assert_eq!(
            c.discover_calendar_home(&DavHost::new("dav.example.com"))
                .unwrap(),
            None
        );
    }

    #[test]
    fn txt_context_path_skips_well_known() {
        let f = Fake::new();
        f.on(
            "PROPFIND",
            "https://dav.example.com/dav/",
            207,
            &fixture("radicale/principal.xml"),
        );
        f.on(
            "PROPFIND",
            "https://dav.example.com/alice/",
            207,
            &fixture("radicale/calendar_home_set.xml"),
        );
        let mut c = f.client();
        let host = DavHost {
            context_path: Some("/dav/".into()),
            ..DavHost::new("dav.example.com")
        };
        let home = c.discover_calendar_home(&host).unwrap().unwrap();
        assert_eq!(home.url, "https://dav.example.com/alice/");
        assert!(f.requests().iter().all(|r| !r.url.contains(".well-known")));
    }

    #[test]
    fn server_header_sets_the_profile() {
        let f = Fake::new();
        f.on_with(
            "PROPFIND",
            "https://dav.example.com/",
            207,
            &[("Server", "Radicale/3.2.1")],
            "<multistatus xmlns='DAV:'/>",
        );
        let mut c = f.client();
        assert_eq!(c.profile_for(CAL).server, DavServer::Generic);
        c.discover_calendar_home(&DavHost::new("dav.example.com"))
            .unwrap();
        assert_eq!(c.profile_for(CAL).server, DavServer::Radicale);
    }
}
