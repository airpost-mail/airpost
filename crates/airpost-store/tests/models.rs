// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Model hooks outside the placement layer: plugin metadata, search tables, contact groups,
//! folder status, generic lookup and account reset.

mod common;

use airpost_core::Metadata;
use airpost_store::{
    Calendar, Contact, ContactBook, ContactGroup, Event, File, FilePart, Folder, IcsEventFields,
    Message, Model, Query, Task, Thread,
};
use common::{attrs, fixture, ACCOUNT};
use serde_json::json;

fn count(f: &common::Fixture, sql: &str) -> i64 {
    f.store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn metadata_rows_follow_versions_and_wake_the_expiration_worker() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("m@x", &inbox, 1, attrs(1, false), None);
    let mut m = f.load(msg.id());
    assert_eq!(
        m.upsert_metadata("snooze", &json!({"expiration": 1_800_000_000}), None),
        Some(1)
    );
    assert_eq!(m.upsert_metadata("empty", &json!({}), None), Some(1));
    // The expiration worker sleeps until later than this expiration, so the save wakes it.
    // Other tests save metadata for the same account and may wake it first; start over then.
    let signal = airpost_core::wake::expiration_signal(ACCOUNT);
    let worker = loop {
        let worker = {
            let signal = signal.clone();
            std::thread::spawn(move || signal.wait_until(1_900_000_000, &|| 1_700_000_000))
        };
        while !signal.is_waiting() && !worker.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if !worker.is_finished() {
            break worker;
        }
    };
    f.store.save(&mut m).unwrap();
    assert!(
        worker.join().unwrap(),
        "the save woke the expiration worker"
    );
    let (object_type, expiration): (String, i64) = f
        .store
        .conn()
        .query_row(
            "SELECT objectType, expiration FROM ModelPluginMetadata WHERE id = ?",
            [m.id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (object_type.as_str(), expiration),
        ("Message", 1_800_000_000)
    );
    assert_eq!(
        count(&f, "SELECT COUNT(*) FROM ModelPluginMetadata"),
        1,
        "empty values have no row"
    );
    f.assert_invariants();

    // A stale version is refused.
    let mut m = f.load(msg.id());
    assert_eq!(m.upsert_metadata("snooze", &json!({}), Some(1)), None);
    assert_eq!(m.upsert_metadata("snooze", &json!({}), Some(2)), Some(2));
    f.store.save(&mut m).unwrap();
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ModelPluginMetadata"), 0);
    f.assert_invariants();

    f.store.remove(&mut m).unwrap();
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ModelPluginMetadata"), 0);
}

#[test]
fn detached_metadata_attaches_on_first_save() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let id = airpost_core::ids::hash_id("m@x");
    f.store
        .save_detached_plugin_metadata(&Metadata {
            object_type: "Message".into(),
            object_id: id.clone(),
            account_id: ACCOUNT.into(),
            plugin_id: "send-reminders".into(),
            version: 3,
            value: json!({"expiration": 1_900_000_000}),
        })
        .unwrap();
    let msg = f.insert("m@x", &inbox, 1, attrs(1, false), None);
    assert_eq!(msg.id(), id);
    let m = f.load(&id);
    assert_eq!(m.json()["metadata"][0]["pluginId"], "send-reminders");
    assert_eq!(m.json()["metadata"][0]["v"], 3);
    assert_eq!(count(&f, "SELECT COUNT(*) FROM DetatchedPluginMetadata"), 0);
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ModelPluginMetadata"), 1);
    assert!(f
        .store
        .find_and_delete_detached_plugin_metadata(ACCOUNT, &id)
        .unwrap()
        .is_empty());
    f.assert_invariants();
}

#[test]
fn find_generic_by_class_name() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("m@x", &inbox, 1, attrs(1, false), None);
    let q = Query::new()
        .equal("id", msg.thread_id())
        .equal("accountId", ACCOUNT);
    let mut t = f.store.find_generic("thread", &q).unwrap().unwrap();
    assert_eq!(t.table(), "Thread");
    assert_eq!(t.upsert_metadata("p", &json!({"a": 1}), None), Some(1));
    t.save(&f.store).unwrap();
    assert_eq!(
        f.thread(msg.thread_id()).unwrap().json()["metadata"][0]["v"],
        1
    );
    assert!(f
        .store
        .find_generic("Message", &Query::new().equal("id", "nope"))
        .unwrap()
        .is_none());
    assert!(f.store.find_generic("Folder", &Query::new()).is_err());
    f.assert_invariants();
}

#[test]
fn contact_search_rows() {
    let f = fixture();
    let mut c = Contact::create("c1", ACCOUNT, "ann@example.com", 1, "mail");
    c.set_name("Ann");
    f.store.save(&mut c).unwrap();
    let content: String = f
        .store
        .conn()
        .query_row(
            "SELECT content FROM ContactSearch WHERE content_id = 'c1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(content, "ann example.com Ann");
    let (email, refs, source): (String, i64, String) = f
        .store
        .conn()
        .query_row("SELECT email, refs, source FROM Contact", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(
        (email.as_str(), refs, source.as_str()),
        ("ann@example.com", 1, "mail")
    );

    // Mail contacts keep their first search text; DAV contacts update it.
    c.set_name("Annie");
    f.store.save(&mut c).unwrap();
    let mut d = Contact::create("c2", ACCOUNT, "bob@example.com", 0, "carddav");
    f.store.save(&mut d).unwrap();
    d.set_name("Bob");
    f.store.save(&mut d).unwrap();
    let rows: Vec<String> = f
        .store
        .conn()
        .prepare("SELECT content FROM ContactSearch ORDER BY content_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows, ["ann example.com Ann", "bob example.com Bob"]);
    f.assert_invariants();
    f.store.remove(&mut d).unwrap();
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ContactSearch"), 1);
    f.assert_invariants();
}

#[test]
fn contact_group_membership_syncs_contacts() {
    let f = fixture();
    let mut book = ContactBook::create("b1", ACCOUNT);
    book.set_url("https://dav/");
    f.store.save(&mut book).unwrap();
    let mut g = ContactGroup::create("g1", ACCOUNT);
    g.set_name("Friends");
    g.set_book_id("b1");
    f.store.save(&mut g).unwrap();
    for id in ["c1", "c2", "c3"] {
        let mut c = Contact::create(id, ACCOUNT, &format!("{id}@x"), 0, "carddav");
        f.store.save(&mut c).unwrap();
    }
    g.sync_members(&f.store, &["c1".into(), "c2".into()])
        .unwrap();
    g.sync_members(&f.store, &["c2".into(), "c3".into()])
        .unwrap();
    let gis = |id: &str| {
        f.store
            .find::<Contact>(&Query::new().equal("id", id))
            .unwrap()
            .unwrap()
            .group_ids()
    };
    assert!(gis("c1").is_empty());
    assert_eq!(gis("c2"), ["g1"]);
    assert_eq!(gis("c3"), ["g1"]);
    let mut members = g.members(&f.store).unwrap();
    members.sort();
    assert_eq!(members, ["c2", "c3"]);
    f.assert_invariants();
    // Like ContactGroup::afterRemove, removing a group empties the join table but leaves
    // `gis` on its members (the DAV worker syncs members first); the check accepts that.
    f.store.remove(&mut g).unwrap();
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ContactContactGroup"), 0);
    assert_eq!(gis("c2"), ["g1"]);
    f.assert_invariants();
    let name: String = f
        .store
        .conn()
        .query_row("SELECT data FROM ContactBook", [], |r| r.get(0))
        .unwrap();
    assert!(name.contains("https://dav/"));
}

#[test]
fn event_columns_and_search() {
    let f = fixture();
    let mut cal = Calendar::new("cal1", ACCOUNT);
    cal.set_color("#AABBCCDD");
    cal.set_name("Home");
    f.store.save(&mut cal).unwrap();
    assert_eq!(cal.color(), "#aabbcc");
    let fields = IcsEventFields {
        uid: "uid-1".into(),
        recurrence_start: 100,
        recurrence_end: 200,
        summary: "Lunch".into(),
        attendees: vec!["a@x".into(), "b@x".into()],
        ..Default::default()
    };
    let mut e = Event::create("etag1", ACCOUNT, "cal1", "BEGIN:VCALENDAR", &fields);
    f.store.save(&mut e).unwrap();
    assert_eq!(e.status(), "CONFIRMED");
    let row: (String, String, i64, i64, String) = f
        .store
        .conn()
        .query_row(
            "SELECT icsuid, etag, recurrenceStart, recurrenceEnd, recurrenceId FROM Event",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        ("uid-1".into(), "etag1".into(), 100, 200, String::new())
    );
    let (title, participants): (String, String) = f
        .store
        .conn()
        .query_row("SELECT title, participants FROM EventSearch", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        (title.as_str(), participants.as_str()),
        ("Lunch", "a@x b@x")
    );
    f.assert_invariants();

    let mut changed = fields.clone();
    changed.summary = "Dinner".into();
    e.apply_ics_event_data("etag2", "/cal/1.ics", "BEGIN:VCALENDAR", &changed);
    f.store.save(&mut e).unwrap();
    let title: String = f
        .store
        .conn()
        .query_row("SELECT title FROM EventSearch", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "Dinner");
    assert_eq!(count(&f, "SELECT COUNT(*) FROM EventSearch"), 1);

    // Loaded from the database: no search content, the row is left alone.
    let mut loaded = f
        .store
        .find::<Event>(&Query::new().equal("id", e.id()))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.href(), "/cal/1.ics");
    f.store.save(&mut loaded).unwrap();
    assert_eq!(
        count(
            &f,
            "SELECT COUNT(*) FROM EventSearch WHERE title = 'Dinner'"
        ),
        1
    );
    f.assert_invariants();
    f.store.remove(&mut loaded).unwrap();
    assert_eq!(count(&f, "SELECT COUNT(*) FROM EventSearch"), 0);
    f.assert_invariants();
}

/// Violation check names, e.g. `contacts` for `[contacts] Contact c1: ...`.
fn violated(f: &common::Fixture) -> std::collections::BTreeSet<String> {
    airpost_store::invariants::check(f.store.conn(), &[])
        .unwrap()
        .iter()
        .map(|v| v[1..v.find(']').unwrap()].to_owned())
        .collect()
}

fn exec(f: &common::Fixture, sql: &str) {
    f.store.conn().execute_batch(sql).unwrap();
}

/// A populated PIM layer: a book, a group with two members, a calendar and an event, a
/// message and thread with plugin metadata.
fn pim_fixture() -> common::Fixture {
    let f = fixture();
    let mut book = ContactBook::create("b1", ACCOUNT);
    f.store.save(&mut book).unwrap();
    let mut g = ContactGroup::create("g1", ACCOUNT);
    g.set_name("Friends");
    g.set_book_id("b1");
    f.store.save(&mut g).unwrap();
    for id in ["c1", "c2"] {
        let mut c = Contact::create(id, ACCOUNT, &format!("{id}@x"), 0, "carddav");
        c.set_book_id("b1");
        c.set_etag("e");
        f.store.save(&mut c).unwrap();
    }
    g.sync_members(&f.store, &["c1".into(), "c2".into()])
        .unwrap();
    let mut cal = Calendar::new("cal1", ACCOUNT);
    cal.set_path("/cal/home/");
    f.store.save(&mut cal).unwrap();
    let fields = IcsEventFields {
        uid: "uid-1".into(),
        recurrence_start: 100,
        recurrence_end: 200,
        summary: "Lunch".into(),
        ..Default::default()
    };
    let mut e = Event::create("etag1", ACCOUNT, "cal1", "BEGIN:VCALENDAR", &fields);
    f.store.save(&mut e).unwrap();

    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("m@x", &inbox, 1, attrs(1, false), None);
    let mut m = f.load(msg.id());
    m.upsert_metadata("snooze", &json!({"expiration": 1_800_000_000}), None);
    f.store.save(&mut m).unwrap();
    let mut t = f.thread(msg.thread_id()).unwrap();
    t.upsert_metadata("notes", &json!({"text": "hi"}), None);
    f.store.save(&mut t).unwrap();
    f.assert_invariants();
    f
}

#[test]
fn invariants_catch_contact_drift() {
    let f = pim_fixture();
    exec(&f, "DELETE FROM ContactSearch WHERE content_id = 'c1'");
    assert_eq!(violated(&f), ["contacts".to_owned()].into());

    let f = pim_fixture();
    exec(&f, "UPDATE Contact SET email = 'other@x' WHERE id = 'c2'");
    assert_eq!(violated(&f), ["contacts".to_owned()].into());

    let f = pim_fixture();
    exec(&f, "DELETE FROM ContactContactGroup WHERE id = 'c1'");
    assert_eq!(violated(&f), ["contacts".to_owned()].into());

    let f = pim_fixture();
    exec(&f, "DELETE FROM ContactBook");
    assert_eq!(
        violated(&f),
        ["contact_groups".to_owned(), "contacts".to_owned()].into()
    );

    let f = pim_fixture();
    exec(&f, "UPDATE ContactGroup SET name = 'Enemies'");
    assert_eq!(violated(&f), ["contact_groups".to_owned()].into());
}

#[test]
fn invariants_catch_event_and_calendar_drift() {
    let f = pim_fixture();
    exec(&f, "UPDATE Event SET recurrenceEnd = 300");
    assert_eq!(violated(&f), ["events".to_owned()].into());

    let f = pim_fixture();
    exec(
        &f,
        "INSERT INTO EventSearch (content_id, title) SELECT id, 'x' FROM Event",
    );
    assert_eq!(violated(&f), ["events".to_owned()].into());

    let f = pim_fixture();
    exec(&f, "DELETE FROM Calendar");
    assert_eq!(violated(&f), ["events".to_owned()].into());

    let f = pim_fixture();
    let mut dup = Calendar::new("cal2", ACCOUNT);
    dup.set_path("/cal/home/");
    f.store.save(&mut dup).unwrap();
    assert_eq!(violated(&f), ["calendars".to_owned()].into());

    let f = pim_fixture();
    exec(&f, "UPDATE Calendar SET accountId = 'other'");
    assert_eq!(violated(&f), ["calendars".to_owned()].into());
}

#[test]
fn invariants_catch_plugin_metadata_drift() {
    let f = pim_fixture();
    exec(
        &f,
        "DELETE FROM ModelPluginMetadata WHERE objectType = 'Thread'",
    );
    assert_eq!(violated(&f), ["plugin_metadata".to_owned()].into());

    let f = pim_fixture();
    exec(&f, "UPDATE ModelPluginMetadata SET expiration = NULL");
    assert_eq!(violated(&f), ["plugin_metadata".to_owned()].into());

    let f = pim_fixture();
    exec(
        &f,
        "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value) VALUES ('nope', 'acc1', 'Message', 'p')",
    );
    assert_eq!(violated(&f), ["plugin_metadata".to_owned()].into());
    let v = airpost_store::invariants::check(f.store.conn(), &["plugin_metadata"]).unwrap();
    assert!(v.is_empty(), "{v:?}");
    assert!(airpost_store::invariants::check(f.store.conn(), &["no_such_check"]).is_err());
}

#[test]
fn scalar_metadata_values_get_a_join_row() {
    // MailModel::afterSave skips only `size() == 0` values (null, {}, []); false is size 1.
    let f = fixture();
    let n = |f: &common::Fixture| count(f, "SELECT COUNT(*) FROM ModelPluginMetadata");
    let before = n(&f);
    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("scalar@x", &inbox, 2, attrs(2, false), None);
    let mut m = f.load(msg.id());
    m.upsert_metadata("flag", &json!(false), None);
    m.upsert_metadata("gone", &json!({}), None);
    f.store.save(&mut m).unwrap();
    assert_eq!(n(&f), before + 1);
    f.assert_invariants();
}

#[test]
fn mutate_vcf_in_info() {
    let mut c = Contact::create("c1", ACCOUNT, "a@x", 0, "carddav");
    c.set_info(json!({"vcf": "BEGIN:VCARD\r\nFN:A\r\nEND:VCARD", "href": "/a.vcf"}));
    // An incomplete card leaves info alone.
    assert!(!c.mutate_vcf_in_info(|_| None));
    assert!(c.mutate_vcf_in_info(|vcf| Some(vcf.replace("FN:A", "FN:B"))));
    assert_eq!(
        c.info(),
        &json!({"vcf": "BEGIN:VCARD\r\nFN:B\r\nEND:VCARD", "href": "/a.vcf"})
    );
}

#[test]
fn files_and_tasks() {
    let f = fixture();
    let msg = Message::new(json!({"id": "m1", "aid": ACCOUNT, "v": 0}));
    let png = FilePart {
        part_id: "1.2",
        content_type: Some("image/png"),
        size: 100,
        ..Default::default()
    };
    let mut file = File::create("f1", &msg, &png);
    assert_eq!(file.filename(), "Unnamed Image.png");
    assert_eq!(file.message_id(), "m1");
    assert_eq!(
        file.to_json().get("contentId"),
        Some(&serde_json::Value::Null)
    );
    f.store.save(&mut file).unwrap();
    let pdf = FilePart {
        part_id: "2",
        content_id: Some("cid1"),
        filename: "a/b:c?.pdf",
        size: 1,
        ..Default::default()
    };
    let mut named = File::create("f2", &msg, &pdf);
    assert_eq!(named.safe_filename(), "a-b-c-.pdf");
    f.store.save(&mut named).unwrap();
    let filename: String = f
        .store
        .conn()
        .query_row("SELECT filename FROM File WHERE id = 'f2'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(filename, "a/b:c?.pdf");

    let mut m = msg.clone();
    m.set_files(&[file, named]);
    assert_eq!(
        m.file_count_for_thread_list(),
        1,
        "small inline parts are not attachments"
    );

    let mut t = Task::create("SyncbackMetadataTask", ACCOUNT, &json!({"pluginId": "p"}));
    f.store.save(&mut t).unwrap();
    t.set_status("remote");
    f.store.save(&mut t).unwrap();
    let status: String = f
        .store
        .conn()
        .query_row("SELECT status FROM Task", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "remote");
    let loaded = f
        .store
        .find::<Task>(&Query::new().equal("status", "remote"))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.class(), "SyncbackMetadataTask");
    assert_eq!(loaded.json()["pluginId"], "p");
    assert_eq!(loaded.version(), 2);
}

#[test]
fn unknown_keys_survive_a_save() {
    let f = fixture();
    let mut t = Task::from_json(json!({"id": "t1", "aid": ACCOUNT, "v": 0, "__cls": "X", "status": "local", "clientOnly": {"a": [1]}})).unwrap();
    f.store.save(&mut t).unwrap();
    let loaded = f
        .store
        .find::<Task>(&Query::new().equal("id", "t1"))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.json()["clientOnly"], json!({"a": [1]}));
}

#[test]
fn save_folder_status_merges_changed_keys() {
    let f = fixture();
    let mut inbox = f.folder("INBOX", "inbox");
    inbox.local_status_mut().insert("uidnext".into(), json!(10));
    inbox.local_status_mut().insert("busy".into(), json!(true));
    f.store.save(&mut inbox).unwrap();

    let mut worker_copy = f
        .store
        .find::<Folder>(&Query::new().equal("id", inbox.id()))
        .unwrap()
        .unwrap();
    let initial = worker_copy.local_status().clone();
    // The harness resets a key in the live database meanwhile.
    let mut live = f
        .store
        .find::<Folder>(&Query::new().equal("id", inbox.id()))
        .unwrap()
        .unwrap();
    live.local_status_mut().insert("lastDeep".into(), json!(0));
    f.store.save(&mut live).unwrap();

    worker_copy
        .local_status_mut()
        .insert("uidnext".into(), json!(20));
    f.store
        .save_folder_status(&mut worker_copy, &initial)
        .unwrap();
    let stored = f
        .store
        .find::<Folder>(&Query::new().equal("id", inbox.id()))
        .unwrap()
        .unwrap();
    assert_eq!(stored.local_status()["uidnext"], 20);
    assert_eq!(stored.local_status()["lastDeep"], 0);
    assert_eq!(stored.local_status()["busy"], true);
}

#[test]
fn folder_cache_sees_saves_from_other_stores() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    assert_eq!(
        f.store
            .folder_by_id(ACCOUNT, inbox.id())
            .unwrap()
            .unwrap()
            .role(),
        "inbox"
    );
    let other = airpost_store::Store::open(f.dir.path(), f.stream.clone()).unwrap();
    let mut renamed = other
        .find::<Folder>(&Query::new().equal("id", inbox.id()))
        .unwrap()
        .unwrap();
    renamed.set_role("archive");
    other.save(&mut renamed).unwrap();
    assert_eq!(
        f.store
            .folder_by_id(ACCOUNT, inbox.id())
            .unwrap()
            .unwrap()
            .role(),
        "archive"
    );
    let label = f.label("Work", "");
    assert_eq!(f.store.all_labels_cache(ACCOUNT).unwrap().len(), 1);
    let as_folder = f.store.folder_by_id(ACCOUNT, label.id()).unwrap().unwrap();
    assert_eq!(as_folder.to_json()["__cls"], "Label");
}

#[test]
fn queries() {
    let f = fixture();
    for (i, s) in ["local", "remote", "complete"].iter().enumerate() {
        let mut t = Task::create("A", ACCOUNT, &json!({"n": i}));
        t.set_status(s);
        f.store.save(&mut t).unwrap();
    }
    let q = Query::new()
        .equal("accountId", ACCOUNT)
        .order_by("status", true)
        .limit(2);
    let found = f.store.find_all::<Task>(&q).unwrap();
    assert_eq!(
        found.iter().map(|t| t.status()).collect::<Vec<_>>(),
        ["complete", "local"]
    );
    let first = f.store.find::<Task>(&q).unwrap().unwrap();
    assert_eq!(first.status(), "complete");
    assert_eq!(f.store.count::<Task>(&q).unwrap(), 3);
    let ids: Vec<String> = found.iter().map(|t| t.id().to_owned()).collect();
    assert_eq!(f.store.find_large_set::<Task>("id", &ids).unwrap().len(), 2);
    assert_eq!(
        f.store
            .find_all_map::<Task>(&Query::new(), "id")
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        f.store
            .count::<Task>(&Query::new().equal_any::<&str>("id", []))
            .unwrap(),
        0
    );
}

#[test]
fn reset_for_account_clears_every_table() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("m@x", &inbox, 1, attrs(1, true), None);
    let mut m = f.load(msg.id());
    m.upsert_metadata("p", &json!({"x": 1}), None);
    f.store.save(&mut m).unwrap();
    let mut c = Contact::create("c1", ACCOUNT, "a@x", 1, "mail");
    f.store.save(&mut c).unwrap();
    let mut t = Task::create("A", ACCOUNT, &json!({}));
    f.store.save(&mut t).unwrap();
    f.store
        .save_key_value(&format!("cursor-{ACCOUNT}"), "1234")
        .unwrap();

    f.store.reset_for_account(ACCOUNT, true).unwrap();
    for table in [
        "Message",
        "Thread",
        "Folder",
        "MessageFolder",
        "MessageOrphan",
        "ThreadCategory",
        "ThreadCounts",
        "ThreadReference",
        "ThreadSearch",
        "Contact",
        "ContactSearch",
        "Task",
        "ModelPluginMetadata",
    ] {
        assert_eq!(
            count(&f, &format!("SELECT COUNT(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    // The message's metadata waits for the resync under its Message-ID.
    assert_eq!(
        count(
            &f,
            "SELECT COUNT(*) FROM DetatchedPluginMetadata WHERE objectId = '<m@x>' AND pluginId = 'p'"
        ),
        1
    );
    assert_eq!(
        f.store
            .get_key_value(&format!("cursor-{ACCOUNT}"))
            .unwrap()
            .as_deref(),
        Some("0")
    );
    assert!(f.thread(msg.thread_id()).is_none());
    let _ = Thread::TABLE;
}

/// TD7: removed threads' references would otherwise only grow the table.
#[test]
fn removing_a_thread_drops_its_references() {
    let f = fixture();
    let mut kept = Thread::create("m1", ACCOUNT, "Kept", 0);
    let mut gone = Thread::create("m2", ACCOUNT, "Gone", 0);
    f.store.save(&mut kept).unwrap();
    f.store.save(&mut gone).unwrap();
    let refs = ["<root@x>".to_owned()];
    f.store
        .upsert_thread_references(kept.id(), ACCOUNT, "<a@x>", &refs)
        .unwrap();
    f.store
        .upsert_thread_references(gone.id(), ACCOUNT, "<b@x>", &refs)
        .unwrap();

    f.store.remove(&mut gone).unwrap();
    let threads: Vec<String> = f
        .store
        .conn()
        .prepare("SELECT DISTINCT threadId FROM ThreadReference")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(threads, [kept.id().to_owned()]);
}

#[test]
fn removing_an_account_drops_its_plugin_metadata() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("m@x", &inbox, 1, attrs(1, true), None);
    let mut m = f.load(msg.id());
    m.upsert_metadata("send-later", &json!({"expiration": 1_900_000_000}), None);
    f.store.save(&mut m).unwrap();
    let mut t = f.thread(msg.thread_id()).unwrap();
    t.upsert_metadata("thread-snooze", &json!({"expiration": 1_900_000_100}), None);
    f.store.save(&mut t).unwrap();
    // Rows an earlier reset kept go too.
    f.store.reset_for_account(ACCOUNT, true).unwrap();
    assert!(count(&f, "SELECT COUNT(*) FROM DetatchedPluginMetadata") > 0);
    f.store
        .conn()
        .execute(
            "INSERT INTO DetatchedPluginMetadata (objectId, objectType, accountId, pluginId, value, version) \
             VALUES ('<o@x>', 'Message', 'other', 'send-later', '{}', 1)",
            [],
        )
        .unwrap();

    f.store.reset_for_account(ACCOUNT, false).unwrap();
    assert_eq!(
        count(
            &f,
            &format!("SELECT COUNT(*) FROM DetatchedPluginMetadata WHERE accountId = '{ACCOUNT}'")
        ),
        0
    );
    assert_eq!(
        count(&f, "SELECT COUNT(*) FROM DetatchedPluginMetadata"),
        1,
        "another account's rows stay"
    );
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ModelPluginMetadata"), 0);
}

#[test]
fn reset_keeps_metadata_for_the_resync() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let first = f.insert("m@x", &inbox, 1, attrs(1, true), None);
    f.insert("m2@x", &inbox, 2, attrs(2, true), Some(first.thread_id()));
    let old_thread = first.thread_id().to_owned();
    let mut m = f.load(first.id());
    m.upsert_metadata("send-later", &json!({"expiration": 1_900_000_000}), None);
    f.store.save(&mut m).unwrap();
    let mut t = f.thread(&old_thread).unwrap();
    t.upsert_metadata("thread-snooze", &json!({"expiration": 1_900_000_100}), None);
    t.upsert_metadata("thread-snooze", &json!({"expiration": 1_900_000_200}), None);
    t.upsert_metadata("cleared", &json!({}), None);
    t.upsert_metadata("airpost-ai-triage", &json!({"category": "other"}), None);
    f.store.save(&mut t).unwrap();

    f.store.reset_for_account(ACCOUNT, true).unwrap();
    // One message row, and the thread's value under each of its two Message-IDs; the empty
    // value and the AI triage value are not kept.
    assert_eq!(count(&f, "SELECT COUNT(*) FROM DetatchedPluginMetadata"), 3);

    // The reply comes back first, in a thread of its own id.
    let reply = f.insert("m2@x", &inbox, 2, attrs(2, true), None);
    assert_ne!(reply.thread_id(), old_thread);
    let thread = f.thread(reply.thread_id()).unwrap();
    let snooze = thread.json()["metadata"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["pluginId"] == "thread-snooze")
        .cloned()
        .unwrap();
    assert_eq!(snooze["value"]["expiration"], 1_900_000_200);
    assert_eq!(snooze["v"], 2);
    let expiration: i64 = f
        .store
        .conn()
        .query_row(
            "SELECT expiration FROM ModelPluginMetadata WHERE id = ? AND value = 'thread-snooze'",
            [thread.id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(expiration, 1_900_000_200);
    // The thread's other row went with it; the message's waits for its message.
    assert_eq!(count(&f, "SELECT COUNT(*) FROM DetatchedPluginMetadata"), 1);

    let first = f.insert("m@x", &inbox, 1, attrs(1, true), Some(reply.thread_id()));
    let m = f.load(first.id());
    assert!(m.json()["metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["pluginId"] == "send-later" && e["value"]["expiration"] == 1_900_000_000));
    assert_eq!(count(&f, "SELECT COUNT(*) FROM DetatchedPluginMetadata"), 0);
    let threads: i64 = count(
        &f,
        "SELECT COUNT(*) FROM ModelPluginMetadata WHERE objectType = 'Thread'",
    );
    assert_eq!(threads, 1);
    f.assert_invariants();
}

#[test]
fn safe_display_name_matches_the_client() {
    let named = |name: &str, ct: &str| File {
        data: serde_json::json!({"id": "AbCdEf12", "filename": name, "contentType": ct}),
    };
    for (name, ct, want) in [
        ("report.pdf ", "", "report.pdf"),
        ("evil.exe. . ", "", "evil.exe"),
        ("report\u{202E}gpj.exe", "", "reportgpj.exe"),
        ("a\u{0007}b\u{0085}c.txt", "", "abc.txt"),
        ("a/b:c?.pdf", "", "a-b-c-.pdf"),
        ("con.txt", "", "_con.txt"),
        ("COM1", "", "_COM1"),
        ("lpt\u{00B2}.log", "", "_lpt\u{00B2}.log"),
        ("console.txt", "", "console.txt"),
        ("CONIN$.dat", "", "_CONIN$.dat"),
        ("", "image/png", "Unnamed Image.png"),
        ("", "image/gif", "Unnamed Attachment"),
        ("...", "", "-"),
    ] {
        assert_eq!(named(name, ct).safe_display_name(), want, "{name:?}");
    }
    let root = std::path::Path::new("/cfg/files");
    assert_eq!(
        named("x.pdf ", "").client_path_in(root),
        root.join("ab").join("cd").join("abcdef12").join("x.pdf")
    );
}

/// TD50: search rows are found by the rowid kept on the model, a row from before that by
/// `content_id` once, and a `searchRowId` naming another model's row is not trusted.
#[test]
fn search_rows_are_found_by_rowid() {
    let f = pim_fixture();
    let rowid_of = |id: &str| -> i64 {
        f.store
            .conn()
            .query_row(
                "SELECT rowid FROM ContactSearch WHERE content_id = ?",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let load = |id: &str| {
        f.store
            .find::<Contact>(&Query::new().equal("id", id))
            .unwrap()
            .unwrap()
    };
    let c1 = load("c1");
    assert_eq!(c1.data["searchRowId"], json!(rowid_of("c1")));

    // A row written before searchRowId: found by content_id and recorded.
    let mut legacy = load("c2");
    legacy.data.as_object_mut().unwrap().remove("searchRowId");
    let before = rowid_of("c2");
    legacy.data["name"] = json!("Two");
    f.store.save(&mut legacy).unwrap();
    assert_eq!(legacy.data["searchRowId"], json!(before));
    assert_eq!(rowid_of("c2"), before);

    // JSON carrying c1's rowid for c2 leaves c1's row alone.
    let mut stale = load("c2");
    stale.data["searchRowId"] = json!(rowid_of("c1"));
    stale.data["name"] = json!("Deux");
    f.store.save(&mut stale).unwrap();
    assert_eq!(stale.data["searchRowId"], json!(before));
    let content = |id: &str| -> String {
        f.store
            .conn()
            .query_row(
                "SELECT content FROM ContactSearch WHERE content_id = ?",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(content("c1"), "c1 x ");
    assert_eq!(content("c2"), "c2 x Deux");
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ContactSearch"), 2);
    f.assert_invariants();

    // Events alike.
    let mut e = f.store.find::<Event>(&Query::new()).unwrap().unwrap();
    let event_row: i64 = f
        .store
        .conn()
        .query_row("SELECT rowid FROM EventSearch", [], |r| r.get(0))
        .unwrap();
    assert_eq!(e.data["searchRowId"], json!(event_row));
    e.data.as_object_mut().unwrap().remove("searchRowId");
    let fields = IcsEventFields {
        uid: "uid-1".into(),
        summary: "Supper".into(),
        ..Default::default()
    };
    e.apply_ics_event_data("etag2", "/cal/1.ics", "BEGIN:VCALENDAR", &fields);
    f.store.save(&mut e).unwrap();
    assert_eq!(e.data["searchRowId"], json!(event_row));
    assert_eq!(
        count(
            &f,
            "SELECT COUNT(*) FROM EventSearch WHERE title = 'Supper'"
        ),
        1
    );
    assert_eq!(count(&f, "SELECT COUNT(*) FROM EventSearch"), 1);
    f.store.remove(&mut e).unwrap();
    assert_eq!(count(&f, "SELECT COUNT(*) FROM EventSearch"), 0);

    f.store.remove(&mut load("c1")).unwrap();
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ContactSearch"), 1);
    f.assert_invariants();
}

/// A new Event or Contact is inserted, so its search row is never looked up by `content_id`
/// (a scan of the whole FTS table); only a stored one without `searchRowId` is.
#[test]
fn new_search_rows_skip_the_content_id_scan() {
    let f = pim_fixture();
    let before = airpost_store::models::search_row_lookups();
    for i in 0..20 {
        let mut c = Contact::create(&format!("n{i}"), ACCOUNT, &format!("n{i}@x"), 1, "mail");
        f.store.save(&mut c).unwrap();
        let mut d = Contact::create(&format!("d{i}"), ACCOUNT, &format!("d{i}@x"), 0, "carddav");
        f.store.save(&mut d).unwrap();
        let fields = IcsEventFields {
            uid: format!("new-{i}"),
            summary: "New".into(),
            ..Default::default()
        };
        let mut e = Event::create("etag", ACCOUNT, "cal1", "BEGIN:VCALENDAR", &fields);
        f.store.save(&mut e).unwrap();
        // A second save finds the row by the rowid recorded on the first.
        d.set_name("Dee");
        f.store.save(&mut d).unwrap();
    }
    assert_eq!(airpost_store::models::search_row_lookups(), before);
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ContactSearch"), 2 + 40);
    assert_eq!(count(&f, "SELECT COUNT(*) FROM EventSearch"), 1 + 20);
    let rowids: Vec<i64> = {
        let mut stmt = f
            .store
            .conn()
            .prepare("SELECT rowid FROM ContactSearch")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let mut unique = rowids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), rowids.len());

    // A stored contact from before searchRowId is looked up once.
    let mut legacy = f
        .store
        .find::<Contact>(&Query::new().equal("id", "d0"))
        .unwrap()
        .unwrap();
    legacy.data.as_object_mut().unwrap().remove("searchRowId");
    f.store.save(&mut legacy).unwrap();
    assert_eq!(airpost_store::models::search_row_lookups(), before + 1);
    assert_eq!(count(&f, "SELECT COUNT(*) FROM ContactSearch"), 42);
    f.assert_invariants();
}
