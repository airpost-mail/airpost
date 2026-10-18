// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The reads behind `counts.get`, `tasks.list`, `metadata.list` and `hidden`
//! (docs/desktop-client-plan.md 3.2), one filter at a time, and the `ThreadCounts` deltas.

mod common;

use std::time::{Duration, Instant};

use common::{attrs, fixture, Fixture, ACCOUNT};
use ms_store::read::{MetadataFilter, MetadataObject, ReadStore, TaskFilter};
use ms_store::{Model, Task, THREAD_COUNTS};
use serde_json::{json, Value};

fn reads(f: &Fixture) -> ReadStore {
    ReadStore::open(f.dir.path()).unwrap()
}

fn accounts(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| (*s).to_owned()).collect()
}

fn ids(models: &[Value]) -> Vec<&str> {
    models.iter().map(|m| m["id"].as_str().unwrap()).collect()
}

/// The `ThreadCounts` delta rows flushed so far, last value per category.
fn counts_deltas(f: &Fixture, until: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    let start = Instant::now();
    loop {
        f.stream.flush();
        let rows: Vec<Value> = f
            .sink
            .lines()
            .into_iter()
            .filter(|l| l["modelClass"] == THREAD_COUNTS)
            .collect();
        if until(&rows) || start.elapsed() > Duration::from_secs(5) {
            return rows;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn counts_by_account() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let label = f.label("Work", "");
    f.insert("m1@x", &inbox, 1, attrs(1, true), None);
    f.store
        .conn()
        .execute_batch(
            "INSERT INTO Folder (id, accountId, version, data, path, role) \
             VALUES ('f-other', 'acc2', 1, '{}', 'INBOX', 'inbox');
             INSERT INTO ThreadCounts VALUES ('f-other', 3, 4);
             INSERT INTO ThreadCounts VALUES ('gone', 1, 1);",
        )
        .unwrap();
    let r = reads(&f);
    let mine = r.counts(&accounts(&[ACCOUNT])).unwrap();
    let mine: Vec<(&str, i64, i64)> = mine
        .iter()
        .map(|(id, c)| (id.as_str(), c.unread, c.total))
        .collect();
    let mut want = vec![(inbox.id(), 1, 1), (label.id(), 0, 0)];
    want.sort();
    assert_eq!(mine, want);
    let both = r.counts(&accounts(&[ACCOUNT, "acc2"])).unwrap();
    assert_eq!(both.len(), 3, "a row without its category is left out");
    assert!(r.counts(&[]).unwrap().is_empty());
}

#[test]
fn counts_delta_after_a_flag_change() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let first = counts_deltas(&f, |rows| !rows.is_empty());
    assert_eq!(
        first[0]["modelJSONs"],
        json!([{"id": inbox.id(), "categoryId": inbox.id(), "unread": 0, "total": 0}]),
        "a new folder announces its row"
    );
    let msg = f.insert("m1@x", &inbox, 1, attrs(1, true), None);
    f.sink.clear();

    let mut m = f.load(msg.id());
    f.store.set_placement_unread(&mut m, false).unwrap();
    f.store.save(&mut m).unwrap();
    let rows = counts_deltas(&f, |rows| !rows.is_empty());
    let last = rows.last().expect("a ThreadCounts delta");
    assert_eq!(last["type"], "persist");
    assert_eq!(
        last["modelJSONs"],
        json!([{"id": inbox.id(), "categoryId": inbox.id(), "unread": 0, "total": 1}])
    );

    // A save that changes no count sends none.
    f.sink.clear();
    let mut m = f.load(msg.id());
    f.store.save(&mut m).unwrap();
    assert!(counts_deltas(&f, |_| true).is_empty());
}

#[test]
fn counts_delta_when_a_folder_goes() {
    let f = fixture();
    let mut work = f.folder("Work", "");
    counts_deltas(&f, |rows| !rows.is_empty());
    f.sink.clear();
    f.store.remove(&mut work).unwrap();
    let rows = counts_deltas(&f, |rows| !rows.is_empty());
    assert_eq!(rows[0]["type"], "unpersist");
    assert_eq!(rows[0]["modelJSONs"][0]["categoryId"], work.id());
}

fn task(id: &str, account: &str, class: &str, status: &str, extra: Value) -> Task {
    let mut data = json!({"id": id, "aid": account, "v": 0, "__cls": class, "status": status});
    for (k, v) in extra.as_object().unwrap() {
        data[k] = v.clone();
    }
    Task { data }
}

#[test]
fn tasks_by_account_status_class_and_draft() {
    let f = fixture();
    for mut t in [
        task("t1", ACCOUNT, "ChangeStarredTask", "complete", json!({})),
        task(
            "t2",
            ACCOUNT,
            "SendDraftTask",
            "remote",
            json!({"headerMessageId": "h1@x"}),
        ),
        task(
            "t3",
            ACCOUNT,
            "SyncbackDraftTask",
            "local",
            json!({"draft": {"hMsgId": "h1@x"}}),
        ),
        task("t4", "acc2", "SendDraftTask", "remote", json!({})),
        task("t5", ACCOUNT, "SendDraftTask", "complete", json!({})),
    ] {
        f.store.save(&mut t).unwrap();
    }
    let r = reads(&f);
    let mine = accounts(&[ACCOUNT]);
    let all = |filter: TaskFilter<'_>| {
        r.tasks(&TaskFilter {
            limit: 50,
            ..filter
        })
        .unwrap()
    };
    assert_eq!(
        ids(&all(TaskFilter {
            accounts: &mine,
            ..Default::default()
        })),
        ["t2", "t3", "t5", "t1"],
        "pending oldest queued first, then finished newest first, other accounts left out"
    );
    let pending = accounts(&["local", "remote"]);
    assert_eq!(
        ids(&all(TaskFilter {
            accounts: &mine,
            statuses: Some(&pending),
            ..Default::default()
        })),
        ["t2", "t3"]
    );
    assert_eq!(
        ids(&all(TaskFilter {
            accounts: &mine,
            class: Some("SendDraftTask"),
            ..Default::default()
        })),
        ["t2", "t5"]
    );
    assert_eq!(
        ids(&all(TaskFilter {
            accounts: &mine,
            header_message_id: Some("h1@x"),
            ..Default::default()
        })),
        ["t2", "t3"],
        "by headerMessageId or the draft's hMsgId"
    );
    let both = accounts(&[ACCOUNT, "acc2"]);
    assert_eq!(
        ids(&r
            .tasks(&TaskFilter {
                accounts: &both,
                limit: 3,
                ..Default::default()
            })
            .unwrap()),
        ["t2", "t3", "t4"],
        "the limit cuts finished tasks before pending ones"
    );
}

#[test]
fn metadata_by_plugin_object_draft_expiration_and_date() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let drafts = f.folder("Drafts", "drafts");
    let save_meta = |id: &str, plugin: &str, value: Value| {
        let mut m = f.load(id);
        m.upsert_metadata(plugin, &value, None).unwrap();
        f.store.save(&mut m).unwrap();
        m
    };
    let a = f.insert("a@x", &inbox, 1, attrs(1, false), None);
    let b = f.insert("b@x", &inbox, 2, attrs(2, false), None);
    let mut dattrs = attrs(3, false);
    dattrs.draft = true;
    let d = f.insert("d@x", &drafts, 3, dattrs, None);
    save_meta(a.id(), "link-tracking", json!({"links": []}));
    save_meta(b.id(), "link-tracking", json!({"links": [{"url": "u"}]}));
    save_meta(d.id(), "send-later", json!({"expiration": 1_900_000_000}));
    save_meta(b.id(), "send-later", json!({"expiration": null}));
    let thread = f.thread(a.thread_id()).unwrap();
    {
        let mut t = thread.clone();
        t.upsert_metadata("thread-snooze", &json!({"expiration": 1_800_000_000}), None)
            .unwrap();
        f.store.save(&mut t).unwrap();
    }

    let r = reads(&f);
    let mine = accounts(&[ACCOUNT]);
    let list = |filter: MetadataFilter<'_>| {
        r.metadata_models(&MetadataFilter {
            accounts: &mine,
            limit: 50,
            ..filter
        })
        .unwrap()
    };
    // Newest first by date: b (uid 2) before a (uid 1).
    let tracked = list(MetadataFilter {
        plugin_id: "link-tracking",
        ..Default::default()
    });
    assert_eq!(ids(&tracked), [b.id(), a.id()]);
    assert_eq!(tracked[0]["hidden"], false);
    assert_eq!(
        ids(&list(MetadataFilter {
            plugin_id: "send-later",
            draft: Some(true),
            ..Default::default()
        })),
        [d.id()]
    );
    assert_eq!(
        ids(&list(MetadataFilter {
            plugin_id: "send-later",
            has_expiration: Some(true),
            ..Default::default()
        })),
        [d.id()]
    );
    assert_eq!(
        ids(&list(MetadataFilter {
            plugin_id: "send-later",
            has_expiration: Some(false),
            ..Default::default()
        })),
        [b.id()]
    );
    assert!(list(MetadataFilter {
        plugin_id: "send-later",
        expires_before: Some(1_900_000_000),
        ..Default::default()
    })
    .is_empty());
    assert_eq!(
        ids(&list(MetadataFilter {
            plugin_id: "send-later",
            expires_before: Some(1_900_000_001),
            ..Default::default()
        })),
        [d.id()]
    );
    let date = |m: &ms_store::Message| m.date();
    assert_eq!(
        ids(&list(MetadataFilter {
            plugin_id: "link-tracking",
            after: Some(date(&a)),
            ..Default::default()
        })),
        [b.id()]
    );
    assert_eq!(
        ids(&list(MetadataFilter {
            plugin_id: "link-tracking",
            before: Some(date(&b)),
            ..Default::default()
        })),
        [a.id()]
    );
    assert_eq!(
        ids(&list(MetadataFilter {
            plugin_id: "link-tracking",
            limit: 1,
            offset: 1,
            ..Default::default()
        })),
        [a.id()]
    );
    let threads = list(MetadataFilter {
        plugin_id: "thread-snooze",
        object: MetadataObject::Thread,
        ..Default::default()
    });
    assert_eq!(ids(&threads), [thread.id()]);
    assert!(
        threads[0].get("hidden").is_none(),
        "threads carry no hidden"
    );
    assert!(list(MetadataFilter {
        plugin_id: "thread-snooze",
        ..Default::default()
    })
    .is_empty());
    let other = accounts(&["acc2"]);
    assert!(r
        .metadata_models(&MetadataFilter {
            accounts: &other,
            plugin_id: "link-tracking",
            limit: 50,
            ..Default::default()
        })
        .unwrap()
        .is_empty());
}

#[test]
fn triaged_inbox_threads_by_category() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let archive = f.folder("Archive", "archive");
    let a = f.insert("a@x", &inbox, 1, attrs(1, false), None);
    let b = f.insert("b@x", &inbox, 2, attrs(2, false), None);
    let c = f.insert("c@x", &inbox, 3, attrs(3, false), None);
    let d = f.insert("d@x", &archive, 4, attrs(4, false), None);
    let triage = |m: &ms_store::Message, value: Value| {
        let mut t = f.thread(m.thread_id()).unwrap();
        t.upsert_metadata("tern-ai-triage", &value, None).unwrap();
        f.store.save(&mut t).unwrap();
        t.id().to_owned()
    };
    let ta = triage(&a, json!({"category": "needs-reply", "needsReply": true}));
    let tb = triage(&b, json!({"category": "important", "needsReply": true}));
    let tc = triage(&c, json!({"category": "newsletter", "needsReply": false}));
    triage(&d, json!({"category": "needs-reply", "needsReply": true}));

    let r = reads(&f);
    let inboxes = accounts(&[inbox.id()]);
    let list = |category, limit, offset| {
        r.triaged_threads(&inboxes, "tern-ai-triage", category, limit, offset)
            .unwrap()
    };
    let (all, total) = list(None, 50, 0);
    assert_eq!((all.len(), total), (3, 3));
    // Newest first: c (uid 3), then b and a.
    assert_eq!(all, [tc.clone(), tb.clone(), ta.clone()]);
    assert_eq!(
        list(Some("needs-reply"), 50, 0),
        (vec![tb.clone(), ta.clone()], 2)
    );
    assert_eq!(list(Some("important"), 50, 0), (vec![tb.clone()], 1));
    assert_eq!(list(Some("needs-reply"), 1, 1), (vec![ta.clone()], 2));
    assert_eq!(list(Some("promotion"), 50, 0), (vec![], 0));
    assert!(r
        .triaged_threads(&[], "tern-ai-triage", None, 50, 0)
        .unwrap()
        .0
        .is_empty());

    let since = r.category_threads_since(&inboxes, 0, 2).unwrap();
    assert_eq!(since, [tc.clone(), tb.clone()]);

    // A reply sent after the last mail came in answers the thread: it leaves needs-reply,
    // unless the user filed it so after replying.
    let sent = f.folder("Sent", "sent");
    f.insert("a-reply@x", &sent, 10, attrs(10, false), Some(&ta));
    f.insert("b-reply@x", &sent, 11, attrs(11, false), Some(&tb));
    let filed = 1_600_000_000 + 20;
    triage(
        &b,
        json!({"category": "important", "needsReply": true, "source": "user", "at": filed}),
    );
    let r = reads(&f);
    let list = |category, limit, offset| {
        r.triaged_threads(&inboxes, "tern-ai-triage", category, limit, offset)
            .unwrap()
    };
    assert_eq!(list(Some("needs-reply"), 50, 0), (vec![tb.clone()], 1));
    assert_eq!(list(None, 50, 0).1, 3);
    triage(
        &b,
        json!({"category": "important", "needsReply": true, "source": "user", "at": 5}),
    );
    let r = reads(&f);
    assert_eq!(
        r.triaged_threads(&inboxes, "tern-ai-triage", Some("needs-reply"), 50, 0)
            .unwrap(),
        (vec![], 0)
    );
    assert_eq!(
        r.triaged_threads(&inboxes, "tern-ai-triage", Some("important"), 50, 0)
            .unwrap(),
        (vec![tb], 1)
    );
    assert!(r
        .category_threads_since(&inboxes, i64::MAX, 50)
        .unwrap()
        .is_empty());
}

#[test]
fn hidden_follows_message_is_hidden() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let plain = f.insert("p@x", &inbox, 1, attrs(1, false), None);
    let reminder = f.insert_with("r@x", &inbox, 2, attrs(2, false), None, |h| {
        h.from = vec![ms_core::Address::new(Some("Me via Tern"), "me@example.com")];
        h.to = vec![ms_core::Address::new(None, "me@example.com")];
    });
    let r = reads(&f);
    assert_eq!(
        r.message(ACCOUNT, plain.id()).unwrap().unwrap()["hidden"],
        false
    );
    assert_eq!(
        r.message(ACCOUNT, reminder.id()).unwrap().unwrap()["hidden"],
        true
    );
    let thread = r.thread_messages(ACCOUNT, reminder.thread_id()).unwrap();
    assert_eq!(thread[0]["hidden"], true);

    use ms_store::models::is_hidden_json;
    assert!(is_hidden_json(&json!({"id": "deleted-abc"})));
    let me = json!([{"email": "me@x.test", "name": "Me via Mailspring"}]);
    assert!(is_hidden_json(
        &json!({"id": "m", "from": me, "to": [{"email": "me@x.test"}]})
    ));
    assert!(!is_hidden_json(
        &json!({"id": "m", "from": me, "to": [{"email": "you@x.test"}]})
    ));
    assert!(!is_hidden_json(
        &json!({"id": "m", "from": [{"email": "me@x.test", "name": "Me"}], "to": [{"email": "me@x.test"}]})
    ));
}
