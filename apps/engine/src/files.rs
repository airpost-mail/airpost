// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `files.get`, `files.read` and `files.put` on the socket.
//! Files live under `<config>/files` at `File::client_path_in`, where the message's body
//! fetch writes them (whole, through a rename). `files.get` for a file that isn't there asks
//! the account for the body and answers from a `files` thread once the file appears, so the
//! connection's other requests aren't held behind the download (T21).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use tern_core::constants::FILES_DIR;
use tern_ipc::{
    code, method, ErrorObject, FilesGetParams, FilesGetResult, FilesPutParams, FilesPutResult,
    FilesReadParams, FilesReadResult, Request as RpcRequest, FILES_READ_MAX_BYTES,
};
use tern_store::{File, Model};

use crate::dispatch::{Deferred, Reply, Request};
use crate::socket::Session;

/// How long `files.get` waits for a body fetch to write the file, and `messages.unsubscribe`
/// for the body that carries the header. Debug builds read
/// `MAILSYNC_FILES_WAIT_MS` so the tests needn't wait it out.
pub(crate) fn download_wait() -> Duration {
    const DEFAULT: Duration = Duration::from_secs(30);
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("MAILSYNC_FILES_WAIT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            return Duration::from_millis(ms);
        }
    }
    DEFAULT
}

const POLL: Duration = Duration::from_millis(100);
/// `files.get` calls waiting for a download at once, engine-wide.
const MAX_WAITING: usize = 64;
static WAITING: AtomicUsize = AtomicUsize::new(0);

/// The longest `files.put` name and content type.
const MAX_NAME_CHARS: usize = 255;
const MAX_CONTENT_TYPE_BYTES: usize = 255;

pub fn is_files_method(name: &str) -> bool {
    matches!(
        name,
        method::FILES_GET | method::FILES_READ | method::FILES_PUT
    )
}

fn files_root(config_dir: &Path) -> PathBuf {
    config_dir.join(FILES_DIR)
}

fn invalid(why: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, format!("invalid params: {why}"))
}

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

fn to_reply(v: impl serde::Serialize) -> Reply {
    serde_json::to_value(v).map_err(internal)
}

fn result(file: &File, path: Option<&Path>) -> Reply {
    let on_disk = path.and_then(|p| std::fs::metadata(p).ok().map(|m| (p, m.len())));
    to_reply(FilesGetResult {
        path: on_disk.map(|(p, _)| p.display().to_string()),
        size: on_disk.map_or(file.size().max(0) as u64, |(_, n)| n),
        content_type: file.content_type().to_owned(),
        downloaded: on_disk.is_some(),
        filename: file.filename().to_owned(),
    })
}

/// Waits for `file` to appear under `root` until `deadline`, then answers. The deadline is
/// taken when the call arrives, so time spent queued for a `files` thread counts against it.
fn wait_for(file: File, root: PathBuf, deadline: Instant) -> Reply {
    struct Slot;
    impl Drop for Slot {
        fn drop(&mut self) {
            WAITING.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _slot = Slot;
    loop {
        if let Some(path) = file.find_in(&root) {
            return result(&file, Some(&path));
        }
        if Instant::now() >= deadline {
            return result(&file, None);
        }
        std::thread::sleep(POLL);
    }
}

impl Session {
    fn lookup_file(&mut self, account_id: &str, file_id: &str) -> Result<File, ErrorObject> {
        self.server.accounts.check(account_id)?;
        let json = self
            .reads
            .file(account_id, file_id)?
            .ok_or_else(|| ErrorObject::new(code::NOT_FOUND, format!("no file {file_id}")))?;
        File::from_json(json).map_err(internal)
    }

    /// Answers one of the [`is_files_method`] methods.
    pub fn files_call(&mut self, req: &RpcRequest) -> Deferred {
        match req.method.as_str() {
            method::FILES_GET => match req.params_as().and_then(|p| self.files_get(p)) {
                Ok(r) => r,
                Err(e) => Deferred::Now(Err(e)),
            },
            method::FILES_READ => Deferred::Now(req.params_as().and_then(|p| self.files_read(p))),
            _ => Deferred::Now(req.params_as().and_then(|p| self.files_put(p))),
        }
    }

    fn files_get(&mut self, p: FilesGetParams) -> Result<Deferred, ErrorObject> {
        let file = self.lookup_file(&p.account_id, &p.file_id)?;
        let root = files_root(&self.server.config_dir);
        if let Some(path) = file.find_in(&root) {
            return Ok(Deferred::Now(result(&file, Some(&path))));
        }
        // An upload whose bytes are gone has no body to fetch them from.
        if file.message_id().is_empty() {
            return Ok(Deferred::Now(result(&file, None)));
        }
        if WAITING.fetch_add(1, Ordering::SeqCst) >= MAX_WAITING {
            WAITING.fetch_sub(1, Ordering::SeqCst);
            return Err(internal(
                "too many files.get calls are waiting for downloads; try again shortly",
            ));
        }
        if let Err(e) = self.dispatch(
            &p.account_id,
            Request::NeedBodies(vec![file.message_id().to_owned()]),
        ) {
            WAITING.fetch_sub(1, Ordering::SeqCst);
            return Err(e);
        }
        let deadline = Instant::now() + download_wait();
        Ok(Deferred::Later(Box::new(move || {
            wait_for(file, root, deadline)
        })))
    }

    fn files_read(&mut self, p: FilesReadParams) -> Reply {
        let file = self.lookup_file(&p.account_id, &p.file_id)?;
        let path = file
            .find_in(&files_root(&self.server.config_dir))
            .ok_or_else(|| {
                ErrorObject::new(
                    code::NOT_FOUND,
                    format!("file {} isn't downloaded; call files.get first", p.file_id),
                )
            })?;
        let mut f = std::fs::File::open(&path).map_err(internal)?;
        let size = f.metadata().map_err(internal)?.len();
        let len = p
            .len
            .unwrap_or(FILES_READ_MAX_BYTES)
            .min(FILES_READ_MAX_BYTES);
        let offset = p.offset.min(size);
        f.seek(SeekFrom::Start(offset)).map_err(internal)?;
        let mut buf = Vec::with_capacity(len.min(size - offset) as usize);
        f.take(len).read_to_end(&mut buf).map_err(internal)?;
        to_reply(FilesReadResult {
            data: base64::engine::general_purpose::STANDARD.encode(&buf),
            offset,
            size,
            eof: offset + buf.len() as u64 >= size,
        })
    }

    fn files_put(&mut self, p: FilesPutParams) -> Reply {
        self.server.accounts.check(&p.account_id)?;
        let name = p.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(invalid(format!(
                "name must have 1 to {MAX_NAME_CHARS} characters"
            )));
        }
        let content_type = match p.content_type.as_deref().map(str::trim) {
            None | Some("") => "application/octet-stream".to_owned(),
            Some(t) if valid_content_type(t) => t.to_ascii_lowercase(),
            Some(_) => return Err(invalid("contentType is not a MIME type")),
        };
        let file_id = tern_core::ids::local_file_id();
        let root = files_root(&self.server.config_dir);
        let draft = File::create_local(&file_id, &p.account_id, "", name, &content_type, 0);
        let written = match (&p.data, &p.path) {
            (Some(data), None) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|e| invalid(format!("data is not base64: {e}")))?;
                draft.write_in(&root, &bytes)
            }
            (None, Some(path)) => {
                let path = Path::new(path);
                if !path.is_absolute() || !path.is_file() {
                    return Err(invalid("path must be an absolute path to a file"));
                }
                draft.copy_in(&root, path)
            }
            _ => return Err(invalid("give one of data and path")),
        };
        let dest = written.map_err(|e| internal(format!("could not store the file: {e}")))?;
        let size = std::fs::metadata(&dest).map_err(internal)?.len();
        let file = File::create_local(&file_id, &p.account_id, "", name, &content_type, size);
        match self.dispatch(&p.account_id, Request::SaveFile(file.data)) {
            Ok(saved) => to_reply(FilesPutResult {
                file_id,
                file: saved,
            }),
            Err(e) => {
                let _ = std::fs::remove_dir_all(draft.dir_in(&root));
                Err(e)
            }
        }
    }
}

/// `type/subtype` with optional parameters, printable ASCII.
fn valid_content_type(t: &str) -> bool {
    let essence = t.split(';').next().unwrap_or("").trim();
    t.len() <= MAX_CONTENT_TYPE_BYTES
        && t.bytes().all(|b| (0x20..0x7f).contains(&b))
        && essence
            .split_once('/')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('/'))
        && !essence.contains(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_queued_past_its_deadline_answers_at_once() {
        let root = std::env::temp_dir().join("tern-files-wait-test-none");
        let file = File::create_local("f1", "a1", "m1", "a.png", "image/png", 3);
        WAITING.fetch_add(1, Ordering::SeqCst);
        let started = Instant::now();
        let reply = wait_for(file, root, started - Duration::from_secs(1)).unwrap();
        assert!(started.elapsed() < POLL);
        assert_eq!(reply["downloaded"], false);
    }

    #[test]
    fn content_types() {
        assert!(valid_content_type("image/png"));
        assert!(valid_content_type("text/plain; charset=utf-8"));
        assert!(!valid_content_type("png"));
        assert!(!valid_content_type("a/b/c"));
        assert!(!valid_content_type("text /plain"));
        assert!(!valid_content_type("text/plain\r\nX: y"));
        assert!(!valid_content_type(&format!("a/{}", "b".repeat(300))));
    }
}
