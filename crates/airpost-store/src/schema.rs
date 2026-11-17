// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The V12 schema and connection setup.
//!
//! [`V12_SCHEMA`] is the final schema the C++ engine reaches on a fresh database, replayed
//! from its V1, V3, V4, V6, V7, V8, V9 and V10 setup and index queries (`constants.h`) and
//! read back from `sqlite_master`. The text is SQLite's stored form, so a database created
//! here has the same `sqlite_master` as one created by the C++ engine (Mailspring-Sync,
//! Copyright © 2017 Foundry 376, GPL-3.0; see NOTICE.md). `ALTER TABLE ADD COLUMN` appended
//! columns to the stored CREATE TABLE text, which is why some columns sit after the closing
//! column list in the original order. V6 dropped and recreated `Event`, so the V1
//! `EventIsSearchIndexedIndex` no longer exists.
//!
//! Regenerate or check with `apps/engine/tools/schema_check.py` (given upstream
//! Mailspring-Sync's `MailSync/constants.h`); `tests/schema.rs` compares a migrated database
//! against `tests/fixtures/v12_schema.json` plus what [`MIGRATIONS`] add
//! (`tests/fixtures/migrations.json`).

use std::path::Path;
use std::time::Duration;

use airpost_core::constants::{
    BASE_SCHEMA_VERSION, DATABASE_FILE, SCHEMA_VERSION, SQLITE_BUSY_TIMEOUT_MS,
};
use airpost_core::Result;
use rusqlite::{Connection, OpenFlags};

/// Statements in the order the C++ engine created the objects. FTS5 shadow tables and
/// `sqlite_autoindex_*` are created by SQLite as a side effect.
pub const V12_SCHEMA: &[&str] = &[
    "CREATE TABLE `_State` (id VARCHAR(40) PRIMARY KEY, value TEXT)",
    "CREATE TABLE `File` (id VARCHAR(40) PRIMARY KEY, version INTEGER, data BLOB, accountId VARCHAR(8), filename TEXT)",
    "CREATE VIRTUAL TABLE `EventSearch` USING fts5(tokenize = 'porter unicode61', content_id UNINDEXED, title, description, location, participants)",
    "CREATE TABLE Label (id VARCHAR(40) PRIMARY KEY,accountId VARCHAR(8),version INTEGER,data TEXT,path VARCHAR(255),role VARCHAR(255),createdAt DATETIME,updatedAt DATETIME)",
    "CREATE TABLE Folder (id VARCHAR(40) PRIMARY KEY,accountId VARCHAR(8),version INTEGER,data TEXT,path VARCHAR(255),role VARCHAR(255),createdAt DATETIME,updatedAt DATETIME)",
    "CREATE TABLE Thread (id VARCHAR(42) PRIMARY KEY,accountId VARCHAR(8),version INTEGER,data TEXT,gThrId VARCHAR(20),subject VARCHAR(500),snippet VARCHAR(255),unread INTEGER,starred INTEGER,firstMessageTimestamp DATETIME,lastMessageTimestamp DATETIME,lastMessageReceivedTimestamp DATETIME,lastMessageSentTimestamp DATETIME,inAllMail TINYINT(1),isSearchIndexed TINYINT(1),participants TEXT,hasAttachments INTEGER)",
    "CREATE INDEX ThreadDateIndex ON `Thread` (lastMessageReceivedTimestamp DESC)",
    "CREATE INDEX ThreadUnreadIndex ON `Thread` (accountId, lastMessageReceivedTimestamp DESC) WHERE unread = 1 AND inAllMail = 1",
    "CREATE INDEX ThreadUnifiedUnreadIndex ON `Thread` (lastMessageReceivedTimestamp DESC) WHERE unread = 1 AND inAllMail = 1",
    "CREATE INDEX ThreadStarredIndex ON `Thread` (accountId, lastMessageReceivedTimestamp DESC) WHERE starred = 1 AND inAllMail = 1",
    "CREATE INDEX ThreadUnifiedStarredIndex ON `Thread` (lastMessageReceivedTimestamp DESC) WHERE starred = 1 AND inAllMail = 1",
    "CREATE INDEX ThreadGmailLookup ON `Thread` (gThrId) WHERE gThrId IS NOT NULL",
    "CREATE INDEX ThreadIsSearchIndexedIndex ON `Thread` (isSearchIndexed, id)",
    "CREATE INDEX ThreadIsSearchIndexedLastMessageReceivedIndex ON `Thread` (isSearchIndexed, lastMessageReceivedTimestamp)",
    "CREATE TABLE ThreadReference (threadId VARCHAR(42),accountId VARCHAR(8),headerMessageId VARCHAR(255),PRIMARY KEY (threadId, accountId, headerMessageId))",
    "CREATE TABLE ThreadCategory (id VARCHAR(40),value VARCHAR(40),inAllMail TINYINT(1),unread TINYINT(1),lastMessageReceivedTimestamp DATETIME,lastMessageSentTimestamp DATETIME,PRIMARY KEY (id, value))",
    "CREATE INDEX `ThreadCategory_id` ON `ThreadCategory` (`id` ASC)",
    "CREATE UNIQUE INDEX `ThreadCategory_val_id` ON `ThreadCategory` (`value` ASC, `id` ASC)",
    "CREATE INDEX ThreadListCategoryIndex ON `ThreadCategory` (lastMessageReceivedTimestamp DESC, value, inAllMail, unread, id)",
    "CREATE INDEX ThreadListCategorySentIndex ON `ThreadCategory` (lastMessageSentTimestamp DESC, value, inAllMail, unread, id)",
    "CREATE TABLE `ThreadCounts` (`categoryId` TEXT PRIMARY KEY, `unread` INTEGER, `total` INTEGER)",
    "CREATE VIRTUAL TABLE `ThreadSearch` USING fts5(tokenize = 'porter unicode61', content_id UNINDEXED, subject, to_, from_, categories, body)",
    "CREATE TABLE `Account` (id VARCHAR(40) PRIMARY KEY, data BLOB, accountId VARCHAR(8), email_address TEXT)",
    "CREATE TABLE Message (id VARCHAR(40) PRIMARY KEY,accountId VARCHAR(8),version INTEGER,data TEXT,headerMessageId VARCHAR(255),gMsgId VARCHAR(255),gThrId VARCHAR(255),subject VARCHAR(500),date DATETIME,draft TINYINT(1),unread TINYINT(1),starred TINYINT(1),replyToHeaderMessageId VARCHAR(255),threadId VARCHAR(40))",
    "CREATE INDEX MessageListThreadIndex ON Message(threadId, date ASC)",
    "CREATE INDEX MessageListHeaderMsgIdIndex ON Message(headerMessageId)",
    "CREATE INDEX MessageListDraftIndex ON Message(accountId, date DESC) WHERE draft = 1",
    "CREATE INDEX MessageListUnifiedDraftIndex ON Message(date DESC) WHERE draft = 1",
    "CREATE TABLE `ModelPluginMetadata` (id VARCHAR(40), `accountId` VARCHAR(8), `objectType` VARCHAR(15), `value` TEXT, `expiration` DATETIME, PRIMARY KEY (`value`, `id`))",
    "CREATE INDEX `ModelPluginMetadata_id` ON `ModelPluginMetadata` (`id` ASC)",
    "CREATE INDEX `ModelPluginMetadata_expiration` ON `ModelPluginMetadata` (`expiration` ASC) WHERE expiration IS NOT NULL",
    "CREATE TABLE `DetatchedPluginMetadata` (objectId VARCHAR(40), objectType VARCHAR(15), accountId VARCHAR(8), pluginId VARCHAR(40), value BLOB, version INTEGER, PRIMARY KEY (`objectId`, `accountId`, `pluginId`))",
    "CREATE TABLE `MessageBody` (id VARCHAR(40) PRIMARY KEY, `value` TEXT, fetchedAt DATETIME)",
    "CREATE UNIQUE INDEX MessageBodyIndex ON MessageBody(id)",
    "CREATE TABLE `Contact` (id VARCHAR(40) PRIMARY KEY, data BLOB, accountId VARCHAR(8), email TEXT, version INTEGER, refs INTEGER DEFAULT 0, hidden TINYINT(1) DEFAULT 0, source VARCHAR(10) DEFAULT 'mail', bookId VARCHAR(40), etag VARCHAR(40))",
    "CREATE INDEX ContactEmailIndex ON Contact(email)",
    "CREATE INDEX ContactAccountEmailIndex ON Contact(accountId, email)",
    "CREATE VIRTUAL TABLE `ContactSearch` USING fts5(tokenize = 'porter unicode61', content_id UNINDEXED, content)",
    "CREATE TABLE `Calendar` (id VARCHAR(40) PRIMARY KEY, data BLOB, accountId VARCHAR(8))",
    "CREATE TABLE `Task` (id VARCHAR(40) PRIMARY KEY, version INTEGER, data BLOB, accountId VARCHAR(8), status VARCHAR(255))",
    "CREATE INDEX TaskByStatus ON Task(accountId, status)",
    "CREATE TABLE `Event` (id VARCHAR(40) PRIMARY KEY, data BLOB, accountId VARCHAR(8), etag VARCHAR(40), calendarId VARCHAR(40), recurrenceStart INTEGER, recurrenceEnd INTEGER, icsuid VARCHAR(150), recurrenceId VARCHAR(50) DEFAULT '')",
    "CREATE INDEX EventETag ON Event(calendarId, etag)",
    "CREATE INDEX EventUID ON Event(accountId, icsuid)",
    "CREATE INDEX ContactBrowseIndex ON Contact(hidden,refs,accountId)",
    "CREATE TABLE `ContactGroup` (`id` varchar(40),`accountId` varchar(40),`bookId` varchar(40), `data` BLOB, `version` INTEGER, `name` varchar(300), PRIMARY KEY (id))",
    "CREATE TABLE `ContactContactGroup` (`id` varchar(40),`value` varchar(40), PRIMARY KEY (id, value))",
    "CREATE TABLE `ContactBook` (`id` varchar(40),`accountId` varchar(40), `data` BLOB, `version` INTEGER, PRIMARY KEY (id))",
    "CREATE INDEX EventRecurrenceId ON Event(calendarId, icsuid, recurrenceId)",
    "CREATE TABLE MessageFolder (rowid INTEGER PRIMARY KEY,accountId VARCHAR(8) NOT NULL,messageId VARCHAR(40) NOT NULL,folderId VARCHAR(40) NOT NULL,remoteUID INTEGER NOT NULL,unread TINYINT(1) NOT NULL DEFAULT 0,starred TINYINT(1) NOT NULL DEFAULT 0,draft TINYINT(1) NOT NULL DEFAULT 0,remoteXGMLabels TEXT NOT NULL DEFAULT '[]',pendingFolderId VARCHAR(40) NULL)",
    "CREATE TABLE MessageOrphan (messageId VARCHAR(40) PRIMARY KEY,accountId VARCHAR(8) NOT NULL,since INTEGER NOT NULL)",
    "CREATE UNIQUE INDEX MessageFolderUIDIndex ON MessageFolder (accountId, folderId, remoteUID) WHERE remoteUID > 0",
    "CREATE INDEX MessageFolderMessageIndex ON MessageFolder (messageId)",
    "CREATE INDEX MessageOrphanSinceIndex ON MessageOrphan (accountId, since)",
    "CREATE INDEX MessageListDateIndex ON Message (accountId, date DESC)",
];

/// One schema change on top of [`V12_SCHEMA`]. `sql` runs in the transaction that sets
/// `user_version` to `version`.
#[derive(Clone, Copy, Debug)]
pub struct Migration {
    pub version: i32,
    pub sql: &'static str,
}

/// Airpost's schema changes after V12, in order, one version apart, ending at
/// [`SCHEMA_VERSION`]. A database runs each entry once, so an entry that has shipped is
/// never edited or removed; a change is a new entry, and `tests/fixtures/migrations.json`
/// lists what the entries add to `sqlite_master`. Additive changes are preferred: the desktop
/// reads the database directly and must keep working on the version it was built against.
///
/// - 13: `Store::find_thread_by_references` looks up by account and Message-ID, which the
///   primary key (threadId first) can't serve. References of removed threads match nothing.
/// - 14: server-synced drafts. `DraftTombstone` holds the Message-IDs of drafts sent or
///   deleted here, whose server copies are deleted instead of synced back.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 13,
        sql: "CREATE INDEX ThreadReferenceLookup ON ThreadReference (accountId, headerMessageId); \
              DELETE FROM ThreadReference WHERE threadId NOT IN (SELECT id FROM Thread);",
    },
    Migration {
        version: 14,
        sql: "CREATE TABLE DraftTombstone (accountId VARCHAR(8) NOT NULL, headerMessageId VARCHAR(255) NOT NULL, since INTEGER NOT NULL, PRIMARY KEY (accountId, headerMessageId));",
    },
];

const _: () = assert!(MIGRATIONS[MIGRATIONS.len() - 1].version == SCHEMA_VERSION);

/// `ACCOUNT_RESET_QUERIES` from upstream `constants.h`, each bound to the account id
/// (`reset` mode).
pub const ACCOUNT_RESET_QUERIES: &[&str] = &[
    "DELETE FROM `ThreadCounts` WHERE `categoryId` IN (SELECT id FROM `Folder` WHERE `accountId` = ?)",
    "DELETE FROM `ThreadCounts` WHERE `categoryId` IN (SELECT id FROM `Label` WHERE `accountId` = ?)",
    "DELETE FROM `ThreadCategory` WHERE `id` IN (SELECT id FROM `Thread` WHERE `accountId` = ?)",
    "DELETE FROM `ThreadSearch` WHERE `content_id` IN (SELECT id FROM `Thread` WHERE `accountId` = ?)",
    "DELETE FROM `ThreadReference` WHERE `accountId` = ?",
    "DELETE FROM `Thread` WHERE `accountId` = ?",
    "DELETE FROM `File` WHERE `accountId` = ?",
    "DELETE FROM `Event` WHERE `accountId` = ?",
    "DELETE FROM `Label` WHERE `accountId` = ?",
    "DELETE FROM `MessageBody` WHERE `id` IN (SELECT id FROM `Message` WHERE `accountId` = ?)",
    "DELETE FROM `MessageFolder` WHERE `accountId` = ?",
    "DELETE FROM `MessageOrphan` WHERE `accountId` = ?",
    "DELETE FROM `DraftTombstone` WHERE `accountId` = ?",
    "DELETE FROM `Message` WHERE `accountId` = ?",
    "DELETE FROM `Task` WHERE `accountId` = ?",
    "DELETE FROM `Folder` WHERE `accountId` = ?",
    "DELETE FROM `ContactSearch` WHERE `content_id` IN (SELECT id FROM `Contact` WHERE `accountId` = ?)",
    "DELETE FROM `Contact` WHERE `accountId` = ?",
    "DELETE FROM `Calendar` WHERE `accountId` = ?",
    "DELETE FROM `ModelPluginMetadata` WHERE `accountId` = ?",
    "DELETE FROM `DetatchedPluginMetadata` WHERE `accountId` = ?",
    "DELETE FROM `Account` WHERE `id` = ?",
];

/// Opens `$CONFIG_DIR_PATH/airpost.db` with the connection settings every C++ `MailStore`
/// applied: 60 s busy timeout, WAL, `page_size` 4096, `cache_size` 10000, `synchronous`
/// NORMAL. These are per connection, so they are set on every open. `journal_size_limit`
/// cuts the WAL back to 64 MB after each checkpoint instead of keeping the size the largest
/// write pass gave it.
pub fn open_connection(config_dir: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        config_dir.join(DATABASE_FILE),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))?;
    // journal_mode returns a row, so it cannot go through execute_batch's step-to-done.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.execute_batch(
        "PRAGMA main.page_size = 4096; PRAGMA main.cache_size = 10000; PRAGMA main.synchronous = NORMAL; PRAGMA main.journal_size_limit = 67108864;",
    )?;
    Ok(conn)
}

/// Pages [`compact`] frees per call: 8 MB at the 4 KB page size.
pub const INCREMENTAL_VACUUM_PAGES: u32 = 2048;

/// Frees up to [`INCREMENTAL_VACUUM_PAGES`] unused pages (a no-op on a database without
/// incremental auto-vacuum) and checkpoints the WAL, truncating it. Neither is needed for
/// correctness, so failures (a reader holding the WAL, a busy database) are only logged.
pub fn compact(conn: &Connection) {
    let freed = conn.execute_batch(&format!(
        "PRAGMA main.incremental_vacuum({INCREMENTAL_VACUUM_PAGES});"
    ));
    if let Err(e) = freed {
        tracing::warn!("-X Incremental vacuum failed: {e}");
    }
    let checkpoint = conn.query_row("PRAGMA main.wal_checkpoint(TRUNCATE)", [], |r| {
        r.get::<_, i64>(0)
    });
    if let Err(e) = checkpoint {
        tracing::warn!("-X WAL checkpoint failed: {e}");
    }
}

/// Points SQLite's temp files at `CONFIG_DIR_PATH` (the C++ set `sqlite3_temp_directory`),
/// so large sorts and VACUUM stay on the volume the user chose. The setting is process-wide;
/// the bundled SQLite guards it with its own mutex, and a directory that no longer exists
/// is skipped in favour of the system temp dir, so setting it again from another
/// connection is safe.
pub fn set_temp_directory(conn: &Connection, config_dir: &Path) -> Result<()> {
    // The pragma is deprecated but still compiled into the bundled SQLite, and it is the only
    // way to set the global without unsafe FFI.
    let dir = config_dir.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!("PRAGMA temp_store_directory = '{dir}';"))?;
    Ok(())
}

/// Creates the V12 schema on an empty database and sets `user_version` to 12; `migrate` then
/// applies [`MIGRATIONS`].
pub fn create_v12(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for sql in V12_SCHEMA {
        tx.execute_batch(sql)?;
    }
    tx.pragma_update(None, "user_version", BASE_SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}
