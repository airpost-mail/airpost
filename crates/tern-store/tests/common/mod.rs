// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Shared fixtures: a migrated database in a temp dir, a store whose delta stream writes
//! into a buffer, and folder / message builders shaped like what the sync worker writes.

#![allow(dead_code)]

use std::io::Write;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tern_core::{ManualClock, MessageHeader};
use tern_store::{
    DeltaStream, Folder, IncomingMessage, Label, Message, MessageAttributes, Model, Store, Thread,
};

pub const ACCOUNT: &str = "acc1";

#[derive(Clone, Default)]
pub struct Sink(pub Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink {
    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }

    pub fn lines(&self) -> Vec<Value> {
        self.text()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub store: Store,
    pub sink: Sink,
    pub stream: Arc<DeltaStream>,
    pub clock: Arc<ManualClock>,
}

pub fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(1_700_000_000));
    let mut out = Vec::new();
    tern_store::migrate::migrate(dir.path(), clock.as_ref(), &mut out).unwrap();
    let sink = Sink::default();
    let stream = DeltaStream::new(Box::new(sink.clone()));
    let store = Store::open(dir.path(), stream.clone()).unwrap();
    store.set_clock(clock.clone());
    Fixture {
        dir,
        store,
        sink,
        stream,
        clock,
    }
}

impl Fixture {
    pub fn folder(&self, path: &str, role: &str) -> Folder {
        let mut f = Folder::new(&tern_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
        f.set_path(path);
        f.set_role(role);
        self.store.save(&mut f).unwrap();
        f
    }

    pub fn label(&self, path: &str, role: &str) -> Label {
        let mut l = Label::new(&tern_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
        l.set_path(path);
        l.set_role(role);
        self.store.save(&mut l).unwrap();
        l
    }

    /// `MailProcessor::insertMessage`: a new message with its first copy, in a new thread
    /// (or `thread_id`'s), counted into the thread before either is saved.
    pub fn insert(
        &self,
        header_id: &str,
        folder: &Folder,
        uid: u32,
        attrs: MessageAttributes,
        thread_id: Option<&str>,
    ) -> Message {
        self.insert_with(header_id, folder, uid, attrs, thread_id, |_| {})
    }

    pub fn insert_with(
        &self,
        header_id: &str,
        folder: &Folder,
        uid: u32,
        attrs: MessageAttributes,
        thread_id: Option<&str>,
        tweak: impl FnOnce(&mut MessageHeader),
    ) -> Message {
        let mut header = MessageHeader {
            date: Some(1_600_000_000 + uid as i64),
            subject: Some(format!("Subject {header_id}")),
            message_id: Some(header_id.to_owned()),
            from: vec![tern_core::Address::new(Some("Ann"), "ann@example.com")],
            to: vec![tern_core::Address::new(None, "me@example.com")],
            ..Default::default()
        };
        tweak(&mut header);
        let id = tern_core::ids::hash_id(header_id);
        let inc = IncomingMessage {
            id: &id,
            header: &header,
            received_date: 1_600_000_000,
            header_message_id: header_id,
            gmail_message_id: 0,
            attrs: &attrs,
            sync_data_timestamp: 1_700_000_000,
        };
        let mut msg = Message::from_incoming(&inc, folder);
        let s = &self.store;
        s.transaction("insertMessage", |s| {
            let mut thread = match thread_id {
                Some(t) => s
                    .find::<Thread>(&tern_store::Query::new().equal("id", t))?
                    .unwrap(),
                None => Thread::create(msg.id(), ACCOUNT, msg.subject(), 0),
            };
            msg.set_thread_id(thread.id());
            s.upsert_placement(&mut msg, folder, uid, &attrs)?;
            s.refresh_message_from_placements(&mut msg)?;
            thread.apply_message_attribute_changes(
                &tern_store::MessageSnapshot::empty(),
                Some(&msg),
                s,
            )?;
            msg.capture_snapshot();
            msg.skip_thread_updates_after_save = true;
            s.append_to_thread_search_content(&mut thread, Some(&msg), None)?;
            s.save(&mut thread)?;
            s.save(&mut msg)?;
            s.upsert_thread_references(
                thread.id(),
                ACCOUNT,
                msg.header_message_id(),
                &header.references,
            )?;
            Ok(())
        })
        .unwrap();
        msg.skip_thread_updates_after_save = false;
        msg
    }

    pub fn load(&self, id: &str) -> Message {
        self.store
            .find::<Message>(&tern_store::Query::new().equal("id", id))
            .unwrap()
            .unwrap()
    }

    /// Loads, refreshes and saves each message, as callers of the bulk helpers do.
    pub fn refresh(&self, ids: &[String]) {
        self.store
            .transaction("refreshMessages", |s| {
                for id in ids {
                    if let Some(mut m) =
                        s.find::<Message>(&tern_store::Query::new().equal("id", id.as_str()))?
                    {
                        m.set_placements_changed(true);
                        s.save(&mut m)?;
                    }
                }
                Ok(())
            })
            .unwrap();
    }

    pub fn thread(&self, id: &str) -> Option<Thread> {
        self.store
            .find::<Thread>(&tern_store::Query::new().equal("id", id))
            .unwrap()
    }

    /// Fails the test with every invariant violation.
    pub fn assert_invariants(&self) {
        let violations = tern_store::invariants::check(self.store.conn(), &[]).unwrap();
        assert!(
            violations.is_empty(),
            "invariant violations:\n{}",
            violations.join("\n")
        );
    }

    pub fn thread_counts(&self, category: &str) -> (i64, i64) {
        self.store
            .conn()
            .query_row(
                "SELECT unread, total FROM ThreadCounts WHERE categoryId = ?",
                [category],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
}

pub fn attrs(uid: u32, unread: bool) -> MessageAttributes {
    MessageAttributes {
        uid,
        unread,
        ..Default::default()
    }
}

pub fn _id<M: Model>(m: &M) -> String {
    m.id().to_owned()
}
