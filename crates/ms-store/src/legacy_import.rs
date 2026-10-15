//! `--mode import-legacy` (docs/engine-port-plan.md section 9): carries what only the C++
//! engine's database held into the fresh one, from the copy `migrate` moved aside.
//!
//! Reads `MailSync/MailStore.cpp`'s schema (V1 through V12, before and after the V10
//! placements migration in `MailSync/constants.h`) and `MailModel::toJSON`'s model JSON,
//! from Mailspring-Sync, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! What is imported:
//! - Plugin metadata of synced messages and threads. Message ids differ between the engines
//!   (D4), so it is stored in `DetatchedPluginMetadata` under the message's Message-ID and
//!   attached when sync ingests a message with that Message-ID (or at once, to messages the
//!   new database already has). Thread metadata is keyed through each message of the legacy
//!   thread; the first of them to arrive attaches it to its thread and drops the others.
//! - Local drafts (UID-0 placements only, never on the server) with their body, their
//!   File rows, their files and their own metadata, under their legacy ids.
//! - `cursor-<accountId>`, the metadata stream position. People sync tokens are not carried:
//!   contacts are not imported, so the first contacts pass must be a full one.
//!
//! The legacy database is opened read-only and never written. The import runs once per
//! database: the `_State` key [`LEGACY_IMPORT_KEY`] records it in the same transaction as
//! the rows, so a second run (or a run after the user deleted an imported draft) changes
//! nothing. After the commit the legacy database is renamed to
//! [`LEGACY_IMPORTED_DATABASE_FILE`], so a new database created later without a new legacy
//! move (the user deleted `edgehill.db*` to resync) does not import it again.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ms_core::constants::{
    DATABASE_FILE, FILES_DIR, LEGACY_DATABASE_FILE, SCHEMA_VERSION, STATE_ENGINE_KEY,
    STATE_ENGINE_RUST,
};
use ms_core::{Error, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use tracing::{info, warn};

use crate::model::{json_i64, json_size, json_str};
use crate::placement::MessageAttributes;
use crate::{DeltaStream, File, Folder, Message, Model, Query, Store, Thread};

/// Where `migrate` keeps the files of the legacy database's local drafts while it empties
/// `files/`, until `import-legacy` copies them back. Removed once the import is recorded.
pub const LEGACY_FILES_DIR: &str = "files.legacy";

/// Where the legacy database (with its `-wal` and `-shm`) goes once imported. Kept rather
/// than deleted, for support; an older copy is replaced.
pub const LEGACY_IMPORTED_DATABASE_FILE: &str = "edgehill.legacy-imported.db";

/// `_State` key holding the time `import-legacy` committed. Present means the legacy
/// database has been imported into this one.
pub const LEGACY_IMPORT_KEY: &str = "legacy-import";

const OBJECT_TYPE_MESSAGE: &str = "Message";
const OBJECT_TYPE_THREAD: &str = "Thread";
/// `objectType` of a pending row naming a reply or forward draft that joins the thread of
/// the message it answers once that message is synced.
const OBJECT_TYPE_DRAFT_THREAD: &str = "DraftThread";

/// Separates the Message-ID from the legacy thread id in a pending thread row's `objectId`.
/// Below every printable character, so the rows of one Message-ID form one key range.
const THREAD_KEY_SEPARATOR: char = '\u{1f}';

/// What one run of [`import_legacy`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    NoLegacyDatabase,
    AlreadyImported,
    Imported(ImportCounts),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportCounts {
    pub drafts: usize,
    /// Drafts the new database already has (same id or Message-ID), or whose folder the
    /// legacy database does not name.
    pub drafts_skipped: usize,
    pub files: usize,
    /// Metadata values attached to messages and threads the new database already has.
    pub metadata_attached: usize,
    /// Metadata values left waiting for their messages to be synced.
    pub metadata_pending: usize,
    pub state_keys: usize,
}

/// `objectId` of a pending row for metadata on the message with this Message-ID. Message
/// ids never contain `<`, so it cannot collide with metadata waiting under a model id.
pub fn pending_message_key(header_message_id: &str) -> String {
    format!("<{header_message_id}>")
}

/// `objectId` of a pending row for metadata on the legacy thread `legacy_thread_id`, reached
/// through its message with this Message-ID.
pub fn pending_thread_key(header_message_id: &str, legacy_thread_id: &str) -> String {
    format!("t:<{header_message_id}>{THREAD_KEY_SEPARATOR}{legacy_thread_id}")
}

/// The `objectId` range `[lo, hi)` of every pending thread row for one Message-ID.
fn pending_thread_key_range(header_message_id: &str) -> (String, String) {
    let lo = format!("t:<{header_message_id}>{THREAD_KEY_SEPARATOR}");
    let next = char::from_u32(THREAD_KEY_SEPARATOR as u32 + 1).expect("valid char");
    let hi = format!("t:<{header_message_id}>{next}");
    (lo, hi)
}

/// Runs `--mode import-legacy` against `config_dir`. `deltas` receives the deltas of the
/// rows written (the bin passes a sink: no client listens to a one-shot mode).
pub fn import_legacy(config_dir: &Path, deltas: Arc<DeltaStream>) -> Result<ImportOutcome> {
    let legacy_path = config_dir.join(LEGACY_DATABASE_FILE);
    if !legacy_path.is_file() {
        info!("No legacy database to import.");
        return Ok(ImportOutcome::NoLegacyDatabase);
    }
    if !config_dir.join(DATABASE_FILE).is_file() {
        return Err(Error::other(
            "edgehill.db does not exist; run --mode migrate before --mode import-legacy",
        ));
    }
    let store = Store::open(config_dir, deltas)?;
    check_rust_database(&store)?;
    if let Some(at) = store.get_key_value(LEGACY_IMPORT_KEY)? {
        info!("Legacy database already imported (at {at}).");
        // Only a fresh database lacks the key, and migrate moves a legacy database aside
        // only when it creates one, so this file is the one already imported.
        retire_legacy_database(config_dir);
        return Ok(ImportOutcome::AlreadyImported);
    }

    info!("Importing from {LEGACY_DATABASE_FILE}.");
    let legacy = LegacyDb::open(&legacy_path)?;
    let files_root = config_dir.join(FILES_DIR);
    let set_aside_root = config_dir.join(LEGACY_FILES_DIR);

    let counts = store.transaction("importLegacy", |s| {
        let mut counts = ImportCounts::default();
        let drafts = legacy.local_drafts()?;
        let draft_ids: HashSet<String> = drafts.iter().map(|d| d.id.clone()).collect();
        for draft in &drafts {
            import_draft(s, &legacy, draft, &files_root, &set_aside_root, &mut counts)?;
        }
        import_message_metadata(s, &legacy, &draft_ids, &mut counts)?;
        import_thread_metadata(s, &legacy, &draft_ids, &mut counts)?;
        for (key, value) in legacy.carried_state()? {
            let n = s
                .conn()
                .prepare_cached("INSERT OR IGNORE INTO `_State` (id, value) VALUES (?, ?)")?
                .execute(params![key, value])?;
            counts.state_keys += n;
        }
        s.save_key_value(LEGACY_IMPORT_KEY, &s.now().to_string())?;
        Ok(counts)
    })?;

    info!(
        "- Imported {} drafts ({} skipped) with {} files, {} metadata values attached and {} waiting for their messages, {} state keys.",
        counts.drafts,
        counts.drafts_skipped,
        counts.files,
        counts.metadata_attached,
        counts.metadata_pending,
        counts.state_keys
    );
    drop(legacy);
    if set_aside_root.exists() {
        if let Err(e) = fs::remove_dir_all(&set_aside_root) {
            warn!("-X Could not remove {LEGACY_FILES_DIR}: {e}");
        }
    }
    retire_legacy_database(config_dir);
    Ok(ImportOutcome::Imported(counts))
}

/// Renames `edgehill.legacy.db*` to [`LEGACY_IMPORTED_DATABASE_FILE`]`*`, replacing an older
/// copy. A failure is logged and left for the next launch, which finds the import recorded.
fn retire_legacy_database(config_dir: &Path) {
    let from = config_dir.join(LEGACY_DATABASE_FILE);
    let to = config_dir.join(LEGACY_IMPORTED_DATABASE_FILE);
    let result = (|| -> io::Result<()> {
        for suffix in ["", "-wal", "-shm"] {
            match fs::remove_file(with_suffix(&to, suffix)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        // The main file last, so a partial rename leaves it for the next launch to retire.
        for suffix in ["-wal", "-shm", ""] {
            let source = with_suffix(&from, suffix);
            if source.exists() {
                fs::rename(&source, with_suffix(&to, suffix))?;
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        warn!("-X Could not rename {LEGACY_DATABASE_FILE} to {LEGACY_IMPORTED_DATABASE_FILE}: {e}");
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn check_rust_database(store: &Store) -> Result<()> {
    let version: i32 = store
        .conn()
        .pragma_query_value(None, "user_version", |r| r.get(0))?;
    let engine = store.get_key_value(STATE_ENGINE_KEY)?;
    if version < SCHEMA_VERSION || engine.as_deref() != Some(STATE_ENGINE_RUST) {
        return Err(Error::other(
            "edgehill.db was not created by this engine; run --mode migrate before --mode import-legacy",
        ));
    }
    Ok(())
}

/// Called by `migrate` after it moved a legacy database aside and before it empties
/// `files/`: moves the directories of the local drafts' files to [`LEGACY_FILES_DIR`], so
/// `import-legacy` (a later process) still finds them. Anything set aside by an earlier
/// migration belongs to the legacy database just replaced and is removed first. A legacy
/// database this cannot read has no drafts to keep (`import-legacy` reports why). Returns
/// the number of directories moved.
pub fn set_aside_draft_files(config_dir: &Path) -> Result<usize> {
    let set_aside_root = config_dir.join(LEGACY_FILES_DIR);
    remove_dir_if_exists(&set_aside_root)?;
    let legacy_path = config_dir.join(LEGACY_DATABASE_FILE);
    let drafts = match LegacyDb::open(&legacy_path).and_then(|l| l.local_drafts()) {
        Ok(drafts) => drafts,
        Err(_) => return Ok(0),
    };
    let files_root = config_dir.join(FILES_DIR);
    let mut moved = 0;
    for draft in drafts {
        for file_id in draft_file_ids(&draft.data) {
            let rel = file_dir_rel(&file_id);
            let from = files_root.join(&rel);
            if !from.is_dir() {
                continue;
            }
            let to = set_aside_root.join(&rel);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&from, &to)?;
            moved += 1;
        }
    }
    Ok(moved)
}

fn remove_dir_if_exists(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// `<id[0:2]>/<id[2:4]>/<id>` with the id lowercased, as [`File::dir_in`].
fn file_dir_rel(file_id: &str) -> PathBuf {
    let id = file_id.to_lowercase();
    PathBuf::from(id.get(0..2).unwrap_or(&id))
        .join(id.get(2..4).unwrap_or(""))
        .join(&id)
}

fn draft_file_ids(data: &Value) -> Vec<String> {
    data.get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .map(|f| json_str(f, "id").to_owned())
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

// -- Drafts ---------------------------------------------------------------------------------

fn import_draft(
    s: &Store,
    legacy: &LegacyDb,
    draft: &LegacyDraft,
    files_root: &Path,
    set_aside_root: &Path,
    counts: &mut ImportCounts,
) -> Result<()> {
    let hmsgid = json_str(&draft.data, "hMsgId").to_owned();
    let exists = s.count::<Message>(&Query::new().equal("id", &draft.id))? > 0
        || (!hmsgid.is_empty()
            && s.count::<Message>(
                &Query::new()
                    .equal("accountId", &draft.account_id)
                    .equal("headerMessageId", &hmsgid),
            )? > 0);
    if exists {
        counts.drafts_skipped += 1;
        return Ok(());
    }
    let Some(mut folder) = draft_folder(s, legacy, draft)? else {
        warn!(
            "-X Skipping draft {}: no folder in this database matches its folder {}",
            draft.id, draft.folder_id
        );
        counts.drafts_skipped += 1;
        return Ok(());
    };
    if folder.version() == 0 {
        s.save(&mut folder)?;
    }

    let mut data = draft.data.clone();
    let metadata = take_metadata(&mut data);
    let (thread_id, answers) = draft_thread(s, &draft.account_id, &data)?;
    if let Some(obj) = data.as_object_mut() {
        // The snapshot is rebuilt from the placement below. Pre-V10 rows kept the location
        // in these keys.
        for key in [
            "folders",
            "folder",
            "remoteFolder",
            "remoteUID",
            "remoteFolderId",
        ] {
            obj.remove(key);
        }
        obj.insert("v".into(), Value::from(0));
        obj.insert("threadId".into(), Value::from(thread_id.as_str()));
        obj.insert("draft".into(), Value::from(true));
    }
    if thread_id.is_empty() && !answers.is_empty() {
        save_pending_draft_thread(s, &answers, &draft.id, &draft.account_id)?;
    }
    let mut msg = Message::new(data);
    for m in &metadata {
        msg.upsert_metadata(&m.plugin_id, &m.value, Some(m.version));
    }
    let attrs = MessageAttributes {
        uid: 0,
        unread: msg.is_unread(),
        starred: msg.is_starred(),
        draft: true,
        labels: Vec::new(),
        message_id: String::new(),
    };
    s.upsert_placement(&mut msg, &folder, 0, &attrs)?;
    s.save(&mut msg)?;

    if let Some((value, fetched_at)) = legacy.body(&draft.id)? {
        s.conn()
            .prepare_cached("REPLACE INTO MessageBody (id, value, fetchedAt) VALUES (?, ?, ?)")?
            .execute(params![draft.id, value, fetched_at])?;
    }

    for file_id in draft_file_ids(&draft.data) {
        if let Some(mut data) = legacy.file_row(&file_id)? {
            if s.count::<File>(&Query::new().equal("id", &file_id))? == 0 {
                data["v"] = Value::from(0);
                s.save(&mut File::from_json(data)?)?;
            }
        }
        let rel = file_dir_rel(&file_id);
        let from = set_aside_root.join(&rel);
        if from.is_dir() {
            counts.files += copy_dir(&from, &files_root.join(&rel))?;
        } else if !files_root.join(&rel).is_dir() {
            warn!(
                "-X The files of draft {} ({file_id}) were not kept by migrate",
                draft.id
            );
        }
    }
    counts.drafts += 1;
    Ok(())
}

/// The thread a reply or forward draft belongs to here, and the Message-ID it answers. The
/// client keeps `threadId` on such drafts (draft-factory.ts) and finds an open reply by it,
/// but the legacy thread id names nothing in this database (D4): the draft joins the thread
/// of the message it answers, now if that message is synced, otherwise when it is (the
/// returned Message-ID is non-empty and the thread id empty).
fn draft_thread(s: &Store, account_id: &str, data: &Value) -> Result<(String, String)> {
    let legacy_thread = json_str(data, "threadId");
    if legacy_thread.is_empty() {
        return Ok((String::new(), String::new()));
    }
    if s.count::<Thread>(&Query::new().equal("id", legacy_thread))? > 0 {
        return Ok((legacy_thread.to_owned(), String::new()));
    }
    let answers = [json_str(data, "rthMsgId"), json_str(data, "fwdMsgId")]
        .into_iter()
        .find(|h| !h.is_empty())
        .unwrap_or("")
        .to_owned();
    if answers.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let thread_id = messages_with_header_message_id(s, account_id, &answers)?
        .into_iter()
        .map(|m| m.thread_id().to_owned())
        .find(|t| !t.is_empty())
        .unwrap_or_default();
    Ok((thread_id, answers))
}

fn save_pending_draft_thread(
    s: &Store,
    answers: &str,
    draft_id: &str,
    account_id: &str,
) -> Result<()> {
    s.conn()
        .prepare_cached(
            "REPLACE INTO DetatchedPluginMetadata (objectId, objectType, accountId, pluginId, value, version) VALUES (?,?,?,'','null',0)",
        )?
        .execute(params![
            pending_thread_key(answers, draft_id),
            OBJECT_TYPE_DRAFT_THREAD,
            account_id
        ])?;
    Ok(())
}

/// The folder of the draft's UID-0 placement in this database, by path: folder ids are
/// `accountId:path` hashes in both engines, but the path is what the server listing will
/// match. Once sync has listed the account's folders and that path is not among them, the
/// folder holding the same role takes the draft, since a folder the server does not list
/// is deleted with its placements on the next pass. Before the first listing the folder is
/// returned unsaved, with an empty `localStatus`, which the first pass treats as a folder
/// seen for the first time.
fn draft_folder(s: &Store, legacy: &LegacyDb, draft: &LegacyDraft) -> Result<Option<Folder>> {
    let Some((path, role)) = legacy.folder(&draft.folder_id)? else {
        return Ok(None);
    };
    let id = ms_core::ids::folder_id(&draft.account_id, &path);
    if let Some(f) = s.find::<Folder>(&Query::new().equal("id", &id))? {
        return Ok(Some(f));
    }
    let account = Query::new().equal("accountId", &draft.account_id);
    if s.count::<Folder>(&account)? > 0 {
        if role.is_empty() {
            return Ok(None);
        }
        return s.find::<Folder>(&account.equal("role", &role));
    }
    let mut f = Folder::new(&id, &draft.account_id, 0);
    f.set_path(&path);
    f.set_role(&role);
    Ok(Some(f))
}

/// Copies every file under `from` into `to`, replacing what is there. Returns the number of
/// files copied.
fn copy_dir(from: &Path, to: &Path) -> Result<usize> {
    fs::create_dir_all(to)?;
    let mut n = 0;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            n += copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
            n += 1;
        }
    }
    Ok(n)
}

// -- Metadata -------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct LegacyMetadata {
    plugin_id: String,
    value: Value,
    version: i64,
}

/// Removes and returns the model JSON's `metadata` entries, empty values included (they
/// carry the version a plugin reached).
fn take_metadata(data: &mut Value) -> Vec<LegacyMetadata> {
    let Some(arr) = data.as_object_mut().and_then(|o| o.remove("metadata")) else {
        return Vec::new();
    };
    arr.as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|m| !json_str(m, "pluginId").is_empty())
                .map(|m| LegacyMetadata {
                    plugin_id: json_str(m, "pluginId").to_owned(),
                    value: m["value"].clone(),
                    version: json_i64(&m["v"]),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The entries that hold something: an empty value is a plugin's removal and leaves no
/// `ModelPluginMetadata` row, so there is nothing to carry.
fn non_empty_metadata(mut data: Value) -> Vec<LegacyMetadata> {
    take_metadata(&mut data)
        .into_iter()
        .filter(|m| json_size(&m.value) > 0)
        .collect()
}

fn import_message_metadata(
    s: &Store,
    legacy: &LegacyDb,
    draft_ids: &HashSet<String>,
    counts: &mut ImportCounts,
) -> Result<()> {
    for id in legacy.ids_with_metadata(OBJECT_TYPE_MESSAGE)? {
        if draft_ids.contains(&id) || id.starts_with("deleted-") {
            continue;
        }
        let Some(m) = legacy.message(&id)? else {
            continue;
        };
        if m.header_message_id.is_empty() {
            continue;
        }
        let metadata = non_empty_metadata(m.data);
        if metadata.is_empty() {
            continue;
        }
        let mut targets = messages_with_header_message_id(s, &m.account_id, &m.header_message_id)?;
        for entry in &metadata {
            if targets.is_empty() {
                save_pending(
                    s,
                    &pending_message_key(&m.header_message_id),
                    OBJECT_TYPE_MESSAGE,
                    &m.account_id,
                    entry,
                )?;
                counts.metadata_pending += 1;
                continue;
            }
            for target in targets.iter_mut() {
                if target
                    .upsert_metadata(&entry.plugin_id, &entry.value, Some(entry.version))
                    .is_some()
                {
                    s.save(target)?;
                }
            }
            counts.metadata_attached += 1;
        }
    }
    Ok(())
}

fn import_thread_metadata(
    s: &Store,
    legacy: &LegacyDb,
    draft_ids: &HashSet<String>,
    counts: &mut ImportCounts,
) -> Result<()> {
    for id in legacy.ids_with_metadata(OBJECT_TYPE_THREAD)? {
        let Some((account_id, data)) = legacy.thread(&id)? else {
            continue;
        };
        let metadata = non_empty_metadata(data);
        if metadata.is_empty() {
            continue;
        }
        let header_ids = legacy.thread_header_message_ids(&id, &account_id, draft_ids)?;
        let mut thread_ids = Vec::new();
        for h in &header_ids {
            for m in messages_with_header_message_id(s, &account_id, h)? {
                if !m.thread_id().is_empty() && !thread_ids.iter().any(|t| t == m.thread_id()) {
                    thread_ids.push(m.thread_id().to_owned());
                }
            }
        }
        let mut threads = s.find_large_set::<Thread>("id", &thread_ids)?;
        for entry in &metadata {
            if threads.is_empty() {
                for h in &header_ids {
                    save_pending(
                        s,
                        &pending_thread_key(h, &id),
                        OBJECT_TYPE_THREAD,
                        &account_id,
                        entry,
                    )?;
                }
                if !header_ids.is_empty() {
                    counts.metadata_pending += 1;
                }
                continue;
            }
            for t in threads.iter_mut() {
                if t.upsert_metadata(&entry.plugin_id, &entry.value, Some(entry.version))
                    .is_some()
                {
                    s.save(t)?;
                }
            }
            counts.metadata_attached += 1;
        }
    }
    Ok(())
}

fn messages_with_header_message_id(
    s: &Store,
    account_id: &str,
    header_message_id: &str,
) -> Result<Vec<Message>> {
    Ok(s.find_all::<Message>(
        &Query::new()
            .equal("accountId", account_id)
            .equal("headerMessageId", header_message_id),
    )?
    .into_iter()
    .filter(|m| !m.id().starts_with("deleted-"))
    .collect())
}

/// A pending row; of two legacy models sharing a key, the higher metadata version wins.
fn save_pending(
    s: &Store,
    object_id: &str,
    object_type: &str,
    account_id: &str,
    m: &LegacyMetadata,
) -> Result<()> {
    s.conn()
        .prepare_cached(
            "INSERT INTO DetatchedPluginMetadata (objectId, objectType, accountId, pluginId, value, version) VALUES (?,?,?,?,?,?) \
             ON CONFLICT (objectId, accountId, pluginId) DO UPDATE SET value = excluded.value, version = excluded.version \
             WHERE excluded.version > DetatchedPluginMetadata.version",
        )?
        .execute(params![
            object_id,
            object_type,
            account_id,
            m.plugin_id,
            serde_json::to_string(&m.value)?,
            m.version
        ])?;
    Ok(())
}

/// Part of `Message::before_save` on a message's first save: metadata imported for its
/// Message-ID is attached to it.
pub(crate) fn attach_pending_message_metadata(msg: &mut Message, store: &Store) -> Result<()> {
    if msg.version() != 1 || msg.id().starts_with("deleted-") {
        return Ok(());
    }
    let hmsgid = msg.header_message_id().to_owned();
    if hmsgid.is_empty() {
        return Ok(());
    }
    let account_id = msg.account_id().to_owned();
    let pending = store
        .find_and_delete_detached_plugin_metadata(&account_id, &pending_message_key(&hmsgid))?;
    for m in pending {
        info!("-- Attaching imported metadata for {}", m.plugin_id);
        msg.upsert_metadata(&m.plugin_id, &m.value, Some(m.version));
    }
    Ok(())
}

/// Part of `Message::after_save` on a message's first save, once its thread is stored:
/// thread metadata imported for its Message-ID is attached to that thread, and the rows
/// the same legacy thread left under its other messages are dropped. Imported drafts that
/// answer it join the thread.
pub(crate) fn attach_pending_thread_metadata(msg: &Message, store: &Store) -> Result<()> {
    if msg.version() != 1 || msg.thread_id().is_empty() || msg.id().starts_with("deleted-") {
        return Ok(());
    }
    let hmsgid = msg.header_message_id();
    if hmsgid.is_empty() {
        return Ok(());
    }
    let (lo, hi) = pending_thread_key_range(hmsgid);
    let rows: Vec<(String, String, String, Value, i64)> = {
        let mut stmt = store.conn().prepare_cached(
            "SELECT objectId, objectType, pluginId, value, version FROM DetatchedPluginMetadata WHERE accountId = ? AND objectId >= ? AND objectId < ?",
        )?;
        let mapped = stmt.query_map(params![msg.account_id(), lo, hi], |r| {
            let value = match r.get_ref(3)? {
                rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
                    serde_json::from_slice(b).unwrap_or(Value::Null)
                }
                _ => Value::Null,
            };
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, value, r.get(4)?))
        })?;
        mapped.collect::<rusqlite::Result<_>>()?
    };
    if rows.is_empty() {
        return Ok(());
    }
    let (drafts, rows): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .partition(|(_, object_type, ..)| object_type == OBJECT_TYPE_DRAFT_THREAD);
    // Thread metadata first: a joining draft saves the thread again, from the database.
    attach_thread_rows(msg, store, &rows)?;
    for (object_id, ..) in &drafts {
        let draft_id = object_id
            .split_once(THREAD_KEY_SEPARATOR)
            .map(|(_, d)| d)
            .unwrap_or("");
        join_thread(store, draft_id, msg.thread_id())?;
        store
            .conn()
            .prepare_cached(
                "DELETE FROM DetatchedPluginMetadata WHERE accountId = ? AND objectId = ? AND objectType = ?",
            )?
            .execute(params![msg.account_id(), object_id, OBJECT_TYPE_DRAFT_THREAD])?;
    }
    Ok(())
}

/// Puts an imported draft that is still unthreaded into `thread_id`. Gone (sent or
/// deleted) or threaded since, it is left alone.
fn join_thread(store: &Store, draft_id: &str, thread_id: &str) -> Result<()> {
    let Some(mut draft) = store.find::<Message>(&Query::new().equal("id", draft_id))? else {
        return Ok(());
    };
    if !draft.is_draft() || !draft.thread_id().is_empty() {
        return Ok(());
    }
    info!("-- Moving imported draft {draft_id} into its thread");
    draft.set_thread_id(thread_id);
    // Nothing of it is counted in that thread yet.
    draft.reset_snapshot();
    store.save(&mut draft)
}

fn attach_thread_rows(
    msg: &Message,
    store: &Store,
    rows: &[(String, String, String, Value, i64)],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let Some(mut thread) = store.find::<Thread>(&Query::new().equal("id", msg.thread_id()))? else {
        return Ok(());
    };
    let mut changed = false;
    for (object_id, _, plugin_id, value, version) in rows {
        info!("-- Attaching imported thread metadata for {plugin_id}");
        changed |= thread
            .upsert_metadata(plugin_id, value, Some(*version))
            .is_some();
        let legacy_thread = object_id
            .split_once(THREAD_KEY_SEPARATOR)
            .map(|(_, t)| t)
            .unwrap_or("");
        let suffix = format!("{THREAD_KEY_SEPARATOR}{legacy_thread}");
        store
            .conn()
            .prepare_cached(
                "DELETE FROM DetatchedPluginMetadata WHERE accountId = :aid AND pluginId = :plugin AND objectType = 'Thread' \
                 AND substr(objectId, 1, 3) = 't:<' AND substr(objectId, -length(:suffix)) = :suffix",
            )?
            .execute(rusqlite::named_params! {
                ":aid": msg.account_id(),
                ":plugin": plugin_id,
                ":suffix": suffix,
            })?;
    }
    if changed {
        store.save(&mut thread)?;
    }
    Ok(())
}

// -- The legacy database ---------------------------------------------------------------------

struct LegacyDraft {
    id: String,
    account_id: String,
    data: Value,
    /// The folder of its UID-0 placement (`remoteFolderId` before V10).
    folder_id: String,
}

struct LegacyMessage {
    account_id: String,
    header_message_id: String,
    data: Value,
}

/// A C++ `edgehill.db`, read-only. `placements` is false before the V10 migration, when a
/// message's single location was `Message.remoteFolderId` / `remoteUID`.
struct LegacyDb {
    conn: Connection,
    placements: bool,
}

impl LegacyDb {
    fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.execute_batch("PRAGMA query_only = 1")?;
        for table in [
            "Message",
            "Thread",
            "Folder",
            "ModelPluginMetadata",
            "_State",
        ] {
            if !has_table(&conn, table)? {
                return Err(Error::other(format!(
                    "{LEGACY_DATABASE_FILE} has no {table} table"
                )));
            }
        }
        let placements = has_table(&conn, "MessageFolder")?;
        if !placements && !has_column(&conn, "Message", "remoteFolderId")? {
            return Err(Error::other(format!(
                "{LEGACY_DATABASE_FILE} records no message locations"
            )));
        }
        Ok(LegacyDb { conn, placements })
    }

    /// Drafts that exist only locally: draft rows with a UID-0 placement and no copy on
    /// the server (a copy there is synced again under its own id). Deletion placeholders
    /// (`deleted-*`) are not drafts.
    fn local_drafts(&self) -> Result<Vec<LegacyDraft>> {
        let sql = if self.placements {
            "SELECT m.id, m.accountId, m.data, (SELECT p.folderId FROM MessageFolder p WHERE p.messageId = m.id AND p.remoteUID = 0 ORDER BY p.rowid LIMIT 1) \
             FROM Message m WHERE m.draft = 1 AND m.id NOT LIKE 'deleted-%' \
             AND EXISTS (SELECT 1 FROM MessageFolder p WHERE p.messageId = m.id AND p.remoteUID = 0) \
             AND NOT EXISTS (SELECT 1 FROM MessageFolder p WHERE p.messageId = m.id AND p.remoteUID > 0) \
             ORDER BY m.rowid"
        } else {
            "SELECT id, accountId, data, remoteFolderId FROM Message \
             WHERE draft = 1 AND remoteUID = 0 AND IFNULL(remoteFolderId, '') != '' AND id NOT LIKE 'deleted-%' \
             ORDER BY rowid"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                json_column(r, 2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, account_id, data, folder_id) = row?;
            let Some(data) = data.filter(Value::is_object) else {
                warn!("-X Skipping draft {id}: its JSON does not parse");
                continue;
            };
            let account_id = account_id
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| json_str(&data, "aid").to_owned());
            out.push(LegacyDraft {
                id,
                account_id,
                data,
                folder_id: folder_id.unwrap_or_default(),
            });
        }
        Ok(out)
    }

    fn folder(&self, id: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .conn
            .query_row("SELECT path, role FROM Folder WHERE id = ?", [id], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })
            .optional()?
            .filter(|(path, _)| !path.is_empty()))
    }

    fn body(&self, id: &str) -> Result<Option<(Option<String>, rusqlite::types::Value)>> {
        let fetched_at = if has_column(&self.conn, "MessageBody", "fetchedAt")? {
            "fetchedAt"
        } else {
            "NULL"
        };
        Ok(self
            .conn
            .query_row(
                &format!("SELECT value, {fetched_at} FROM MessageBody WHERE id = ?"),
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    fn file_row(&self, id: &str) -> Result<Option<Value>> {
        if !has_table(&self.conn, "File")? {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row("SELECT data FROM File WHERE id = ?", [id], |r| {
                json_column(r, 0)
            })
            .optional()?
            .flatten()
            .filter(Value::is_object))
    }

    fn ids_with_metadata(&self, object_type: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT id FROM ModelPluginMetadata WHERE objectType = ? ORDER BY id",
        )?;
        let ids = stmt
            .query_map([object_type], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    fn message(&self, id: &str) -> Result<Option<LegacyMessage>> {
        let row = self
            .conn
            .query_row(
                "SELECT accountId, headerMessageId, data FROM Message WHERE id = ?",
                [id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        json_column(r, 2)?,
                    ))
                },
            )
            .optional()?;
        let Some((account_id, hmsgid, Some(data))) = row else {
            return Ok(None);
        };
        Ok(Some(LegacyMessage {
            account_id: account_id.unwrap_or_else(|| json_str(&data, "aid").to_owned()),
            header_message_id: hmsgid.unwrap_or_else(|| json_str(&data, "hMsgId").to_owned()),
            data,
        }))
    }

    fn thread(&self, id: &str) -> Result<Option<(String, Value)>> {
        let row = self
            .conn
            .query_row(
                "SELECT accountId, data FROM Thread WHERE id = ?",
                [id],
                |r| Ok((r.get::<_, Option<String>>(0)?, json_column(r, 1)?)),
            )
            .optional()?;
        let Some((account_id, Some(data))) = row else {
            return Ok(None);
        };
        let account_id = account_id.unwrap_or_else(|| json_str(&data, "aid").to_owned());
        Ok(Some((account_id, data)))
    }

    /// The Message-IDs of a legacy thread's messages that sync can bring back: not
    /// deletion placeholders, not the local drafts imported as they are.
    fn thread_header_message_ids(
        &self,
        thread_id: &str,
        account_id: &str,
        draft_ids: &HashSet<String>,
    ) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, headerMessageId FROM Message WHERE threadId = ? AND accountId = ? AND id NOT LIKE 'deleted-%' ORDER BY date",
        )?;
        let rows = stmt.query_map([thread_id, account_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        let mut out: Vec<String> = Vec::new();
        for row in rows {
            let (id, h) = row?;
            let Some(h) = h.filter(|h| !h.is_empty()) else {
                continue;
            };
            if !draft_ids.contains(&id) && !out.contains(&h) {
                out.push(h);
            }
        }
        Ok(out)
    }

    /// `_State` keys that still mean something to this engine: the metadata cursors.
    fn carried_state(&self) -> Result<BTreeMap<String, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, value FROM `_State` WHERE id LIKE 'cursor-%'")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?)))?
            .collect::<rusqlite::Result<Vec<(String, Option<String>)>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(k, v)| v.map(|v| (k, v)))
            .collect())
    }
}

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(`{table}`)"))?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|n| n == column))
}

/// A `data` column (TEXT in some tables, BLOB in others) as JSON; `None` when it is NULL
/// or does not parse.
fn json_column(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<Value>> {
    Ok(match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
            serde_json::from_slice(b).ok()
        }
        _ => None,
    })
}
