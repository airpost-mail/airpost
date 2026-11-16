// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `.eml` export: GetMessageRFC2822Task (one message) and GetManyRFC2822Task (a folder).
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp`
//! (`performRemoteGetMessageRFC2822`, `sanitizeEmlFilename`, `performRemoteGetManyRFC2822`),
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::{json, Value};
use tern_core::constants::FILES_DIR;
use tern_core::{ErrorCode, Result, SyncError};
use tern_imap::ImapSession;
use tern_store::{File, Message, Model, Query, Task};
use tracing::{error, info, warn};

use crate::json::{at, get_i64, get_str, has};
use crate::TaskProcessor;

/// Messages fetched between progress saves and cancellation checks.
const EXPORT_CHUNK: usize = 50;
/// The pause after a full chunk, so sync and other tasks get the connection.
const EXPORT_CHUNK_PAUSE: Duration = Duration::from_secs(1);

/// Writes `data` to `path` and dates the file like the message. Like mailcore's
/// `Data::writeToFile`, a failed write does not fail the export; it is logged.
fn write_eml(path: &Path, data: &[u8], date: i64) {
    if let Err(e) = std::fs::write(path, data) {
        warn!("Could not write {}: {e}", path.display());
        return;
    }
    let time = UNIX_EPOCH + Duration::from_secs(date.max(0) as u64);
    let times = std::fs::FileTimes::new()
        .set_accessed(time)
        .set_modified(time);
    let set = std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_times(times));
    if let Err(e) = set {
        warn!(
            "Could not set the modification time of {}: {e}",
            path.display()
        );
    }
}

/// `{index:05} - {subject} - {YYYY-MM-DD}.eml`. The subject (or "untitled") is cut to 80
/// bytes on a character boundary, characters illegal in file names on some platform and
/// control characters become `_`, and trailing dots and spaces are dropped (Windows).
/// The date is UTC.
pub fn sanitize_eml_filename(subject: &str, date: i64, index: i64) -> String {
    let mut safe: String = if subject.is_empty() {
        "untitled".to_owned()
    } else {
        let mut end = subject.len().min(80);
        while !subject.is_char_boundary(end) {
            end -= 1;
        }
        subject[..end].to_owned()
    };
    safe = safe
        .chars()
        .map(|c| match c {
            '\0'..='\x1f' | '\x7f' | '/' | '?' | '<' | '>' | '\\' | ':' | '*' | '|' | '"' => '_',
            c => c,
        })
        .collect();
    let trimmed = safe.trim_end_matches(['.', ' ']);
    let safe = if trimmed.is_empty() {
        "untitled"
    } else {
        trimmed
    };
    let day = chrono::DateTime::from_timestamp(date, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_owned());
    format!("{index:05} - {safe} - {day}.eml")
}

/// Whether a GetMessageRFC2822Task is in file-id mode: no `filepath` (absent, null or ""),
/// so the engine saves the message as a File in its files directory and the client reads it
/// with `files.get`.
pub(crate) fn exports_to_file_id(data: &Value) -> bool {
    data.get("filepath")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
}

/// The local phase of a GetMessageRFC2822Task in file-id mode: a new `fileId`, chosen here
/// and never by the client, so an export can't overwrite another file's bytes.
pub(crate) fn assign_export_file_id(data: &mut Value) {
    if exports_to_file_id(data) {
        data["fileId"] = json!(tern_core::ids::local_file_id());
    }
}

impl TaskProcessor<'_> {
    pub(crate) fn perform_remote_get_message_rfc2822(
        &self,
        data: &Value,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let id = get_str(at(data, "messageId"))?;
        let Some(msg) = self.store.find::<Message>(&Query::new().equal("id", &id))? else {
            return Err(
                SyncError::fatal("not-found", "Message not found for RFC2822 fetch").into(),
            );
        };

        let mut bytes = None;
        for copy in self.store.fetchable_copies_of_message(&msg, None)? {
            match session.fetch_message_bytes(copy.folder.path(), copy.uid) {
                Ok(Some(b)) => {
                    bytes = Some(b);
                    break;
                }
                Ok(None) => error!(
                    "fetchMessageByUID returned null data for message ({} UID {})",
                    copy.folder.path(),
                    copy.uid
                ),
                Err(e) => {
                    error!(
                        "Unable to fetch rfc2822 for message ({} UID {}). Error {}",
                        copy.folder.path(),
                        copy.uid,
                        e.code.name()
                    );
                    // This copy may have been expunged since the last scan.
                    if e.code == ErrorCode::Fetch {
                        continue;
                    }
                    return Err(e.into_sync("performRemoteGetMessageRFC2822").into());
                }
            }
        }
        let Some(bytes) = bytes else {
            return Err(SyncError::from_code(
                ErrorCode::Fetch,
                "performRemoteGetMessageRFC2822 - no copy on the server",
            )
            .into());
        };
        if !exports_to_file_id(data) {
            let filepath = get_str(at(data, "filepath"))?;
            write_eml(Path::new(&filepath), &bytes, msg.date());
            return Ok(());
        }
        self.save_export_file(data, &msg, &bytes)
    }

    /// File-id mode: the message's bytes as the File `data.fileId`, named after its subject.
    fn save_export_file(&self, data: &Value, msg: &Message, bytes: &[u8]) -> Result<()> {
        let file_id = get_str(at(data, "fileId"))?;
        let Some(config_dir) = &self.config_dir else {
            return Err(
                SyncError::fatal("no-files-dir", "The engine has no files directory").into(),
            );
        };
        let subject = msg.subject();
        let stem = if subject.trim().is_empty() {
            "message"
        } else {
            subject
        };
        let file = File::create_local(
            &file_id,
            self.account.id(),
            msg.id(),
            &format!(
                "{}.eml",
                stem.chars().take(80).collect::<String>().trim_end()
            ),
            "message/rfc822",
            bytes.len() as u64,
        );
        file.write_in(&config_dir.join(FILES_DIR), bytes)
            .map_err(|e| {
                SyncError::fatal(
                    "file-write-failed",
                    format!("Could not save the message: {e}"),
                )
            })?;
        let mut file = file;
        self.store.save(&mut file)
    }

    /// Exports every message with a copy in the folder, in UID order, 50 per chunk. After
    /// each chunk the progress is saved on the task and a cancel request checked, in one
    /// transaction: the client sets `should_cancel` on the row from the main thread, and
    /// re-reading and saving under one write lock means a request is either seen here or
    /// ordered after this commit and seen on the next chunk, never overwritten. A resumed
    /// task carries on from its saved `progress.lastUID`.
    pub(crate) fn perform_remote_get_many_rfc2822(
        &self,
        task: &mut Task,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let account_id = task.account_id().to_owned();
        let folder_id = get_str(at(&task.data, "folderId"))?;
        let folder_path = get_str(at(&task.data, "folderPath"))?;
        let output_dir = get_str(at(&task.data, "outputDir"))?;

        // The same join as the page query, so a placement whose Message row is missing is
        // not counted as an export that never happens.
        let total: i64 = self.store.conn().query_row(
            "SELECT COUNT(*) FROM MessageFolder INNER JOIN Message ON Message.id = MessageFolder.messageId \
             WHERE MessageFolder.accountId = ? AND MessageFolder.folderId = ? AND MessageFolder.remoteUID > 0",
            rusqlite::params![account_id, folder_id],
            |r| r.get(0),
        )?;

        let mut progress = json!({"total": total, "exported": 0, "failed": 0, "errors": []});
        let mut cursor_uid: u32 = 0;
        let mut global_index: i64 = 0;
        if has(&task.data, "progress") {
            let prev = at(&task.data, "progress");
            if has(prev, "exported") {
                progress["exported"] = prev["exported"].clone();
                global_index = get_i64(&prev["exported"])?;
            }
            if has(prev, "failed") {
                progress["failed"] = prev["failed"].clone();
            }
            if has(prev, "errors") {
                progress["errors"] = prev["errors"].clone();
            }
            if has(prev, "lastUID") {
                cursor_uid = get_i64(&prev["lastUID"])? as u32;
            }
        }
        let mut exported = get_i64(&progress["exported"])?;
        let mut failed = get_i64(&progress["failed"])?;

        loop {
            let page: Vec<(u32, String, String, i64)> = self
                .store
                .conn()
                .prepare_cached(
                    "SELECT MessageFolder.remoteUID, Message.id, Message.subject, Message.date FROM MessageFolder \
                     INNER JOIN Message ON Message.id = MessageFolder.messageId \
                     WHERE MessageFolder.accountId = ? AND MessageFolder.folderId = ? AND MessageFolder.remoteUID > ? \
                     ORDER BY MessageFolder.remoteUID ASC LIMIT ?",
                )?
                .query_map(
                    rusqlite::params![account_id, folder_id, cursor_uid as i64, EXPORT_CHUNK as i64],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)? as u32,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                            r.get::<_, Option<f64>>(3)?.unwrap_or(0.0) as i64,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<_>>()?;
            if page.is_empty() {
                break;
            }

            for (uid, id, subject, date) in &page {
                let filename = sanitize_eml_filename(subject, *date, global_index);
                let filepath = Path::new(&output_dir).join(filename);
                let fetched = match session.fetch_message_bytes(&folder_path, *uid) {
                    Ok(Some(b)) => Ok(b),
                    Ok(None) => Err(SyncError::from_code(
                        ErrorCode::Fetch,
                        "GetManyRFC2822 - null data",
                    )),
                    Err(e) => Err(e.into_sync("GetManyRFC2822 fetch")),
                };
                match fetched {
                    Ok(bytes) => {
                        write_eml(&filepath, &bytes, *date);
                        exported += 1;
                    }
                    Err(e) => {
                        error!(
                            "GetManyRFC2822: failed to export message {id} (UID {uid}): {}",
                            e.to_json()
                        );
                        failed += 1;
                        // Note: the C++ read `toJSON()["error"]`, a key the error JSON does
                        // not have, so every entry's `error` is null. The client reads it.
                        if let Some(errors) = progress["errors"].as_array_mut() {
                            errors
                                .push(json!({"messageId": id, "subject": subject, "error": null}));
                        }
                    }
                }
                cursor_uid = *uid;
                global_index += 1;
            }

            progress["exported"] = json!(exported);
            progress["failed"] = json!(failed);
            progress["lastUID"] = json!(cursor_uid);

            let task_id = task.id().to_owned();
            let cancelled = self.store.transaction("GetManyRFC2822 progress", |s| {
                let refreshed = s.find::<Task>(&Query::new().equal("id", &task_id))?;
                let cancelled = refreshed.is_some_and(|t| t.should_cancel());
                if cancelled {
                    progress["cancelled"] = json!(true);
                    // Kept on this copy so the save below preserves it and performRemote
                    // marks the task cancelled.
                    task.set_should_cancel();
                }
                task.data["progress"] = progress.clone();
                s.save(task)?;
                Ok(cancelled)
            })?;
            if cancelled {
                info!("GetManyRFC2822: cancelled after exporting {exported} of {total} messages");
                return Ok(());
            }
            if page.len() == EXPORT_CHUNK {
                std::thread::sleep(EXPORT_CHUNK_PAUSE);
            }
        }

        task.data["result"] = json!({
            "total": total,
            "exported": exported,
            "failed": failed,
            "outputDir": output_dir,
            "errors": progress["errors"],
        });
        self.store.save(task)?;
        info!(
            "GetManyRFC2822: completed. Exported {exported} of {total} messages ({failed} failed)"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eml_filenames() {
        // 2024-03-05 12:00:00 UTC
        let date = 1_709_640_000;
        assert_eq!(
            sanitize_eml_filename("Re: a/b?", date, 7),
            "00007 - Re_ a_b_ - 2024-03-05.eml"
        );
        assert_eq!(
            sanitize_eml_filename("", date, 0),
            "00000 - untitled - 2024-03-05.eml"
        );
        assert_eq!(
            sanitize_eml_filename(" . ", date, 1),
            "00001 - untitled - 2024-03-05.eml"
        );
        let long = "é".repeat(60);
        let name = sanitize_eml_filename(&long, date, 123456);
        assert!(name.starts_with("123456 - éé"));
        assert_eq!(name.matches('é').count(), 40);
    }
}
