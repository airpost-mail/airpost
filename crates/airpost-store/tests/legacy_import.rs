// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `import-legacy` against C++-shaped databases: V12 (with placements) and pre-V10 (a single
//! `remoteFolderId` / `remoteUID` per message), through `migrate` as the client runs them.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use airpost_core::ManualClock;
use airpost_store::legacy_import::{
    import_legacy, pending_message_key, ImportOutcome, LEGACY_FILES_DIR,
    LEGACY_IMPORTED_DATABASE_FILE, LEGACY_IMPORT_KEY,
};
use airpost_store::{DeltaStream, Message, MessageAttributes, Model, Query, Store, Thread};
use common::{Fixture, Sink, ACCOUNT};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const DRAFT_ID: &str = "cppdraft0000000000000000000000000000000001";
const DRAFT_HMSGID: &str = "local-draft@example.test";
const FILE_ID: &str = "DraftFile01";
const LEGACY_THREAD: &str = "t:cppmessage1";

fn attrs(uid: u32) -> MessageAttributes {
    MessageAttributes {
        uid,
        unread: false,
        starred: false,
        draft: false,
        labels: Vec::new(),
        message_id: String::new(),
    }
}

fn legacy_folder_id(path: &str) -> String {
    airpost_core::ids::folder_id(ACCOUNT, path)
}

/// What the C++ engine leaves after a snooze on a two-message thread, a reminder on a
/// message, and a local draft with an attachment and send-later metadata.
fn write_v12(conn: &Connection) {
    airpost_store::schema::create_v12(conn).unwrap();
    conn.execute_batch("PRAGMA user_version = 12").unwrap();
    for (path, role) in [("INBOX", "inbox"), ("Drafts", "drafts")] {
        let id = legacy_folder_id(path);
        let data = json!({"id": id, "aid": ACCOUNT, "v": 1, "path": path, "role": role,
                          "localStatus": {"uidvalidity": 7, "syncedMinUID": 1}});
        conn.execute(
            "INSERT INTO Folder (id, accountId, version, data, path, role) VALUES (?,?,?,?,?,?)",
            params![id, ACCOUNT, 1, data.to_string(), path, role],
        )
        .unwrap();
    }
    let inbox = legacy_folder_id("INBOX");
    let drafts = legacy_folder_id("Drafts");
    let messages = [
        (
            "cppmessage1",
            "m1@example.test",
            LEGACY_THREAD,
            1,
            json!([]),
        ),
        (
            "cppmessage2",
            "m2@example.test",
            LEGACY_THREAD,
            2,
            json!([]),
        ),
        (
            "cppmessage3",
            "m3@example.test",
            "t:cppmessage3",
            3,
            json!([{"pluginId": "send-reminders", "v": 4, "value": {"expiration": 1_900_000_100}},
                   {"pluginId": "cleared", "v": 2, "value": {}}]),
        ),
    ];
    for (id, hmsgid, thread, uid, metadata) in messages {
        let data = json!({"id": id, "aid": ACCOUNT, "v": 3, "hMsgId": hmsgid, "threadId": thread,
                          "folders": {inbox.clone(): 0}, "metadata": metadata, "draft": false});
        conn.execute(
            "INSERT INTO Message (id, accountId, version, data, headerMessageId, draft, threadId) VALUES (?,?,?,?,?,0,?)",
            params![id, ACCOUNT, 3, data.to_string(), hmsgid, thread],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO MessageFolder (accountId, messageId, folderId, remoteUID) VALUES (?,?,?,?)",
            params![ACCOUNT, id, inbox, uid],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) VALUES ('cppmessage3', ?, 'Message', 'send-reminders', 1900000100)",
        [ACCOUNT],
    )
    .unwrap();
    let thread = json!({"id": LEGACY_THREAD, "aid": ACCOUNT, "v": 2,
                        "metadata": [{"pluginId": "thread-snooze", "v": 2, "value": {"expiration": 1_900_000_000}}]});
    conn.execute(
        "INSERT INTO Thread (id, accountId, version, data) VALUES (?,?,2,?)",
        params![LEGACY_THREAD, ACCOUNT, thread.to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) VALUES (?, ?, 'Thread', 'thread-snooze', 1900000000)",
        [LEGACY_THREAD, ACCOUNT],
    )
    .unwrap();

    let draft = draft_json(json!({drafts.clone(): 4}));
    conn.execute(
        "INSERT INTO Message (id, accountId, version, data, headerMessageId, draft, threadId) VALUES (?,?,2,?,?,1,'')",
        params![DRAFT_ID, ACCOUNT, draft.to_string(), DRAFT_HMSGID],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO MessageFolder (accountId, messageId, folderId, remoteUID, draft) VALUES (?,?,?,0,1)",
        params![ACCOUNT, DRAFT_ID, drafts],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) VALUES (?, ?, 'Message', 'send-later', 1900000200)",
        [DRAFT_ID, ACCOUNT],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO MessageBody (id, value, fetchedAt) VALUES (?, '<p>draft</p>', NULL)",
        [DRAFT_ID],
    )
    .unwrap();
    // A deletion placeholder for a draft whose delete has not reached the server yet.
    conn.execute(
        "INSERT INTO Message (id, accountId, version, data, headerMessageId, draft, threadId) VALUES ('deleted-x', ?, 1, '{}', 'deleted-x', 1, '')",
        [ACCOUNT],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO MessageFolder (accountId, messageId, folderId, remoteUID, draft) VALUES (?, 'deleted-x', ?, 0, 1)",
        params![ACCOUNT, drafts],
    )
    .unwrap();
    for (k, v) in [
        ("cursor-acc1", "42"),
        ("gsynctoken-contacts-acc1", "token"),
        ("VACUUM_TIME", "1"),
    ] {
        conn.execute("INSERT INTO _State (id, value) VALUES (?, ?)", [k, v])
            .unwrap();
    }
}

const REPLY_ID: &str = "cppdraft0000000000000000000000000000000002";

/// [`write_v12`] plus a local reply draft to m3, which the client saved in m3's thread.
fn write_v12_with_reply(conn: &Connection) {
    write_v12(conn);
    let drafts = legacy_folder_id("Drafts");
    let reply = json!({
        "id": REPLY_ID, "aid": ACCOUNT, "v": 1, "__cls": "Message", "hMsgId": "reply@example.test",
        "subject": "Re: Subject m3", "draft": true, "unread": false, "starred": false,
        "threadId": "t:cppmessage3", "rthMsgId": "m3@example.test",
        "folders": {drafts.clone(): 4}, "labels": [], "date": 1_700_000_100, "files": [],
    });
    conn.execute(
        "INSERT INTO Message (id, accountId, version, data, headerMessageId, draft, threadId) VALUES (?,?,1,?,'reply@example.test',1,'t:cppmessage3')",
        params![REPLY_ID, ACCOUNT, reply.to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO MessageFolder (accountId, messageId, folderId, remoteUID, draft) VALUES (?,?,?,0,1)",
        params![ACCOUNT, REPLY_ID, drafts],
    )
    .unwrap();
}

fn draft_json(folders: Value) -> Value {
    json!({
        "id": DRAFT_ID, "aid": ACCOUNT, "v": 2, "__cls": "Message", "hMsgId": DRAFT_HMSGID,
        "subject": "Plans", "draft": true, "unread": false, "starred": false, "threadId": "",
        "folders": folders, "labels": [], "date": 1_700_000_000,
        "files": [{"__cls": "File", "id": FILE_ID, "aid": ACCOUNT, "filename": "notes.txt",
                   "contentType": "text/plain", "size": 5}],
        "metadata": [{"pluginId": "send-later", "v": 3, "value": {"expiration": 1_900_000_200}}],
    })
}

/// Before V10 a message had one location, in Message columns and in its JSON's `folder`.
fn write_v9(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE `_State` (id VARCHAR(40) PRIMARY KEY, value TEXT);
         CREATE TABLE Folder (id VARCHAR(40) PRIMARY KEY,accountId VARCHAR(8),version INTEGER,data TEXT,path VARCHAR(255),role VARCHAR(255),createdAt DATETIME,updatedAt DATETIME);
         CREATE TABLE Thread (id VARCHAR(42) PRIMARY KEY,accountId VARCHAR(8),version INTEGER,data TEXT,gThrId VARCHAR(20),subject VARCHAR(500),snippet VARCHAR(255),unread INTEGER,starred INTEGER,firstMessageTimestamp DATETIME,lastMessageTimestamp DATETIME,lastMessageReceivedTimestamp DATETIME,lastMessageSentTimestamp DATETIME,inAllMail TINYINT(1),isSearchIndexed TINYINT(1),participants TEXT,hasAttachments INTEGER);
         CREATE TABLE Message (id VARCHAR(40) PRIMARY KEY,accountId VARCHAR(8),version INTEGER,data TEXT,headerMessageId VARCHAR(255),gMsgId VARCHAR(255),gThrId VARCHAR(255),subject VARCHAR(500),date DATETIME,draft TINYINT(1),unread TINYINT(1),starred TINYINT(1),remoteUID INTEGER,remoteXGMLabels TEXT,remoteFolderId VARCHAR(40),replyToHeaderMessageId VARCHAR(255),threadId VARCHAR(40));
         CREATE TABLE `ModelPluginMetadata` (id VARCHAR(40), `accountId` VARCHAR(8), `objectType` VARCHAR(15), `value` TEXT, `expiration` DATETIME, PRIMARY KEY (`value`, `id`));
         CREATE TABLE `DetatchedPluginMetadata` (objectId VARCHAR(40), objectType VARCHAR(15), accountId VARCHAR(8), pluginId VARCHAR(40), value BLOB, version INTEGER, PRIMARY KEY (`objectId`, `accountId`, `pluginId`));
         CREATE TABLE `MessageBody` (id VARCHAR(40) PRIMARY KEY, `value` TEXT, fetchedAt DATETIME);
         CREATE TABLE `File` (id VARCHAR(40) PRIMARY KEY, version INTEGER, data BLOB, accountId VARCHAR(8), filename TEXT);
         PRAGMA user_version = 9;",
    )
    .unwrap();
    let drafts = legacy_folder_id("Drafts");
    conn.execute(
        "INSERT INTO Folder (id, accountId, version, data, path, role) VALUES (?,?,1,'{}','Drafts','drafts')",
        params![drafts, ACCOUNT],
    )
    .unwrap();
    let mut draft = draft_json(json!({}));
    let obj = draft.as_object_mut().unwrap();
    obj.remove("folders");
    obj.insert("folder".into(), json!({"id": drafts, "path": "Drafts"}));
    obj.insert(
        "remoteFolder".into(),
        json!({"id": drafts, "path": "Drafts"}),
    );
    obj.insert("remoteUID".into(), json!(0));
    conn.execute(
        "INSERT INTO Message (id, accountId, version, data, headerMessageId, draft, remoteUID, remoteFolderId, threadId) VALUES (?,?,2,?,?,1,0,?,'')",
        params![DRAFT_ID, ACCOUNT, draft.to_string(), DRAFT_HMSGID, drafts],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) VALUES (?, ?, 'Message', 'send-later', 1900000200)",
        [DRAFT_ID, ACCOUNT],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO MessageBody (id, value) VALUES (?, '<p>draft</p>')",
        [DRAFT_ID],
    )
    .unwrap();
}

struct Legacy {
    dir: tempfile::TempDir,
}

/// A config dir holding a C++ database at `airpost.db` (written by `write`), where the desktop's
/// Import from Mailspring puts it, and `files/` with the draft's attachment and a stray body
/// file.
fn legacy(write: fn(&Connection), keep_wal: bool) -> Legacy {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("airpost.db")).unwrap();
    let _: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .unwrap();
    if keep_wal {
        conn.execute_batch("PRAGMA wal_autocheckpoint = 0").unwrap();
    }
    write(&conn);
    if keep_wal {
        // Never closed, so the frames stay in airpost.db-wal as a killed engine leaves them.
        std::mem::forget(conn);
    } else {
        drop(conn);
    }
    let file_dir = dir
        .path()
        .join("files")
        .join("dr")
        .join("af")
        .join("draftfile01");
    fs::create_dir_all(&file_dir).unwrap();
    fs::write(file_dir.join("notes.txt"), "notes").unwrap();
    let stray = dir
        .path()
        .join("files")
        .join("zz")
        .join("zz")
        .join("zzbody");
    fs::create_dir_all(&stray).unwrap();
    fs::write(stray.join("inline.png"), "png").unwrap();
    Legacy { dir }
}

fn migrate(dir: &Path) -> String {
    let mut out = Vec::new();
    airpost_store::migrate::migrate(dir, &ManualClock::new(1_700_000_000), &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn import(dir: &Path) -> ImportOutcome {
    import_legacy(dir, DeltaStream::new(Box::new(std::io::sink()))).unwrap()
}

fn fixture_in(legacy: Legacy) -> Fixture {
    let clock = Arc::new(ManualClock::new(1_700_000_000));
    let sink = Sink::default();
    let stream = DeltaStream::new(Box::new(sink.clone()));
    let store = Store::open(legacy.dir.path(), stream.clone()).unwrap();
    store.set_clock(clock.clone());
    Fixture {
        dir: legacy.dir,
        store,
        sink,
        stream,
        clock,
    }
}

fn count(f: &Fixture, sql: &str) -> i64 {
    f.store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn plugin_version(data: &Value, plugin: &str) -> Option<i64> {
    data["metadata"]
        .as_array()?
        .iter()
        .find(|m| m["pluginId"] == plugin)
        .and_then(|m| m["v"].as_i64())
}

/// The legacy database's files, under their names before the import renamed them.
fn file_hashes(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("airpost.legacy")
        })
        .map(|e| {
            (
                e.file_name()
                    .to_string_lossy()
                    .replace(LEGACY_IMPORTED_DATABASE_FILE, "airpost.legacy.db"),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

fn assert_draft_imported(f: &Fixture) {
    let draft = f.load(DRAFT_ID);
    assert!(draft.is_draft());
    assert_eq!(draft.thread_id(), "");
    assert_eq!(plugin_version(draft.json(), "send-later"), Some(3));
    assert!(draft.json().get("folder").is_none() && draft.json().get("remoteUID").is_none());
    let placements = f.store.placements_for_message(DRAFT_ID).unwrap();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].remote_uid, 0);
    assert_eq!(placements[0].folder_id, legacy_folder_id("Drafts"));
    assert_eq!(
        count(
            f,
            "SELECT count(*) FROM ModelPluginMetadata WHERE value = 'send-later' AND expiration = 1900000200"
        ),
        1
    );
    let body: String = f
        .store
        .conn()
        .query_row(
            "SELECT value FROM MessageBody WHERE id = ?",
            [DRAFT_ID],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, "<p>draft</p>");
    let file = f.dir.path().join("files/dr/af/draftfile01/notes.txt");
    assert_eq!(fs::read_to_string(file).unwrap(), "notes");
    assert!(!f.dir.path().join(LEGACY_FILES_DIR).exists());
}

#[test]
fn migrate_sets_draft_files_aside_and_import_restores_them() {
    let l = legacy(write_v12, true);
    let out = migrate(l.dir.path());
    assert!(out.contains("Running Migration"), "{out}");
    let files: Vec<_> = walk(&l.dir.path().join("files"));
    assert!(files.is_empty(), "files/ still holds {files:?}");
    assert!(l
        .dir
        .path()
        .join(LEGACY_FILES_DIR)
        .join("dr/af/draftfile01/notes.txt")
        .is_file());

    let before = file_hashes(l.dir.path());
    let ImportOutcome::Imported(counts) = import(l.dir.path()) else {
        panic!("not imported");
    };
    assert_eq!(
        file_hashes(l.dir.path()),
        before,
        "the legacy database changed"
    );
    assert!(!l.dir.path().join("airpost.legacy.db").exists());
    assert!(l.dir.path().join(LEGACY_IMPORTED_DATABASE_FILE).is_file());
    assert_eq!(counts.drafts, 1);
    assert_eq!(counts.files, 1);
    assert_eq!(counts.metadata_pending, 2);
    assert_eq!(counts.state_keys, 1);

    let f = fixture_in(l);
    assert_draft_imported(&f);
    assert_eq!(
        f.store.get_key_value("cursor-acc1").unwrap().as_deref(),
        Some("42")
    );
    assert_eq!(
        f.store.get_key_value("gsynctoken-contacts-acc1").unwrap(),
        None
    );
    assert_ne!(
        f.store.get_key_value("VACUUM_TIME").unwrap().as_deref(),
        Some("1")
    );
    assert_eq!(count(&f, "SELECT count(*) FROM Message"), 1);
    assert_eq!(
        count(
            &f,
            &format!(
                "SELECT count(*) FROM DetatchedPluginMetadata WHERE objectId = '{}'",
                pending_message_key("m3@example.test")
            )
        ),
        1
    );
    // One pending row per message of the snoozed thread.
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM DetatchedPluginMetadata WHERE objectType = 'Thread'"
        ),
        2
    );
    f.assert_invariants();
}

#[test]
fn pending_metadata_attaches_as_sync_ingests_the_messages() {
    let l = legacy(write_v12, false);
    migrate(l.dir.path());
    import(l.dir.path());
    let f = fixture_in(l);
    let inbox = f.folder("INBOX", "inbox");

    let m3 = f.insert("m3@example.test", &inbox, 3, attrs(3), None);
    let m3 = f.load(m3.id());
    assert_eq!(plugin_version(m3.json(), "send-reminders"), Some(4));
    assert_eq!(plugin_version(m3.json(), "cleared"), None);
    assert_eq!(
        count(
            &f,
            &format!(
                "SELECT count(*) FROM ModelPluginMetadata WHERE id = '{}' AND expiration = 1900000100",
                m3.id()
            )
        ),
        1
    );

    // The second message of the snoozed thread arrives first; its thread takes the snooze
    // and the row left under the other message goes.
    let m2 = f.insert("m2@example.test", &inbox, 2, attrs(2), None);
    let thread = f.thread(f.load(m2.id()).thread_id()).unwrap();
    assert_eq!(plugin_version(thread.json(), "thread-snooze"), Some(2));
    assert_eq!(
        count(
            &f,
            &format!(
                "SELECT count(*) FROM ModelPluginMetadata WHERE id = '{}' AND objectType = 'Thread' AND expiration = 1900000000",
                thread.id()
            )
        ),
        1
    );
    assert_eq!(count(&f, "SELECT count(*) FROM DetatchedPluginMetadata"), 0);

    // Unsnoozed by the client, then the thread's other message arrives in it: nothing old
    // comes back.
    let mut t = thread;
    t.upsert_metadata("thread-snooze", &json!({}), None);
    f.store.save(&mut t).unwrap();
    let m1 = f.insert("m1@example.test", &inbox, 1, attrs(1), Some(t.id()));
    let t = f.thread(f.load(m1.id()).thread_id()).unwrap();
    assert_eq!(t.json()["metadata"][0]["value"], json!({}));
    assert_eq!(plugin_version(t.json(), "thread-snooze"), Some(3));
    f.assert_invariants();
}

#[test]
fn import_attaches_to_messages_already_synced() {
    let l = legacy(write_v12, false);
    migrate(l.dir.path());
    let f = fixture_in(l);
    let inbox = f.folder("INBOX", "inbox");
    // The server's Drafts folder has another name now.
    let drafts = f.folder("Brouillons", "drafts");
    let m1 = f.insert("m1@example.test", &inbox, 1, attrs(1), None);
    let m3 = f.insert("m3@example.test", &inbox, 3, attrs(3), None);

    let ImportOutcome::Imported(counts) = import(f.dir.path()) else {
        panic!("not imported");
    };
    assert_eq!(counts.metadata_attached, 2);
    assert_eq!(counts.metadata_pending, 0);
    let t = f.thread(f.load(m1.id()).thread_id()).unwrap();
    assert_eq!(plugin_version(t.json(), "thread-snooze"), Some(2));
    assert_eq!(
        plugin_version(f.load(m3.id()).json(), "send-reminders"),
        Some(4)
    );
    assert_eq!(count(&f, "SELECT count(*) FROM DetatchedPluginMetadata"), 0);
    // The two attached here and the draft's send-later.
    assert_eq!(count(&f, "SELECT count(*) FROM ModelPluginMetadata"), 3);
    let placements = f.store.placements_for_message(DRAFT_ID).unwrap();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].folder_id, drafts.id());
    assert!(f
        .store
        .find::<airpost_store::Folder>(&Query::new().equal("path", "Drafts"))
        .unwrap()
        .is_none());
    f.assert_invariants();
}

#[test]
fn a_second_import_changes_nothing() {
    let l = legacy(write_v12, false);
    migrate(l.dir.path());
    assert!(matches!(import(l.dir.path()), ImportOutcome::Imported(_)));
    let f = fixture_in(l);
    // The user deletes the imported draft; running the import again must not bring it back.
    let mut draft = f.load(DRAFT_ID);
    f.store.remove(&mut draft).unwrap();
    let pending = count(&f, "SELECT count(*) FROM DetatchedPluginMetadata");

    assert_eq!(import(f.dir.path()), ImportOutcome::NoLegacyDatabase);
    // A legacy database left in place (its rename failed) is recognised as imported.
    let dir = f.dir.path();
    fs::copy(
        dir.join(LEGACY_IMPORTED_DATABASE_FILE),
        dir.join("airpost.legacy.db"),
    )
    .unwrap();
    assert_eq!(import(dir), ImportOutcome::AlreadyImported);
    assert!(!dir.join("airpost.legacy.db").exists());
    assert_eq!(count(&f, "SELECT count(*) FROM Message"), 0);
    assert_eq!(
        count(&f, "SELECT count(*) FROM DetatchedPluginMetadata"),
        pending
    );
    assert!(f.store.get_key_value(LEGACY_IMPORT_KEY).unwrap().is_some());

    // A later launch's migrate sees its own database and leaves everything alone.
    let out = migrate(dir);
    assert!(!out.contains("Running Migration"), "{out}");
    assert_eq!(import(dir), ImportOutcome::NoLegacyDatabase);
}

#[test]
fn a_recreated_database_does_not_import_again() {
    let l = legacy(write_v12, false);
    migrate(l.dir.path());
    assert!(matches!(import(l.dir.path()), ImportOutcome::Imported(_)));
    // The user deletes airpost.db* to force a resync.
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(l.dir.path().join(format!("airpost.db{suffix}")));
    }
    let out = migrate(l.dir.path());
    assert!(out.contains("Running Setup"), "{out}");
    assert_eq!(import(l.dir.path()), ImportOutcome::NoLegacyDatabase);
    let f = fixture_in(l);
    assert_eq!(count(&f, "SELECT count(*) FROM Message"), 0);
    assert_eq!(count(&f, "SELECT count(*) FROM DetatchedPluginMetadata"), 0);
}

#[test]
fn a_reply_draft_joins_the_thread_of_its_message_when_it_syncs() {
    let l = legacy(write_v12_with_reply, false);
    migrate(l.dir.path());
    let ImportOutcome::Imported(counts) = import(l.dir.path()) else {
        panic!("not imported");
    };
    assert_eq!(counts.drafts, 2);
    let f = fixture_in(l);
    assert_eq!(f.load(REPLY_ID).thread_id(), "");

    let inbox = f.folder("INBOX", "inbox");
    let m3 = f.insert("m3@example.test", &inbox, 3, attrs(3), None);
    let thread_id = f.load(m3.id()).thread_id().to_owned();
    assert_eq!(f.load(REPLY_ID).thread_id(), thread_id);
    let thread = f.thread(&thread_id).unwrap();
    let drafts = legacy_folder_id("Drafts");
    assert!(
        thread
            .folders()
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == drafts),
        "{:?}",
        thread.folders()
    );
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM DetatchedPluginMetadata WHERE objectType = 'DraftThread'"
        ),
        0
    );
    f.assert_invariants();
}

#[test]
fn a_reply_draft_joins_a_thread_already_synced() {
    let l = legacy(write_v12_with_reply, false);
    migrate(l.dir.path());
    let f = fixture_in(l);
    let inbox = f.folder("INBOX", "inbox");
    f.folder("Drafts", "drafts");
    let m3 = f.insert("m3@example.test", &inbox, 3, attrs(3), None);
    assert!(matches!(import(f.dir.path()), ImportOutcome::Imported(_)));
    let thread_id = f.load(m3.id()).thread_id().to_owned();
    assert_eq!(f.load(REPLY_ID).thread_id(), thread_id);
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM DetatchedPluginMetadata WHERE objectType = 'DraftThread'"
        ),
        0
    );
    f.assert_invariants();
}

#[test]
fn import_skips_a_draft_the_new_database_already_has() {
    let l = legacy(write_v12, false);
    migrate(l.dir.path());
    let f = fixture_in(l);
    let drafts = f.folder("Drafts", "drafts");
    let mut existing = Message::new(json!({"id": "other-id", "aid": ACCOUNT, "v": 0,
                                           "hMsgId": DRAFT_HMSGID, "draft": true, "threadId": ""}));
    let a = MessageAttributes {
        draft: true,
        ..attrs(0)
    };
    f.store
        .upsert_placement(&mut existing, &drafts, 0, &a)
        .unwrap();
    f.store.save(&mut existing).unwrap();

    let ImportOutcome::Imported(counts) = import(f.dir.path()) else {
        panic!("not imported");
    };
    assert_eq!((counts.drafts, counts.drafts_skipped), (0, 1));
    assert!(f
        .store
        .find::<Message>(&Query::new().equal("id", DRAFT_ID))
        .unwrap()
        .is_none());
}

#[test]
fn pre_v10_database() {
    let l = legacy(write_v9, false);
    migrate(l.dir.path());
    let before = file_hashes(l.dir.path());
    let ImportOutcome::Imported(counts) = import(l.dir.path()) else {
        panic!("not imported");
    };
    // Read-only: no -wal or -shm appears next to a database closed cleanly.
    assert_eq!(file_hashes(l.dir.path()), before);
    assert_eq!((counts.drafts, counts.files), (1, 1));
    let f = fixture_in(l);
    assert_draft_imported(&f);
    // The Drafts folder is created for the placement, unsynced.
    let folder = f
        .store
        .folder_by_id(ACCOUNT, &legacy_folder_id("Drafts"))
        .unwrap()
        .unwrap();
    assert_eq!(folder.role(), "drafts");
    assert!(folder.local_status().is_null());
    f.assert_invariants();
}

#[test]
fn nothing_to_import_without_a_legacy_database() {
    let dir = tempfile::tempdir().unwrap();
    migrate(dir.path());
    assert_eq!(import(dir.path()), ImportOutcome::NoLegacyDatabase);
    let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    assert_eq!(store.get_key_value(LEGACY_IMPORT_KEY).unwrap(), None);
    assert_eq!(store.count::<Thread>(&Query::new()).unwrap(), 0);
}

#[test]
fn import_before_migrate_fails() {
    let l = legacy(write_v12, false);
    fs::rename(
        l.dir.path().join("airpost.db"),
        l.dir.path().join("airpost.legacy.db"),
    )
    .unwrap();
    let err = import_legacy(l.dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap_err();
    assert!(err.to_string().contains("--mode migrate"), "{err}");
    assert!(!l.dir.path().join("airpost.db").exists());
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_type().unwrap().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path().to_string_lossy().into_owned());
            }
        }
    }
    out
}
