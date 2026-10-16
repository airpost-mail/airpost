// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! TaskProcessor against a migrated database and an in-memory IMAP server, every case
//! checked with `ms_store::invariants::check`. TaskProcessor.cpp is the reference; the
//! placement rules are in "Message Identity and Placements" in legacy/mailsync-cpp/CLAUDE.md.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ms_core::{
    Account, Address, ErrorCode, MailError, ManualClock, MessageFlags, MessageHeader, UidSet,
};
use ms_imap::{
    Capabilities, FetchKind, FetchResult, FolderStatus, IdleInterrupt, ImapAuth, ImapMessage,
    ImapSession, ListedFolder, Namespace, SelectInfo, StoreOp, SyncResult, UidMapping,
};
use ms_providers::ServerProfile;
use ms_store::{DeltaStream, Folder, Message, Model, Query, Store, Task};
use ms_sync::mail_processor::{id_for_message, MailProcessor};
use ms_sync::RemoteTaskRunner;
use ms_tasks::{Mailer, TaskProcessor};
use serde_json::{json, Value};

const ACCOUNT: &str = "acc1";
const T0: i64 = 1_700_000_000;

// -- An in-memory server ------------------------------------------------------------------

#[derive(Default)]
struct Mailbox {
    uid_next: u32,
    messages: BTreeMap<u32, ImapMessage>,
}

struct FakeSession {
    caps: Capabilities,
    profile: ServerProfile,
    namespace: Namespace,
    boxes: BTreeMap<String, Mailbox>,
    /// MOVE / COPY answer with the destination UIDs reversed, as Yahoo does.
    permute_copyuid: bool,
    /// Commands that answer NO, by name (`STORE`, `MOVE`, ...).
    reject: Vec<&'static str>,
    log: Vec<String>,
    /// Where the "server" files its own copy of every SMTP submission, as Exchange and
    /// Gmail do. Copies appear the next time the folder is looked at.
    files_sent_copies_in: Option<String>,
    outbox: Outbox,
    filed: usize,
}

impl FakeSession {
    fn new(caps: &[&str]) -> Self {
        FakeSession {
            caps: Capabilities::new(caps.iter().copied()),
            profile: ServerProfile::default(),
            namespace: Namespace {
                prefix: String::new(),
                delimiter: Some('/'),
            },
            boxes: BTreeMap::new(),
            permute_copyuid: false,
            reject: Vec::new(),
            log: Vec::new(),
            files_sent_copies_in: None,
            outbox: Outbox::default(),
            filed: 0,
        }
    }

    fn file_sent_copies(&mut self) {
        let Some(path) = self.files_sent_copies_in.clone() else {
            return;
        };
        let submissions = self.outbox.sent.lock().unwrap().clone();
        for sub in &submissions[self.filed..] {
            let m = ImapMessage {
                internal_date: Some(T0),
                header: Some(header_from_raw(&sub.raw)),
                ..Default::default()
            };
            self.add(&path, m);
        }
        self.filed = submissions.len();
    }

    fn add(&mut self, path: &str, m: ImapMessage) -> u32 {
        let b = self.boxes.entry(path.to_owned()).or_insert(Mailbox {
            uid_next: 1,
            ..Default::default()
        });
        let uid = b.uid_next;
        b.uid_next += 1;
        b.messages.insert(uid, ImapMessage { uid, ..m });
        uid
    }

    fn uids(&self, path: &str) -> Vec<u32> {
        self.boxes
            .get(path)
            .map(|b| b.messages.keys().copied().collect())
            .unwrap_or_default()
    }

    fn check(&mut self, cmd: &'static str, detail: String) -> ms_imap::Result<()> {
        self.log.push(format!("{cmd} {detail}"));
        if self.reject.contains(&cmd) {
            let code = match cmd {
                "STORE" => ErrorCode::Store,
                "EXPUNGE" | "UID EXPUNGE" => ErrorCode::Expunge,
                _ => ErrorCode::Copy,
            };
            return Err(MailError::new(code, "NO rejected"));
        }
        Ok(())
    }

    fn transfer(&mut self, path: &str, uids: &UidSet, dest: &str, remove: bool) -> UidMapping {
        let moved: Vec<ImapMessage> = {
            let b = self.boxes.get_mut(path).unwrap();
            let picked: Vec<u32> = b
                .messages
                .keys()
                .copied()
                .filter(|u| uids.contains(*u))
                .collect();
            picked
                .iter()
                .map(|u| {
                    if remove {
                        b.messages.remove(u).unwrap()
                    } else {
                        b.messages[u].clone()
                    }
                })
                .collect()
        };
        let mut pairs = Vec::new();
        for m in moved {
            let src = m.uid;
            let mut copy = m.clone();
            copy.flags.remove(MessageFlags::DELETED);
            let new_uid = self.add(dest, copy);
            pairs.push((src, new_uid));
        }
        if self.permute_copyuid && pairs.len() > 1 {
            let dests: Vec<u32> = pairs.iter().rev().map(|p| p.1).collect();
            for (p, d) in pairs.iter_mut().zip(dests) {
                p.1 = d;
            }
        }
        UidMapping(pairs)
    }
}

impl ImapSession for FakeSession {
    fn connect_if_needed(&mut self) -> ms_imap::Result<()> {
        Ok(())
    }
    fn login_if_needed(&mut self) -> ms_imap::Result<()> {
        Ok(())
    }
    fn is_disconnected(&self) -> bool {
        false
    }
    fn disconnect(&mut self) {}
    fn set_auth(&mut self, _: ImapAuth) {}
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn profile(&self) -> &ServerProfile {
        &self.profile
    }
    fn default_namespace(&self) -> Option<&Namespace> {
        Some(&self.namespace)
    }
    fn is_qresync_enabled(&self) -> bool {
        false
    }
    fn message_limit(&self) -> Option<u32> {
        None
    }
    fn current_folder(&self) -> Option<&str> {
        None
    }
    fn last_tls_error(&self) -> Option<&str> {
        None
    }
    fn list_folders(&mut self) -> ms_imap::Result<Vec<ListedFolder>> {
        unimplemented!()
    }
    fn select(&mut self, path: &str) -> ms_imap::Result<SelectInfo> {
        self.log.push(format!("SELECT {path}"));
        let b = &self.boxes[path];
        Ok(SelectInfo {
            uid_validity: 1,
            uid_next: b.uid_next,
            highest_modseq: 0,
            exists: b.messages.len() as u32,
        })
    }
    fn select_if_needed(&mut self, _: &str) -> ms_imap::Result<()> {
        Ok(())
    }
    fn noop(&mut self) -> ms_imap::Result<()> {
        Ok(())
    }
    fn status(&mut self, path: &str) -> ms_imap::Result<FolderStatus> {
        let b = &self.boxes[path];
        Ok(FolderStatus {
            message_count: b.messages.len() as u32,
            uid_next: b.uid_next,
            uid_validity: 1,
            ..Default::default()
        })
    }
    fn create_folder(&mut self, path: &str) -> ms_imap::Result<()> {
        self.check("CREATE", path.to_owned())?;
        self.boxes.entry(path.to_owned()).or_insert(Mailbox {
            uid_next: 1,
            ..Default::default()
        });
        Ok(())
    }
    fn rename_folder(&mut self, path: &str, new_path: &str) -> ms_imap::Result<()> {
        self.check("RENAME", format!("{path} {new_path}"))?;
        let b = self.boxes.remove(path).unwrap_or_default();
        self.boxes.insert(new_path.to_owned(), b);
        Ok(())
    }
    fn delete_folder(&mut self, path: &str) -> ms_imap::Result<()> {
        self.check("DELETE", path.to_owned())?;
        self.boxes.remove(path);
        Ok(())
    }
    fn fetch_by_uid(
        &mut self,
        path: &str,
        _: FetchKind,
        uids: &UidSet,
    ) -> ms_imap::Result<FetchResult> {
        self.log.push(format!("FETCH {path} {uids}"));
        let messages = self.boxes[path]
            .messages
            .values()
            .filter(|m| uids.contains(m.uid))
            .cloned()
            .collect();
        Ok(FetchResult {
            messages,
            hit_message_limit: false,
        })
    }
    fn sync_by_uid(
        &mut self,
        _: &str,
        _: FetchKind,
        _: &UidSet,
        _: u64,
    ) -> ms_imap::Result<SyncResult> {
        unimplemented!()
    }
    fn take_vanished(&mut self, _: &str) -> UidSet {
        UidSet::new()
    }
    fn fetch_message_bytes(&mut self, path: &str, uid: u32) -> ms_imap::Result<Option<Vec<u8>>> {
        Ok(self.boxes[path].messages.get(&uid).map(|m| {
            let h = m.header.as_ref().unwrap();
            format!(
                "Message-ID: <{}>\r\nSubject: {}\r\n\r\nbody\r\n",
                h.message_id.as_deref().unwrap(),
                h.subject.as_deref().unwrap()
            )
            .into_bytes()
        }))
    }
    fn store_flags(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        flags: MessageFlags,
    ) -> ms_imap::Result<()> {
        self.check("STORE", format!("{path} {uids} {op:?} {flags:?}"))?;
        for m in self.boxes.get_mut(path).unwrap().messages.values_mut() {
            if uids.contains(m.uid) {
                match op {
                    StoreOp::Add => m.flags.insert(flags),
                    StoreOp::Remove => m.flags.remove(flags),
                    StoreOp::Set => m.flags = flags,
                }
            }
        }
        Ok(())
    }
    fn store_labels(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        labels: &[String],
    ) -> ms_imap::Result<()> {
        self.check("STORE", format!("{path} {uids} {op:?} {labels:?}"))
    }
    fn move_uids(&mut self, path: &str, uids: &UidSet, dest: &str) -> ms_imap::Result<UidMapping> {
        self.check("MOVE", format!("{path} {uids} {dest}"))?;
        Ok(self.transfer(path, uids, dest, true))
    }
    fn copy_uids(&mut self, path: &str, uids: &UidSet, dest: &str) -> ms_imap::Result<UidMapping> {
        self.check("COPY", format!("{path} {uids} {dest}"))?;
        Ok(self.transfer(path, uids, dest, false))
    }
    fn expunge(&mut self, path: &str) -> ms_imap::Result<()> {
        self.check("EXPUNGE", path.to_owned())?;
        self.boxes
            .get_mut(path)
            .unwrap()
            .messages
            .retain(|_, m| !m.flags.contains(MessageFlags::DELETED));
        Ok(())
    }
    fn expunge_uids(&mut self, path: &str, uids: &UidSet) -> ms_imap::Result<()> {
        self.check("UID EXPUNGE", format!("{path} {uids}"))?;
        self.boxes
            .get_mut(path)
            .unwrap()
            .messages
            .retain(|u, m| !(uids.contains(*u) && m.flags.contains(MessageFlags::DELETED)));
        Ok(())
    }
    fn append(
        &mut self,
        path: &str,
        data: &[u8],
        flags: MessageFlags,
    ) -> ms_imap::Result<Option<u32>> {
        self.check("APPEND", path.to_owned())?;
        let uid = self.add(
            path,
            ImapMessage {
                flags,
                internal_date: Some(T0),
                header: Some(header_from_raw(data)),
                ..Default::default()
            },
        );
        Ok(self.caps.has("UIDPLUS").then_some(uid))
    }
    fn find_uids_of_recent_header_message_id(
        &mut self,
        path: &str,
        header_message_id: &str,
    ) -> ms_imap::Result<Vec<u32>> {
        self.log.push(format!("FIND {path} {header_message_id}"));
        self.file_sent_copies();
        Ok(self.boxes[path]
            .messages
            .values()
            .filter(|m| {
                m.header.as_ref().and_then(|h| h.message_id.as_deref()) == Some(header_message_id)
            })
            .map(|m| m.uid)
            .collect())
    }
    fn supports_idle(&self) -> bool {
        true
    }
    fn idle(&mut self, _: &str, _: &IdleInterrupt) -> ms_imap::Result<()> {
        unimplemented!()
    }
}

/// The headers the fake server reports for a message it was given as bytes.
fn header_from_raw(raw: &[u8]) -> MessageHeader {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or_default();
    let field = |name: &str| {
        head.split("\r\n")
            .find_map(|l| l.strip_prefix(&format!("{name}: ")))
            .map(str::to_owned)
    };
    MessageHeader {
        date_raw: field("Date"),
        date: Some(T0),
        subject_raw: field("Subject").map(String::into_bytes),
        subject: field("Subject"),
        message_id: field("Message-ID").map(|v| v.trim_matches(['<', '>']).to_owned()),
        from: vec![Address::new(None, "me@example.com")],
        to: vec![Address::new(None, "bob@example.net")],
        ..Default::default()
    }
}

// -- SMTP ---------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Submission {
    /// The explicit envelope of a per-recipient send; `None` for a header envelope.
    envelope: Option<(String, Vec<String>)>,
    raw: Vec<u8>,
}

impl Submission {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.raw).into_owned()
    }
}

#[derive(Clone, Default)]
struct Outbox {
    sent: Arc<Mutex<Vec<Submission>>>,
    /// Submissions from this index on are refused.
    refuse_from: Arc<Mutex<Option<usize>>>,
}

impl Outbox {
    fn submissions(&self) -> Vec<Submission> {
        self.sent.lock().unwrap().clone()
    }

    fn push(&self, sub: Submission) -> Result<(), MailError> {
        let mut sent = self.sent.lock().unwrap();
        if let Some(n) = *self.refuse_from.lock().unwrap() {
            if sent.len() >= n {
                return Err(MailError::new(ErrorCode::SendMessage, "550 refused"));
            }
        }
        sent.push(sub);
        Ok(())
    }
}

struct FakeMailer(Outbox);

impl Mailer for FakeMailer {
    fn send(&mut self, raw: &[u8]) -> Result<(), MailError> {
        self.0.push(Submission {
            envelope: None,
            raw: raw.to_vec(),
        })
    }
    fn send_to(&mut self, from: &str, recipients: &[String], raw: &[u8]) -> Result<(), MailError> {
        self.0.push(Submission {
            envelope: Some((from.to_owned(), recipients.to_vec())),
            raw: raw.to_vec(),
        })
    }
}

// -- Fixtures -----------------------------------------------------------------------------

struct Env {
    _dir: tempfile::TempDir,
    store: Store,
    account: Arc<Account>,
    imap: FakeSession,
    /// The waits the send path asked for, in seconds.
    sleeps: Arc<Mutex<Vec<u64>>>,
}

fn account() -> Arc<Account> {
    Arc::new(
        Account::from_json(json!({
            "id": ACCOUNT,
            "provider": "imap",
            "emailAddress": "me@example.com",
            "settings": {},
        }))
        .unwrap(),
    )
}

fn env(caps: &[&str]) -> Env {
    let dir = tempfile::tempdir().unwrap();
    ms_store::migrate::migrate(dir.path(), &ManualClock::new(T0), &mut Vec::new()).unwrap();
    let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    store.set_clock(Arc::new(ManualClock::new(T0)));
    Env {
        _dir: dir,
        store,
        account: account(),
        imap: FakeSession::new(caps),
        sleeps: Arc::default(),
    }
}

fn header(n: u32) -> MessageHeader {
    MessageHeader {
        date_raw: Some("Sun, 13 Sep 2020 12:26:40 +0000".into()),
        date: Some(1_600_000_000 + n as i64),
        subject_raw: Some(format!("Subject {n}").into_bytes()),
        subject: Some(format!("Subject {n}")),
        message_id: Some(format!("m{n}@x")),
        from: vec![Address::new(Some("Ann"), "ann@example.com")],
        to: vec![Address::new(None, "me@example.com")],
        ..Default::default()
    }
}

fn remote(n: u32, flags: MessageFlags) -> ImapMessage {
    ImapMessage {
        flags,
        internal_date: Some(1_600_000_000),
        header: Some(header(n)),
        ..Default::default()
    }
}

impl Env {
    fn folder(&mut self, path: &str, role: &str) -> Folder {
        let mut f = Folder::new(&ms_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
        f.set_path(path);
        f.set_role(role);
        self.store.save(&mut f).unwrap();
        self.imap.boxes.entry(path.to_owned()).or_insert(Mailbox {
            uid_next: 1,
            ..Default::default()
        });
        f
    }

    /// Puts message `n` in `folder` on the server and syncs that copy. Returns its id.
    fn deliver(&mut self, folder: &Folder, n: u32, flags: MessageFlags) -> String {
        let uid = self.imap.add(folder.path(), remote(n, flags));
        let fetched = self.imap.boxes[folder.path()].messages[&uid].clone();
        MailProcessor::new(&self.account, &self.store)
            .insert_fallback_to_update_message(&fetched, folder, T0)
            .unwrap();
        id_for_message(ACCOUNT, folder.path(), &fetched)
    }

    fn queue(&self, class: &str, data: Value) -> Task {
        let mut task = Task::from_json(json!({"__cls": class, "id": format!("t-{class}-{}", rand()), "aid": ACCOUNT, "status": "local", "v": 0})).unwrap();
        for (k, v) in data.as_object().unwrap() {
            task.data[k] = v.clone();
        }
        self.processor().perform_local(&mut task).unwrap();
        task
    }

    fn processor(&self) -> TaskProcessor<'_> {
        processor(&self.store, self)
    }

    fn remote(&mut self, task: &mut Task) {
        processor(&self.store, self.harness())
            .perform_remote(task, &mut self.imap)
            .unwrap();
    }

    fn harness(&self) -> Harness {
        Harness {
            account: self.account.clone(),
            dir: self._dir.path().to_owned(),
            outbox: self.imap.outbox.clone(),
            sleeps: self.sleeps.clone(),
        }
    }

    fn message(&self, id: &str) -> Option<Message> {
        self.store
            .find::<Message>(&Query::new().equal("id", id))
            .unwrap()
    }

    /// (folder path, UID, pending folder path) of each copy.
    fn copies(&self, id: &str) -> Vec<(String, u32, Option<String>)> {
        let path = |fid: &str| {
            self.store
                .folder_by_id(ACCOUNT, fid)
                .unwrap()
                .map(|f| f.path().to_owned())
                .unwrap_or_else(|| fid.to_owned())
        };
        self.store
            .placements_for_message(id)
            .unwrap()
            .into_iter()
            .map(|p| {
                (
                    path(&p.folder_id),
                    p.remote_uid,
                    p.pending_folder_id.as_deref().map(path),
                )
            })
            .collect()
    }

    fn assert_invariants(&self) {
        let violations = ms_store::invariants::check(self.store.conn(), &[]).unwrap();
        assert!(violations.is_empty(), "{violations:#?}");
    }
}

/// What a processor in these tests is built from, cloned out of [`Env`] so the store
/// and the fake server can be borrowed separately.
struct Harness {
    account: Arc<Account>,
    dir: std::path::PathBuf,
    outbox: Outbox,
    sleeps: Arc<Mutex<Vec<u64>>>,
}

impl From<&Env> for Harness {
    fn from(e: &Env) -> Harness {
        e.harness()
    }
}

fn processor(store: &Store, h: impl Into<Harness>) -> TaskProcessor<'_> {
    let h = h.into();
    let outbox = h.outbox;
    let sleeps = h.sleeps;
    TaskProcessor::new(h.account, store)
        .with_config_dir(h.dir)
        .with_mailer(Arc::new(move |_: &Account| {
            Ok(Box::new(FakeMailer(outbox.clone())) as Box<dyn Mailer>)
        }))
        .with_sleeper(Arc::new(move |d| sleeps.lock().unwrap().push(d.as_secs())))
}

fn rand() -> String {
    ms_core::ids::random_id()
}

fn folder_json(f: &Folder) -> Value {
    f.to_json()
}

// -- Cases --------------------------------------------------------------------------------

#[test]
fn move_with_permuted_copyuid_lands_each_copy_on_its_own_uid() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let archive = e.folder("Archive", "archive");
    e.imap.add("Archive", remote(90, MessageFlags::SEEN));
    let ids: Vec<String> = (1..=3)
        .map(|n| e.deliver(&inbox, n, MessageFlags::empty()))
        .collect();
    e.imap.permute_copyuid = true;

    let mut task = e.queue(
        "ChangeFolderTask",
        json!({"messageIds": ids, "folder": folder_json(&archive)}),
    );
    assert_eq!(task.status(), "remote");
    for id in &ids {
        let m = e.message(id).unwrap();
        assert_eq!(
            m.folder_ids(),
            vec![archive.id().to_owned()],
            "shown in Archive at once"
        );
        assert_eq!(
            task.data["undoPlacements"][id],
            json!([{"folderId": inbox.id(), "bits": 1}])
        );
    }

    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(task.data["error"].is_null());
    let server = &e.imap.boxes["Archive"].messages;
    for id in &ids {
        let copies = e.copies(id);
        assert_eq!(copies.len(), 1);
        let (path, uid, pending) = &copies[0];
        assert_eq!((path.as_str(), pending), ("Archive", &None));
        assert_eq!(&id_for_message(ACCOUNT, "Archive", &server[uid]), id);
        let m = e.message(id).unwrap();
        assert_eq!(m.sync_unsaved_changes(), 0);
        assert_eq!(m.synced_at(), T0);
    }
    assert!(e.imap.uids("INBOX").is_empty());
    e.assert_invariants();
}

#[test]
fn move_without_move_capability_copies_expunges_and_reapplies_flags() {
    let mut e = env(&["UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let archive = e.folder("Archive", "archive");
    let kept = MessageFlags::ANSWERED | MessageFlags::FORWARDED;
    let id = e.deliver(&inbox, 1, MessageFlags::SEEN | MessageFlags::FLAGGED | kept);
    // Another client marks it a draft after the last scan; the local placement wins.
    e.imap
        .boxes
        .get_mut("INBOX")
        .unwrap()
        .messages
        .get_mut(&1)
        .unwrap()
        .flags |= MessageFlags::DRAFT;

    let mut task = e.queue(
        "ChangeFolderTask",
        json!({"messageIds": [id], "folder": folder_json(&archive)}),
    );
    e.remote(&mut task);

    assert_eq!(task.status(), "complete");
    assert!(e.imap.log.iter().any(|l| l.starts_with("COPY ")));
    assert!(e
        .imap
        .log
        .iter()
        .any(|l| l.starts_with("UID EXPUNGE INBOX")));
    assert!(!e.imap.log.iter().any(|l| l.starts_with("EXPUNGE ")));
    let copies = e.copies(&id);
    assert_eq!(copies, vec![("Archive".to_owned(), 1, None)]);
    let flags = e.imap.boxes["Archive"].messages[&1].flags;
    assert_eq!(flags, MessageFlags::SEEN | MessageFlags::FLAGGED | kept);
    assert!(!e.imap.log.iter().any(|l| l.contains(" Set ")));
    e.assert_invariants();
}

#[test]
fn rejected_move_releases_the_messages_where_the_server_has_them() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let archive = e.folder("Archive", "archive");
    let id = e.deliver(&inbox, 1, MessageFlags::empty());
    e.imap.reject.push("MOVE");

    let mut task = e.queue(
        "ChangeFolderTask",
        json!({"messageIds": [id], "folder": folder_json(&archive)}),
    );
    e.remote(&mut task);

    assert_eq!(task.status(), "complete");
    assert_eq!(task.data["error"]["key"], "ErrorCopy");
    assert_eq!(task.data["error"]["debuginfo"], "moveMessages");
    assert_eq!(e.copies(&id), vec![("INBOX".to_owned(), 1, None)]);
    let m = e.message(&id).unwrap();
    assert_eq!(m.folder_ids(), vec![inbox.id().to_owned()]);
    assert_eq!(m.sync_unsaved_changes(), 0);
    e.assert_invariants();
}

#[test]
fn undo_queued_before_the_move_reached_the_server_lands_home() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let sent = e.folder("Sent", "sent");
    let trash = e.folder("Trash", "trash");
    // A self-addressed message: unread in INBOX, read in Sent.
    let id = e.deliver(&inbox, 1, MessageFlags::empty());
    e.deliver(&sent, 1, MessageFlags::SEEN);

    let mut trash_task = e.queue(
        "ChangeFolderTask",
        json!({"messageIds": [id], "folder": folder_json(&trash), "sourceFolderIds": [inbox.id()]}),
    );
    let undo_placements = trash_task.data["undoPlacements"][&id].clone();
    assert_eq!(
        undo_placements.as_array().unwrap().len(),
        2,
        "trash takes every copy"
    );

    let mut undo = e.queue(
        "ChangeFolderTask",
        json!({
            "messageIds": [id],
            "folder": folder_json(&inbox),
            "isUndo": true,
            "restorePlacements": {&id: undo_placements},
            "sourceFolderIds": [trash.id()],
        }),
    );
    let m = e.message(&id).unwrap();
    let mut shown = m.folder_ids();
    shown.sort();
    let mut home = vec![inbox.id().to_owned(), sent.id().to_owned()];
    home.sort();
    assert_eq!(
        shown, home,
        "the undo marks both copies home before either moved"
    );

    e.remote(&mut trash_task);
    assert_eq!(trash_task.status(), "complete");
    // Moved to Trash, still shown at home through the undo's markers.
    for (path, _, pending) in e.copies(&id) {
        assert_eq!(path, "Trash");
        assert!(pending.is_some());
    }
    e.remote(&mut undo);
    assert_eq!(undo.status(), "complete", "{}", undo.data);

    let mut copies = e.copies(&id);
    copies.sort();
    assert_eq!(
        copies
            .iter()
            .map(|c| (c.0.as_str(), c.2.clone()))
            .collect::<Vec<_>>(),
        vec![("INBOX", None), ("Sent", None)]
    );
    let p = e.store.placements_for_message(&id).unwrap();
    let bits: BTreeMap<String, i64> = p
        .iter()
        .map(|p| (p.folder_id.clone(), p.flag_bits()))
        .collect();
    assert_eq!(bits[inbox.id()], 1, "the unread copy went back to INBOX");
    assert_eq!(bits[sent.id()], 0);
    assert_eq!(e.imap.uids("Trash").len(), 0);
    let m = e.message(&id).unwrap();
    assert_eq!(m.sync_unsaved_changes(), 0);
    e.assert_invariants();
}

#[test]
fn rejected_star_fails_the_task_and_rescans_the_folder() {
    let mut e = env(&["MOVE", "UIDPLUS", "CONDSTORE"]);
    let inbox = e.folder("INBOX", "inbox");
    let id = e.deliver(&inbox, 1, MessageFlags::empty());
    let mut f = e
        .store
        .find::<Folder>(&Query::new().equal("id", inbox.id()))
        .unwrap()
        .unwrap();
    let initial = f.local_status().clone();
    f.local_status_mut().insert("lastDeep".into(), json!(T0));
    e.store.save_folder_status(&mut f, &initial).unwrap();
    e.imap.reject.push("STORE");

    let mut task = e.queue(
        "ChangeStarredTask",
        json!({"messageIds": [id], "starred": true}),
    );
    assert!(e.message(&id).unwrap().is_starred());
    e.remote(&mut task);

    assert_eq!(task.status(), "complete");
    assert_eq!(task.data["error"]["key"], "ErrorStore");
    let f = e
        .store
        .find::<Folder>(&Query::new().equal("id", inbox.id()))
        .unwrap()
        .unwrap();
    assert_eq!(f.local_status()["lastDeep"], json!(0));
    assert_eq!(e.message(&id).unwrap().sync_unsaved_changes(), 0);
    e.assert_invariants();
}

#[test]
fn mark_read_by_thread_fans_out_and_recounts() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let sent = e.folder("Sent", "sent");
    let id = e.deliver(&inbox, 1, MessageFlags::empty());
    e.deliver(&sent, 1, MessageFlags::empty());
    let thread_id = e.message(&id).unwrap().thread_id().to_owned();

    let mut task = e.queue(
        "ChangeUnreadTask",
        json!({"threadIds": [thread_id], "unread": false}),
    );
    assert!(!e.message(&id).unwrap().is_unread());
    e.assert_invariants();
    e.remote(&mut task);
    assert_eq!(task.status(), "complete");
    for path in ["INBOX", "Sent"] {
        assert!(e.imap.boxes[path].messages[&1]
            .flags
            .contains(MessageFlags::SEEN));
    }
    e.assert_invariants();
}

#[test]
fn destroyed_drafts_leave_placeholders_until_the_server_copy_is_gone() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.folder("INBOX", "inbox");
    let drafts = e.folder("Drafts", "drafts");
    e.folder("Trash", "trash");
    let synced = e.deliver(&drafts, 1, MessageFlags::DRAFT | MessageFlags::SEEN);

    let save = e.queue(
        "SyncbackDraftTask",
        json!({"draft": {"aid": ACCOUNT, "hMsgId": "local@x", "subject": "Unsent", "body": "<p>hi</p>", "v": 0, "__cls": "Message"}}),
    );
    assert_eq!(save.status(), "remote");
    let local = ms_core::ids::draft_id(ACCOUNT, "local@x");
    assert_eq!(e.copies(&local), vec![("Drafts".to_owned(), 0, None)]);
    let body: String = e
        .store
        .conn()
        .query_row(
            "SELECT value FROM MessageBody WHERE id = ?",
            [&local],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, "<p>hi</p>");
    e.assert_invariants();

    let mut destroy = e.queue("DestroyDraftTask", json!({"messageIds": [synced, local]}));
    assert!(e.message(&synced).is_none());
    assert!(e.message(&local).is_none());
    let stubs = destroy.data["stubIds"].as_array().unwrap().clone();
    assert_eq!(stubs.len(), 2);
    // Not checked here: a placeholder is not a draft while its row keeps the draft's
    // flags, and it names the draft's removed thread, until the remote phase removes it.

    e.remote(&mut destroy);
    assert_eq!(destroy.status(), "complete", "{}", destroy.data);
    for stub in &stubs {
        assert!(e.message(stub.as_str().unwrap()).is_none());
    }
    assert!(e.imap.uids("Drafts").is_empty());
    assert!(
        e.imap.uids("Trash").is_empty(),
        "moved to Trash and expunged there"
    );
    e.assert_invariants();
}

#[test]
fn local_failures_end_the_task_with_error_json() {
    let e = env(&[]);
    let wrong = {
        let mut t = Task::from_json(json!({"__cls": "ChangeUnreadTask", "id": "w", "aid": "other", "v": 0, "status": "local"})).unwrap();
        TaskProcessor::new(e.account.clone(), &e.store)
            .perform_local(&mut t)
            .unwrap();
        t
    };
    assert_eq!(wrong.status(), "complete");
    assert_eq!(
        wrong.data["error"],
        json!({"what": "std::exception", "key": "generic", "debuginfo": "You must provide an account id.", "retryable": false, "offline": false})
    );

    let mut e = e;
    let inbox = e.folder("INBOX", "inbox");
    let id = e.deliver(&inbox, 1, MessageFlags::empty());
    let bad = e.queue("ChangeUnreadTask", json!({"messageIds": [id]}));
    assert_eq!(bad.status(), "complete");
    assert_eq!(bad.data["error"]["key"], "unhandled-exception");
    assert_eq!(
        bad.data["error"]["what"],
        "[json.exception.type_error.302] type must be boolean, but is null"
    );
    // The transaction rolled back: the message is not held.
    assert_eq!(e.message(&id).unwrap().sync_unsaved_changes(), 0);

    let stored = e
        .store
        .find::<Task>(&Query::new().equal("id", bad.id()))
        .unwrap()
        .unwrap();
    assert_eq!(stored.status(), "complete");
}

#[test]
fn unknown_tasks_complete() {
    let mut e = env(&[]);
    let mut unknown = e.queue("MadeUpTask", json!({}));
    e.remote(&mut unknown);
    assert_eq!(unknown.status(), "complete");
    assert!(unknown.data["error"].is_null());

    let mut usage = e.queue("SendFeatureUsageEventTask", json!({"feature": "snooze"}));
    e.remote(&mut usage);
    assert_eq!(usage.status(), "complete");
    assert!(usage.data["error"].is_null());
}

#[test]
fn cancel_before_the_remote_phase_and_cleanup_after_launch() {
    let mut e = env(&[]);
    let mut t = e.queue(
        "ChangeStarredTask",
        json!({"messageIds": [], "starred": true}),
    );
    let p = TaskProcessor::new(e.account.clone(), &e.store);
    p.cancel(t.id()).unwrap();
    let mut t2 = e
        .store
        .find::<Task>(&Query::new().equal("id", t.id()))
        .unwrap()
        .unwrap();
    e.remote(&mut t2);
    assert_eq!(t2.status(), "cancelled");
    t = t2;
    assert!(t.should_cancel());

    let mut stuck = Task::create("ChangeUnreadTask", ACCOUNT, &json!({}));
    e.store.save(&mut stuck).unwrap();
    TaskProcessor::new(e.account.clone(), &e.store)
        .cleanup_tasks_after_launch()
        .unwrap();
    assert!(e
        .store
        .find::<Task>(&Query::new().equal("id", stuck.id()))
        .unwrap()
        .is_none());
    assert!(e
        .store
        .find::<Task>(&Query::new().equal("id", t.id()))
        .unwrap()
        .is_some());
}

#[test]
fn metadata_is_upserted_locally_with_a_new_version() {
    let mut e = env(&[]);
    let inbox = e.folder("INBOX", "inbox");
    let id = e.deliver(&inbox, 1, MessageFlags::empty());
    let mut task = e.queue(
        "SyncbackMetadataTask",
        json!({"modelId": id, "modelClassName": "message", "pluginId": "snooze", "value": {"expiration": T0 + 60}}),
    );
    assert_eq!(task.data["modelMetadataNewVersion"], 1);
    let again = e.queue(
        "SyncbackMetadataTask",
        json!({"modelId": id, "modelClassName": "Message", "pluginId": "snooze", "value": {}}),
    );
    assert_eq!(again.data["modelMetadataNewVersion"], 2);
    let m = e.message(&id).unwrap();
    assert_eq!(m.json()["metadata"][0]["v"], 2);
    e.remote(&mut task);
    assert_eq!(task.status(), "complete");
    e.assert_invariants();
}

#[test]
fn category_tasks() {
    let mut e = env(&[]);
    let inbox = e.folder("INBOX", "inbox");
    let mut create = e.queue("SyncbackCategoryTask", json!({"path": "Projects/2026"}));
    e.remote(&mut create);
    assert_eq!(create.status(), "complete", "{}", create.data);
    assert!(e.imap.boxes.contains_key("Projects/2026"));
    let id = ms_core::ids::folder_id(ACCOUNT, "Projects/2026");
    assert_eq!(create.data["created"]["id"], json!(id));
    assert!(e
        .store
        .find::<Folder>(&Query::new().equal("id", &id))
        .unwrap()
        .is_some());

    let roles = e.queue(
        "ChangeRoleMappingTask",
        json!({"path": "Projects/2026", "role": "inbox"}),
    );
    assert_eq!(roles.status(), "remote");
    let old = e
        .store
        .find::<Folder>(&Query::new().equal("id", inbox.id()))
        .unwrap()
        .unwrap();
    assert_eq!(old.role(), "");

    let missing = e.queue(
        "ChangeRoleMappingTask",
        json!({"path": "Nope", "role": "trash"}),
    );
    assert_eq!(missing.data["error"]["key"], "no-matching-folder");

    let mut destroy = e.queue("DestroyCategoryTask", json!({"path": "Projects/2026"}));
    e.remote(&mut destroy);
    assert_eq!(destroy.status(), "complete");
    assert!(!e.imap.boxes.contains_key("Projects/2026"));
}

#[test]
fn expunge_all_in_folder_removes_messages_only_there() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let trash = e.folder("Trash", "trash");
    let only = e.deliver(&trash, 1, MessageFlags::SEEN);
    let both = e.deliver(&trash, 2, MessageFlags::SEEN);
    e.deliver(&inbox, 2, MessageFlags::SEEN);

    let mut task = e.queue(
        "ExpungeAllInFolderTask",
        json!({"folder": folder_json(&trash)}),
    );
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(e.imap.uids("Trash").is_empty());
    assert!(e.message(&only).is_none());
    assert_eq!(e.copies(&both), vec![("INBOX".to_owned(), 1, None)]);
    e.assert_invariants();
}

#[test]
fn get_many_rfc2822_exports_the_folder() {
    let mut e = env(&[]);
    let inbox = e.folder("INBOX", "inbox");
    for n in 1..=3 {
        e.deliver(&inbox, n, MessageFlags::SEEN);
    }
    let out = tempfile::tempdir().unwrap();
    let mut task = e.queue(
        "GetManyRFC2822Task",
        json!({"folderId": inbox.id(), "folderPath": "INBOX", "outputDir": out.path().to_str().unwrap()}),
    );
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert_eq!(task.data["result"]["exported"], 3);
    assert_eq!(task.data["progress"]["lastUID"], 3);
    let mut names: Vec<String> = std::fs::read_dir(out.path())
        .unwrap()
        .map(|d| d.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names[0], "00000 - Subject 1 - 2020-09-13.eml");
    assert_eq!(names.len(), 3);
}

// -- Send ---------------------------------------------------------------------------------

/// A draft as the client serializes it, with `extra` keys added or replaced.
fn draft_json(header_message_id: &str, extra: Value) -> Value {
    let mut d = json!({
        "__cls": "Message",
        "aid": ACCOUNT,
        "v": 0,
        "hMsgId": header_message_id,
        "subject": "Hello Bob",
        "body": "<p>Hi Bob</p>",
        "plaintext": false,
        "from": [{"name": "Me", "email": "me@example.com"}],
        "to": [{"name": "Bob", "email": "bob@example.net"}],
        "cc": [],
        "bcc": [{"email": "boss@example.net"}],
        "replyTo": [],
        "files": [],
        "rthMsgId": "parent@example.net",
        "fwdMsgId": "",
    });
    for (k, v) in extra.as_object().unwrap() {
        d[k] = v.clone();
    }
    d
}

impl Env {
    fn mail_folders(&mut self) -> (Folder, Folder, Folder) {
        self.folder("INBOX", "inbox");
        self.folder("Trash", "trash");
        (
            self.folder("Sent", "sent"),
            self.folder("Drafts", "drafts"),
            self.folder("Archive", "archive"),
        )
    }

    fn tasks_of(&self, class: &str) -> Vec<Task> {
        self.store
            .find_all::<Task>(&Query::new().equal("accountId", ACCOUNT))
            .unwrap()
            .into_iter()
            .filter(|t| t.class() == class)
            .collect()
    }

    fn sent_folder(&self) -> Folder {
        self.store
            .find::<Folder>(&Query::new().equal("role", "sent"))
            .unwrap()
            .unwrap()
    }

    fn body_of(&self, id: &str) -> Option<String> {
        self.store
            .conn()
            .query_row("SELECT value FROM MessageBody WHERE id = ?", [id], |r| {
                r.get(0)
            })
            .ok()
    }
}

#[test]
fn send_appends_to_sent_when_the_server_files_no_copy() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (sent, drafts, _) = e.mail_folders();
    let draft_id = e.deliver(&drafts, 7, MessageFlags::DRAFT);
    let mut task = e.queue(
        "SendDraftTask",
        json!({
            "draft": draft_json("sent-1@example.test", json!({
                "id": draft_id,
                "hImportance": "high",
                "metadata": [
                    {"pluginId": "open-tracking", "v": 1, "value": {"uid": "x"}},
                    {"pluginId": "thread:snooze", "v": 1, "value": {"expiration": T0 + 60}},
                ],
            })),
            "perRecipientBodies": null,
        }),
    );
    assert_eq!(task.status(), "remote");
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(task.data["error"].is_null());
    assert_eq!(task.data["_performRemoteRan"], true);

    let subs = e.imap.outbox.submissions();
    assert_eq!(subs.len(), 1);
    assert!(subs[0].envelope.is_none());
    let wire = subs[0].text();
    for h in [
        "Message-ID: <sent-1@example.test>",
        "In-Reply-To: <parent@example.net>",
        "References: <parent@example.net>",
        "X-Priority: 1 (Highest)",
        "X-Mailer: Tern",
        "multipart/alternative",
    ] {
        assert!(wire.contains(h), "{h} missing from\n{wire}");
    }

    // The draft's server copy went to Trash and was expunged there; the draft is gone.
    assert!(e.imap.uids("Drafts").is_empty());
    assert!(e.imap.uids("Trash").is_empty());
    assert!(e.message(&draft_id).is_none());

    // The short poll series ran out, so our copy (Bcc kept) was appended \Seen.
    assert_eq!(*e.sleeps.lock().unwrap(), [1, 1, 2]);
    let sent_uids = e.imap.uids("Sent");
    assert_eq!(sent_uids.len(), 1);
    let copy = e.imap.boxes["Sent"].messages[&sent_uids[0]].clone();
    assert!(copy.flags.contains(MessageFlags::SEEN));
    assert!(e.imap.log.iter().any(|l| l == "APPEND Sent"));

    let id = id_for_message(ACCOUNT, "Sent", &copy);
    let local = e.message(&id).unwrap();
    assert_eq!(e.copies(&id), [("Sent".to_owned(), sent_uids[0], None)]);
    assert_eq!(local.json()["_asc"]["f"], sent.id());
    assert_eq!(local.json()["_asc"]["u"], sent_uids[0]);
    assert_eq!(local.json()["_asc"]["x"], T0 + 7 * 24 * 60 * 60);
    assert!(e.body_of(&id).unwrap().contains("Hi Bob"));
    assert!(e
        .sent_folder()
        .local_status()
        .get("serverSavesSent")
        .is_none());

    let metadata = e.tasks_of("SyncbackMetadataTask");
    assert_eq!(metadata.len(), 2);
    let on = |class: &str| {
        metadata
            .iter()
            .find(|t| t.data["modelClassName"] == class)
            .unwrap()
    };
    assert_eq!(on("message").data["modelId"], id);
    assert_eq!(
        on("message").data["modelHeaderMessageId"],
        "sent-1@example.test"
    );
    assert_eq!(on("message").data["pluginId"], "open-tracking");
    assert_eq!(on("thread").data["modelId"], local.thread_id());
    assert_eq!(on("thread").data["pluginId"], "snooze");
    assert!(metadata.iter().all(|t| t.status() == "remote"));
    e.assert_invariants();

    // A send never runs twice.
    e.remote(&mut task);
    assert_eq!(e.imap.outbox.submissions().len(), 1);
}

#[test]
fn send_records_the_copy_the_server_filed() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    e.imap.files_sent_copies_in = Some("Sent".into());
    let mut task = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("sent-2@example.test", json!({})), "perRecipientBodies": null}),
    );
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(task.data["error"].is_null());

    assert!(e.sleeps.lock().unwrap().is_empty());
    assert!(!e.imap.log.iter().any(|l| l.starts_with("APPEND")));
    let sent_uids = e.imap.uids("Sent");
    assert_eq!(sent_uids.len(), 1);
    let copy = e.imap.boxes["Sent"].messages[&sent_uids[0]].clone();
    let id = id_for_message(ACCOUNT, "Sent", &copy);
    let local = e.message(&id).unwrap();
    assert!(local.json().get("_asc").is_none());
    assert!(e.body_of(&id).is_some());
    assert_eq!(e.sent_folder().local_status()["serverSavesSent"], true);
    e.assert_invariants();
}

#[test]
fn multisend_sends_each_body_and_keeps_only_the_self_copy() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    e.imap.files_sent_copies_in = Some("Sent".into());
    let mut task = e.queue(
        "SendDraftTask",
        json!({
            "draft": draft_json("multi-1@example.test", json!({
                "to": [{"email": "bob@example.net"}, {"email": "carol@example.net"}],
            })),
            "perRecipientBodies": {
                "self": "<p>Hi all</p>",
                "carol@example.net": "<p>Hi carol pixel</p>",
                "bob@example.net": "<p>Hi bob pixel</p>",
            },
        }),
    );
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(task.data["error"].is_null());

    let subs = e.imap.outbox.submissions();
    assert_eq!(subs.len(), 2);
    for (sub, who) in subs.iter().zip(["bob", "carol"]) {
        let rcpt = format!("{who}@example.net");
        assert_eq!(
            sub.envelope,
            Some(("me@example.com".to_owned(), vec![rcpt]))
        );
        let text = sub.text();
        assert!(text.contains(&format!("Hi {who} pixel")), "{text}");
        assert!(text.contains("carol@example.net") && text.contains("bob@example.net"));
    }

    // The two tracked copies the server filed were deleted; ours, with the self body,
    // is the one left.
    let sent_uids = e.imap.uids("Sent");
    assert_eq!(sent_uids.len(), 1);
    assert!(e.imap.log.iter().any(|l| l == "APPEND Sent"));
    let copy = e.imap.boxes["Sent"].messages[&sent_uids[0]].clone();
    let id = id_for_message(ACCOUNT, "Sent", &copy);
    assert_eq!(e.copies(&id), [("Sent".to_owned(), sent_uids[0], None)]);
    let body = e.body_of(&id).unwrap();
    assert!(body.contains("Hi all") && !body.contains("pixel"), "{body}");
    e.assert_invariants();
}

#[test]
fn smtp_failures_end_the_send_with_their_keys() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (_, drafts, _) = e.mail_folders();
    let draft_id = e.deliver(&drafts, 7, MessageFlags::DRAFT);

    *e.imap.outbox.refuse_from.lock().unwrap() = Some(1);
    let mut partial = e.queue(
        "SendDraftTask",
        json!({
            "draft": draft_json("multi-2@example.test", json!({"id": draft_id})),
            "perRecipientBodies": {"self": "x", "a@example.net": "a", "b@example.net": "b"},
        }),
    );
    e.remote(&mut partial);
    assert_eq!(partial.status(), "complete");
    assert_eq!(partial.data["error"]["key"], "send-partially-failed");
    assert_eq!(
        partial.data["error"]["debuginfo"],
        "ErrorSendMessage:::\n - a@example.net"
    );
    // Nothing reached Sent and the draft is kept for the user to retry.
    assert!(e.message(&draft_id).is_some());
    assert!(e.imap.uids("Sent").is_empty());

    *e.imap.outbox.refuse_from.lock().unwrap() = Some(0);
    let mut failed = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("one@example.test", json!({})), "perRecipientBodies": null}),
    );
    e.remote(&mut failed);
    assert_eq!(failed.data["error"]["key"], "send-failed");
    assert_eq!(failed.data["error"]["debuginfo"], "ErrorSendMessage");

    let mut no_self = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("two@example.test", json!({})), "perRecipientBodies": {"a@example.net": "a"}}),
    );
    e.remote(&mut no_self);
    assert_eq!(no_self.data["error"]["key"], "no-self-body");
    e.assert_invariants();
}

#[test]
fn late_sent_copies_are_removed_keeping_the_appended_one() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let mut task = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("late-1@example.test", json!({})), "perRecipientBodies": null}),
    );
    e.remote(&mut task);
    let kept = e.imap.uids("Sent")[0];
    let ours = e.imap.boxes["Sent"].messages[&kept].clone();
    let id = id_for_message(ACCOUNT, "Sent", &ours);

    // The server's own copy turns up a few minutes later and a scan records it.
    let sent = e.sent_folder();
    let late = e.imap.add(
        "Sent",
        ImapMessage {
            flags: MessageFlags::empty(),
            ..ours
        },
    );
    let fetched = e.imap.boxes["Sent"].messages[&late].clone();
    MailProcessor::new(&e.account, &e.store)
        .insert_fallback_to_update_message(&fetched, &sent, T0)
        .unwrap();
    assert_eq!(e.copies(&id).len(), 2);

    let mut p = processor(&e.store, e.harness());
    p.remove_late_sent_copies(&sent, &mut e.imap).unwrap();
    drop(p);

    assert_eq!(e.imap.uids("Sent"), [kept]);
    assert_eq!(e.copies(&id), [("Sent".to_owned(), kept, None)]);
    assert_eq!(e.sent_folder().local_status()["serverSavesSent"], true);
    e.assert_invariants();
}

#[test]
fn late_sent_copies_are_removed_days_after_the_send() {
    for (after, removed) in [(2 * 60 * 60, true), (8 * 24 * 60 * 60, false)] {
        let mut e = env(&["MOVE", "UIDPLUS"]);
        e.mail_folders();
        let mut task = e.queue(
            "SendDraftTask",
            json!({"draft": draft_json("late-3@example.test", json!({})), "perRecipientBodies": null}),
        );
        e.remote(&mut task);
        let kept = e.imap.uids("Sent")[0];
        let ours = e.imap.boxes["Sent"].messages[&kept].clone();
        let id = id_for_message(ACCOUNT, "Sent", &ours);

        let sent = e.sent_folder();
        let late = e.imap.add("Sent", ours.clone());
        let fetched = e.imap.boxes["Sent"].messages[&late].clone();
        MailProcessor::new(&e.account, &e.store)
            .insert_fallback_to_update_message(&fetched, &sent, T0)
            .unwrap();

        // Past the C++ 30 minutes, within the 7 days; then past those too.
        e.store.set_clock(Arc::new(ManualClock::new(T0 + after)));
        let mut p = processor(&e.store, e.harness());
        p.remove_late_sent_copies(&sent, &mut e.imap).unwrap();
        drop(p);

        let expected: Vec<u32> = if removed {
            vec![kept]
        } else {
            vec![kept, late]
        };
        assert_eq!(e.imap.uids("Sent"), expected, "{after} s after the send");
        assert_eq!(e.copies(&id).len(), expected.len());
        e.assert_invariants();
    }
}

#[test]
fn late_sent_copies_with_another_id_are_matched_by_message_id() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let mut task = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("late-2@example.test", json!({})), "perRecipientBodies": null}),
    );
    e.remote(&mut task);
    let kept = e.imap.uids("Sent")[0];
    let ours = e.imap.boxes["Sent"].messages[&kept].clone();
    let id = id_for_message(ACCOUNT, "Sent", &ours);

    // The provider's copy lists other recipients, so it hashes to another id.
    let sent = e.sent_folder();
    let mut header = ours.header.clone().unwrap();
    header.cc = vec![Address::new(None, "carol@example.net")];
    let late = e.imap.add(
        "Sent",
        ImapMessage {
            header: Some(header),
            ..ours.clone()
        },
    );
    let fetched = e.imap.boxes["Sent"].messages[&late].clone();
    let late_id = id_for_message(ACCOUNT, "Sent", &fetched);
    assert_ne!(late_id, id);
    MailProcessor::new(&e.account, &e.store)
        .insert_fallback_to_update_message(&fetched, &sent, T0)
        .unwrap();

    // A row with the Message-ID that holds no Sent copy (an orphan of a moved INBOX copy)
    // is not the cleanup's to remove.
    let inbox = e
        .store
        .find::<Folder>(&Query::new().equal("role", "inbox"))
        .unwrap()
        .unwrap();
    let mut header = ours.header.clone().unwrap();
    header.cc = vec![Address::new(None, "dave@example.net")];
    let inbox_uid = e.imap.add(
        "INBOX",
        ImapMessage {
            header: Some(header),
            ..ours
        },
    );
    let inbox_copy = e.imap.boxes["INBOX"].messages[&inbox_uid].clone();
    let orphan_id = id_for_message(ACCOUNT, "INBOX", &inbox_copy);
    let processor_ = MailProcessor::new(&e.account, &e.store);
    processor_
        .insert_fallback_to_update_message(&inbox_copy, &inbox, T0)
        .unwrap();
    processor_
        .delete_vanished_placements(&inbox, &[inbox_uid])
        .unwrap();
    assert!(e.message(&orphan_id).is_some());

    let mut p = processor(&e.store, e.harness());
    p.remove_late_sent_copies(&sent, &mut e.imap).unwrap();
    drop(p);

    assert_eq!(e.imap.uids("Sent"), [kept]);
    assert_eq!(e.copies(&id), [("Sent".to_owned(), kept, None)]);
    assert!(
        e.message(&late_id).is_none(),
        "removed, not left to the orphan sweep"
    );
    assert!(e.message(&orphan_id).is_some(), "left to the orphan sweep");
    e.assert_invariants();
}

#[test]
fn send_without_uidplus_finds_the_appended_copy_by_message_id() {
    let mut e = env(&["MOVE"]);
    let (sent, _, _) = e.mail_folders();
    let mut task = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("no-uidplus@example.test", json!({})), "perRecipientBodies": null}),
    );
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(task.data["error"].is_null());

    let sent_uids = e.imap.uids("Sent");
    assert_eq!(sent_uids.len(), 1);
    let copy = e.imap.boxes["Sent"].messages[&sent_uids[0]].clone();
    let id = id_for_message(ACCOUNT, "Sent", &copy);
    let local = e.message(&id).expect("the appended copy is ingested");
    assert_eq!(local.json()["_asc"]["f"], sent.id());
    assert_eq!(local.json()["_asc"]["u"], sent_uids[0]);
    assert!(e.body_of(&id).unwrap().contains("Hi Bob"));
    e.assert_invariants();
}

#[test]
fn a_send_stopped_after_smtp_drops_the_draft_on_relaunch() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (_, drafts, _) = e.mail_folders();
    let delivered = e.deliver(&drafts, 7, MessageFlags::DRAFT);
    let not_delivered = e.deliver(&drafts, 8, MessageFlags::DRAFT);
    let stopped = |e: &Env, draft_id: &str, smtp_done: bool| {
        let mut task = e.queue(
            "SendDraftTask",
            json!({
                "draft": draft_json(&format!("{draft_id}@example.test"), json!({"id": draft_id})),
                "perRecipientBodies": null,
            }),
        );
        // What a run stopped during the Sent poll (or before SMTP) left on the task.
        task.data["_performRemoteRan"] = json!(true);
        if smtp_done {
            task.data["_smtpDone"] = json!(true);
        }
        e.store.save(&mut task).unwrap();
        task
    };

    let mut task = stopped(&e, &delivered, true);
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(e.message(&delivered).is_none());
    assert_eq!(
        e.imap.uids("Drafts").len(),
        1,
        "its server copy is gone too"
    );

    // SMTP may not have run: the draft stays for the user to send again.
    let mut task = stopped(&e, &not_delivered, false);
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(e.message(&not_delivered).is_some());

    assert!(
        e.imap.outbox.submissions().is_empty(),
        "nothing is sent again"
    );
    e.assert_invariants();
}

// -- RSVP ---------------------------------------------------------------------------------

fn reply_ics(method: &str, attendees: &[&str], extra: &str) -> String {
    let mut ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\n\
         UID:ev-1@example.net\r\nDTSTAMP:20261015T120000Z\r\nDTSTART:20261020T150000Z\r\n\
         DTEND:20261020T160000Z\r\nSUMMARY:Planning\r\nORGANIZER;CN=Org:mailto:org@example.net\r\n{extra}"
    );
    for a in attendees {
        ics.push_str(&format!("ATTENDEE;PARTSTAT=ACCEPTED:mailto:{a}\r\n"));
    }
    ics.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    ics
}

#[test]
fn rsvp_sends_an_imip_reply_to_the_organizer() {
    let mut e = env(&[]);
    let mut task = e.queue(
        "EventRSVPTask",
        json!({
            "ics": reply_ics("REPLY", &["me@example.com"], ""),
            "subject": "Accepted: Planning",
            "to": "org@example.net",
            "icsRSVPStatus": "ACCEPTED",
        }),
    );
    assert_eq!(task.status(), "remote");
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(task.data["error"].is_null());

    let subs = e.imap.outbox.submissions();
    assert_eq!(subs.len(), 1);
    let text = subs[0].text();
    for s in [
        "To: <org@example.net>",
        "From: <me@example.com>",
        "Subject: Accepted: Planning",
        "multipart/alternative",
        "text/calendar",
        "method=",
        "invite.ics",
    ] {
        assert!(text.contains(s), "{s} missing from\n{text}");
    }
    let parsed = ms_mime::parse_message(&subs[0].raw).unwrap();
    assert!(
        parsed
            .text
            .contains("has accepted the invitation to: Planning"),
        "{}",
        parsed.text
    );
    // No Sent copy.
    assert!(e.imap.log.is_empty(), "{:?}", e.imap.log);
}

#[test]
fn rsvp_rejects_ics_that_breaks_rfc_5546() {
    let mut e = env(&[]);
    {
        let mut run = |data: Value| {
            let mut task = e.queue("EventRSVPTask", data);
            e.remote(&mut task);
            assert_eq!(task.status(), "complete");
            (
                task.data["error"]["key"].as_str().unwrap_or("").to_owned(),
                task.data["error"]["debuginfo"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
            )
        };
        let base = |ics: String| json!({"ics": ics, "subject": "Re", "to": "org@example.net"});

        assert_eq!(run(json!({"ics": "x", "subject": "Re"})).0, "missing-json");
        let mut cancel = base(reply_ics("CANCEL", &["me@example.com"], ""));
        cancel["method"] = json!("CANCEL");
        assert_eq!(run(cancel).1, "Unsupported iTIP method: CANCEL");
        assert_eq!(
            run(base(reply_ics("REQUEST", &["me@example.com"], ""))).1,
            "ICS data must contain METHOD:REPLY"
        );
        assert_eq!(
            run(base(reply_ics(
                "REPLY",
                &["me@example.com", "you@example.com"],
                ""
            )))
            .1,
            "ICS REPLY must contain exactly one ATTENDEE (the replying user), found 2"
        );
        assert_eq!(
            run(base(
                reply_ics("REPLY", &["me@example.com"], "").replace("UID:ev-1@example.net\r\n", "")
            ))
            .1,
            "ICS REPLY must contain UID property matching the original invitation"
        );
        assert_eq!(
            run(base(
                reply_ics("REPLY", &["me@example.com"], "")
                    .replace("PARTSTAT=ACCEPTED", "PARTSTAT=NEEDS-ACTION")
            ))
            .1,
            "ATTENDEE must have valid PARTSTAT parameter (ACCEPTED, DECLINED, or TENTATIVE)"
        );
        let mut counter = base(reply_ics(
            "COUNTER",
            &["me@example.com", "you@example.com"],
            "",
        ));
        counter["method"] = json!("COUNTER");
        counter["comment"] = json!("Thursday?");
        assert_eq!(run(counter).0, "");
    }
    let subs = e.imap.outbox.submissions();
    assert_eq!(subs.len(), 1, "only the COUNTER was sent");
    let parsed = ms_mime::parse_message(&subs[0].raw).unwrap();
    assert!(parsed.text.contains("proposed a new time for: Planning"));
    assert!(parsed.text.contains("Thursday?"));
}

#[test]
fn send_reads_attachments_from_the_files_dir() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let file = |id: &str, name: &str, cid: Value| json!({"id": id, "aid": ACCOUNT, "v": 1, "__cls": "File", "filename": name, "contentId": cid});
    // The client stores "report.pdf " as "report.pdf" (`safeDisplayName`).
    for (id, name, data) in [
        ("abcdef12", "notes.txt", "plain notes"),
        ("12345678", "logo.png", "PNG"),
        ("9abcdef0", "report.pdf", "PDF bytes"),
    ] {
        let dir = e
            ._dir
            .path()
            .join("files")
            .join(&id[0..2])
            .join(&id[2..4])
            .join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), data).unwrap();
    }
    let mut task = e.queue(
        "SendDraftTask",
        json!({
            "draft": draft_json("files-1@example.test", json!({
                "body": "<p>See <img src=\"cid:logo-1\"></p>",
                "files": [
                    file("abcdef12", "notes.txt", Value::Null),
                    file("12345678", "logo.png", json!("logo-1")),
                    file("9abcdef0", "report.pdf ", Value::Null),
                ],
            })),
            "perRecipientBodies": null,
        }),
    );
    e.remote(&mut task);
    assert!(task.data["error"].is_null(), "{}", task.data);
    let wire = e.imap.outbox.submissions()[0].text();
    for s in [
        "multipart/mixed",
        "multipart/related",
        "notes.txt",
        "<logo-1>",
        "cGxhaW4gbm90ZXM=",
        "UERGIGJ5dGVz",
    ] {
        assert!(wire.contains(s), "{s} missing from\n{wire}");
    }

    let mut missing = e.queue(
        "SendDraftTask",
        json!({
            "draft": draft_json("files-2@example.test", json!({"files": [file("ffffffff", "gone.pdf", Value::Null)]})),
            "perRecipientBodies": null,
        }),
    );
    e.remote(&mut missing);
    assert_eq!(missing.data["error"]["key"], "send-failed");
    assert_eq!(
        e.imap.outbox.submissions().len(),
        1,
        "nothing sent without the file"
    );
    e.assert_invariants();
}
