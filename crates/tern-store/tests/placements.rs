// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Placement helpers, thread refcounting and the derived tables, each scenario checked
//! against the invariants ported from `apps/engine/test/harness/invariants.py`. The flows
//! mirror what MailProcessor and TaskProcessor do with the helpers (see "Message Identity
//! and Placements" in legacy/mailsync-cpp/CLAUDE.md).

mod common;

use common::{attrs, fixture, ACCOUNT};
use serde_json::json;
use tern_core::UidSet;
use tern_store::{Message, MessageAttributes, Model, Query, Thread};

fn folder_bits(m: &Message) -> serde_json::Value {
    m.folders().clone()
}

#[test]
fn insert_builds_snapshot_thread_and_counts() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("m1@x", &inbox, 10, attrs(10, true), None);
    f.assert_invariants();

    let stored = f.load(msg.id());
    assert_eq!(folder_bits(&stored), json!({ inbox.id(): 1 }));
    assert!(stored.is_unread());
    let thread = f.thread(stored.thread_id()).unwrap();
    assert_eq!(thread.unread(), 1);
    assert!(thread.in_all_mail());
    assert_eq!(thread.folders()[0]["_refs"], 1);
    assert_eq!(thread.folders()[0]["_u"], 1);
    assert!(thread.folders()[0].get("localStatus").is_none());
    assert_eq!(thread.folders()[0]["__cls"], "Folder");
    assert_eq!(f.thread_counts(inbox.id()), (1, 1));
    assert!(thread.search_row_id() > 0);
    let categories: String = f
        .store
        .conn()
        .query_row(
            "SELECT categories FROM ThreadSearch WHERE rowid = ?",
            [thread.search_row_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(categories, "inbox ");
}

#[test]
fn second_copy_and_flag_fan_out() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let sent = f.folder("Sent", "sent");
    let msg = f.insert("self@x", &inbox, 1, attrs(1, true), None);

    // The Sent copy arrives read: the Inbox copy stays unread.
    let mut m = f.load(msg.id());
    f.store
        .upsert_placement(&mut m, &sent, 5, &attrs(5, false))
        .unwrap();
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    let m = f.load(msg.id());
    assert_eq!(folder_bits(&m), json!({ inbox.id(): 1, sent.id(): 0 }));
    assert!(m.is_unread());
    assert_eq!(f.thread_counts(inbox.id()), (1, 1));
    assert_eq!(f.thread_counts(sent.id()), (0, 1));

    // Mark read fans out to both copies.
    let mut m = f.load(msg.id());
    f.store.set_placement_unread(&mut m, false).unwrap();
    f.store.set_placement_starred(&mut m, true).unwrap();
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    let m = f.load(msg.id());
    assert_eq!(folder_bits(&m), json!({ inbox.id(): 2, sent.id(): 2 }));
    assert!(!m.is_unread() && m.is_starred());
    assert_eq!(f.thread_counts(inbox.id()), (0, 1));
    let t = f.thread(m.thread_id()).unwrap();
    assert_eq!((t.unread(), t.starred()), (0, 1));
    assert!(
        t.last_message_sent_timestamp() > 0,
        "a copy in Sent makes it the user's"
    );
}

#[test]
fn two_copies_in_one_folder_are_one_reference() {
    let f = fixture();
    let sent = f.folder("Sent", "sent");
    let msg = f.insert("dup@x", &sent, 1, attrs(1, false), None);
    let mut m = f.load(msg.id());
    f.store
        .upsert_placement(&mut m, &sent, 2, &attrs(2, true))
        .unwrap();
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    let m = f.load(msg.id());
    assert_eq!(folder_bits(&m), json!({ sent.id(): 1 }));
    assert_eq!(f.store.placements_for_message(m.id()).unwrap().len(), 2);
    let t = f.thread(m.thread_id()).unwrap();
    assert_eq!(t.folders()[0]["_refs"], 1);
}

#[test]
fn optimistic_move_then_commit() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let archive = f.folder("Archive", "archive");
    let msg = f.insert("mv@x", &inbox, 3, attrs(3, true), None);

    let mut m = f.load(msg.id());
    f.store
        .begin_placement_move(&mut m, inbox.id(), 3, archive.id())
        .unwrap();
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    let m = f.load(msg.id());
    assert_eq!(
        folder_bits(&m),
        json!({ archive.id(): 1 }),
        "reported under the destination"
    );
    assert_eq!(f.thread_counts(inbox.id()), (0, 0));
    assert_eq!(f.thread_counts(archive.id()), (1, 1));

    // A scan of the source during the move keeps the marker.
    let mut m = f.load(msg.id());
    f.store
        .upsert_placement(&mut m, &inbox, 3, &attrs(3, true))
        .unwrap();
    f.store.save(&mut m).unwrap();
    let p = f.store.placements_for_message(m.id()).unwrap();
    assert_eq!(p[0].pending_folder_id.as_deref(), Some(archive.id()));

    let mut m = f.load(msg.id());
    let displaced = f
        .store
        .commit_placement_move(&mut m, inbox.id(), 3, archive.id(), 40)
        .unwrap();
    assert_eq!(displaced, None);
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    let p = f.store.placements_for_message(m.id()).unwrap();
    assert_eq!(
        (p.len(), p[0].folder_id.as_str(), p[0].remote_uid),
        (1, archive.id(), 40)
    );
    assert_eq!(p[0].pending_folder_id, None);
}

#[test]
fn commit_when_destination_already_scanned() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let archive = f.folder("Archive", "archive");
    let msg = f.insert("mv@x", &inbox, 3, attrs(3, false), None);
    let mut m = f.load(msg.id());
    f.store
        .begin_placement_move(&mut m, inbox.id(), 3, archive.id())
        .unwrap();
    f.store.save(&mut m).unwrap();
    // The destination scan records the moved copy before the commit.
    let mut m = f.load(msg.id());
    f.store
        .upsert_placement(&mut m, &archive, 40, &attrs(40, false))
        .unwrap();
    f.store.save(&mut m).unwrap();
    let mut m = f.load(msg.id());
    f.store
        .commit_placement_move(&mut m, inbox.id(), 3, archive.id(), 40)
        .unwrap();
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    assert_eq!(f.store.placements_for_message(m.id()).unwrap().len(), 1);

    // Settling a copy already in place only clears the marker.
    let mut m = f.load(msg.id());
    f.store
        .commit_placement_move(&mut m, archive.id(), 40, archive.id(), 40)
        .unwrap();
    f.store.save(&mut m).unwrap();
    assert_eq!(f.store.placements_for_message(m.id()).unwrap().len(), 1);
    f.assert_invariants();
}

#[test]
fn commit_displaces_a_stale_holder() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let archive = f.folder("Archive", "archive");
    let a = f.insert("a@x", &inbox, 3, attrs(3, false), None);
    let b = f.insert("b@x", &archive, 40, attrs(40, false), None);
    let mut m = f.load(a.id());
    f.store
        .begin_placement_move(&mut m, inbox.id(), 3, archive.id())
        .unwrap();
    f.store.save(&mut m).unwrap();
    let mut m = f.load(a.id());
    let displaced = f
        .store
        .commit_placement_move(&mut m, inbox.id(), 3, archive.id(), 40)
        .unwrap();
    assert_eq!(displaced.as_deref(), Some(b.id()));
    f.store.save(&mut m).unwrap();
    f.refresh(&[b.id().to_owned()]);
    f.assert_invariants();
    assert_eq!(
        f.store
            .orphan_message_ids_before(ACCOUNT, i64::MAX)
            .unwrap(),
        [b.id()]
    );
}

#[test]
fn abandon_keeps_a_later_tasks_marker() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let archive = f.folder("Archive", "archive");
    let trash = f.folder("Trash", "trash");
    let msg = f.insert("ab@x", &inbox, 3, attrs(3, false), None);
    let mut m = f.load(msg.id());
    f.store
        .begin_placement_move(&mut m, inbox.id(), 3, trash.id())
        .unwrap();
    f.store.save(&mut m).unwrap();
    let mut m = f.load(msg.id());
    f.store
        .abandon_placement_move(&mut m, inbox.id(), 3, archive.id())
        .unwrap();
    assert!(!m.placements_changed());
    f.store
        .abandon_placement_move(&mut m, inbox.id(), 3, trash.id())
        .unwrap();
    assert!(m.placements_changed());
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    let m = f.load(msg.id());
    assert_eq!(folder_bits(&m), json!({ inbox.id(): 0 }));
    let t = f.thread(m.thread_id()).unwrap();
    assert!(t.in_all_mail());
}

#[test]
fn vanished_copy_leaves_an_orphan_that_a_later_upsert_clears() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let archive = f.folder("Archive", "archive");
    let msg = f.insert("v@x", &inbox, 9, attrs(9, true), None);

    // Plain-IMAP move seen source first: persist {folders: {}}, never unpersist.
    let ids = f
        .store
        .delete_vanished_placements(&inbox, &[9, 10])
        .unwrap();
    assert_eq!(ids, [msg.id()]);
    f.refresh(&ids);
    f.assert_invariants();
    let m = f.load(msg.id());
    assert_eq!(folder_bits(&m), json!({}));
    assert!(m.is_unread(), "an orphan keeps its flags");
    let t = f.thread(m.thread_id()).unwrap();
    assert!(!t.in_all_mail());
    assert_eq!(t.unread(), 1);
    assert_eq!(f.thread_counts(inbox.id()), (0, 0));
    assert_eq!(
        f.store
            .orphan_message_ids_before(ACCOUNT, i64::MAX)
            .unwrap(),
        [msg.id()]
    );
    let since: i64 = f
        .store
        .conn()
        .query_row("SELECT since FROM MessageOrphan", [], |r| r.get(0))
        .unwrap();
    assert_eq!(since, 1_700_000_000, "from the store's clock");
    assert!(f
        .store
        .orphan_message_ids_before(ACCOUNT, since)
        .unwrap()
        .is_empty());

    let mut m = f.load(msg.id());
    f.store
        .upsert_placement(&mut m, &archive, 3, &attrs(3, true))
        .unwrap();
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    assert!(f
        .store
        .orphan_message_ids_before(ACCOUNT, i64::MAX)
        .unwrap()
        .is_empty());
    assert_eq!(f.thread_counts(archive.id()), (1, 1));
}

#[test]
fn open_ended_vanished_range() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let a = f.insert("a@x", &inbox, 5, attrs(5, false), None);
    let b = f.insert("b@x", &inbox, 50, attrs(50, false), None);
    let ids = f
        .store
        .delete_vanished_placements_in(&inbox, &UidSet::range(10, None))
        .unwrap();
    assert_eq!(ids, [b.id()]);
    f.refresh(&ids);
    f.assert_invariants();
    assert_eq!(f.store.placements_for_message(a.id()).unwrap().len(), 1);
}

#[test]
fn orphan_sweep_removes_message_and_empty_thread() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("o@x", &inbox, 9, attrs(9, true), None);
    let reply = f.insert_with(
        "r@x",
        &inbox,
        10,
        attrs(10, false),
        Some(msg.thread_id()),
        |h| h.references = vec!["o@x".into()],
    );
    let ids = f
        .store
        .delete_vanished_placements(&inbox, &[9, 10])
        .unwrap();
    f.refresh(&ids);
    f.assert_invariants();

    let orphans = f
        .store
        .orphan_message_ids_before(ACCOUNT, i64::MAX)
        .unwrap();
    assert_eq!(orphans.len(), 2);
    let among = f
        .store
        .orphan_message_ids_before_among(ACCOUNT, i64::MAX, &[reply.id().to_owned()])
        .unwrap();
    assert_eq!(among, [reply.id()]);
    let thread_id = msg.thread_id().to_owned();
    f.store
        .transaction("sweepExpiredOrphans", |s| {
            for id in &orphans {
                let mut m = s
                    .find::<Message>(&Query::new().equal("id", id.as_str()))?
                    .unwrap();
                s.remove(&mut m)?;
            }
            Ok(())
        })
        .unwrap();
    f.assert_invariants();
    assert!(
        f.thread(&thread_id).is_none(),
        "the thread had no folders left"
    );
    let refs: i64 = f
        .store
        .conn()
        .query_row("SELECT COUNT(*) FROM ThreadSearch", [], |r| r.get(0))
        .unwrap();
    assert_eq!(refs, 0);
}

#[test]
fn thread_stays_while_another_message_holds_it() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let a = f.insert("a@x", &inbox, 1, attrs(1, true), None);
    let b = f.insert("b@x", &inbox, 2, attrs(2, false), Some(a.thread_id()));
    let mut ma = f.load(a.id());
    f.store.remove(&mut ma).unwrap();
    f.assert_invariants();
    let t = f.thread(b.thread_id()).unwrap();
    assert_eq!(t.unread(), 0);
    assert_eq!(t.folders()[0]["_refs"], 1);
    assert_eq!(f.thread_counts(inbox.id()), (0, 1));
}

#[test]
fn uid_reused_by_another_message() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let a = f.insert("a@x", &inbox, 7, attrs(7, false), None);
    let mut b = Message::from_incoming(
        &tern_store::IncomingMessage {
            id: "bbb",
            header: &Default::default(),
            received_date: 1,
            header_message_id: "b@x",
            gmail_message_id: 0,
            attrs: &attrs(7, false),
            sync_data_timestamp: 1,
        },
        &inbox,
    );
    let displaced = f
        .store
        .transaction("insertMessage", |s| {
            let mut thread = Thread::create(b.id(), ACCOUNT, b.subject(), 0);
            b.set_thread_id(thread.id());
            let d = s.upsert_placement(&mut b, &inbox, 7, &attrs(7, false))?;
            s.refresh_message_from_placements(&mut b)?;
            thread.apply_message_attribute_changes(
                &tern_store::MessageSnapshot::empty(),
                Some(&b),
                s,
            )?;
            b.capture_snapshot();
            b.skip_thread_updates_after_save = true;
            s.save(&mut thread)?;
            s.save(&mut b)?;
            Ok(d)
        })
        .unwrap();
    assert_eq!(displaced.as_deref(), Some(a.id()));
    f.refresh(&[a.id().to_owned()]);
    f.assert_invariants();
    assert_eq!(folder_bits(&f.load(a.id())), json!({}));
}

#[test]
fn uidvalidity_reset_relinks_and_drops_the_rest() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let drafts = f.folder("Drafts", "drafts");
    let a = f.insert("a@x", &inbox, 1, attrs(1, false), None);
    let b = f.insert("b@x", &inbox, 2, attrs(2, true), None);
    let d = f.insert("d@x", &drafts, 3, attrs(3, false), None);
    assert!(
        f.load(d.id()).is_draft(),
        "a copy in Drafts makes it a draft"
    );

    f.store.reset_placement_uids(&inbox).unwrap();
    f.store.reset_placement_uids(&drafts).unwrap();
    f.assert_invariants();
    assert_eq!(
        f.store.placements_for_message(a.id()).unwrap()[0].remote_uid,
        0
    );

    // The rebuild relinks a at a new UID and replaces its UID 0 row.
    let mut m = f.load(a.id());
    f.store
        .upsert_placement(&mut m, &inbox, 100, &attrs(100, false))
        .unwrap();
    f.store.save(&mut m).unwrap();
    let rows = f.store.placements_for_message(a.id()).unwrap();
    assert_eq!((rows.len(), rows[0].remote_uid), (1, 100));

    let gone = f.store.delete_unassigned_placements(&inbox).unwrap();
    assert_eq!(gone, [b.id()]);
    f.refresh(&gone);
    // Draft rows survive the cleanup in their folder.
    let mut draft_row = f.store.placements_for_message(d.id()).unwrap();
    assert_eq!(draft_row.len(), 1);
    f.store
        .conn()
        .execute(
            "UPDATE MessageFolder SET draft = 1 WHERE messageId = ?",
            [d.id()],
        )
        .unwrap();
    assert!(f
        .store
        .delete_unassigned_placements(&drafts)
        .unwrap()
        .is_empty());
    draft_row = f.store.placements_for_message(d.id()).unwrap();
    assert_eq!(draft_row.len(), 1);
    f.refresh(&[d.id().to_owned()]);
    f.assert_invariants();
}

#[test]
fn local_draft_at_uid_zero_is_refreshed_in_place() {
    let f = fixture();
    let drafts = f.folder("Drafts", "drafts");
    let mut d = Message::new(
        json!({"id": "draft1", "aid": ACCOUNT, "v": 0, "subject": "hi", "date": 5, "hMsgId": "d@x", "threadId": ""}),
    );
    let draft_attrs = MessageAttributes {
        draft: true,
        ..Default::default()
    };
    f.store
        .upsert_placement(&mut d, &drafts, 0, &draft_attrs)
        .unwrap();
    f.store.save(&mut d).unwrap();
    f.store
        .upsert_placement(&mut d, &drafts, 0, &draft_attrs)
        .unwrap();
    f.store.save(&mut d).unwrap();
    assert_eq!(f.store.placements_for_message("draft1").unwrap().len(), 1);
    assert!(f.load("draft1").is_draft());
    f.assert_invariants();
    assert!(f
        .store
        .fetchable_copies_of_message(&d, None)
        .unwrap()
        .is_empty());
}

#[test]
fn deleting_a_folder_detaches_its_messages() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let work = f.folder("Work", "");
    let a = f.insert("a@x", &work, 1, attrs(1, false), None);
    let b = f.insert("b@x", &inbox, 2, attrs(2, false), None);
    let mut mb = f.load(b.id());
    f.store
        .begin_placement_move(&mut mb, inbox.id(), 2, work.id())
        .unwrap();
    f.store.save(&mut mb).unwrap();

    let ids = f
        .store
        .message_ids_with_placements_in_folder(work.id())
        .unwrap();
    assert_eq!(ids.len(), 2);
    let affected = f.store.delete_placements_for_folder(work.id()).unwrap();
    assert_eq!(affected.len(), 2);
    let mut w = work.clone();
    f.store.remove(&mut w).unwrap();
    f.refresh(&affected);
    f.assert_invariants();
    assert_eq!(
        folder_bits(&f.load(b.id())),
        json!({ inbox.id(): 0 }),
        "the move towards it is abandoned"
    );
    assert_eq!(folder_bits(&f.load(a.id())), json!({}));
    let threads: i64 = f
        .store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM ThreadCounts WHERE categoryId = ?",
            [work.id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(threads, 0, "the folder's ThreadCounts row goes with it");
}

#[test]
fn delete_placements_for_folder_among_chunks() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let work = f.folder("Work", "");
    let a = f.insert("a@x", &work, 1, attrs(1, false), None);
    let mut ma = f.load(a.id());
    f.store
        .upsert_placement(&mut ma, &inbox, 1, &attrs(1, false))
        .unwrap();
    f.store.save(&mut ma).unwrap();
    f.store
        .delete_placements_for_folder_among(work.id(), &[a.id().to_owned()])
        .unwrap();
    f.refresh(&[a.id().to_owned()]);
    f.assert_invariants();
    assert_eq!(folder_bits(&f.load(a.id())), json!({ inbox.id(): 0 }));
}

#[test]
fn gmail_labels_and_single_placement() {
    let f = fixture();
    let all = f.folder("[Gmail]/All Mail", "all");
    let trash = f.folder("[Gmail]/Trash", "trash");
    let inbox_label = f.label("INBOX", "inbox");
    let work = f.label("Work", "");
    let gattrs = |uid: u32, unread: bool| MessageAttributes {
        uid,
        unread,
        labels: vec!["Work".into(), "\\Inbox".into()],
        ..Default::default()
    };
    let msg = f.insert("g@x", &all, 1, gattrs(1, true), None);
    f.assert_invariants();
    let t = f.thread(msg.thread_id()).unwrap();
    let label_ids: Vec<&str> = t
        .labels()
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["id"].as_str().unwrap())
        .collect();
    assert!(label_ids.contains(&inbox_label.id()) && label_ids.contains(&work.id()));
    assert_eq!(f.thread_counts(work.id()), (1, 1));
    assert_eq!(t.labels()[0]["__cls"], "Label");

    // Trashed: the trash copy replaces the All Mail one; labels stop counting unread.
    let mut m = f.load(msg.id());
    f.store
        .upsert_placement(&mut m, &trash, 9, &attrs(9, true))
        .unwrap();
    f.store
        .remove_placements_outside_folder(&mut m, trash.id())
        .unwrap();
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    let m = f.load(msg.id());
    assert_eq!(folder_bits(&m), json!({ trash.id(): 1 }));
    assert_eq!(m.labels(), &json!([]));
    let t = f.thread(m.thread_id()).unwrap();
    assert!(!t.in_all_mail());
    assert_eq!(f.thread_counts(work.id()), (0, 0));

    // Labels change on the one placement.
    let mut m = f.load(msg.id());
    f.store
        .set_placement_labels(&mut m, &["Work".into()])
        .unwrap();
    f.store.save(&mut m).unwrap();
    f.assert_invariants();
    assert_eq!(f.load(msg.id()).labels(), &json!(["Work"]));
}

fn thread_label_paths(f: &common::Fixture, thread_id: &str) -> Vec<String> {
    let mut paths: Vec<String> = f
        .thread(thread_id)
        .unwrap()
        .labels()
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["path"].as_str().unwrap().to_owned())
        .collect();
    paths.sort();
    paths
}

#[test]
fn gmail_system_labels_match_by_role_then_gmail_path() {
    use tern_store::models::label_for_xgm_label_name;
    let label = |path: &str, role: &str| {
        let mut l = tern_store::Label::new(&tern_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
        l.set_path(path);
        l.set_role(role);
        l
    };
    let found = |name: &str, labels: &[tern_store::Label]| {
        label_for_xgm_label_name(name, labels).map(|l| l.path().to_owned())
    };
    // A user label named Important, listed before the system one.
    let both = [
        label("Important", ""),
        label("[Gmail]/Important", "important"),
    ];
    assert_eq!(
        found("\\Important", &both).as_deref(),
        Some("[Gmail]/Important")
    );
    assert_eq!(found("Important", &both).as_deref(), Some("Important"));
    // Important hidden from IMAP: the user label must not stand in for it.
    let user_only = [label("Important", "")];
    assert_eq!(found("\\Important", &user_only), None);
    // Listed without \Important (no role): the [Gmail]/ path still names it.
    let no_role = [
        label("[Google Mail]/Important", ""),
        label("[Gmail]/Drafts", ""),
    ];
    assert_eq!(
        found("\\Important", &no_role).as_deref(),
        Some("[Google Mail]/Important")
    );
    assert_eq!(
        found("\\Draft", &no_role).as_deref(),
        Some("[Gmail]/Drafts")
    );
    assert_eq!(
        found("\\Inbox", &[label("INBOX", "inbox")]).as_deref(),
        Some("INBOX")
    );
}

#[test]
fn threads_recount_when_a_label_appears_or_gains_a_role() {
    let f = fixture();
    let all = f.folder("[Gmail]/All Mail", "all");
    f.label("INBOX", "inbox");
    let user_important = f.label("Important", "");
    let gattrs = |uid: u32| MessageAttributes {
        uid,
        unread: true,
        labels: vec!["\\Important".into(), "\\Inbox".into()],
        ..Default::default()
    };
    let msg = f.insert("i@x", &all, 1, gattrs(1), None);
    // Important hidden from IMAP: no label for \Important, and not the user label.
    assert_eq!(thread_label_paths(&f, msg.thread_id()), ["INBOX"]);
    f.assert_invariants();

    // Shown again, without the flag at first: the label appears without a role.
    let before = f.store.all_labels_cache(ACCOUNT).unwrap();
    let mut important = f.label("[Gmail]/Important", "");
    let after = f.store.all_labels_cache(ACCOUNT).unwrap();
    let ids = Thread::ids_affected_by_label_change(&f.store, ACCOUNT, &before, &after).unwrap();
    assert_eq!(ids, [msg.thread_id().to_owned()]);
    f.store
        .transaction("recount", |s| Thread::recount_from_messages(s, &ids))
        .unwrap();
    assert_eq!(
        thread_label_paths(&f, msg.thread_id()),
        ["INBOX", "[Gmail]/Important"]
    );
    assert_eq!(f.thread_counts(important.id()), (1, 1));
    assert_eq!(f.thread_counts(user_important.id()), (0, 0));
    f.assert_invariants();

    // Gaining the role changes no resolution but the role the thread embeds.
    let before = f.store.all_labels_cache(ACCOUNT).unwrap();
    important.set_role("important");
    f.store.save(&mut important).unwrap();
    let after = f.store.all_labels_cache(ACCOUNT).unwrap();
    let ids = Thread::ids_affected_by_label_change(&f.store, ACCOUNT, &before, &after).unwrap();
    assert_eq!(ids, [msg.thread_id().to_owned()]);
    f.store
        .transaction("recount", |s| Thread::recount_from_messages(s, &ids))
        .unwrap();
    let t = f.thread(msg.thread_id()).unwrap();
    let embedded = t
        .labels()
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == important.id())
        .unwrap()
        .clone();
    assert_eq!(embedded["role"], "important");
    f.assert_invariants();

    // Nothing changed: nothing to recount.
    let same = f.store.all_labels_cache(ACCOUNT).unwrap();
    assert!(
        Thread::ids_affected_by_label_change(&f.store, ACCOUNT, &same, &same)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fetchable_copies_are_ranked() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let trash = f.folder("Trash", "trash");
    let other = f.folder("Other", "");
    let msg = f.insert("c@x", &trash, 1, attrs(1, false), None);
    let mut m = f.load(msg.id());
    f.store
        .upsert_placement(&mut m, &inbox, 2, &attrs(2, false))
        .unwrap();
    f.store
        .upsert_placement(&mut m, &other, 3, &attrs(3, false))
        .unwrap();
    f.store.save(&mut m).unwrap();
    let copies = f
        .store
        .fetchable_copies_of_message(&m, Some(&other))
        .unwrap();
    let order: Vec<&str> = copies.iter().map(|c| c.folder.path()).collect();
    assert_eq!(order, ["Other", "INBOX", "Trash"]);
}

#[test]
fn attributes_in_range_and_uid_at_depth() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    for uid in [3, 5, 9, 20] {
        f.insert(
            &format!("{uid}@x"),
            &inbox,
            uid,
            attrs(uid, uid % 2 == 1),
            None,
        );
    }
    let all = f
        .store
        .fetch_messages_attributes_in_range(&UidSet::range(4, None), &inbox)
        .unwrap();
    let mut uids: Vec<u32> = all.keys().copied().collect();
    uids.sort();
    assert_eq!(uids, [5, 9, 20]);
    assert!(all[&5].unread && !all[&20].unread);
    assert!(!all[&5].message_id.is_empty());
    assert!(all[&5].matches(&attrs(5, true)));
    assert_eq!(
        f.store
            .fetch_message_uid_at_depth(&inbox, 1, u32::MAX)
            .unwrap(),
        9
    );
    assert_eq!(f.store.fetch_message_uid_at_depth(&inbox, 0, 9).unwrap(), 5);
    assert_eq!(
        f.store
            .fetch_message_uid_at_depth(&inbox, 10, u32::MAX)
            .unwrap(),
        1
    );
}

#[test]
fn deletion_placeholder_takes_over_the_drafts_copy() {
    let f = fixture();
    let drafts = f.folder("Drafts", "drafts");
    let draft = f.insert("d@x", &drafts, 4, attrs(4, false), None);
    let mut d = f.load(draft.id());
    let mut stub = Message::deletion_placeholder_for(&d, 100);
    assert!(stub.is_deletion_placeholder());
    assert_eq!(stub.header_message_id(), format!("deleted-{}", stub.id()));
    assert_eq!(stub.version(), 0);
    assert_eq!(stub.synced_at(), 100 + 3600);
    f.store
        .transaction("destroyDraft", |s| {
            s.save(&mut stub)?;
            s.commit_placement_move(&mut d, drafts.id(), 4, drafts.id(), 4)?;
            s.conn().execute(
                "UPDATE MessageFolder SET messageId = ? WHERE messageId = ?",
                [stub.id(), d.id()],
            )?;
            s.refresh_message_from_placements(&mut stub)?;
            s.save(&mut stub)?;
            s.remove(&mut d)?;
            Ok(())
        })
        .unwrap();
    f.assert_invariants();
    let thread = f.thread(stub.thread_id()).unwrap();
    assert_eq!(
        thread.last_message_timestamp(),
        0,
        "drafts and placeholders do not date a thread"
    );
}

#[test]
fn lmrt_falls_back_to_sent_mail() {
    let f = fixture();
    let sent = f.folder("Sent", "sent");
    let inbox = f.folder("INBOX", "inbox");
    let a = f.insert("a@x", &sent, 1, attrs(1, false), None);
    let t = f.thread(a.thread_id()).unwrap();
    assert_eq!(t.last_message_received_timestamp(), 1_600_000_001);
    assert_eq!(t.data["lmrt_is_fallback"], true);
    let b = f.insert("b@x", &inbox, 2, attrs(2, false), Some(a.thread_id()));
    let t = f.thread(b.thread_id()).unwrap();
    assert!(t.data.get("lmrt_is_fallback").is_none());
    assert_eq!(t.last_message_received_timestamp(), 1_600_000_002);
    assert_eq!(t.participants().as_array().unwrap().len(), 2);
    f.assert_invariants();
}

#[test]
fn hidden_reminders_do_not_move_lmst() {
    let f = fixture();
    let sent = f.folder("Sent", "sent");
    // "via Mailspring": reminders created before the rename.
    for (i, name) in ["Ben via Tern", "Ben via Mailspring"]
        .into_iter()
        .enumerate()
    {
        let uid = 1 + i as u32;
        let r = f.insert_with(
            &format!("r{uid}@x"),
            &sent,
            uid,
            attrs(uid, false),
            None,
            |h| h.from = vec![tern_core::Address::new(Some(name), "me@example.com")],
        );
        assert!(f.load(r.id()).is_hidden_reminder(), "{name}");
        assert_eq!(
            f.thread(r.thread_id())
                .unwrap()
                .last_message_sent_timestamp(),
            0,
            "{name}"
        );
    }
    for (i, name) in ["via Tern", "via Mailspring", "Ben via Ternary"]
        .into_iter()
        .enumerate()
    {
        let uid = 10 + i as u32;
        let r = f.insert_with(
            &format!("n{uid}@x"),
            &sent,
            uid,
            attrs(uid, false),
            None,
            |h| h.from = vec![tern_core::Address::new(Some(name), "me@example.com")],
        );
        assert!(!f.load(r.id()).is_hidden_reminder(), "{name}");
    }
}

#[test]
fn thread_lookup_by_references() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let a = f.insert_with("root@x", &inbox, 1, attrs(1, false), None, |h| {
        h.references = vec!["r0@x".into(), "r1@x".into()]
    });
    for hmid in ["root@x", "r0@x", "r1@x"] {
        let t = f
            .store
            .find_thread_by_references(ACCOUNT, "nope", &[hmid.to_owned()])
            .unwrap();
        assert_eq!(t.unwrap().id(), a.thread_id());
    }
    let t = f
        .store
        .find_thread_by_references(ACCOUNT, "root@x", &[])
        .unwrap();
    assert!(t.is_some());
    assert!(f
        .store
        .find_thread_by_references("other", "root@x", &[])
        .unwrap()
        .is_none());

    let mut thread = f.thread(a.thread_id()).unwrap();
    f.store
        .append_to_thread_search_content(&mut thread, None, Some("hello body"))
        .unwrap();
    let (from, body): (String, String) = f
        .store
        .conn()
        .query_row(
            "SELECT from_, body FROM ThreadSearch WHERE rowid = ?",
            [thread.search_row_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(from, " ann@example.com Ann");
    assert_eq!(body, " hello body");
}

#[test]
fn many_references_index_root_and_recent() {
    let f = fixture();
    let refs: Vec<String> = (0..150).map(|i| format!("ref{i}@x")).collect();
    f.store
        .upsert_thread_references("t:1", ACCOUNT, "self@x", &refs)
        .unwrap();
    let n: i64 = f
        .store
        .conn()
        .query_row("SELECT COUNT(*) FROM ThreadReference", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1 + 1 + 99);
    let has = |h: &str| -> bool {
        f.store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM ThreadReference WHERE headerMessageId = ?",
                [h],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    };
    assert!(has("ref0@x") && has("ref51@x") && has("ref149@x") && !has("ref50@x"));
}

#[test]
fn recount_from_scratch_matches_incremental_state() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let sent = f.folder("Sent", "sent");
    let a = f.insert("a@x", &inbox, 1, attrs(1, true), None);
    f.insert("b@x", &sent, 2, attrs(2, false), Some(a.thread_id()));
    let before = f.thread(a.thread_id()).unwrap();
    // `_migrateToV11`'s recount: reset, re-apply every message, save.
    f.store
        .transaction("recount", |s| {
            let mut t = s
                .find::<Thread>(&Query::new().equal("id", a.thread_id()))?
                .unwrap();
            t.reset_counted_attributes();
            for m in s.find_all::<Message>(&Query::new().equal("threadId", a.thread_id()))? {
                t.apply_message_attribute_changes(
                    &tern_store::MessageSnapshot::empty(),
                    Some(&m),
                    s,
                )?;
            }
            s.save(&mut t)
        })
        .unwrap();
    let after = f.thread(a.thread_id()).unwrap();
    assert_eq!(before.folders(), after.folders());
    assert_eq!(before.unread(), after.unread());
    f.assert_invariants();
}

#[test]
fn saving_a_thread_twice_counts_once() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let a = f.insert("a@x", &inbox, 1, attrs(1, true), None);
    let mut t = f.thread(a.thread_id()).unwrap();
    f.store.save(&mut t).unwrap();
    f.store.save(&mut t).unwrap();
    assert_eq!(f.thread_counts(inbox.id()), (1, 1));
    f.assert_invariants();
}

#[test]
fn renamed_entry_rewrites_search_categories() {
    let f = fixture();
    let projects = f.folder("Projects", "");
    let a = f.insert("a@x", &projects, 1, attrs(1, true), None);
    let search = |t: &Thread| -> String {
        f.store
            .conn()
            .query_row(
                "SELECT categories FROM ThreadSearch WHERE rowid = ?",
                [t.search_row_id()],
                |r| r.get(0),
            )
            .unwrap()
    };
    let mut t = f.thread(a.thread_id()).unwrap();
    assert_eq!(search(&t), "Projects ");

    // A rename keeps the id: only the entry's path changes.
    let mut renamed = projects.clone();
    renamed.set_path("Clients");
    f.store.save(&mut renamed).unwrap();
    assert!(t.refresh_category_entry(&renamed.to_json()));
    f.store.save(&mut t).unwrap();
    assert_eq!(search(&t), "Clients ");
    assert_eq!(f.thread_counts(projects.id()), (1, 1));
    f.assert_invariants();
}

#[test]
fn invariants_catch_a_broken_snapshot() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let a = f.insert("a@x", &inbox, 1, attrs(1, true), None);
    f.store
        .conn()
        .execute(
            "UPDATE MessageFolder SET unread = 0 WHERE messageId = ?",
            [a.id()],
        )
        .unwrap();
    f.store
        .conn()
        .execute("UPDATE ThreadCounts SET total = 5", [])
        .unwrap();
    f.store
        .conn()
        .execute("DELETE FROM MessageFolder", [])
        .unwrap();
    let v = tern_store::invariants::check(f.store.conn(), &[]).unwrap();
    let names: std::collections::BTreeSet<&str> =
        v.iter().map(|s| &s[1..s.find(']').unwrap()]).collect();
    assert!(
        names.contains("message_snapshot")
            && names.contains("thread_counts")
            && names.contains("orphans"),
        "{v:?}"
    );
    let v = tern_store::invariants::check(
        f.store.conn(),
        &["message_snapshot", "thread_counts", "orphans"],
    )
    .unwrap();
    assert!(v.is_empty(), "{v:?}");
}
