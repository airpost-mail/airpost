// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Draft identity with "Save drafts to server" on and off (docs/drafts-sync-design.md 3.1):
//! the id a fetched draft copy is stored under, and the re-keying of a draft row that
//! predates it. Every case ends with `tern_store::invariants::check`.

use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use tern_core::{Account, Address, ManualClock, MessageFlags, MessageHeader};
use tern_imap::ImapMessage;
use tern_store::{DeltaStream, Folder, Message, Model, Query, Store};
use tern_sync::mail_processor::{id_for_copy, id_for_message, CopyLabels, MailProcessor};

const ACCOUNT: &str = "acc1";
const T0: i64 = 1_700_000_000;

fn account(drafts_sync: bool) -> Account {
    Account::from_json(json!({
        "id": ACCOUNT,
        "provider": "imap",
        "emailAddress": "me@example.com",
        "settings": {"save_drafts_to_server": drafts_sync},
    }))
    .unwrap()
}

fn store(dir: &Path) -> Store {
    tern_store::migrate::migrate(dir, &ManualClock::new(T0), &mut Vec::new()).unwrap();
    let store = Store::open(dir, DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    store.set_clock(Arc::new(ManualClock::new(T0)));
    store
}

fn folder(store: &Store, path: &str, role: &str) -> Folder {
    let mut f = Folder::new(&tern_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
    f.set_path(path);
    f.set_role(role);
    store.save(&mut f).unwrap();
    f
}

fn copy(message_id: Option<&str>, uid: u32, flags: MessageFlags) -> ImapMessage {
    ImapMessage {
        uid,
        flags,
        internal_date: Some(1_600_000_000),
        header: Some(MessageHeader {
            date_raw: Some("Sun, 13 Sep 2020 12:26:40 +0000".into()),
            date: Some(1_600_000_000),
            subject_raw: Some(b"Plans".to_vec()),
            subject: Some("Plans".into()),
            message_id: message_id.map(str::to_owned),
            from: vec![Address::new(Some("Me"), "me@example.com")],
            to: vec![Address::new(None, "ann@example.com")],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn assert_invariants(store: &Store) {
    let violations = tern_store::invariants::check(store.conn(), &[]).unwrap();
    assert!(violations.is_empty(), "{violations:#?}");
}

#[test]
fn draft_copies_are_keyed_by_message_id_when_drafts_sync() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let drafts = folder(&store, "Drafts", "drafts");
    let inbox = folder(&store, "INBOX", "inbox");
    let trash = folder(&store, "Trash", "trash");
    let on = account(true);
    let off = account(false);
    let draft_id = tern_core::ids::draft_id(ACCOUNT, "d1@example.com");
    let labels = CopyLabels::None;

    let in_drafts = copy(Some("d1@example.com"), 4, MessageFlags::empty());
    let flagged = copy(Some("d1@example.com"), 9, MessageFlags::DRAFT);
    let no_mid = copy(None, 5, MessageFlags::DRAFT);
    assert_eq!(id_for_copy(&on, &drafts, &in_drafts, labels), draft_id);
    assert_eq!(id_for_copy(&on, &inbox, &flagged, labels), draft_id);
    // Deleted elsewhere, or sent: an ordinary message.
    assert_eq!(
        id_for_copy(&on, &trash, &flagged, labels),
        id_for_message(ACCOUNT, "Trash", &flagged)
    );
    assert_eq!(
        id_for_copy(
            &on,
            &inbox,
            &copy(Some("d1@example.com"), 9, MessageFlags::SEEN),
            labels
        ),
        id_for_message(ACCOUNT, "INBOX", &flagged)
    );
    assert_eq!(
        id_for_copy(&on, &drafts, &no_mid, labels),
        id_for_message(ACCOUNT, "Drafts", &no_mid)
    );
    // Off: the C++ engine's header-hash id.
    assert_eq!(
        id_for_copy(&off, &drafts, &in_drafts, labels),
        id_for_message(ACCOUNT, "Drafts", &in_drafts)
    );
}

/// A server draft synced before (header-hash id, as the C++ engine and the setting off key
/// it) whose row the user edited is re-keyed by the next scan instead of being replaced by
/// a fresh row and swept: its body and thread carry over.
#[test]
fn a_draft_row_under_its_old_id_is_rekeyed_onto_its_copy() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let drafts = folder(&store, "Drafts", "drafts");
    let remote = copy(
        Some("d1@example.com"),
        4,
        MessageFlags::DRAFT | MessageFlags::SEEN,
    );

    let off = account(false);
    let old = MailProcessor::new(&off, &store)
        .insert_fallback_to_update_message(&remote, &drafts, T0)
        .unwrap();
    assert_eq!(old.id(), id_for_message(ACCOUNT, "Drafts", &remote));
    store
        .conn()
        .execute(
            "REPLACE INTO MessageBody (id, value) VALUES (?, '<p>edited here</p>')",
            [old.id()],
        )
        .unwrap();

    let on = account(true);
    let rekeyed = MailProcessor::new(&on, &store)
        .insert_fallback_to_update_message(&remote, &drafts, T0 + 60)
        .unwrap();
    let draft_id = tern_core::ids::draft_id(ACCOUNT, "d1@example.com");
    assert_eq!(rekeyed.id(), draft_id);
    assert_eq!(rekeyed.thread_id(), old.thread_id());
    assert!(store
        .find::<Message>(&Query::new().equal("id", old.id()))
        .unwrap()
        .is_none());
    assert_eq!(store.count::<Message>(&Query::new()).unwrap(), 1);
    let body: String = store
        .conn()
        .query_row(
            "SELECT value FROM MessageBody WHERE id = ?",
            [&draft_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, "<p>edited here</p>");
    let placements = store.placements_for_message(&draft_id).unwrap();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].remote_uid, 4);
    assert_invariants(&store);
}
