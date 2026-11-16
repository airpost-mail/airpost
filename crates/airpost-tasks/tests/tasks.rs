// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! TaskProcessor against a migrated database and an in-memory IMAP server, every case
//! checked with `airpost_store::invariants::check`. TaskProcessor.cpp is the reference; the
//! placement rules are in "Message Identity and Placements" in Mailspring-Sync's CLAUDE.md.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use airpost_core::{
    Account, Address, ErrorCode, MailError, ManualClock, MessageFlags, MessageHeader, UidSet,
};
use airpost_imap::{
    Capabilities, FetchKind, FetchResult, FolderStatus, IdleInterrupt, ImapAuth, ImapMessage,
    ImapSession, ListedFolder, Namespace, SelectInfo, StoreOp, SyncResult, UidMapping,
};
use airpost_providers::ServerProfile;
use airpost_store::{DeltaStream, File, Folder, Message, Model, Query, Store, Task, Thread};
use airpost_sync::expiration_executor::{Executor, Outcome};
use airpost_sync::mail_processor::{id_for_message, MailProcessor};
use airpost_sync::pending_sends::{PendingBodies, PendingSends};
use airpost_sync::{EngineSettings, RemoteTaskRunner};
use airpost_tasks::{Mailer, TaskProcessor};
use serde_json::{json, Value};

const ACCOUNT: &str = "acc1";
const T0: i64 = 1_700_000_000;

#[path = "../../airpost-net/tests/common/mod.rs"]
mod common;

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
    /// The bytes of every APPEND, in order.
    appended: Vec<Vec<u8>>,
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
            appended: Vec::new(),
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

    fn check(&mut self, cmd: &'static str, detail: String) -> airpost_imap::Result<()> {
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
    fn connect_if_needed(&mut self) -> airpost_imap::Result<()> {
        Ok(())
    }
    fn login_if_needed(&mut self) -> airpost_imap::Result<()> {
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
    fn list_folders(&mut self) -> airpost_imap::Result<Vec<ListedFolder>> {
        unimplemented!()
    }
    fn select(&mut self, path: &str) -> airpost_imap::Result<SelectInfo> {
        self.log.push(format!("SELECT {path}"));
        let b = &self.boxes[path];
        Ok(SelectInfo {
            uid_validity: 1,
            uid_next: b.uid_next,
            highest_modseq: 0,
            exists: b.messages.len() as u32,
            ..Default::default()
        })
    }
    fn select_if_needed(&mut self, _: &str) -> airpost_imap::Result<()> {
        Ok(())
    }
    fn noop(&mut self) -> airpost_imap::Result<()> {
        Ok(())
    }
    fn status(&mut self, path: &str) -> airpost_imap::Result<FolderStatus> {
        let b = &self.boxes[path];
        Ok(FolderStatus {
            message_count: b.messages.len() as u32,
            uid_next: b.uid_next,
            uid_validity: 1,
            ..Default::default()
        })
    }
    fn create_folder(&mut self, path: &str) -> airpost_imap::Result<()> {
        self.check("CREATE", path.to_owned())?;
        self.boxes.entry(path.to_owned()).or_insert(Mailbox {
            uid_next: 1,
            ..Default::default()
        });
        Ok(())
    }
    fn rename_folder(&mut self, path: &str, new_path: &str) -> airpost_imap::Result<()> {
        self.check("RENAME", format!("{path} {new_path}"))?;
        let b = self.boxes.remove(path).unwrap_or_default();
        self.boxes.insert(new_path.to_owned(), b);
        Ok(())
    }
    fn delete_folder(&mut self, path: &str) -> airpost_imap::Result<()> {
        self.check("DELETE", path.to_owned())?;
        self.boxes.remove(path);
        Ok(())
    }
    fn fetch_by_uid(
        &mut self,
        path: &str,
        _: FetchKind,
        uids: &UidSet,
    ) -> airpost_imap::Result<FetchResult> {
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
    ) -> airpost_imap::Result<SyncResult> {
        unimplemented!()
    }
    fn take_vanished(&mut self, _: &str) -> UidSet {
        UidSet::new()
    }
    fn fetch_message_bytes(
        &mut self,
        path: &str,
        uid: u32,
    ) -> airpost_imap::Result<Option<Vec<u8>>> {
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
    ) -> airpost_imap::Result<()> {
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
    fn store_keywords(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        keywords: &[String],
    ) -> airpost_imap::Result<()> {
        self.check("STORE", format!("{path} {uids} {op:?} {keywords:?}"))?;
        for m in self.boxes.get_mut(path).unwrap().messages.values_mut() {
            if uids.contains(m.uid) {
                match op {
                    StoreOp::Add => m.keywords.extend(keywords.iter().cloned()),
                    StoreOp::Remove => m.keywords.retain(|k| !keywords.contains(k)),
                    StoreOp::Set => unreachable!("keyword STOREs add or remove"),
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
    ) -> airpost_imap::Result<()> {
        self.check("STORE", format!("{path} {uids} {op:?} {labels:?}"))
    }
    fn move_uids(
        &mut self,
        path: &str,
        uids: &UidSet,
        dest: &str,
    ) -> airpost_imap::Result<UidMapping> {
        self.check("MOVE", format!("{path} {uids} {dest}"))?;
        Ok(self.transfer(path, uids, dest, true))
    }
    fn copy_uids(
        &mut self,
        path: &str,
        uids: &UidSet,
        dest: &str,
    ) -> airpost_imap::Result<UidMapping> {
        self.check("COPY", format!("{path} {uids} {dest}"))?;
        Ok(self.transfer(path, uids, dest, false))
    }
    fn expunge(&mut self, path: &str) -> airpost_imap::Result<()> {
        self.check("EXPUNGE", path.to_owned())?;
        self.boxes
            .get_mut(path)
            .unwrap()
            .messages
            .retain(|_, m| !m.flags.contains(MessageFlags::DELETED));
        Ok(())
    }
    fn expunge_uids(&mut self, path: &str, uids: &UidSet) -> airpost_imap::Result<()> {
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
    ) -> airpost_imap::Result<Option<u32>> {
        self.check("APPEND", path.to_owned())?;
        self.appended.push(data.to_vec());
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
    ) -> airpost_imap::Result<Vec<u32>> {
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
    fn idle(&mut self, _: &str, _: &IdleInterrupt) -> airpost_imap::Result<()> {
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
    /// The server can't be reached.
    offline: Arc<Mutex<bool>>,
}

impl Outbox {
    fn submissions(&self) -> Vec<Submission> {
        self.sent.lock().unwrap().clone()
    }

    fn push(&self, sub: Submission) -> Result<(), MailError> {
        if *self.offline.lock().unwrap() {
            return Err(MailError::new(ErrorCode::Connection, "connection refused"));
        }
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

/// The engine's tests pin the C++ behaviour, so drafts stay local unless a case turns
/// "Save drafts to server" on ([`Env::sync_drafts_to_server`]).
fn account() -> Arc<Account> {
    Arc::new(
        Account::from_json(json!({
            "id": ACCOUNT,
            "provider": "imap",
            "emailAddress": "me@example.com",
            "settings": {"save_drafts_to_server": false},
        }))
        .unwrap(),
    )
}

fn env(caps: &[&str]) -> Env {
    let dir = tempfile::tempdir().unwrap();
    airpost_store::migrate::migrate(dir.path(), &ManualClock::new(T0), &mut Vec::new()).unwrap();
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
        let mut f = Folder::new(&airpost_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
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
        let violations = airpost_store::invariants::check(self.store.conn(), &[]).unwrap();
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
    airpost_core::ids::random_id()
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
    assert_eq!(task.data["reverted"], json!(true));
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
    // The server never took the star, so the local one is taken back at once.
    assert!(!e.message(&id).unwrap().is_starred());
    assert_eq!(task.data["reverted"], json!(true));
    e.assert_invariants();
}

#[test]
fn a_rejected_mark_read_puts_back_exactly_the_messages_it_changed() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let unread = e.deliver(&inbox, 1, MessageFlags::empty());
    let thread_id = e.message(&unread).unwrap().thread_id().to_owned();
    e.imap.reject.push("STORE");

    let mut task = e.queue(
        "ChangeUnreadTask",
        json!({"threadIds": [thread_id], "unread": false}),
    );
    assert_eq!(task.data["undoFlags"], json!({ unread.clone(): true }));
    let thread = |e: &Env| {
        e.store
            .find::<Thread>(&Query::new().equal("id", thread_id.as_str()))
            .unwrap()
            .unwrap()
    };
    assert_eq!(thread(&e).unread(), 0);
    e.remote(&mut task);

    assert_eq!(task.data["error"]["key"], "ErrorStore");
    assert_eq!(task.data["reverted"], json!(true));
    assert!(e.message(&unread).unwrap().is_unread());
    assert_eq!(thread(&e).unread(), 1);
    e.assert_invariants();
}

#[test]
fn a_task_that_reached_the_server_is_not_reverted() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let id = e.deliver(&inbox, 1, MessageFlags::empty());
    let mut task = e.queue(
        "ChangeStarredTask",
        json!({"messageIds": [id], "starred": true}),
    );
    e.remote(&mut task);
    assert_eq!(task.status(), "complete");
    assert!(task.data.get("reverted").is_none());
    assert!(e.message(&id).unwrap().is_starred());
}

#[test]
fn the_engine_built_undo_of_a_move_sends_each_copy_home() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let inbox = e.folder("INBOX", "inbox");
    let sent = e.folder("Sent", "sent");
    let trash = e.folder("Trash", "trash");
    let id = e.deliver(&inbox, 1, MessageFlags::empty());
    e.deliver(&sent, 1, MessageFlags::SEEN);

    let mut task = e.queue(
        "ChangeFolderTask",
        json!({"messageIds": [id], "folder": folder_json(&trash)}),
    );
    e.remote(&mut task);
    assert_eq!(e.copies(&id).len(), 2);
    assert!(e.copies(&id).iter().all(|(path, _, _)| path == "Trash"));
    assert!(airpost_tasks::undo::has_undo(&task.data));

    let undo = airpost_tasks::undo::undo_tasks(&e.store, &task.data, None).unwrap();
    assert_eq!(undo.len(), 1);
    let mut undo = undo.into_iter().next().unwrap();
    assert_eq!(undo.data["sourceFolderIds"], json!([trash.id()]));
    e.processor().perform_local(&mut undo).unwrap();
    e.remote(&mut undo);
    assert_eq!(undo.status(), "complete");
    assert!(undo.data["error"].is_null(), "{}", undo.data["error"]);
    let mut paths: Vec<String> = e.copies(&id).into_iter().map(|(p, _, _)| p).collect();
    paths.sort();
    assert_eq!(paths, ["INBOX", "Sent"]);
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
    let local = airpost_core::ids::draft_id(ACCOUNT, "local@x");
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
    let id = airpost_core::ids::folder_id(ACCOUNT, "Projects/2026");
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
fn renaming_a_folder_keeps_its_id() {
    let mut e = env(&[]);
    let projects = e.folder("Projects", "");
    let child = e.folder("Projects/2026", "");
    let id = e.deliver(&projects, 1, MessageFlags::SEEN);
    let path_of = |e: &Env, id: &str| {
        e.store
            .find::<Folder>(&Query::new().equal("id", id))
            .unwrap()
            .map(|f| f.path().to_owned())
    };

    let mut rename = e.queue(
        "SyncbackCategoryTask",
        json!({"path": "Clients", "existingPath": "Projects"}),
    );
    e.remote(&mut rename);
    assert_eq!(rename.status(), "complete", "{}", rename.data);
    assert_eq!(rename.data["created"]["id"], json!(projects.id()));
    assert_eq!(path_of(&e, projects.id()).as_deref(), Some("Clients"));
    assert_eq!(path_of(&e, child.id()).as_deref(), Some("Clients/2026"));
    let thread = e.thread_of(&id);
    assert_eq!(thread.folders()[0]["id"], json!(projects.id()));
    assert_eq!(thread.folders()[0]["path"], "Clients");
    assert_eq!(thread.folders()[0]["_refs"], 1);
    assert_eq!(e.copies(&id), vec![("Clients".to_owned(), 1, None)]);
    let records = airpost_sync::folder_identity::recorded_renames(&e.store, ACCOUNT).unwrap();
    assert_eq!(
        (records[0].from.as_str(), records[0].to.as_str()),
        ("Projects", "Clients")
    );

    // Found by path, not by the hash of it.
    let mut again = e.queue(
        "SyncbackCategoryTask",
        json!({"path": "Archive/Clients", "existingPath": "Clients"}),
    );
    e.remote(&mut again);
    assert_eq!(again.data["created"]["id"], json!(projects.id()));
    assert_eq!(
        path_of(&e, projects.id()).as_deref(),
        Some("Archive/Clients")
    );

    // A new folder at the old path can't take the id the renamed one kept.
    let mut create = e.queue("SyncbackCategoryTask", json!({"path": "Projects"}));
    e.remote(&mut create);
    let new_id = create.data["created"]["id"].as_str().unwrap().to_owned();
    assert_ne!(new_id, projects.id());
    assert_eq!(path_of(&e, &new_id).as_deref(), Some("Projects"));
    assert_eq!(
        path_of(&e, projects.id()).as_deref(),
        Some("Archive/Clients")
    );
    e.assert_invariants();
}

#[test]
fn a_queued_rename_refresh_waits_for_the_next_run() {
    let mut e = env(&[]);
    let mut projects = e.folder("Projects", "");
    let id = e.deliver(&projects, 1, MessageFlags::SEEN);
    // The path changed and the process quit before threads caught up.
    projects.set_path("Clients");
    e.store
        .transaction("test", |s| {
            s.save(&mut projects)?;
            airpost_sync::folder_identity::queue_refresh(s, ACCOUNT, projects.id(), "Projects")
        })
        .unwrap();
    assert_eq!(e.thread_of(&id).folders()[0]["path"], "Projects");
    airpost_sync::folder_identity::run_pending_refreshes(&e.store, ACCOUNT).unwrap();
    assert_eq!(e.thread_of(&id).folders()[0]["path"], "Clients");
    let key = format!("folderRenameRefresh-{ACCOUNT}");
    assert_eq!(e.store.get_key_value(&key).unwrap().as_deref(), Some("[]"));
    airpost_sync::folder_identity::run_pending_refreshes(&e.store, ACCOUNT).unwrap();
    e.assert_invariants();
}

#[test]
fn a_refused_rename_leaves_no_record() {
    let mut e = env(&[]);
    e.folder("Projects", "");
    e.imap.reject.push("RENAME");
    let mut rename = e.queue(
        "SyncbackCategoryTask",
        json!({"path": "Clients", "existingPath": "Projects"}),
    );
    let _ = processor(&e.store, e.harness()).perform_remote(&mut rename, &mut e.imap);
    assert!(
        airpost_sync::folder_identity::recorded_renames(&e.store, ACCOUNT)
            .unwrap()
            .is_empty()
    );
    let kept = e
        .store
        .find::<Folder>(&Query::new().equal("path", "Projects"))
        .unwrap();
    assert!(kept.is_some());
}

#[test]
fn renaming_a_gmail_label_renames_it_on_messages() {
    let mut e = env(&["X-GM-EXT-1"]);
    e.imap.profile.mailbox_model = airpost_providers::MailboxModelKind::GmailLabels;
    let all = e.folder("[Gmail]/All Mail", "all");
    let mut work =
        airpost_store::Label::new(&airpost_core::ids::folder_id(ACCOUNT, "Work"), ACCOUNT, 0);
    work.set_path("Work");
    e.store.save(&mut work).unwrap();
    let mut tagged = remote(1, MessageFlags::SEEN);
    tagged.gmail_labels = vec!["Work".into(), "\\Important".into()];
    let uid = e.imap.add(all.path(), tagged);
    let fetched = e.imap.boxes[all.path()].messages[&uid].clone();
    let mut mp = MailProcessor::new(&e.account, &e.store);
    mp.is_gmail = true;
    mp.insert_fallback_to_update_message(&fetched, &all, T0)
        .unwrap();
    let id = id_for_message(ACCOUNT, all.path(), &fetched);
    assert_eq!(e.thread_of(&id).labels()[0]["path"], "Work");

    let mut rename = e.queue(
        "SyncbackCategoryTask",
        json!({"path": "Clients", "existingPath": "Work"}),
    );
    e.remote(&mut rename);
    assert_eq!(rename.status(), "complete", "{}", rename.data);
    assert_eq!(rename.data["created"]["id"], json!(work.id()));
    let placements = e.store.placements_for_message(&id).unwrap();
    assert_eq!(placements[0].labels, vec!["Clients", "\\Important"]);
    assert_eq!(
        e.message(&id).unwrap().labels(),
        &json!(["Clients", "\\Important"])
    );
    let thread = e.thread_of(&id);
    let labels = thread.labels().as_array().unwrap();
    assert_eq!(labels.len(), 1, "{labels:?}");
    assert_eq!(labels[0]["id"], json!(work.id()));
    assert_eq!(labels[0]["path"], "Clients");
    assert_eq!(labels[0]["_refs"], 1);
    assert!(labels[0].get("localStatus").is_none());
    e.assert_invariants();
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

#[test]
fn get_message_rfc2822_saves_a_file_without_a_filepath() {
    let mut e = env(&[]);
    let inbox = e.folder("INBOX", "inbox");
    let id = e.deliver(&inbox, 1, MessageFlags::SEEN);
    // The engine picks the id; one the client sends is replaced.
    let mut task = e.queue(
        "GetMessageRFC2822Task",
        json!({"messageId": id, "fileId": "victim-file"}),
    );
    let file_id = task.data["fileId"].as_str().unwrap().to_owned();
    assert_ne!(file_id, "victim-file");
    assert_eq!(file_id.len(), 40);
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    let file = e
        .store
        .find::<File>(&Query::new().equal("id", &file_id))
        .unwrap()
        .unwrap();
    assert_eq!(file.filename(), "Subject 1.eml");
    assert_eq!(file.content_type(), "message/rfc822");
    assert_eq!(file.message_id(), id);
    let path = file.find_in(&e._dir.path().join("files")).unwrap();
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(file.size() as usize, bytes.len());
    assert!(String::from_utf8_lossy(&bytes).contains("Subject: Subject 1"));

    // With a filepath it writes there and makes no File.
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("one.eml");
    let mut task = e.queue(
        "GetMessageRFC2822Task",
        json!({"messageId": id, "filepath": target.to_str().unwrap()}),
    );
    assert!(task.data.get("fileId").is_none());
    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(target.is_file());
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
        "X-Mailer: Airpost",
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

#[test]
fn a_send_stopped_during_smtp_drops_the_draft_once_sent_shows_the_message() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (sent, drafts, _) = e.mail_folders();
    let draft = e.deliver(&drafts, 7, MessageFlags::DRAFT);
    let hmid = "stopped-in-smtp@example.test";
    let mut task = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json(hmid, json!({"id": draft})), "perRecipientBodies": null}),
    );
    // Stopped after the server took the message, before `_smtpDone` was saved; the
    // provider filed its copy.
    task.data["_performRemoteRan"] = json!(true);
    e.store.save(&mut task).unwrap();
    let mut copy = remote(9, MessageFlags::SEEN);
    copy.header.as_mut().unwrap().message_id = Some(hmid.to_owned());
    e.imap.add(sent.path(), copy);

    e.remote(&mut task);
    assert_eq!(task.status(), "complete", "{}", task.data);
    assert!(e.message(&draft).is_none());
    assert!(
        e.imap.uids("Drafts").is_empty(),
        "its server copy is gone too"
    );
    assert_eq!(e.imap.uids("Sent").len(), 1, "the Sent copy stays");
    assert!(e.imap.outbox.submissions().is_empty());
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
    let parsed = airpost_mime::parse_message(&subs[0].raw).unwrap();
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
    let parsed = airpost_mime::parse_message(&subs[0].raw).unwrap();
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

// -- The expiration executor --------------------------------------------------------------

struct TestSettings(Value);

impl EngineSettings for TestSettings {
    fn get(&self, key: &str) -> Option<Value> {
        self.0.get(key).cloned()
    }
}

/// Bodies the executor asked the foreground worker for.
type Fetched = Arc<Mutex<Vec<String>>>;

impl Env {
    fn executor(&self, settings: Value) -> (Executor<'_>, Fetched) {
        let fetched = Fetched::default();
        let into = fetched.clone();
        let processor = self.processor();
        let executor = Executor::new(
            self.account.clone(),
            &self.store,
            self._dir.path(),
            Arc::new(TestSettings(settings)),
            Box::new(move |task| processor.perform_local(task)),
            Box::new(move |ids| into.lock().unwrap().extend(ids)),
            Box::new(|| {}),
        );
        (executor, fetched)
    }

    fn handle(&self, class: &str, id: &str, plugin: &str, now: i64) -> Outcome {
        self.executor(json!({}))
            .0
            .handle(class, id, plugin, now)
            .unwrap()
    }

    /// A draft saved as the composer saves one, with `send-later` metadata. Returns its id.
    fn scheduled_draft(&self, header_message_id: &str, extra: Value, value: Value) -> String {
        self.queue(
            "SyncbackDraftTask",
            json!({"draft": draft_json(header_message_id, extra)}),
        );
        let id = self
            .store
            .find::<Message>(&Query::new().equal("headerMessageId", header_message_id))
            .unwrap()
            .unwrap()
            .id()
            .to_owned();
        self.queue(
            "SyncbackMetadataTask",
            json!({
                "modelId": id, "modelClassName": "message",
                "modelHeaderMessageId": header_message_id, "pluginId": "send-later",
                "value": value,
            }),
        );
        id
    }

    fn metadata_of<M: Model>(&self, id: &str, plugin: &str) -> Value {
        let model = self
            .store
            .find::<M>(&Query::new().equal("id", id))
            .unwrap()
            .unwrap();
        airpost_store::model::metadata_value(model.json(), plugin)
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn thread_of(&self, message_id: &str) -> Thread {
        let m = self.message(message_id).unwrap();
        self.store
            .find::<Thread>(&Query::new().equal("id", m.thread_id()))
            .unwrap()
            .unwrap()
    }

    fn set_thread_metadata(&self, thread_id: &str, plugin: &str, value: Value) {
        self.queue(
            "SyncbackMetadataTask",
            json!({"modelId": thread_id, "modelClassName": "thread", "pluginId": plugin, "value": value}),
        );
    }
}

#[test]
fn executor_sends_a_due_draft_once_and_clears_its_schedule() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (_, _, archive) = e.mail_folders();
    let inbox = e
        .store
        .find::<Folder>(&Query::new().equal("role", "inbox"))
        .unwrap()
        .unwrap();
    let parent = e.deliver(&inbox, 1, MessageFlags::empty());
    let thread_id = e.message(&parent).unwrap().thread_id().to_owned();
    let id = e.scheduled_draft(
        "later-1@example.test",
        json!({"threadId": thread_id, "body": "<p>Hi\u{200B}</p><img src=\"cid:kept\">", "files": [
            {"id": "f-kept", "aid": ACCOUNT, "v": 1, "__cls": "File", "filename": "a.png", "contentId": "kept"},
            {"id": "f-gone", "aid": ACCOUNT, "v": 1, "__cls": "File", "filename": "b.png", "contentId": "gone"},
        ]}),
        json!({"expiration": T0 + 10, "actionKey": "send-and-archive"}),
    );
    let dir = e._dir.path().join("files/f-/ke/f-kept");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.png"), "PNG").unwrap();

    assert_eq!(
        e.handle("Message", &id, "send-later", T0 + 10),
        Outcome::Done
    );
    let value = e.metadata_of::<Message>(&id, "send-later");
    assert!(value["expiration"].is_null(), "{value}");
    let sends = e.tasks_of("SendDraftTask");
    assert_eq!(sends.len(), 1);
    let send = &sends[0];
    assert_eq!(send.status(), "remote");
    assert_eq!(send.data["silent"], false);
    assert_eq!(send.data["retryWhenOnline"], true);
    let files: Vec<&str> = send.data["draft"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["f-kept"], "the unreferenced inline file is pruned");
    let archive_task = &e.tasks_of("ChangeFolderTask")[0];
    assert_eq!(archive_task.data["folder"]["id"], archive.id());
    assert_eq!(archive_task.data["threadIds"], json!([thread_id]));

    let mut send = send.clone();
    e.remote(&mut send);
    assert!(send.data["error"].is_null(), "{}", send.data);
    assert_eq!(e.imap.outbox.submissions().len(), 1);
    // A second look finds nothing due: the row is gone with the expiration.
    assert_eq!(
        e.store
            .conn()
            .query_row(
                "SELECT count(*) FROM ModelPluginMetadata WHERE expiration IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn executor_leaves_client_finalized_and_unknown_actions_to_the_client() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let finalize = e.scheduled_draft(
        "client-1@example.test",
        json!({}),
        json!({"expiration": T0, "clientFinalize": true}),
    );
    let custom = e.scheduled_draft(
        "client-2@example.test",
        json!({}),
        json!({"expiration": T0, "actionKey": "send-and-snooze"}),
    );
    assert_eq!(
        e.handle("Message", &finalize, "send-later", T0),
        Outcome::Declined
    );
    assert_eq!(
        e.handle("Message", &custom, "send-later", T0),
        Outcome::Declined
    );
    assert_eq!(
        e.handle("Message", &custom, "open-tracking", T0),
        Outcome::Declined
    );
    assert!(e.tasks_of("SendDraftTask").is_empty());
}

#[test]
fn executor_fails_a_draft_whose_from_is_not_the_account() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let id = e.scheduled_draft(
        "stranger-1@example.test",
        json!({"from": [{"email": "someone@else.example"}]}),
        json!({"expiration": T0, "actionKey": "send"}),
    );
    assert_eq!(e.handle("Message", &id, "send-later", T0), Outcome::Done);
    let value = e.metadata_of::<Message>(&id, "send-later");
    assert!(value["expiration"].is_null());
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("someone@else.example"),
        "{value}"
    );
    assert!(e.tasks_of("SendDraftTask").is_empty());
}

/// A due row the scan read before a cancel (or a send) cleared it isn't sent, and one moved
/// later waits.
#[test]
fn executor_rereads_the_schedule_before_sending() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let id = e.scheduled_draft(
        "cancelled-1@example.test",
        json!({}),
        json!({"expiration": T0, "isUndoSend": true, "actionKey": "send"}),
    );
    e.queue(
        "SyncbackMetadataTask",
        json!({"modelId": id, "modelClassName": "message", "pluginId": "send-later",
               "value": {"expiration": null, "isUndoSend": true}}),
    );
    assert_eq!(e.handle("Message", &id, "send-later", T0), Outcome::Done);
    let later = e.scheduled_draft(
        "later-2@example.test",
        json!({}),
        json!({"expiration": T0 + 60, "actionKey": "send"}),
    );
    assert_eq!(
        e.handle("Message", &later, "send-later", T0),
        Outcome::Waiting(T0 + 60)
    );
    assert!(e.tasks_of("SendDraftTask").is_empty());
}

/// "Send now" in the Undo Send toast moves the window earlier and keeps `isUndoSend`: the
/// bodies draft.send kept still go out. A plain send-later moved earlier doesn't take them.
#[test]
fn executor_keeps_draft_send_bodies_when_send_now_moves_the_window() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let pending = PendingSends::new(e._dir.path(), ACCOUNT);
    let bodies = PendingBodies {
        expiration: T0 + 5,
        self_body: Some("<p>sent copy</p>".into()),
        per_recipient: [
            ("bob@example.net".to_owned(), "<p>for bob</p>".to_owned()),
            ("boss@example.net".to_owned(), "<p>for boss</p>".to_owned()),
        ]
        .into(),
    };
    let now = e.scheduled_draft(
        "now-1@example.test",
        json!({}),
        json!({"expiration": T0, "isUndoSend": true, "actionKey": "send"}),
    );
    pending.put(&now, &bodies).unwrap();
    let plain = e.scheduled_draft(
        "now-2@example.test",
        json!({}),
        json!({"expiration": T0, "actionKey": "send"}),
    );
    pending.put(&plain, &bodies).unwrap();
    assert_eq!(e.handle("Message", &now, "send-later", T0), Outcome::Done);
    assert_eq!(e.handle("Message", &plain, "send-later", T0), Outcome::Done);
    let sends = e.tasks_of("SendDraftTask");
    let of = |hmid: &str| {
        sends
            .iter()
            .find(|t| t.data["headerMessageId"] == hmid)
            .unwrap()
            .data
            .clone()
    };
    let kept = of("now-1@example.test");
    assert_eq!(kept["draft"]["body"], "<p>sent copy</p>");
    assert_eq!(
        kept["perRecipientBodies"],
        json!({"self": "<p>sent copy</p>", "bob@example.net": "<p>for bob</p>", "boss@example.net": "<p>for boss</p>"})
    );
    let stored = of("now-2@example.test");
    assert_eq!(stored["draft"]["body"], "<p>Hi Bob</p>");
    assert!(stored["perRecipientBodies"].is_null());
    assert!(pending.ids().is_empty());
}

/// Bodies kept for recipients the draft no longer has fail the send instead of skipping one.
#[test]
fn executor_fails_a_send_whose_kept_bodies_miss_a_recipient() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let id = e.scheduled_draft(
        "changed-1@example.test",
        json!({}),
        json!({"expiration": T0, "isUndoSend": true, "actionKey": "send"}),
    );
    PendingSends::new(e._dir.path(), ACCOUNT)
        .put(
            &id,
            &PendingBodies {
                expiration: T0,
                self_body: None,
                per_recipient: [("bob@example.net".to_owned(), "<p>b</p>".to_owned())].into(),
            },
        )
        .unwrap();
    assert_eq!(e.handle("Message", &id, "send-later", T0), Outcome::Done);
    let value = e.metadata_of::<Message>(&id, "send-later");
    assert!(value["expiration"].is_null(), "{value}");
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("boss@example.net"),
        "{value}"
    );
    assert!(e.tasks_of("SendDraftTask").is_empty());
}

#[test]
fn executor_waits_for_an_incomplete_draft_then_fails_it() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (_, drafts, _) = e.mail_folders();
    // A draft synced from another device: headers only, no body yet.
    let id = e.deliver(&drafts, 7, MessageFlags::DRAFT);
    let mut draft = e.message(&id).unwrap();
    draft.data["from"] = json!([{"email": "me@example.com"}]);
    draft.upsert_metadata(
        "send-later",
        &json!({"expiration": T0, "actionKey": "send"}),
        None,
    );
    e.store.save(&mut draft).unwrap();

    let (executor, fetched) = e.executor(json!({}));
    let mut now = T0;
    for wait in [60, 120, 240, 480] {
        assert_eq!(
            executor.handle("Message", &id, "send-later", now).unwrap(),
            Outcome::Waiting(now + wait)
        );
        assert_eq!(
            executor
                .handle("Message", &id, "send-later", now + 1)
                .unwrap(),
            Outcome::Waiting(now + wait),
            "no second fetch before the look is due"
        );
        now += wait;
    }
    assert_eq!(fetched.lock().unwrap().len(), 4);
    assert_eq!(
        executor.handle("Message", &id, "send-later", now).unwrap(),
        Outcome::Done
    );
    drop(executor);
    let value = e.metadata_of::<Message>(&id, "send-later");
    assert!(value["error"].as_str().unwrap().contains("body"), "{value}");
    assert!(e.tasks_of("SendDraftTask").is_empty());
}

#[test]
fn executor_body_fetch_completes_a_waiting_draft() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (_, drafts, _) = e.mail_folders();
    let id = e.deliver(&drafts, 8, MessageFlags::DRAFT);
    let mut draft = e.message(&id).unwrap();
    draft.data["from"] = json!([{"email": "me@example.com"}]);
    draft.upsert_metadata(
        "send-later",
        &json!({"expiration": T0, "actionKey": "send"}),
        None,
    );
    e.store.save(&mut draft).unwrap();
    let (executor, _) = e.executor(json!({}));
    assert_eq!(
        executor.handle("Message", &id, "send-later", T0).unwrap(),
        Outcome::Waiting(T0 + 60)
    );
    // The body arrives.
    let mut draft = e.message(&id).unwrap();
    draft.data["plaintext"] = json!(false);
    e.store.save(&mut draft).unwrap();
    e.store
        .conn()
        .execute(
            "REPLACE INTO MessageBody (id, value) VALUES (?, '<p>Body</p>')",
            [&id],
        )
        .unwrap();
    assert_eq!(
        executor
            .handle("Message", &id, "send-later", T0 + 60)
            .unwrap(),
        Outcome::Done
    );
    drop(executor);
    assert_eq!(e.tasks_of("SendDraftTask").len(), 1);
}

#[test]
fn executor_clears_send_later_on_a_message_that_is_no_longer_a_draft() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (sent, _, _) = e.mail_folders();
    let id = e.deliver(&sent, 9, MessageFlags::SEEN);
    let mut m = e.message(&id).unwrap();
    m.upsert_metadata("send-later", &json!({"expiration": T0}), None);
    e.store.save(&mut m).unwrap();
    assert_eq!(e.handle("Message", &id, "send-later", T0), Outcome::Done);
    assert!(e.metadata_of::<Message>(&id, "send-later")["expiration"].is_null());
    assert!(e.tasks_of("SendDraftTask").is_empty());
}

#[test]
fn executor_unsnoozes_to_the_inbox_and_resurfaces() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let (_, _, _) = e.mail_folders();
    let snoozed = e.folder("Snoozed", "snoozed");
    let inbox = e
        .store
        .find::<Folder>(&Query::new().equal("role", "inbox"))
        .unwrap()
        .unwrap();
    let id = e.deliver(&snoozed, 3, MessageFlags::SEEN);
    let thread = e.thread_of(&id);
    e.set_thread_metadata(thread.id(), "thread-snooze", json!({"expiration": T0}));

    let (executor, _) = e.executor(json!({"resurfacing.snoozeBody": "<p>Back on top</p>"}));
    assert_eq!(
        executor
            .handle("Thread", thread.id(), "thread-snooze", T0)
            .unwrap(),
        Outcome::Done
    );
    drop(executor);
    assert!(e.metadata_of::<Thread>(thread.id(), "thread-snooze")["expiration"].is_null());
    let moves = e.tasks_of("ChangeFolderTask");
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].data["folder"]["id"], inbox.id());
    assert_eq!(moves[0].data["sourceFolderIds"], json!([snoozed.id()]));
    let sends = e.tasks_of("SendDraftTask");
    assert_eq!(sends.len(), 1);
    let draft = &sends[0].data["draft"];
    assert_eq!(sends[0].data["silent"], true);
    assert_eq!(draft["from"][0]["name"], "me@example.com via Airpost");
    assert_eq!(draft["to"][0]["email"], "me@example.com");
    assert_eq!(draft["rthMsgId"], "m3@x");
    assert_eq!(draft["subject"], thread.subject());
    assert_eq!(draft["body"], "<p>Back on top</p>");
    assert!(e.tasks_of("ChangeUnreadTask").is_empty());

    // On the wire: from the account to itself, in reply to the thread's newest message.
    let mut send = sends[0].clone();
    e.remote(&mut send);
    assert!(send.data["error"].is_null(), "{}", send.data);
    let wire = e.imap.outbox.submissions()[0].text();
    for h in [
        "via Airpost\" <me@example.com>",
        "To: <me@example.com>",
        "In-Reply-To: <m3@x>",
    ] {
        assert!(wire.contains(h), "{h} missing from\n{wire}");
    }
    assert!(airpost_store::models::is_hidden_json(draft));
}

#[test]
fn executor_unsnooze_without_resurfacing_marks_unread() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let snoozed = e.folder("Snoozed", "snoozed");
    let id = e.deliver(&snoozed, 4, MessageFlags::SEEN);
    let thread = e.thread_of(&id);
    e.set_thread_metadata(thread.id(), "thread-snooze", json!({"expiration": T0}));
    let (executor, _) = e.executor(json!({"core.notifications.unsnoozeToTop": false}));
    assert_eq!(
        executor
            .handle("Thread", thread.id(), "thread-snooze", T0)
            .unwrap(),
        Outcome::Done
    );
    drop(executor);
    assert_eq!(e.tasks_of("ChangeFolderTask").len(), 1);
    assert!(e.tasks_of("SendDraftTask").is_empty());
    let unread = e.tasks_of("ChangeUnreadTask");
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].data["unread"], true);
}

#[test]
fn executor_swaps_the_snoozed_label_for_the_inbox_on_gmail() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    let all = e.folder("[Gmail]/All Mail", "all");
    let label = |path: &str, role: &str| {
        let mut l =
            airpost_store::Label::new(&airpost_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
        l.set_path(path);
        l.set_role(role);
        l
    };
    let mut inbox = label("INBOX", "inbox");
    let mut snoozed = label("[Mailspring]/Snoozed", "snoozed");
    e.store.save(&mut inbox).unwrap();
    e.store.save(&mut snoozed).unwrap();
    let id = e.deliver(&all, 5, MessageFlags::SEEN);
    let mut thread = e.thread_of(&id);
    thread.data["labels"] = json!([snoozed.to_json()]);
    e.store.save(&mut thread).unwrap();
    e.set_thread_metadata(thread.id(), "thread-snooze", json!({"expiration": T0}));
    assert_eq!(
        e.handle("Thread", thread.id(), "thread-snooze", T0),
        Outcome::Done
    );
    let changes = e.tasks_of("ChangeLabelsTask");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].data["labelsToAdd"][0]["id"], inbox.id());
    assert_eq!(changes[0].data["labelsToRemove"][0]["id"], snoozed.id());
    assert!(e.tasks_of("ChangeFolderTask").is_empty());
}

#[test]
fn executor_clears_a_snooze_on_a_thread_that_left_snoozed() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    e.folder("Snoozed", "snoozed");
    let inbox = e
        .store
        .find::<Folder>(&Query::new().equal("role", "inbox"))
        .unwrap()
        .unwrap();
    let id = e.deliver(&inbox, 6, MessageFlags::SEEN);
    let thread = e.thread_of(&id);
    e.set_thread_metadata(thread.id(), "thread-snooze", json!({"expiration": T0}));
    assert_eq!(
        e.handle("Thread", thread.id(), "thread-snooze", T0),
        Outcome::Done
    );
    assert!(e.metadata_of::<Thread>(thread.id(), "thread-snooze")["expiration"].is_null());
    assert!(e.tasks_of("ChangeFolderTask").is_empty());
    assert!(e.tasks_of("SendDraftTask").is_empty());
}

#[test]
fn executor_reminds_without_a_reply_and_a_reply_clears_the_reminder() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let inbox = e
        .store
        .find::<Folder>(&Query::new().equal("role", "inbox"))
        .unwrap()
        .unwrap();
    let (quiet, answered) = (
        e.deliver(&inbox, 10, MessageFlags::SEEN),
        e.deliver(&inbox, 11, MessageFlags::SEEN),
    );
    let (quiet, answered) = (e.thread_of(&quiet), e.thread_of(&answered));
    for t in [&quiet, &answered] {
        e.set_thread_metadata(
            t.id(),
            "send-reminders",
            json!({"expiration": T0 + 100, "sentHeaderMessageId": "mine@example.com"}),
        );
        let value = e.metadata_of::<Thread>(t.id(), "send-reminders");
        assert_eq!(
            value["lastReplyTimestamp"],
            t.last_message_received_timestamp(),
            "baseline"
        );
        assert_eq!(value["shouldNotify"], false);
    }
    // A reply moves the answered thread's lastMessageReceivedTimestamp: the save clears it.
    let mut t = e
        .store
        .find::<Thread>(&Query::new().equal("id", answered.id()))
        .unwrap()
        .unwrap();
    t.data["lmrt"] = json!(t.last_message_received_timestamp() + 50);
    e.store.save(&mut t).unwrap();
    assert_eq!(
        e.metadata_of::<Thread>(answered.id(), "send-reminders"),
        json!({})
    );

    let (executor, _) = e.executor(json!({}));
    assert_eq!(
        executor
            .handle("Thread", quiet.id(), "send-reminders", T0 + 100)
            .unwrap(),
        Outcome::Done
    );
    drop(executor);
    let value = e.metadata_of::<Thread>(quiet.id(), "send-reminders");
    assert!(value["expiration"].is_null(), "{value}");
    assert_eq!(value["shouldNotify"], true);
    let sends = e.tasks_of("SendDraftTask");
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].data["draft"]["rthMsgId"], "mine@example.com");
    assert!(sends[0].data["draft"]["body"]
        .as_str()
        .unwrap()
        .contains("no one has replied"));
}

#[test]
fn engine_sends_wait_out_an_unreachable_server() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    *e.imap.outbox.offline.lock().unwrap() = true;
    let mut deferred = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("offline-1@example.test", json!({})), "perRecipientBodies": null, "retryWhenOnline": true}),
    );
    let mut failed = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("offline-2@example.test", json!({})), "perRecipientBodies": null}),
    );
    let err = processor(&e.store, e.harness())
        .perform_remote(&mut deferred, &mut e.imap)
        .unwrap_err();
    assert!(err.is_retryable(), "{err}");
    assert_eq!(deferred.status(), "remote");
    assert!(deferred.data.get("_performRemoteRan").is_none());
    assert_eq!(deferred.data["_offlineSince"], T0);
    // A client's own send fails as before, now marked retryable.
    e.remote(&mut failed);
    assert_eq!(failed.status(), "complete");
    assert_eq!(failed.data["error"]["key"], "send-failed");
    assert_eq!(failed.data["error"]["retryable"], true);

    *e.imap.outbox.offline.lock().unwrap() = false;
    e.remote(&mut deferred);
    assert_eq!(deferred.status(), "complete");
    assert!(deferred.data["error"].is_null(), "{}", deferred.data);
    assert_eq!(e.imap.outbox.submissions().len(), 1);

    // After a day of failures the send fails like any other.
    *e.imap.outbox.offline.lock().unwrap() = true;
    let mut old = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("offline-3@example.test", json!({})), "perRecipientBodies": null,
               "retryWhenOnline": true, "_offlineSince": T0 - 24 * 60 * 60}),
    );
    e.remote(&mut old);
    assert_eq!(old.status(), "complete");
    assert_eq!(old.data["error"]["key"], "send-failed");
}

#[test]
fn expiration_scan_runs_the_executor_on_due_rows_only() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let snoozed = e.folder("Snoozed", "snoozed");
    let (a, b) = (
        e.deliver(&snoozed, 20, MessageFlags::SEEN),
        e.deliver(&snoozed, 21, MessageFlags::SEEN),
    );
    let (a, b) = (e.thread_of(&a), e.thread_of(&b));
    e.set_thread_metadata(a.id(), "thread-snooze", json!({"expiration": T0}));
    e.set_thread_metadata(b.id(), "thread-snooze", json!({"expiration": T0 + 3600}));
    let (executor, _) = e.executor(json!({"core.notifications.unsnoozeToTop": false}));
    let clock = Arc::new(ManualClock::new(T0));
    let worker = airpost_sync::metadata_expiration::MetadataExpirationWorker::new(
        ACCOUNT,
        &e.store,
        DeltaStream::new(Box::new(std::io::sink())),
        clock,
    )
    .with_executor(executor);
    assert_eq!(
        worker.scan().unwrap(),
        T0 + 3600,
        "wakes for the other snooze"
    );
    drop(worker);
    let moves = e.tasks_of("ChangeFolderTask");
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].data["threadIds"], json!([a.id()]));
    assert!(e.metadata_of::<Thread>(b.id(), "thread-snooze")["expiration"].is_number());
}

// -- threads.snooze, threads.unsnooze, messages.unsubscribe by mail ------------------------

mod mail_actions {
    use super::*;
    use airpost_tasks::mail_actions::{
        snooze_tasks, unsnooze_tasks, unsubscribe_send_task, ActionError,
    };

    fn run_local(e: &Env, tasks: Vec<Task>) -> Vec<Task> {
        tasks
            .into_iter()
            .map(|mut t| {
                e.processor().perform_local(&mut t).unwrap();
                assert!(t.data["error"].is_null(), "{}", t.data);
                t
            })
            .collect()
    }

    #[test]
    fn snooze_moves_the_inbox_copy_and_unsnooze_brings_it_back() {
        let mut e = env(&["MOVE", "UIDPLUS"]);
        e.mail_folders();
        let snoozed = e.folder("Snoozed", "snoozed");
        let inbox = e
            .store
            .find::<Folder>(&Query::new().equal("role", "inbox"))
            .unwrap()
            .unwrap();
        let a = e.deliver(&inbox, 20, MessageFlags::SEEN);
        let b = e.deliver(&inbox, 21, MessageFlags::SEEN);
        let (ta, tb) = (e.thread_of(&a), e.thread_of(&b));
        let ids = vec![ta.id().to_owned(), tb.id().to_owned(), ta.id().to_owned()];

        let tasks = snooze_tasks(&e.store, ACCOUNT, &ids, T0 + 3600).unwrap();
        let classes: Vec<&str> = tasks.iter().map(Task::class).collect();
        assert_eq!(
            classes,
            [
                "ChangeFolderTask",
                "SyncbackMetadataTask",
                "SyncbackMetadataTask"
            ]
        );
        assert_eq!(tasks[0].data["folder"]["id"], snoozed.id());
        assert_eq!(tasks[0].data["sourceFolderIds"], json!([inbox.id()]));
        assert_eq!(tasks[0].data["threadIds"], json!([ta.id(), tb.id()]));
        assert_eq!(tasks[0].data["canBeUndone"], true);
        let mut tasks = run_local(&e, tasks);
        e.remote(&mut tasks[0]);
        for t in [&ta, &tb] {
            let value = e.metadata_of::<Thread>(t.id(), "thread-snooze");
            assert_eq!(value["expiration"], T0 + 3600);
        }
        assert_eq!(e.copies(&a)[0].0, "Snoozed");
        e.assert_invariants();

        // Only the thread still in Snoozed moves back; both lose the snooze.
        let ta = e.thread_of(&a);
        let c = e.deliver(&inbox, 25, MessageFlags::SEEN);
        let moved_out = e.thread_of(&c);
        let ids = vec![ta.id().to_owned(), moved_out.id().to_owned()];
        let tasks = unsnooze_tasks(&e.store, ACCOUNT, &ids).unwrap();
        let classes: Vec<&str> = tasks.iter().map(Task::class).collect();
        assert_eq!(
            classes,
            [
                "ChangeFolderTask",
                "SyncbackMetadataTask",
                "SyncbackMetadataTask"
            ]
        );
        assert_eq!(tasks[0].data["threadIds"], json!([ta.id()]));
        assert_eq!(tasks[0].data["folder"]["id"], inbox.id());
        assert_eq!(tasks[0].data["sourceFolderIds"], json!([snoozed.id()]));
        assert_eq!(tasks[0].data["canBeUndone"], false);
        let mut tasks = run_local(&e, tasks);
        e.remote(&mut tasks[0]);
        assert_eq!(e.copies(&a)[0].0, "INBOX");
        for t in [&ta, &moved_out] {
            assert!(e.metadata_of::<Thread>(t.id(), "thread-snooze")["expiration"].is_null());
        }
        e.assert_invariants();
    }

    #[test]
    fn gmail_swaps_labels() {
        let mut e = env(&["MOVE", "UIDPLUS"]);
        let all = e.folder("[Gmail]/All Mail", "all");
        let label = |path: &str, role: &str| {
            let mut l =
                airpost_store::Label::new(&airpost_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
            l.set_path(path);
            l.set_role(role);
            l
        };
        let mut inbox = label("INBOX", "inbox");
        let mut snoozed = label("[Mailspring]/Snoozed", "snoozed");
        e.store.save(&mut inbox).unwrap();
        e.store.save(&mut snoozed).unwrap();
        let id = e.deliver(&all, 22, MessageFlags::SEEN);
        let mut thread = e.thread_of(&id);
        let ids = vec![thread.id().to_owned()];
        let tasks = snooze_tasks(&e.store, ACCOUNT, &ids, T0 + 60).unwrap();
        assert_eq!(tasks[0].class(), "ChangeLabelsTask");
        assert_eq!(tasks[0].data["labelsToAdd"][0]["id"], snoozed.id());
        assert_eq!(tasks[0].data["labelsToRemove"][0]["id"], inbox.id());

        thread.data["labels"] = json!([snoozed.to_json()]);
        e.store.save(&mut thread).unwrap();
        let tasks = unsnooze_tasks(&e.store, ACCOUNT, &ids).unwrap();
        assert_eq!(tasks[0].class(), "ChangeLabelsTask");
        assert_eq!(tasks[0].data["labelsToAdd"][0]["id"], inbox.id());
        assert_eq!(tasks[0].data["labelsToRemove"][0]["id"], snoozed.id());
    }

    #[test]
    fn refusals() {
        let mut e = env(&["MOVE", "UIDPLUS"]);
        e.mail_folders();
        let inbox = e
            .store
            .find::<Folder>(&Query::new().equal("role", "inbox"))
            .unwrap()
            .unwrap();
        let id = e.deliver(&inbox, 23, MessageFlags::SEEN);
        let thread = e.thread_of(&id).id().to_owned();
        // No Snoozed folder yet.
        let r = snooze_tasks(&e.store, ACCOUNT, std::slice::from_ref(&thread), T0 + 60);
        assert!(matches!(r, Err(ActionError::Unavailable(_))), "{r:?}");
        e.folder("Snoozed", "snoozed");
        let r = snooze_tasks(&e.store, ACCOUNT, &[thread.clone(), "nope".into()], T0 + 60);
        assert!(
            matches!(r, Err(ActionError::NotFound(ref s)) if s.contains("nope")),
            "{r:?}"
        );
        let r = snooze_tasks(&e.store, "other", std::slice::from_ref(&thread), T0 + 60);
        assert!(matches!(r, Err(ActionError::NotFound(_))), "{r:?}");
        assert!(matches!(
            snooze_tasks(&e.store, ACCOUNT, &[], T0),
            Err(ActionError::NotFound(_))
        ));
        // A thread that isn't snoozed only has its (absent) snooze cleared.
        let tasks = unsnooze_tasks(&e.store, ACCOUNT, std::slice::from_ref(&thread)).unwrap();
        let classes: Vec<&str> = tasks.iter().map(Task::class).collect();
        assert_eq!(classes, ["SyncbackMetadataTask"]);
    }

    #[test]
    fn unsubscribe_mail_goes_from_the_address_the_list_wrote_to() {
        let mut e = env(&["MOVE", "UIDPLUS"]);
        e.mail_folders();
        let inbox = e
            .store
            .find::<Folder>(&Query::new().equal("role", "inbox"))
            .unwrap()
            .unwrap();
        let id = e.deliver(&inbox, 24, MessageFlags::SEEN);
        let mut message = e.message(&id).unwrap();
        message.data["to"] = json!([{"name": "", "email": "Lists@Example.com"}]);
        e.store.save(&mut message).unwrap();
        let account = Account::from_json(json!({
            "id": ACCOUNT, "provider": "imap", "emailAddress": "me@example.com", "name": "Me",
            "aliases": ["Lists Me <lists@example.com>", "other@example.com"], "settings": {},
        }))
        .unwrap();
        let mailto = airpost_net::unsubscribe::parse_mailto(
            "mailto:leave@lists.example.net?subject=remove%20me",
        )
        .unwrap();
        let task = unsubscribe_send_task(&e.store, &account, &id, &mailto).unwrap();
        assert_eq!(task.class(), "SendDraftTask");
        let draft = &task.data["draft"];
        assert_eq!(
            draft["from"],
            json!([{"name": "Lists Me", "email": "lists@example.com"}])
        );
        assert_eq!(draft["to"][0]["email"], "leave@lists.example.net");
        assert_eq!(draft["subject"], "remove me");
        assert_eq!(draft["body"], "unsubscribe");
        assert!(draft["hMsgId"].as_str().unwrap().ends_with("@example.com"));
        assert_eq!(task.data["retryWhenOnline"], true);

        let mut task = run_local(&e, vec![task]).remove(0);
        e.remote(&mut task);
        assert!(task.data["error"].is_null(), "{}", task.data);
        let wire = e.imap.outbox.submissions()[0].text();
        for h in [
            "From: \"Lists Me\" <lists@example.com>",
            "To: <leave@lists.example.net>",
            "Subject: remove me",
            "text/plain",
        ] {
            assert!(wire.contains(h), "{h} missing from\n{wire}");
        }

        // Mail to the account's own address goes from it; an unknown message is refused.
        message.data["to"] = json!([{"name": "", "email": "someone@else.example"}]);
        e.store.save(&mut message).unwrap();
        let task = unsubscribe_send_task(&e.store, &account, &id, &mailto).unwrap();
        assert_eq!(task.data["draft"]["from"][0]["email"], "me@example.com");
        assert!(matches!(
            unsubscribe_send_task(&e.store, &account, "missing", &mailto),
            Err(ActionError::NotFound(_))
        ));
    }
}

// -- Tracking and claims with a Airpost server (docs/backend-plan.md 3.6, 3.8) ------------------

const NS: &str = "nsnsnsnsnsnsnsnsnsnsns";

/// A Airpost server: `/airpost/v1/me` with tracking on or off, claims answered `claim_status`.
fn airpost_server(tracking: bool, claim_status: u16) -> common::MockServer {
    common::MockServer::start(move |r: &common::Request| match r.path() {
        "/airpost/v1/me" => common::Reply::json(
            200,
            json!({"userId": "u1", "email": "me@example.com", "trackingNS": NS, "serverId": "s1",
                   "publicURL": "https://t.example", "features": {"tracking": tracking}}),
        ),
        p if p.starts_with("/airpost/v1/claim/") => common::Reply::json(claim_status, json!({})),
        _ => common::Reply::json(404, json!({})),
    })
}

fn http_backend(url: &str, dir: &std::path::Path) -> Arc<dyn airpost_net::MetadataBackend> {
    let server = airpost_net::AirpostServer {
        url: url.to_owned(),
        token: "airpost_t".into(),
        user_id: "u1".into(),
        server_id: "s1".into(),
    };
    Arc::new(airpost_net::HttpMetadataBackend::new(server, ACCOUNT, dir, Box::new(|| {})).unwrap())
}

fn claims(server: &common::MockServer) -> Vec<String> {
    server
        .requests()
        .iter()
        .filter(|r| r.path().starts_with("/airpost/v1/claim/"))
        .map(|r| format!("{} {}", r.method, r.path()))
        .collect()
}

/// The text of a submission with quoted-printable soft breaks and `=3D` undone.
fn decoded(sub: &Submission) -> String {
    sub.text().replace("=\r\n", "").replace("=3D", "=")
}

impl Env {
    fn tracked_draft(&self, hmid: &str, body: &str) -> String {
        let id = self.scheduled_draft(hmid, json!({"body": body}), json!({"expiration": T0}));
        for (plugin, value) in [
            ("open-tracking", json!({"open_count": 0, "open_data": []})),
            ("link-tracking", json!({"tracked": true})),
        ] {
            self.queue(
                "SyncbackMetadataTask",
                json!({"modelId": id, "modelClassName": "message",
                       "modelHeaderMessageId": hmid, "pluginId": plugin, "value": value}),
            );
        }
        id
    }
}

#[test]
fn executor_claims_then_sends_a_tracked_draft_with_a_body_per_recipient() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let server = airpost_server(true, 200);
    let body = r#"<p>Hi <a href="https://a.example/x">a</a></p><blockquote class="gmail_quote"><a href="https://q.example">q</a></blockquote>"#;
    let id = e.tracked_draft("track-1@example.test", body);
    let backend = http_backend(&server.url(), e._dir.path());
    let executor = e.executor(json!({})).0.with_metadata(backend);
    assert_eq!(
        executor.handle("Message", &id, "send-later", T0).unwrap(),
        Outcome::Done
    );
    drop(executor);
    assert_eq!(
        claims(&server),
        [format!(
            "POST /airpost/v1/claim/acc1/track-1@example.test/send-later/{T0}"
        )]
    );

    let sends = e.tasks_of("SendDraftTask");
    assert_eq!(sends.len(), 1);
    let data = &sends[0].data;
    let bodies = data["perRecipientBodies"].as_object().unwrap();
    let keys: Vec<&String> = bodies.keys().collect();
    assert_eq!(keys, ["bob@example.net", "boss@example.net", "self"]);
    assert_eq!(bodies["self"], body, "the sent copy stays untracked");
    let open = airpost_store::model::metadata_value(&data["draft"], "open-tracking").unwrap();
    let link = airpost_store::model::metadata_value(&data["draft"], "link-tracking").unwrap();
    let tid = open["trackingId"].as_str().unwrap().to_owned();
    assert_eq!(link["trackingId"], tid);
    assert_eq!(
        link["links"],
        json!([{"url": "https://a.example/x", "click_count": 0, "click_data": []}])
    );
    let rids: BTreeMap<String, String> = open["recipients"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(rid, email)| (email.as_str().unwrap().to_owned(), rid.clone()))
        .collect();
    assert_eq!(rids.len(), 2);
    let bob = bodies["bob@example.net"].as_str().unwrap();
    let rid = &rids["bob@example.net"];
    assert!(bob.contains(&format!(
        r#"<img alt="" src="https://t.example/t/o/{NS}/{tid}.{rid}.png"><blockquote class="gmail_quote"><a href="https://q.example">q</a>"#
    )));
    assert!(bob.contains(&format!(
        r#"href="https://t.example/t/l/{NS}/{tid}/0.{rid}?u=https%3A%2F%2Fa.example%2Fx""#
    )));

    // On the wire: one submission per recipient, each with its own ids.
    let mut send = sends[0].clone();
    e.remote(&mut send);
    assert!(send.data["error"].is_null(), "{}", send.data);
    let subs = e.imap.outbox.submissions();
    assert_eq!(subs.len(), 2);
    for sub in &subs {
        let (_, to) = sub.envelope.clone().unwrap();
        assert_eq!(to.len(), 1);
        let text = decoded(sub);
        assert!(
            text.contains(&format!("/t/o/{NS}/{tid}.{}.png", rids[&to[0]])),
            "{text}"
        );
    }
    // The values with the ids go to the sent message, which pushes them to the server.
    let pushed: Vec<Task> = e
        .tasks_of("SyncbackMetadataTask")
        .into_iter()
        .filter(|t| t.data["pluginId"] == "open-tracking" && t.data["modelId"] != id)
        .collect();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].data["value"]["trackingId"], tid);
}

#[test]
fn executor_sends_untracked_when_the_server_has_no_tracking() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let server = airpost_server(false, 200);
    let id = e.tracked_draft("track-2@example.test", "<p>Hi</p>");
    let executor = e
        .executor(json!({}))
        .0
        .with_metadata(http_backend(&server.url(), e._dir.path()));
    assert_eq!(
        executor.handle("Message", &id, "send-later", T0).unwrap(),
        Outcome::Done
    );
    drop(executor);
    let sends = e.tasks_of("SendDraftTask");
    assert!(sends[0].data["perRecipientBodies"].is_null());
    let open =
        airpost_store::model::metadata_value(&sends[0].data["draft"], "open-tracking").unwrap();
    assert!(open.get("trackingId").is_none());
}

#[test]
fn executor_leaves_a_snooze_another_device_claimed() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let snoozed = e.folder("Snoozed", "snoozed");
    let first = e.deliver(&snoozed, 3, MessageFlags::SEEN);
    let thread = e.thread_of(&first);
    e.set_thread_metadata(thread.id(), "thread-snooze", json!({"expiration": T0}));
    let server = airpost_server(true, 409);
    let executor = e
        .executor(json!({}))
        .0
        .with_metadata(http_backend(&server.url(), e._dir.path()));
    for _ in 0..2 {
        assert_eq!(
            executor
                .handle("Thread", thread.id(), "thread-snooze", T0)
                .unwrap(),
            Outcome::Done
        );
    }
    drop(executor);
    // Keyed on the thread's oldest message, asked once.
    assert_eq!(
        claims(&server),
        [format!(
            "POST /airpost/v1/claim/acc1/m3@x/thread-snooze/{T0}"
        )]
    );
    assert!(e.tasks_of("ChangeFolderTask").is_empty());
    assert!(e.tasks_of("SendDraftTask").is_empty());
    assert_eq!(
        e.metadata_of::<Thread>(thread.id(), "thread-snooze")["expiration"],
        T0
    );
}

#[test]
fn executor_acts_when_the_claim_gets_no_answer() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let snoozed = e.folder("Snoozed", "snoozed");
    let first = e.deliver(&snoozed, 3, MessageFlags::SEEN);
    let thread = e.thread_of(&first);
    e.set_thread_metadata(thread.id(), "thread-snooze", json!({"expiration": T0}));
    let executor = e
        .executor(json!({}))
        .0
        .with_metadata(http_backend(&common::closed_port_url(), e._dir.path()));
    assert_eq!(
        executor
            .handle("Thread", thread.id(), "thread-snooze", T0)
            .unwrap(),
        Outcome::Done
    );
    drop(executor);
    assert_eq!(e.tasks_of("ChangeFolderTask").len(), 1);
}

// -- Server-synced drafts (docs/drafts-sync-design.md 3.2) ---------------------------------

impl Env {
    fn sync_drafts_to_server(&mut self) {
        self.account = Arc::new(
            Account::from_json(json!({
                "id": ACCOUNT,
                "provider": "imap",
                "emailAddress": "me@example.com",
                "settings": {"save_drafts_to_server": true},
            }))
            .unwrap(),
        );
    }

    fn at(&self, t: i64) {
        self.store.set_clock(Arc::new(ManualClock::new(t)));
    }

    /// The foreground's draft step, as after its queued tasks: how long until the next push.
    fn push_drafts(&mut self) -> Option<std::time::Duration> {
        self.sync_drafts_deleting(&[])
    }

    /// The same, with the copies of tombstoned drafts that scans skipped.
    fn sync_drafts_deleting(
        &mut self,
        tombstoned: &[airpost_sync::drafts::TombstonedCopy],
    ) -> Option<std::time::Duration> {
        processor(&self.store, self.harness())
            .sync_drafts(&mut self.imap, tombstoned)
            .unwrap()
    }

    fn save_draft(&self, hmid: &str, extra: Value) -> String {
        let save = self.queue(
            "SyncbackDraftTask",
            json!({"draft": draft_json(hmid, extra)}),
        );
        assert_eq!(save.status(), "remote", "{}", save.data);
        airpost_core::ids::draft_id(ACCOUNT, hmid)
    }

    fn draft_sync(&self, id: &str) -> airpost_store::drafts::DraftSync {
        self.message(id).unwrap().draft_sync().unwrap()
    }
}

#[test]
fn a_saved_draft_is_pushed_once_left_alone_and_replaces_its_older_copy() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    e.mail_folders();
    let id = e.save_draft("d1@example.com", json!({}));
    let sync = e.draft_sync(&id);
    assert_eq!((sync.rev, sync.pushed_rev, sync.saved_at), (1, 0, T0));

    // Still being edited: nothing goes up, and the foreground wakes when it is due.
    assert_eq!(e.push_drafts(), Some(std::time::Duration::from_secs(20)));
    assert!(e.imap.appended.is_empty());

    e.at(T0 + 20);
    assert_eq!(e.push_drafts(), None);
    assert_eq!(e.imap.uids("Drafts"), [1]);
    assert_eq!(e.copies(&id), [("Drafts".to_owned(), 1, None)]);
    let sync = e.draft_sync(&id);
    assert_eq!((sync.rev, sync.pushed_rev), (1, 1));
    assert_eq!(sync.base.as_ref().map(|b| b.u), Some(1));
    let wire = String::from_utf8_lossy(&e.imap.appended[0]).into_owned();
    for h in [
        "Message-ID: <d1@example.com>",
        "In-Reply-To: <parent@example.net>",
        "References: <parent@example.net>",
        "Bcc: <boss@example.net>",
        "X-Airpost-Draft: 1",
        "Hi Bob",
    ] {
        assert!(wire.contains(h), "{h} missing from\n{wire}");
    }
    let m = e.message(&id).unwrap();
    assert!(m.is_draft() && !m.is_unread());
    e.assert_invariants();

    // The scan that sees the copy finds it already recorded on the draft.
    let copy = e.imap.boxes["Drafts"].messages[&1].clone();
    let drafts = e
        .store
        .find::<Folder>(&Query::new().equal("role", "drafts"))
        .unwrap()
        .unwrap();
    let seen = MailProcessor::new(&e.account, &e.store)
        .insert_fallback_to_update_message(&copy, &drafts, T0 + 21)
        .unwrap();
    assert_eq!(seen.id(), id);
    assert_eq!(e.store.count::<Message>(&Query::new()).unwrap(), 1);

    // The next revision replaces it; the client's echo of `_dsync` is ignored.
    e.at(T0 + 100);
    e.save_draft(
        "d1@example.com",
        json!({"id": id, "body": "<p>Hi Bob, v2</p>", "_dsync": {"rev": 0, "pushedRev": 9}}),
    );
    let sync = e.draft_sync(&id);
    assert_eq!((sync.rev, sync.pushed_rev, sync.saved_at), (2, 1, T0 + 100));
    e.at(T0 + 130);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [2]);
    assert!(
        e.imap.uids("Trash").is_empty(),
        "moved to Trash and expunged there"
    );
    assert_eq!(e.copies(&id), [("Drafts".to_owned(), 2, None)]);
    assert!(String::from_utf8_lossy(&e.imap.appended[1]).contains("Hi Bob, v2"));
    e.assert_invariants();
}

#[test]
fn drafts_are_not_pushed_while_pristine_or_while_a_send_is_queued() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    e.mail_folders();
    let pristine = e.save_draft("p@example.com", json!({"pristine": true}));
    let sending = e.save_draft("s@example.com", json!({}));
    let send = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("s@example.com", json!({"id": sending})), "perRecipientBodies": null}),
    );
    assert_eq!(send.status(), "remote");
    e.at(T0 + 60);
    assert_eq!(e.push_drafts(), None);
    assert!(e.imap.appended.is_empty());
    assert_eq!(e.draft_sync(&pristine).pushed_rev, 0);
}

#[test]
fn without_uidplus_the_pushed_copy_is_found_by_its_message_id() {
    let mut e = env(&["MOVE"]);
    e.sync_drafts_to_server();
    e.mail_folders();
    let id = e.save_draft("d1@example.com", json!({}));
    e.at(T0 + 20);
    e.push_drafts();
    e.at(T0 + 40);
    e.save_draft("d1@example.com", json!({"id": id}));
    e.at(T0 + 60);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [2]);
    assert_eq!(e.copies(&id), [("Drafts".to_owned(), 2, None)]);
    e.assert_invariants();
}

#[test]
fn a_failed_push_backs_off_and_an_oversized_draft_waits_for_the_next_save() {
    let mut e = env(&["MOVE", "UIDPLUS", "APPENDLIMIT=2000"]);
    e.sync_drafts_to_server();
    e.mail_folders();
    e.imap.reject.push("APPEND");
    let id = e.save_draft("d1@example.com", json!({}));
    e.at(T0 + 20);
    assert_eq!(e.push_drafts(), Some(std::time::Duration::from_secs(30)));
    let sync = e.draft_sync(&id);
    assert_eq!((sync.fails, sync.next_at, sync.pushed_rev), (1, T0 + 50, 0));

    e.imap.reject.clear();
    e.at(T0 + 50);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [1]);
    assert_eq!(e.draft_sync(&id).fails, 0);

    e.at(T0 + 100);
    let long = "x".repeat(3000);
    e.save_draft("d1@example.com", json!({"id": id, "body": long}));
    e.at(T0 + 200);
    assert_eq!(e.push_drafts(), None);
    assert_eq!(e.imap.appended.len(), 1, "only the first push");
    let sync = e.draft_sync(&id);
    assert_eq!((sync.rev, sync.pushed_rev, sync.held), (2, 1, Some(2)));
    assert_eq!(e.imap.uids("Drafts"), [1], "the older copy stays");

    e.save_draft("d1@example.com", json!({"id": id, "body": "<p>short</p>"}));
    e.at(T0 + 300);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [2]);
    e.assert_invariants();
}

#[test]
fn a_draft_under_another_id_moves_to_its_message_id_before_it_is_pushed() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    e.mail_folders();
    e.save_draft("d1@example.com", json!({"id": "local-1234-abcd"}));
    assert!(e.message("local-1234-abcd").is_some());
    e.at(T0 + 20);
    e.push_drafts();
    let id = airpost_core::ids::draft_id(ACCOUNT, "d1@example.com");
    assert!(e.message("local-1234-abcd").is_none());
    assert_eq!(e.copies(&id), [("Drafts".to_owned(), 1, None)]);
    // The composer still names the old id.
    e.save_draft(
        "d1@example.com",
        json!({"id": "local-1234-abcd", "body": "<p>v2</p>"}),
    );
    assert_eq!(e.draft_sync(&id).rev, 2);
    assert_eq!(e.body_of(&id).as_deref(), Some("<p>v2</p>"));
    assert!(e.message("local-1234-abcd").is_none());
    e.assert_invariants();
}

impl Env {
    /// Another device saves a revision of draft `hmid` to Drafts, and a scan sees it.
    fn revision_from_elsewhere(&mut self, hmid: &str, subject: &str) -> u32 {
        let drafts = self
            .store
            .find::<Folder>(&Query::new().equal("role", "drafts"))
            .unwrap()
            .unwrap();
        let mut h = header(90);
        h.message_id = Some(hmid.to_owned());
        h.subject = Some(subject.to_owned());
        h.subject_raw = Some(subject.as_bytes().to_vec());
        let uid = self.imap.add(
            "Drafts",
            ImapMessage {
                flags: MessageFlags::DRAFT | MessageFlags::SEEN,
                internal_date: Some(T0),
                header: Some(h),
                ..Default::default()
            },
        );
        let copy = self.imap.boxes["Drafts"].messages[&uid].clone();
        MailProcessor::new(&self.account, &self.store)
            .insert_fallback_to_update_message(&copy, &drafts, T0)
            .unwrap();
        uid
    }
}

#[test]
fn another_devices_revision_is_taken_when_nothing_here_is_unpushed() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    e.mail_folders();
    let id = e.save_draft("d1@example.com", json!({}));
    e.at(T0 + 20);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [1]);

    let theirs = e.revision_from_elsewhere("d1@example.com", "Edited elsewhere");
    assert_eq!(
        e.copies(&id).len(),
        2,
        "both copies collapse onto the draft"
    );
    e.at(T0 + 40);
    assert_eq!(e.push_drafts(), None);
    let m = e.message(&id).unwrap();
    assert_eq!(m.subject(), "Edited elsewhere");
    assert_eq!(e.body_of(&id).as_deref().map(str::trim), Some("body"));
    let sync = e.draft_sync(&id);
    assert_eq!(sync.base.as_ref().map(|b| b.u), Some(theirs));
    assert!(!sync.is_dirty());
    assert_eq!(e.imap.appended.len(), 1, "taking a revision pushes nothing");
    e.assert_invariants();
}

#[test]
fn unpushed_edits_here_win_over_a_revision_from_elsewhere() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    e.mail_folders();
    let id = e.save_draft("d1@example.com", json!({}));
    e.at(T0 + 20);
    e.push_drafts();
    e.at(T0 + 30);
    e.save_draft("d1@example.com", json!({"id": id, "body": "<p>Mine</p>"}));
    let theirs = e.revision_from_elsewhere("d1@example.com", "Edited elsewhere");

    e.at(T0 + 35);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [1], "their copy went to Trash");
    assert_eq!(e.imap.uids("Trash").len(), 1, "and was not expunged there");
    assert!(e.copies(&id).iter().all(|(_, u, _)| *u != theirs));
    assert_eq!(e.message(&id).unwrap().subject(), "Hello Bob");

    e.at(T0 + 60);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [3]);
    assert_eq!(e.imap.uids("Trash").len(), 1);
    assert!(String::from_utf8_lossy(&e.imap.appended[1]).contains("Mine"));
    e.assert_invariants();
}

#[test]
fn copies_of_sent_or_destroyed_drafts_are_deleted_until_the_draft_is_saved_again() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    let (_, drafts, _) = e.mail_folders();
    let id = e.save_draft("d1@example.com", json!({}));
    e.at(T0 + 20);
    e.push_drafts();
    let mut send = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("d1@example.com", json!({"id": id})), "perRecipientBodies": null}),
    );
    e.remote(&mut send);
    assert_eq!(send.status(), "complete", "{}", send.data);
    assert!(e.imap.uids("Drafts").is_empty());
    assert!(e
        .store
        .is_draft_tombstoned(ACCOUNT, "d1@example.com")
        .unwrap());

    // A copy saved late by another device comes back to Drafts; the scan skips it and the
    // foreground deletes it.
    let late = e.revision_from_elsewhere_unsynced("d1@example.com");
    e.sync_drafts_deleting(&[airpost_sync::drafts::TombstonedCopy {
        folder_id: drafts.id().to_owned(),
        uid: late,
        header_message_id: "d1@example.com".to_owned(),
    }]);
    assert!(e.imap.uids("Drafts").is_empty());

    // Undo send: the client writes the draft again.
    e.save_draft("d1@example.com", json!({}));
    assert!(!e
        .store
        .is_draft_tombstoned(ACCOUNT, "d1@example.com")
        .unwrap());

    e.queue("DestroyDraftTask", json!({"messageIds": [id]}));
    assert!(e
        .store
        .is_draft_tombstoned(ACCOUNT, "d1@example.com")
        .unwrap());
    e.at(T0 + 31 * 24 * 60 * 60);
    e.push_drafts();
    assert!(!e
        .store
        .is_draft_tombstoned(ACCOUNT, "d1@example.com")
        .unwrap());
}

#[test]
fn a_new_draft_with_a_stale_version_is_inserted() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let save = e.queue(
        "SyncbackDraftTask",
        json!({"draft": draft_json("backup@example.com", json!({"v": 7}))}),
    );
    assert_eq!(save.status(), "remote", "{}", save.data);
    let id = airpost_core::ids::draft_id(ACCOUNT, "backup@example.com");
    let draft = e.message(&id).expect("the draft is saved");
    assert_eq!(draft.version(), 1);
    assert_eq!(e.body_of(&id).as_deref(), Some("<p>Hi Bob</p>"));
    e.assert_invariants();
}

#[test]
fn drafts_leave_no_tombstones_when_they_are_not_synced() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let save = e.queue(
        "SyncbackDraftTask",
        json!({"draft": draft_json("d1@example.com", json!({}))}),
    );
    assert_eq!(save.status(), "remote");
    let id = airpost_core::ids::draft_id(ACCOUNT, "d1@example.com");
    assert!(e.message(&id).unwrap().draft_sync().is_none());
    e.queue("DestroyDraftTask", json!({"messageIds": [id]}));
    assert!(!e
        .store
        .is_draft_tombstoned(ACCOUNT, "d1@example.com")
        .unwrap());
}

impl Env {
    /// A copy of draft `hmid` lands in Drafts on the server; no scan sees it.
    fn revision_from_elsewhere_unsynced(&mut self, hmid: &str) -> u32 {
        let mut h = header(91);
        h.message_id = Some(hmid.to_owned());
        self.imap.add(
            "Drafts",
            ImapMessage {
                flags: MessageFlags::DRAFT | MessageFlags::SEEN,
                internal_date: Some(T0),
                header: Some(h),
                ..Default::default()
            },
        )
    }
}

#[test]
fn drafts_saved_before_drafts_synced_are_pushed_once_turned_on() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.mail_folders();
    let written = e.save_draft("d1@example.com", json!({}));
    let empty = e.save_draft(
        "d2@example.com",
        json!({"body": "", "to": [], "bcc": [], "rthMsgId": ""}),
    );
    let pristine = e.save_draft("d3@example.com", json!({"pristine": true}));
    assert!(e.message(&written).unwrap().draft_sync().is_none());

    e.sync_drafts_to_server();
    // Not until Sent has been walked once: it might hold the copy of a draft that was sent.
    assert_eq!(e.push_drafts(), Some(std::time::Duration::from_secs(30)));
    assert!(e.imap.appended.is_empty());
    e.sent_walked();
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [1]);
    assert_eq!(e.copies(&written), [("Drafts".to_owned(), 1, None)]);
    for id in [&empty, &pristine] {
        let sync = e.draft_sync(id);
        assert_eq!((sync.rev, sync.pushed_rev), (0, 0), "{id}");
        assert_eq!(e.copies(id), [("Drafts".to_owned(), 0, None)]);
    }
    e.push_drafts();
    assert_eq!(e.imap.appended.len(), 1);

    // The next save of a draft that stayed local pushes it.
    e.at(T0 + 10);
    e.save_draft("d2@example.com", json!({"id": empty, "body": "<p>Now</p>"}));
    e.at(T0 + 30);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [1, 2]);
    e.assert_invariants();
}

impl Env {
    /// The first walk of Sent has reached UID 1.
    fn sent_walked(&self) {
        let mut sent = self.sent_folder();
        let initial = sent.local_status().clone();
        sent.local_status_mut()
            .insert("syncedMinUID".into(), json!(1));
        self.store.save_folder_status(&mut sent, &initial).unwrap();
    }

    /// The message `hmid` went out: its copy in `folder` (Sent) is on the server and synced.
    fn sent_copy(&mut self, folder: &Folder, n: u32, hmid: &str) -> String {
        let mut m = remote(n, MessageFlags::SEEN);
        m.header.as_mut().unwrap().message_id = Some(hmid.to_owned());
        let uid = self.imap.add(folder.path(), m);
        let fetched = self.imap.boxes[folder.path()].messages[&uid].clone();
        MailProcessor::new(&self.account, &self.store)
            .insert_fallback_to_update_message(&fetched, folder, T0)
            .unwrap()
            .id()
            .to_owned()
    }
}

#[test]
fn a_draft_whose_message_is_in_sent_is_removed_instead_of_pushed() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    let (sent, _, _) = e.mail_folders();
    let leftover = e.save_draft("gone@example.com", json!({}));
    let kept = e.save_draft("kept@example.com", json!({}));
    let copy = e.sent_copy(&sent, 1, "gone@example.com");

    e.at(T0 + 20);
    e.push_drafts();
    assert!(e.message(&leftover).is_none());
    assert!(e
        .store
        .is_draft_tombstoned(ACCOUNT, "gone@example.com")
        .unwrap());
    assert_eq!(e.imap.appended.len(), 1, "only the other draft is pushed");
    assert!(String::from_utf8_lossy(&e.imap.appended[0]).contains("kept@example.com"));
    assert!(e.message(&kept).is_some());
    assert!(e.message(&copy).is_some(), "the sent message stays");
    assert_eq!(e.imap.uids("Sent"), [1]);
    e.assert_invariants();
}

#[test]
fn a_pushed_draft_whose_message_is_in_sent_goes_from_drafts_and_sent_keeps_its_copy() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    let (sent, _, _) = e.mail_folders();
    let id = e.save_draft("gone@example.com", json!({}));
    e.at(T0 + 20);
    e.push_drafts();
    assert_eq!(e.imap.uids("Drafts"), [1]);

    let copy = e.sent_copy(&sent, 1, "gone@example.com");
    e.at(T0 + 40);
    e.push_drafts();
    assert!(e.message(&id).is_none());
    assert!(
        e.imap.uids("Drafts").is_empty(),
        "the pushed copy is deleted"
    );
    assert!(e.imap.uids("Trash").is_empty(), "and expunged from Trash");
    assert_eq!(e.imap.uids("Sent"), [1]);
    assert!(e.message(&copy).is_some());
    e.assert_invariants();
}

#[test]
fn drafts_a_send_names_or_whose_twin_is_not_in_sent_are_not_removed() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    let (sent, _, archive) = e.mail_folders();
    let sending = e.save_draft("sending@example.com", json!({}));
    let failed = e.save_draft("failed@example.com", json!({}));
    let archived = e.save_draft("archived@example.com", json!({}));
    let mut send = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("sending@example.com", json!({"id": sending})), "perRecipientBodies": null}),
    );
    assert_eq!(send.status(), "remote");
    let mut failure = e.queue(
        "SendDraftTask",
        json!({"draft": draft_json("failed@example.com", json!({"id": failed})), "perRecipientBodies": null}),
    );
    failure.set_error(json!({"key": "send-partially-failed"}));
    failure.set_status("complete");
    e.store.save(&mut failure).unwrap();
    e.sent_copy(&sent, 1, "sending@example.com");
    e.sent_copy(&sent, 2, "failed@example.com");
    e.sent_copy(&archive, 3, "archived@example.com");

    e.at(T0 + 20);
    e.push_drafts();
    for id in [&sending, &failed, &archived] {
        assert!(e.message(id).is_some(), "{id}");
    }
    send.set_status("complete");
    e.store.save(&mut send).unwrap();
    e.push_drafts();
    assert!(e.message(&sending).is_none(), "once the send has ended");
}

impl Env {
    fn thread_folder_ids(&self, message_id: &str) -> Vec<String> {
        self.thread_of(message_id)
            .folders()
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["id"].as_str().unwrap().to_owned())
            .collect()
    }
}

#[test]
fn drafts_in_no_thread_join_the_thread_they_answer_or_get_their_own() {
    let mut e = env(&["MOVE", "UIDPLUS"]);
    e.sync_drafts_to_server();
    let (_, drafts, _) = e.mail_folders();
    let inbox = e
        .store
        .find::<Folder>(&Query::new().equal("role", "inbox"))
        .unwrap()
        .unwrap();
    let original = e.deliver(&inbox, 7, MessageFlags::SEEN);
    let thread = e.thread_of(&original);
    let reply = e.save_draft("reply@example.com", json!({"rthMsgId": "m7@x"}));
    let new = e.save_draft("new@example.com", json!({"rthMsgId": ""}));
    let follow_up = e.save_draft(
        "follow-up@example.com",
        json!({"rthMsgId": "new@example.com"}),
    );
    for id in [&reply, &new, &follow_up] {
        assert!(e.message(id).unwrap().thread_id().is_empty(), "{id}");
    }

    e.push_drafts();
    assert_eq!(e.message(&reply).unwrap().thread_id(), thread.id());
    assert!(e
        .thread_folder_ids(&reply)
        .contains(&drafts.id().to_owned()));
    let own = airpost_core::ids::thread_id(&new);
    assert_eq!(e.message(&new).unwrap().thread_id(), own);
    assert_eq!(e.message(&follow_up).unwrap().thread_id(), own);
    assert_eq!(e.thread_folder_ids(&new), [drafts.id().to_owned()]);
    e.assert_invariants();

    // A composer that opened the reply before it was threaded saves it again.
    e.save_draft(
        "reply@example.com",
        json!({"id": reply, "threadId": "", "rthMsgId": "m7@x"}),
    );
    assert_eq!(e.message(&reply).unwrap().thread_id(), thread.id());
    e.assert_invariants();

    // A thread of drafts alone goes with them.
    let mut destroy = e.queue("DestroyDraftTask", json!({"messageIds": [new, follow_up]}));
    e.remote(&mut destroy);
    assert!(e
        .store
        .find::<Thread>(&Query::new().equal("id", &own))
        .unwrap()
        .is_none());
    e.assert_invariants();
}

#[test]
fn on_gmail_a_draft_takes_the_thread_of_its_server_copy() {
    let mut e = env(&["X-GM-EXT-1", "UIDPLUS"]);
    e.imap.profile.mailbox_model = airpost_providers::MailboxModelKind::GmailLabels;
    e.sync_drafts_to_server();
    let all = e.folder("[Gmail]/All Mail", "all");
    e.folder("[Gmail]/Trash", "trash");
    let ingest = |e: &mut Env, m: ImapMessage| {
        let uid = e.imap.add(all.path(), m);
        let fetched = e.imap.boxes[all.path()].messages[&uid].clone();
        let mut mp = MailProcessor::new(&e.account, &e.store);
        mp.is_gmail = true;
        mp.insert_fallback_to_update_message(&fetched, &all, T0)
            .unwrap()
            .id()
            .to_owned()
    };
    // Another message of the Gmail thread is here; the one the draft answers is not.
    let mut sibling = remote(1, MessageFlags::SEEN);
    sibling.gmail_thread_id = Some(777);
    let sibling = ingest(&mut e, sibling);
    let thread = e.thread_of(&sibling);

    let id = e.save_draft("g@example.com", json!({}));
    e.push_drafts();
    assert!(
        e.message(&id).unwrap().thread_id().is_empty(),
        "with no server copy yet it waits for its push"
    );

    let mut copy = remote(2, MessageFlags::DRAFT | MessageFlags::SEEN);
    copy.header.as_mut().unwrap().message_id = Some("g@example.com".into());
    copy.gmail_thread_id = Some(777);
    copy.gmail_labels = vec!["\\Draft".into()];
    assert_eq!(ingest(&mut e, copy), id);
    assert!(e.message(&id).unwrap().thread_id().is_empty());
    e.push_drafts();
    assert_eq!(e.message(&id).unwrap().thread_id(), thread.id());
    e.assert_invariants();
}
