// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The CardDAV and CalDAV task phases against a scripted server: a new group card across
//! a sync that lands before its remote phase, and what a refused remote phase leaves
//! behind for DestroyContact, DestroyContactGroup and SyncbackEvent. TaskProcessor.cpp and
//! DAVWorker.cpp are the reference; the differences are in `contacts.rs` and `events.rs`.

#[path = "../../ms-sync/tests/support/dav_server.rs"]
mod dav_server;
#[path = "../../ms-dav/tests/common/mod.rs"]
mod fake;

use std::sync::Arc;

use dav_server::*;
use fake::Fake;
use ms_core::{ids, Security};
use ms_imap::{ImapAuth, ImapConfig, Session};
use ms_store::{Calendar, Contact, ContactGroup, Event, Model, Query, Store, Task};
use ms_sync::dav::DavWorker;
use ms_sync::RemoteTaskRunner;
use ms_tasks::TaskProcessor;
use serde_json::{json, Value};

const WORK: &str = "/dav/cal/me/work/";
const WORK_URL: &str = "https://dav.test/dav/cal/me/work/";

struct Env {
    _dir: tempfile::TempDir,
    store: Store,
    f: Fake,
    session: Session,
}

fn env() -> Env {
    let (dir, store) = store();
    // These tasks never touch IMAP; the session is never connected.
    let session = Session::new(ImapConfig::new(
        "127.0.0.1",
        1,
        Security::Clear,
        "me",
        ImapAuth::Password(String::new()),
    ));
    Env {
        _dir: dir,
        store,
        f: Fake::new(),
        session,
    }
}

fn card_href(uid: &str) -> String {
    format!("{BOOK_PATH}{uid}.vcf")
}

fn card_url(uid: &str) -> String {
    format!("https://dav.test{}", card_href(uid))
}

fn processor<'a>(store: &'a Store, f: &Fake) -> TaskProcessor<'a> {
    let f = f.clone();
    TaskProcessor::new(account(), store).with_dav(Arc::new(move || {
        (
            f.client(),
            Box::new(Resolver) as Box<dyn ms_dav::HostResolver>,
        )
    }))
}

impl Env {
    fn processor(&self) -> TaskProcessor<'_> {
        processor(&self.store, &self.f)
    }

    fn local(&self, class: &str, data: Value) -> Task {
        let mut task = Task::create(class, ACCOUNT, &data);
        self.processor().perform_local(&mut task).unwrap();
        assert_eq!(
            task.status(),
            "remote",
            "{class} local: {}",
            task.data["error"]
        );
        task
    }

    fn remote(&mut self, task: &mut Task) {
        let mut p = processor(&self.store, &self.f);
        p.perform_remote(task, &mut self.session).unwrap();
        assert_eq!(task.status(), "complete");
    }

    fn worker(&self) -> DavWorker<'_> {
        DavWorker::with_client(account(), &self.store, self.f.client(), Box::new(Resolver))
    }

    /// The address book as the server holds it, read by the etag listing.
    fn serve_book(&self, ctag: &str, cards: &[(&str, &str, String)]) {
        self.f.reset();
        address_book_routes(&self.f, ctag);
        self.f.on_body("REPORT", BOOK, "sync-collection", 501, "");
        let listing: Vec<String> = cards
            .iter()
            .map(|(uid, etag, _)| etag_entry(&card_href(uid), etag))
            .collect();
        self.f.on_body(
            "REPORT",
            BOOK,
            "addressbook-query",
            207,
            &multistatus(&listing, None),
        );
        let data: Vec<String> = cards
            .iter()
            .map(|(uid, etag, vcf)| card_entry(&card_href(uid), etag, vcf))
            .collect();
        self.f.on_body(
            "REPORT",
            BOOK,
            "addressbook-multiget",
            207,
            &multistatus(&data, None),
        );
    }

    fn contact(&self, id: &str) -> Option<Contact> {
        self.store
            .find::<Contact>(&Query::new().equal("id", id))
            .unwrap()
    }

    fn group(&self, id: &str) -> Option<ContactGroup> {
        self.store
            .find::<ContactGroup>(&Query::new().equal("id", id))
            .unwrap()
    }

    fn event(&self, id: &str) -> Option<Event> {
        self.store
            .find::<Event>(&Query::new().equal("id", id))
            .unwrap()
    }
}

fn error_key(task: &Task) -> String {
    task.data["error"]["key"].as_str().unwrap_or("").to_string()
}

fn alice() -> (&'static str, &'static str, String) {
    ("a", "ea", vcard("a", "Alice", "alice@example.test"))
}

fn bob() -> (&'static str, &'static str, String) {
    ("b", "eb", vcard("b", "Bob", "bob@example.test"))
}

fn climbing() -> (&'static str, &'static str, String) {
    ("g", "eg", group_vcard("g", "Climbing", &["a"]))
}

#[test]
fn a_new_group_card_survives_an_etag_listing_before_its_remote_phase() {
    let mut e = env();
    e.serve_book("c1", &[alice()]);
    let mut w = e.worker();
    w.run_contacts().unwrap();

    let mut task = e.local(
        "SyncbackContactGroupTask",
        json!({"group": {"name": "Book Club"}}),
    );
    let id = task.data["group"]["id"].as_str().unwrap().to_string();
    assert_eq!(e.group(&id).unwrap().name(), "Book Club");

    // The server does not list the new card yet (its etag is still empty): the listing
    // must not take it for a card deleted on the server.
    e.serve_book("c2", &[alice()]);
    w.run_contacts().unwrap();
    drop(w);
    assert!(e.contact(&id).is_some());
    assert!(e.group(&id).is_some());

    let vcf = e.contact(&id).unwrap().info()["vcf"]
        .as_str()
        .unwrap()
        .to_string();
    e.f.reset();
    e.f.on("PUT", &card_url(&id), 201, "");
    e.f.on_body(
        "REPORT",
        BOOK,
        "addressbook-multiget",
        207,
        &multistatus(&[card_entry(&card_href(&id), "en", &vcf)], None),
    );
    e.remote(&mut task);
    assert!(task.data["error"].is_null(), "{}", task.data["error"]);
    let card = e.contact(&id).unwrap();
    assert_eq!(card.etag(), "en");
    assert!(card.hidden());
    assert!(!card.book_id().is_empty());
    assert_eq!(e.group(&id).unwrap().book_id(), card.book_id());
    check_invariants(&e.store);
}

#[test]
fn contacts_a_failed_delete_did_not_remove_are_shown_again() {
    let mut e = env();
    e.serve_book("c1", &[alice(), bob(), climbing()]);
    e.worker().run_contacts().unwrap();

    let mut task = e.local(
        "DestroyContactTask",
        json!({"contacts": [{"id": "a"}, {"id": "b"}]}),
    );
    assert!(e.contact("a").unwrap().hidden());
    assert!(e.contact("b").unwrap().hidden());

    e.f.reset();
    e.f.on("DELETE", &card_url("a"), 204, "");
    e.f.on("DELETE", &card_url("b"), 503, "");
    e.remote(&mut task);
    assert!(!error_key(&task).is_empty());
    assert!(e.contact("a").is_none());
    assert!(
        !e.contact("b").unwrap().hidden(),
        "b is still on the server"
    );
    assert!(e.contact("g").unwrap().hidden(), "group cards stay hidden");
    check_invariants(&e.store);
}

#[test]
fn a_failed_group_delete_rebuilds_the_group() {
    let mut e = env();
    e.serve_book("c1", &[alice(), climbing()]);
    e.worker().run_contacts().unwrap();

    let mut task = e.local("DestroyContactGroupTask", json!({"group": {"id": "g"}}));
    assert!(e.group("g").is_none());

    e.f.reset();
    e.f.on("DELETE", &card_url("g"), 500, "");
    e.remote(&mut task);
    assert!(!error_key(&task).is_empty());
    let g = e.group("g").expect("the card is still on the server");
    assert_eq!(g.name(), "Climbing");
    assert_eq!(g.members(&e.store).unwrap(), vec!["a".to_string()]);
    check_invariants(&e.store);
}

fn synced_event(e: &Env) -> Event {
    let mut cal = Calendar::new(&ids::calendar_id(ACCOUNT, WORK), ACCOUNT);
    cal.set_path(WORK);
    cal.set_name("Work");
    e.store.save(&mut cal).unwrap();
    let ics = ics("p1", "20240611T090000Z", "Planning");
    let parsed = ms_dav::ical::parse_events(&ics);
    let fields = ms_sync::dav::ics_event_fields(&parsed[0]);
    let mut ev = Event::create("srv1", ACCOUNT, cal.id(), &ics, &fields);
    ev.set_href(&format!("{WORK}p1.ics"));
    e.store.save(&mut ev).unwrap();
    ev
}

#[test]
fn a_refused_event_update_is_restored_from_the_server() {
    let mut e = env();
    let ev = synced_event(&e);
    let mut task = e.local(
        "SyncbackEventTask",
        json!({
            "calendarId": ev.calendar_id(),
            "event": {"id": ev.id(), "ics": ics("p1", "20240611T090000Z", "Edited here")},
        }),
    );
    assert!(e.event(ev.id()).unwrap().ics_data().contains("Edited here"));

    calendar_home_routes(&e.f, &[]);
    e.f.on("PUT", &format!("https://dav.test{WORK}p1.ics"), 403, "");
    e.f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-multiget",
        207,
        &multistatus(
            &[event_entry(
                &format!("{WORK}p1.ics"),
                "srv1",
                &ics("p1", "20240611T090000Z", "Planning"),
            )],
            None,
        ),
    );
    e.remote(&mut task);
    assert!(!error_key(&task).is_empty());
    let stored = e.event(ev.id()).unwrap();
    assert!(stored.ics_data().contains("SUMMARY:Planning"));
    assert_eq!(stored.etag(), "srv1");
    check_invariants(&e.store);
}

#[test]
fn a_refused_event_create_removes_the_event() {
    let mut e = env();
    let ev = synced_event(&e);
    let mut task = e.local(
        "SyncbackEventTask",
        json!({
            "calendarId": ev.calendar_id(),
            "event": {"ics": ics("new1", "20240612T090000Z", "Offsite")},
        }),
    );
    let id = task.data["event"]["id"].as_str().unwrap().to_string();
    assert!(e.event(&id).is_some());

    calendar_home_routes(&e.f, &[]);
    e.f.on("PUT", &format!("https://dav.test{WORK}new1.ics"), 403, "");
    e.remote(&mut task);
    assert!(!error_key(&task).is_empty());
    assert!(e.event(&id).is_none(), "the sync would never remove it");
    assert!(e.event(ev.id()).is_some());
    check_invariants(&e.store);
}

#[test]
fn an_undo_is_resent_at_the_current_sequence() {
    let e = env();
    let ev = synced_event(&e);
    let edited =
        ics("p1", "20240611T100000Z", "Moved").replace("END:VEVENT", "SEQUENCE:4\r\nEND:VEVENT");
    e.local(
        "SyncbackEventTask",
        json!({"calendarId": ev.calendar_id(), "event": {"id": ev.id(), "ics": edited}}),
    );
    let snapshot = ics("p1", "20240611T090000Z", "Planning");
    let task = e.local(
        "SyncbackEventTask",
        json!({
            "calendarId": ev.calendar_id(),
            "event": {"id": ev.id(), "ics": snapshot},
            "restore": true,
            "tz": "Europe/Lisbon",
        }),
    );
    let stored = e.event(ev.id()).unwrap();
    assert!(
        stored.ics_data().contains("SUMMARY:Planning"),
        "{}",
        stored.ics_data()
    );
    assert!(
        stored.ics_data().contains("SEQUENCE:4"),
        "{}",
        stored.ics_data()
    );
    assert_eq!(task.data["event"]["ics"].as_str(), Some(stored.ics_data()));
    check_invariants(&e.store);
}
