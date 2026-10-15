//! DavWorker against a scripted server: the etag listings that cover servers without
//! sync-collection, the sync-token paths' edge cases (Yahoo's empty answer, 507 paging, an
//! empty multiget, the sync window, a deleted series master), calendar pruning and event
//! creates. DAVWorker.cpp is the reference; every case ends in `invariants::check`.

#[path = "support/dav_server.rs"]
mod dav_server;
#[path = "../../ms-dav/tests/common/mod.rs"]
mod fake;

use dav_server::*;
use fake::Fake;
use ms_core::ids;
use ms_store::{Calendar, Contact, ContactBook, ContactGroup, Event, Model, Query, Store};
use ms_sync::dav::DavWorker;

fn worker<'a>(store: &'a Store, f: &Fake) -> DavWorker<'a> {
    DavWorker::with_client(account(), store, f.client(), Box::new(Resolver))
}

fn contact(store: &Store, id: &str) -> Option<Contact> {
    store
        .find::<Contact>(&Query::new().equal("id", id))
        .unwrap()
}

fn group(store: &Store, id: &str) -> Option<ContactGroup> {
    store
        .find::<ContactGroup>(&Query::new().equal("id", id))
        .unwrap()
}

fn book(store: &Store) -> ContactBook {
    store
        .find::<ContactBook>(&Query::new().equal("accountId", ACCOUNT))
        .unwrap()
        .unwrap()
}

fn events(store: &Store) -> Vec<Event> {
    store.find_all::<Event>(&Query::new()).unwrap()
}

fn event_uids(store: &Store) -> Vec<String> {
    let mut uids: Vec<String> = events(store)
        .iter()
        .map(|e| format!("{}{}", e.ics_uid(), e.recurrence_id()))
        .collect();
    uids.sort();
    uids
}

fn calendar(store: &Store, path: &str) -> Option<Calendar> {
    store
        .find::<Calendar>(&Query::new().equal("id", ids::calendar_id(ACCOUNT, path)))
        .unwrap()
}

/// sync-collection refused with an HTTP error other than a token rejection: the worker
/// falls back to the etag listing (Zimbra, Posteo).
fn refuse_sync_collection(f: &Fake, url: &str) {
    f.on_body("REPORT", url, "sync-collection", 501, "");
}

fn card_href(uid: &str) -> String {
    format!("{BOOK_PATH}{uid}.vcf")
}

// -- CardDAV ------------------------------------------------------------------------------

#[test]
fn address_book_etag_listing_adds_and_deletes_cards() {
    let (_dir, store) = store();
    let f = Fake::new();
    let a = vcard("a", "Alice", "alice@example.test");
    let b = vcard("b", "Bob", "bob@example.test");
    let g = group_vcard("g", "Climbing", &["a"]);

    address_book_routes(&f, "c1");
    refuse_sync_collection(&f, BOOK);
    f.on_body(
        "REPORT",
        BOOK,
        "addressbook-query",
        207,
        &multistatus(
            &[
                etag_entry(&card_href("a"), "ea"),
                etag_entry(&card_href("b"), "eb"),
                etag_entry(&card_href("g"), "eg"),
            ],
            None,
        ),
    );
    f.on_body(
        "REPORT",
        BOOK,
        "addressbook-multiget",
        207,
        &multistatus(
            &[
                card_entry(&card_href("a"), "ea", &a),
                card_entry(&card_href("b"), "eb", &b),
                card_entry(&card_href("g"), "eg", &g),
            ],
            None,
        ),
    );
    let mut w = worker(&store, &f);
    w.run_contacts().unwrap();

    let alice = contact(&store, "a").unwrap();
    assert_eq!((alice.name(), alice.etag()), ("Alice", "ea"));
    assert!(!alice.hidden());
    assert_eq!(alice.book_id(), book(&store).id());
    assert!(contact(&store, "g").unwrap().hidden());
    let climbing = group(&store, "g").unwrap();
    assert_eq!(climbing.name(), "Climbing");
    assert_eq!(climbing.members(&store).unwrap(), vec!["a".to_string()]);
    assert_eq!(book(&store).ctag(), "c1");
    check_invariants(&store);

    // b and the group card are gone, c is new: only c is fetched.
    let c = vcard("c", "Carol", "carol@example.test");
    f.reset();
    address_book_routes(&f, "c2");
    refuse_sync_collection(&f, BOOK);
    f.on_body(
        "REPORT",
        BOOK,
        "addressbook-query",
        207,
        &multistatus(
            &[
                etag_entry(&card_href("a"), "ea"),
                etag_entry(&card_href("c"), "ec"),
            ],
            None,
        ),
    );
    f.on_body(
        "REPORT",
        BOOK,
        "addressbook-multiget",
        207,
        &multistatus(&[card_entry(&card_href("c"), "ec", &c)], None),
    );
    w.run_contacts().unwrap();

    assert!(contact(&store, "a").is_some());
    assert!(contact(&store, "b").is_none());
    assert!(contact(&store, "g").is_none());
    assert!(group(&store, "g").is_none(), "the group goes with its card");
    assert_eq!(contact(&store, "c").unwrap().name(), "Carol");
    let multigets = bodies(&f, "REPORT", BOOK, "addressbook-multiget");
    let last = multigets.last().unwrap();
    assert!(last.contains("c.vcf") && !last.contains("a.vcf"), "{last}");
    assert_eq!(book(&store).ctag(), "c2");
    check_invariants(&store);

    // Unchanged ctag: no listing at all.
    let before = f.requests().len();
    w.run_contacts().unwrap();
    let after: Vec<_> = f.requests()[before..]
        .iter()
        .map(|r| r.method.clone())
        .collect();
    assert_eq!(after, vec!["PROPFIND"], "only the ctag check");
}

#[test]
fn an_empty_multiget_keeps_the_address_book_token() {
    let (_dir, store) = store();
    let f = Fake::new();
    address_book_routes(&f, "c1");
    f.on_body(
        "REPORT",
        BOOK,
        "sync-collection",
        207,
        &multistatus(&[etag_entry(&card_href("a"), "ea")], Some("t1")),
    );
    f.on_body(
        "REPORT",
        BOOK,
        "addressbook-multiget",
        207,
        &multistatus(&[], None),
    );
    let mut w = worker(&store, &f);
    w.run_contacts().unwrap();
    assert!(contact(&store, "a").is_none());
    assert_eq!(
        book(&store).sync_token(),
        "",
        "t1 would skip the card for good"
    );

    // The next pass starts from scratch again and gets it.
    f.reset();
    address_book_routes(&f, "c2");
    f.on_body(
        "REPORT",
        BOOK,
        "<D:sync-token/>",
        207,
        &multistatus(&[etag_entry(&card_href("a"), "ea")], Some("t2")),
    );
    f.on_body(
        "REPORT",
        BOOK,
        "addressbook-multiget",
        207,
        &multistatus(
            &[card_entry(
                &card_href("a"),
                "ea",
                &vcard("a", "Alice", "alice@example.test"),
            )],
            None,
        ),
    );
    w.run_contacts().unwrap();
    assert!(contact(&store, "a").is_some());
    assert_eq!(book(&store).sync_token(), "t2");
    check_invariants(&store);
}

#[test]
fn a_rejected_address_book_token_restarts_from_scratch() {
    let (_dir, store) = store();
    let f = Fake::new();
    address_book_routes(&f, "c1");
    f.on_body(
        "REPORT",
        BOOK,
        "<D:sync-token/>",
        207,
        &multistatus(
            &[card_entry(
                &card_href("a"),
                "ea",
                &vcard("a", "Alice", "alice@example.test"),
            )],
            Some("t1"),
        ),
    );
    let mut w = worker(&store, &f);
    w.run_contacts().unwrap();
    assert_eq!(book(&store).sync_token(), "t1");

    f.reset();
    address_book_routes(&f, "c2");
    f.on_body("REPORT", BOOK, "<D:sync-token>t1<", 403, "");
    f.on_body(
        "REPORT",
        BOOK,
        "<D:sync-token/>",
        207,
        &multistatus(&[], Some("t9")),
    );
    w.run_contacts().unwrap();
    assert_eq!(book(&store).sync_token(), "t9");
    assert!(
        contact(&store, "a").is_some(),
        "a full re-sync deletes nothing"
    );
    check_invariants(&store);
}

// -- CalDAV -------------------------------------------------------------------------------

const WORK: &str = "/dav/cal/me/work/";
const WORK_URL: &str = "https://dav.test/dav/cal/me/work/";
const HOME_CAL: &str = "/dav/cal/me/home/";
const HOME_URL: &str = "https://dav.test/dav/cal/me/home/";

fn event_href(cal: &str, name: &str) -> String {
    format!("{cal}{name}.ics")
}

/// A daily series of five with its third occurrence moved, in one resource.
fn series(summary: &str) -> String {
    vcalendar(&[
        vevent(
            "s1",
            "20240610T100000Z",
            summary,
            "RRULE:FREQ=DAILY;COUNT=5\r\n",
        ),
        vevent(
            "s1",
            "20240612T150000Z",
            "Moved",
            "RECURRENCE-ID:20240612T100000Z\r\n",
        ),
    ])
}

#[test]
fn calendar_etag_listing_fetches_changes_and_removes_missing_events() {
    let (_dir, store) = store();
    let f = Fake::new();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k1")]);
    refuse_sync_collection(&f, WORK_URL);
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-query",
        207,
        &multistatus(
            &[
                etag_entry(&event_href(WORK, "s1"), "e1"),
                etag_entry(&event_href(WORK, "b"), "eb"),
            ],
            None,
        ),
    );
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-multiget",
        207,
        &multistatus(
            &[
                event_entry(&event_href(WORK, "s1"), "e1", &series("Standup")),
                event_entry(
                    &event_href(WORK, "b"),
                    "eb",
                    &ics("b", "20240611T090000Z", "Lunch"),
                ),
            ],
            None,
        ),
    );
    let mut w = worker(&store, &f);
    w.run_calendars().unwrap();
    assert_eq!(event_uids(&store), vec!["b", "s1", "s120240612T100000Z"]);
    let master_id = events(&store)
        .into_iter()
        .find(|e| e.ics_uid() == "s1" && e.recurrence_id().is_empty())
        .unwrap()
        .id()
        .to_string();
    assert_eq!(calendar(&store, WORK).unwrap().ctag(), "k1");
    check_invariants(&store);

    // b is gone and the series changed: the series is re-read and updated in place.
    f.reset();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k2")]);
    refuse_sync_collection(&f, WORK_URL);
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-query",
        207,
        &multistatus(&[etag_entry(&event_href(WORK, "s1"), "e2")], None),
    );
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-multiget",
        207,
        &multistatus(
            &[event_entry(
                &event_href(WORK, "s1"),
                "e2",
                &series("Standup (new room)"),
            )],
            None,
        ),
    );
    w.run_calendars().unwrap();
    assert_eq!(event_uids(&store), vec!["s1", "s120240612T100000Z"]);
    let master = store
        .find::<Event>(&Query::new().equal("id", &master_id))
        .unwrap()
        .expect("updated in place, same id");
    assert_eq!(master.etag(), "e2");
    assert!(master.ics_data().contains("Standup (new room)"));
    let multiget = bodies(&f, "REPORT", WORK_URL, "calendar-multiget");
    assert!(!multiget.last().unwrap().contains("b.ics"));
    check_invariants(&store);
}

#[test]
fn an_empty_sync_answer_without_a_token_falls_back_to_the_listing() {
    // Yahoo answers sync-collection with nothing and never sends a token.
    let (_dir, store) = store();
    let f = Fake::new();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k1")]);
    f.on_body(
        "REPORT",
        WORK_URL,
        "sync-collection",
        207,
        &multistatus(&[], None),
    );
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-query",
        207,
        &multistatus(&[etag_entry(&event_href(WORK, "b"), "eb")], None),
    );
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-multiget",
        207,
        &multistatus(
            &[event_entry(
                &event_href(WORK, "b"),
                "eb",
                &ics("b", "20240611T090000Z", "Lunch"),
            )],
            None,
        ),
    );
    worker(&store, &f).run_calendars().unwrap();
    assert_eq!(event_uids(&store), vec!["b"]);
    assert_eq!(calendar(&store, WORK).unwrap().sync_token(), "");
    check_invariants(&store);
}

#[test]
fn sync_collection_pages_through_http_and_in_body_507() {
    let (_dir, store) = store();
    let f = Fake::new();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k1")]);
    // Page 1: HTTP 507 with the partial multistatus as its body.
    f.once(
        "REPORT",
        WORK_URL,
        Some("<D:sync-token/>"),
        507,
        &[],
        &multistatus(&[etag_entry(&event_href(WORK, "a"), "ea")], Some("p1")),
    );
    // Page 2: a 207 carrying a response-level 507.
    f.once(
        "REPORT",
        WORK_URL,
        Some("<D:sync-token>p1<"),
        207,
        &[],
        &multistatus(
            &[
                etag_entry(&event_href(WORK, "b"), "eb"),
                truncated_entry(WORK),
            ],
            Some("p2"),
        ),
    );
    f.once(
        "REPORT",
        WORK_URL,
        Some("<D:sync-token>p2<"),
        207,
        &[],
        &multistatus(&[etag_entry(&event_href(WORK, "c"), "ec")], Some("p3")),
    );
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-multiget",
        207,
        &multistatus(
            &[
                event_entry(
                    &event_href(WORK, "a"),
                    "ea",
                    &ics("a", "20240611T090000Z", "A"),
                ),
                event_entry(
                    &event_href(WORK, "b"),
                    "eb",
                    &ics("b", "20240612T090000Z", "B"),
                ),
                event_entry(
                    &event_href(WORK, "c"),
                    "ec",
                    &ics("c", "20240613T090000Z", "C"),
                ),
            ],
            None,
        ),
    );
    worker(&store, &f).run_calendars().unwrap();
    assert_eq!(event_uids(&store), vec!["a", "b", "c"]);
    assert_eq!(calendar(&store, WORK).unwrap().sync_token(), "p3");
    let pages = bodies(&f, "REPORT", WORK_URL, "sync-collection");
    assert_eq!(pages.len(), 3);
    // Every page asks for the initial sync's properties.
    assert!(pages.iter().all(|p| !p.contains("calendar-data")));
    check_invariants(&store);
}

#[test]
fn the_sync_token_path_creates_only_events_inside_the_window() {
    let (_dir, store) = store();
    let f = Fake::new();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k1")]);
    f.on_body(
        "REPORT",
        WORK_URL,
        "<D:sync-token/>",
        207,
        &multistatus(
            &[
                etag_entry(&event_href(WORK, "near"), "en"),
                etag_entry(&event_href(WORK, "far"), "ef"),
            ],
            Some("t1"),
        ),
    );
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-multiget",
        207,
        &multistatus(
            &[
                event_entry(
                    &event_href(WORK, "near"),
                    "en",
                    &ics("near", "20240611T090000Z", "Near"),
                ),
                // Bedework and SOGo return events outside the window.
                event_entry(
                    &event_href(WORK, "far"),
                    "ef",
                    &ics("far", "20300101T090000Z", "Far"),
                ),
            ],
            None,
        ),
    );
    let mut w = worker(&store, &f);
    w.run_calendars().unwrap();
    assert_eq!(event_uids(&store), vec!["near"]);
    assert_eq!(calendar(&store, WORK).unwrap().sync_token(), "t1");

    // A known event moved out of the window is still updated, and a new one inside it
    // arriving with its data is created.
    f.reset();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k2")]);
    f.on_body(
        "REPORT",
        WORK_URL,
        "<D:sync-token>t1<",
        207,
        &multistatus(
            &[
                event_entry(
                    &event_href(WORK, "near"),
                    "en2",
                    &ics("near", "20310101T090000Z", "Moved far"),
                ),
                event_entry(
                    &event_href(WORK, "new"),
                    "ew",
                    &ics("new", "20240620T090000Z", "New"),
                ),
                event_entry(
                    &event_href(WORK, "far2"),
                    "ef2",
                    &ics("far2", "20310101T090000Z", "Far too"),
                ),
            ],
            Some("t2"),
        ),
    );
    w.run_calendars().unwrap();
    assert_eq!(event_uids(&store), vec!["near", "new"]);
    let near = events(&store)
        .into_iter()
        .find(|e| e.ics_uid() == "near")
        .unwrap();
    assert_eq!(near.etag(), "en2");
    assert_eq!(calendar(&store, WORK).unwrap().sync_token(), "t2");
    check_invariants(&store);
}

#[test]
fn a_deleted_master_takes_its_exceptions_with_it() {
    let (_dir, store) = store();
    let f = Fake::new();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k1")]);
    // The master and one exception are separate resources; another exception shares the
    // master's.
    f.on_body(
        "REPORT",
        WORK_URL,
        "<D:sync-token/>",
        207,
        &multistatus(
            &[
                event_entry(&event_href(WORK, "s1"), "e1", &series("Standup")),
                event_entry(
                    &event_href(WORK, "s1-x"),
                    "ex",
                    &vcalendar(&[vevent(
                        "s1",
                        "20240613T160000Z",
                        "Moved too",
                        "RECURRENCE-ID:20240613T100000Z\r\n",
                    )]),
                ),
                event_entry(
                    &event_href(WORK, "b"),
                    "eb",
                    &ics("b", "20240611T090000Z", "Lunch"),
                ),
            ],
            Some("t1"),
        ),
    );
    let mut w = worker(&store, &f);
    w.run_calendars().unwrap();
    assert_eq!(
        event_uids(&store),
        vec!["b", "s1", "s120240612T100000Z", "s120240613T100000Z"]
    );

    f.reset();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k2")]);
    f.on_body(
        "REPORT",
        WORK_URL,
        "<D:sync-token>t1<",
        207,
        &multistatus(&[gone_entry(&event_href(WORK, "s1"))], Some("t2")),
    );
    w.run_calendars().unwrap();
    assert_eq!(event_uids(&store), vec!["b"]);
    check_invariants(&store);
}

#[test]
fn calendars_are_pruned_only_when_the_listing_matches_one_held() {
    let (_dir, store) = store();
    let f = Fake::new();
    let one_event = |f: &Fake, url: &str, cal: &str, uid: &str| {
        f.on_body(
            "REPORT",
            url,
            "sync-collection",
            207,
            &multistatus(
                &[event_entry(
                    &event_href(cal, uid),
                    "e",
                    &ics(uid, "20240611T090000Z", uid),
                )],
                Some("t1"),
            ),
        );
    };
    calendar_home_routes(
        &f,
        &[
            calendar_entry(WORK, "Work", "k1"),
            calendar_entry(HOME_CAL, "Home", "h1"),
        ],
    );
    one_event(&f, WORK_URL, WORK, "w");
    one_event(&f, HOME_URL, HOME_CAL, "h");
    let mut w = worker(&store, &f);
    w.run_calendars().unwrap();
    assert_eq!(event_uids(&store), vec!["h", "w"]);

    // Home is no longer listed: it goes, with its events.
    f.reset();
    calendar_home_routes(&f, &[calendar_entry(WORK, "Work", "k1")]);
    w.run_calendars().unwrap();
    assert!(calendar(&store, HOME_CAL).is_none());
    assert_eq!(event_uids(&store), vec!["w"]);

    // Every href spelled differently (here absolute URLs): nothing matches what is held,
    // so nothing is pruned.
    f.reset();
    calendar_home_routes(&f, &[calendar_entry(WORK_URL, "Work", "k1")]);
    one_event(&f, WORK_URL, WORK, "w");
    w.run_calendars().unwrap();
    assert!(calendar(&store, WORK).is_some());
    assert!(calendar(&store, WORK_URL).is_some());
    check_invariants(&store);
}

// -- Event writes -------------------------------------------------------------------------

fn synced_calendar(store: &Store) -> Calendar {
    let mut cal = Calendar::new(&ids::calendar_id(ACCOUNT, WORK), ACCOUNT);
    cal.set_path(WORK);
    cal.set_name("Work");
    store.save(&mut cal).unwrap();
    cal
}

fn new_event(store: &Store, uid: &str) -> Event {
    let ics = ics(uid, "20240611T090000Z", "Planning");
    let parsed = ms_dav::ical::parse_events(&ics);
    let fields = ms_sync::dav::ics_event_fields(&parsed[0]);
    let mut e = Event::create("", ACCOUNT, &ids::calendar_id(ACCOUNT, WORK), &ics, &fields);
    store.save(&mut e).unwrap();
    e
}

#[test]
fn a_new_event_is_created_without_overwriting_and_an_update_carries_its_etag() {
    let (_dir, store) = store();
    synced_calendar(&store);
    let mut event = new_event(&store, "p1");
    let f = Fake::new();
    calendar_home_routes(&f, &[]);
    let url = format!("https://dav.test{}", event_href(WORK, "p1"));
    f.on("PUT", &url, 201, "");
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-multiget",
        207,
        &multistatus(
            &[event_entry(
                &event_href(WORK, "p1"),
                "srv1",
                &ics("p1", "20240611T090000Z", "Planning"),
            )],
            None,
        ),
    );
    let mut w = worker(&store, &f);
    w.write_and_resync_event(&mut event).unwrap();
    assert_eq!(
        (event.href(), event.etag()),
        (event_href(WORK, "p1").as_str(), "srv1")
    );
    w.write_and_resync_event(&mut event).unwrap();

    let puts: Vec<_> = f
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .collect();
    assert_eq!(puts.len(), 2);
    assert_eq!(puts[0].header("If-None-Match"), Some("*"));
    assert_eq!(puts[0].header("If-Match"), None);
    assert_eq!(puts[1].header("If-Match"), Some("srv1"));
    assert_eq!(puts[1].header("If-None-Match"), None);
    check_invariants(&store);
}

#[test]
fn a_create_over_an_unsynced_resource_is_an_etag_conflict() {
    let (_dir, store) = store();
    synced_calendar(&store);
    let mut event = new_event(&store, "p1");
    let f = Fake::new();
    calendar_home_routes(&f, &[]);
    let url = format!("https://dav.test{}", event_href(WORK, "p1"));
    f.on("PUT", &url, 412, "");
    let err = worker(&store, &f)
        .write_and_resync_event(&mut event)
        .unwrap_err();
    assert_eq!(err.as_sync().unwrap().key, ms_dav::ETAG_CONFLICT);
}

#[test]
fn a_refused_event_is_restored_from_the_server() {
    let (_dir, store) = store();
    synced_calendar(&store);
    let mut event = new_event(&store, "p1");
    let server_copy = ics("p1", "20240611T090000Z", "Planning");
    event.set_href(&event_href(WORK, "p1"));
    event.set_etag("srv1");
    event.set_ics_data(&ics("p1", "20240611T090000Z", "Edited here"));
    store.save(&mut event).unwrap();

    let f = Fake::new();
    calendar_home_routes(&f, &[]);
    f.on_body(
        "REPORT",
        WORK_URL,
        "calendar-multiget",
        207,
        &multistatus(
            &[event_entry(&event_href(WORK, "p1"), "srv1", &server_copy)],
            None,
        ),
    );
    assert!(worker(&store, &f)
        .restore_event_from_server(&mut event)
        .unwrap());
    let stored = store
        .find::<Event>(&Query::new().equal("id", event.id()))
        .unwrap()
        .unwrap();
    assert!(stored.ics_data().contains("SUMMARY:Planning"));
    assert!(!stored.ics_data().contains("Edited here"));
    assert_eq!(stored.etag(), "srv1");
    check_invariants(&store);
}
