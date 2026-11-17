// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `MailProcessor` against a migrated database, every case checked with
//! `airpost_store::invariants::check`. The races use a second connection that holds the write
//! lock while the first reads, as the other worker does (see "Message Identity and
//! Placements" in Mailspring-Sync's CLAUDE.md; MailProcessor.cpp is the reference).

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use airpost_core::{Account, Address, ManualClock, MessageFlags, MessageHeader};
use airpost_imap::ImapMessage;
use airpost_store::{DeltaStream, Folder, Message, Model, Query, Store};
use airpost_sync::mail_processor::{id_for_message, MailProcessor};
use serde_json::json;

const ACCOUNT: &str = "acc1";
const T0: i64 = 1_700_000_000;
/// Long enough that the reader has reached its write transaction before the holder commits.
const HOLD: Duration = Duration::from_millis(500);

struct Null;

impl Write for Null {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn account() -> Account {
    Account::from_json(json!({
        "id": ACCOUNT,
        "provider": "imap",
        "emailAddress": "me@example.com",
        "settings": {},
    }))
    .unwrap()
}

fn open(dir: &Path, now: i64) -> Store {
    let store = Store::open(dir, DeltaStream::new(Box::new(Null))).unwrap();
    store.set_clock(Arc::new(ManualClock::new(now)));
    store
}

fn migrated() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    airpost_store::migrate::migrate(dir.path(), &ManualClock::new(T0), &mut out).unwrap();
    dir
}

fn folder(store: &Store, path: &str, role: &str) -> Folder {
    let mut f = Folder::new(&airpost_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
    f.set_path(path);
    f.set_role(role);
    store.save(&mut f).unwrap();
    f
}

fn remote(header_id: &str, uid: u32, flags: MessageFlags) -> ImapMessage {
    ImapMessage {
        uid,
        flags,
        internal_date: Some(1_600_000_000),
        header: Some(MessageHeader {
            date_raw: Some("Sun, 13 Sep 2020 12:26:40 +0000".into()),
            date: Some(1_600_000_000),
            subject_raw: Some(format!("Subject {header_id}").into_bytes()),
            subject: Some(format!("Subject {header_id}")),
            message_id: Some(header_id.to_owned()),
            from: vec![Address::new(Some("Ann"), "ann@example.com")],
            to: vec![Address::new(None, "me@example.com")],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn load(store: &Store, id: &str) -> Option<Message> {
    store
        .find::<Message>(&Query::new().equal("id", id))
        .unwrap()
}

fn assert_invariants(store: &Store) {
    let violations = airpost_store::invariants::check(store.conn(), &[]).unwrap();
    assert!(violations.is_empty(), "{violations:#?}");
}

fn folder_ids(m: &Message) -> Vec<String> {
    let mut ids: Vec<String> = m
        .folders()
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    ids.sort();
    ids
}

/// Both workers ingest one message: the second read before the first committed, so its
/// insert hits the primary key, rolls back and records the copy as an update
/// (`insertFallbackToUpdateMessage`, MailProcessor.cpp:131-156).
#[test]
fn concurrent_insert_falls_back_to_update() {
    let dir = migrated();
    let acct = account();
    let store = open(dir.path(), T0);
    let inbox = folder(&store, "INBOX", "inbox");
    let msg = remote("race@x", 4, MessageFlags::empty());
    let id = id_for_message(ACCOUNT, inbox.path(), &msg);

    let (ready, wait) = mpsc::channel();
    let held = Arc::new(AtomicBool::new(false));
    let released = held.clone();
    let holder = {
        let (path, acct, inbox, msg) = (
            dir.path().to_owned(),
            acct.clone(),
            inbox.clone(),
            msg.clone(),
        );
        std::thread::spawn(move || {
            let store = open(&path, T0);
            let processor = MailProcessor::new(&acct, &store);
            store
                .transaction("otherWorker", |_| {
                    processor.insert_fallback_to_update_message(&msg, &inbox, T0)?;
                    ready.send(()).unwrap();
                    std::thread::sleep(HOLD);
                    released.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap();
        })
    };
    wait.recv().unwrap();
    assert!(
        load(&store, &id).is_none(),
        "the other worker has not committed"
    );

    let processor = MailProcessor::new(&acct, &store);
    let saved = processor
        .insert_fallback_to_update_message(&msg, &inbox, T0)
        .unwrap();
    assert!(
        held.load(Ordering::SeqCst),
        "the insert waited for the other worker's lock"
    );
    holder.join().unwrap();

    assert_eq!(saved.id(), id);
    assert_eq!(store.placements_for_message(&id).unwrap().len(), 1);
    assert_eq!(store.count::<Message>(&Query::new()).unwrap(), 1);
    assert_invariants(&store);
}

/// While a task holds the message (`syncedAt` ahead of the scan), a copy already recorded
/// keeps the task's flags, but a copy in a new folder is still recorded and the lock is
/// left for the task (MailProcessor.cpp:254-321).
#[test]
fn synced_at_guard_protects_recorded_copies_only() {
    let dir = migrated();
    let acct = account();
    let store = open(dir.path(), T0);
    let inbox = folder(&store, "INBOX", "inbox");
    let archive = folder(&store, "Archive", "archive");
    let processor = MailProcessor::new(&acct, &store);

    let m = processor
        .insert_fallback_to_update_message(&remote("lock@x", 1, MessageFlags::empty()), &inbox, T0)
        .unwrap();
    let mut locked = load(&store, m.id()).unwrap();
    locked.set_synced_at(T0 + 100);
    store.save(&mut locked).unwrap();

    processor
        .insert_fallback_to_update_message(
            &remote("lock@x", 1, MessageFlags::SEEN),
            &inbox,
            T0 + 10,
        )
        .unwrap();
    let after = load(&store, m.id()).unwrap();
    assert!(
        after.is_unread(),
        "the recorded copy keeps the task's flags"
    );
    assert!(store.placements_for_message(m.id()).unwrap()[0].unread);

    processor
        .insert_fallback_to_update_message(
            &remote("lock@x", 5, MessageFlags::SEEN),
            &archive,
            T0 + 10,
        )
        .unwrap();
    let after = load(&store, m.id()).unwrap();
    let mut expected = vec![inbox.id().to_owned(), archive.id().to_owned()];
    expected.sort();
    assert_eq!(
        folder_ids(&after),
        expected,
        "the new copy is recorded under the lock"
    );
    assert_eq!(after.synced_at(), T0 + 100, "the lock is left for the task");
    assert_invariants(&store);
}

/// A UID reused by another message takes the row; the message that held it becomes an
/// orphan and the sweep removes it (MailProcessor.cpp:326-332).
#[test]
fn displaced_uid_orphans_the_previous_holder() {
    let dir = migrated();
    let acct = account();
    let store = open(dir.path(), T0);
    let inbox = folder(&store, "INBOX", "inbox");
    let processor = MailProcessor::new(&acct, &store);

    let first = processor
        .insert_fallback_to_update_message(&remote("first@x", 7, MessageFlags::empty()), &inbox, T0)
        .unwrap();
    let second = processor
        .insert_fallback_to_update_message(
            &remote("second@x", 7, MessageFlags::empty()),
            &inbox,
            T0,
        )
        .unwrap();
    assert_ne!(first.id(), second.id());

    let orphan = load(&store, first.id()).unwrap();
    assert!(folder_ids(&orphan).is_empty());
    assert_eq!(
        store.orphan_message_ids_before(ACCOUNT, T0 + 1).unwrap(),
        vec![first.id().to_owned()]
    );
    assert_invariants(&store);

    processor.sweep_expired_orphans(T0 + 1, T0 + 1).unwrap();
    assert!(load(&store, first.id()).is_none());
    assert_eq!(
        folder_ids(&load(&store, second.id()).unwrap()),
        vec![inbox.id().to_owned()]
    );
    assert_invariants(&store);
}

/// The foreground worker revives a sweep candidate and loses it again after the sweep read
/// its candidates, restarting its grace period: the sweep re-reads the record under the
/// lock and keeps it (MailProcessor.cpp:704-723).
#[test]
fn sweep_rereads_orphans_under_the_lock() {
    let dir = migrated();
    let acct = account();
    let store = open(dir.path(), T0);
    let inbox = folder(&store, "INBOX", "inbox");
    let processor = MailProcessor::new(&acct, &store);

    let mut ids = Vec::new();
    for (i, h) in ["expired@x", "revived@x"].iter().enumerate() {
        let m = processor
            .insert_fallback_to_update_message(
                &remote(h, i as u32 + 1, MessageFlags::empty()),
                &inbox,
                T0,
            )
            .unwrap();
        ids.push(m.id().to_owned());
    }
    processor
        .delete_vanished_placements(&inbox, &[1, 2])
        .unwrap();
    let sweep_before = T0 + 1;
    assert_eq!(
        store
            .orphan_message_ids_before(ACCOUNT, sweep_before)
            .unwrap()
            .len(),
        2
    );

    let (ready, wait) = mpsc::channel();
    let held = Arc::new(AtomicBool::new(false));
    let released = held.clone();
    let holder = {
        let (path, acct, inbox, revived) = (
            dir.path().to_owned(),
            acct.clone(),
            inbox.clone(),
            ids[1].clone(),
        );
        std::thread::spawn(move || {
            let store = open(&path, sweep_before + 60);
            let processor = MailProcessor::new(&acct, &store);
            store
                .transaction("foreground", |s| {
                    processor.insert_fallback_to_update_message(
                        &remote("revived@x", 9, MessageFlags::empty()),
                        &inbox,
                        T0,
                    )?;
                    let affected = s.delete_vanished_placements(&inbox, &[9])?;
                    assert_eq!(affected, vec![revived.clone()]);
                    processor.refresh_messages_in_open_transaction(
                        s,
                        &affected,
                        airpost_sync::mail_processor::UnplacedMessages::Keep,
                        true,
                    )?;
                    ready.send(()).unwrap();
                    std::thread::sleep(HOLD);
                    released.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap();
        })
    };
    wait.recv().unwrap();

    processor
        .sweep_expired_orphans(sweep_before, sweep_before)
        .unwrap();
    assert!(held.load(Ordering::SeqCst), "the sweep waited for the lock");
    holder.join().unwrap();

    assert!(
        load(&store, &ids[0]).is_none(),
        "the expired orphan is swept"
    );
    assert!(
        load(&store, &ids[1]).is_some(),
        "the re-orphaned message gets a new grace period"
    );
    assert!(store
        .orphan_message_ids_before(ACCOUNT, sweep_before)
        .unwrap()
        .is_empty());
    assert_invariants(&store);
}

/// On Gmail a message is in exactly one of All Mail, Spam and Trash: recording it in one
/// drops its copy in the others (MailProcessor.cpp:299-304).
#[test]
fn gmail_copy_in_trash_replaces_all_mail() {
    let dir = migrated();
    let acct = account();
    let store = open(dir.path(), T0);
    let all = folder(&store, "[Gmail]/All Mail", "all");
    let trash = folder(&store, "[Gmail]/Trash", "trash");
    let mut processor = MailProcessor::new(&acct, &store);
    processor.is_gmail = true;

    let m = processor
        .insert_fallback_to_update_message(&remote("gm@x", 3, MessageFlags::empty()), &all, T0)
        .unwrap();
    processor
        .insert_fallback_to_update_message(&remote("gm@x", 8, MessageFlags::empty()), &trash, T0)
        .unwrap();
    let placements = store.placements_for_message(m.id()).unwrap();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].folder_id, trash.id());
    assert_eq!(
        folder_ids(&load(&store, m.id()).unwrap()),
        vec![trash.id().to_owned()]
    );
    assert_invariants(&store);

    processor.is_gmail = false;
    processor
        .insert_fallback_to_update_message(&remote("gm@x", 3, MessageFlags::empty()), &all, T0)
        .unwrap();
    assert_eq!(
        store.placements_for_message(m.id()).unwrap().len(),
        2,
        "folders keep both copies"
    );
    assert_invariants(&store);
}

/// Off Gmail, a copy's label keywords are its labels: lowercased,
/// flag keywords left out, and a keyword change alone updates the copy.
#[test]
fn keyword_labels_on_copies() {
    let dir = migrated();
    let acct = account();
    let store = open(dir.path(), T0);
    let inbox = folder(&store, "INBOX", "inbox");
    let mut processor = MailProcessor::new(&acct, &store);
    processor.keyword_labels = true;
    let labels_of = |id: &str| {
        let rows = store.placements_for_message(id).unwrap();
        assert_eq!(rows.len(), 1);
        (
            rows[0].labels.clone(),
            load(&store, id).unwrap().data["labels"].clone(),
        )
    };

    let mut msg = remote("kw@x", 5, MessageFlags::SEEN);
    msg.keywords = ["Work", "$Junk", "$label2", "NonJunk"]
        .map(String::from)
        .into();
    let m = processor
        .insert_fallback_to_update_message(&msg, &inbox, T0)
        .unwrap();
    assert_eq!(
        labels_of(m.id()),
        (
            vec!["$label2".into(), "work".into()],
            json!(["$label2", "work"])
        )
    );
    assert_invariants(&store);

    msg.keywords = vec!["$Label2".into()];
    processor
        .insert_fallback_to_update_message(&msg, &inbox, T0)
        .unwrap();
    assert_eq!(
        labels_of(m.id()),
        (vec!["$label2".into()], json!(["$label2"]))
    );
    assert_invariants(&store);

    processor.keyword_labels = false;
    processor
        .insert_fallback_to_update_message(&msg, &inbox, T0)
        .unwrap();
    assert_eq!(labels_of(m.id()), (vec![], json!([])));
    assert_invariants(&store);
}
