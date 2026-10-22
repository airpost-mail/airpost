// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern attachment get|save` and `tern export`. Attachments come through `files.get`, which
//! downloads a missing one first, and `files.read`; a message's or folder's raw mail from the
//! server through GetMessageRFC2822Task and GetManyRFC2822Task, the desktop's Export, which
//! the engine writes to a path. A folder export is staged as .eml files beside the output and
//! assembled into an mbox here, as the desktop's mbox export runner does.

use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::{json, Value};
use tern_client::tasks::{self as builders, TASK_CLASS};
use tern_ipc::{feature, method, FilesGetResult, FilesReadResult, FILES_READ_MAX_BYTES};

use crate::app::{call, call_raw, is_not_found, Ctx};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::mail_actions::{by_account, require};
use crate::organize::{display_name, queue, stamp};
use crate::output;
use crate::read::{list_categories, Kind};
use crate::tasks::{describe_error, wait_all};
use crate::watch::Subscriptions;

/// How long `export <message-id>` waits by default.
const MESSAGE_EXPORT_WAIT: u64 = 120;

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn exists_error(path: &Path) -> CliError {
    CliError::failed(format!(
        "{} exists; give --force to replace it",
        path.display()
    ))
}

/// `files.get` on the account that has `file_id`.
fn locate(
    engines: &mut [Engine],
    targets: &[Target],
    file_id: &str,
) -> Result<(Target, FilesGetResult)> {
    for t in targets {
        require(engines, t, feature::FILES, "read attachments")?;
        match call_raw::<FilesGetResult>(
            engines,
            t,
            method::FILES_GET,
            json!({ "accountId": t.account_id, "fileId": file_id }),
        ) {
            Ok(got) => return Ok((t.clone(), got)),
            Err(e) if is_not_found(&e) => {}
            Err(e) => return Err(CliError::call(&t.email, method::FILES_GET, e)),
        }
    }
    Err(CliError::failed(format!(
        "no attachment {file_id} in {}; `tern show <message-id>` lists a message's files",
        match targets {
            [one] => one.email.as_str(),
            _ => "any account",
        }
    )))
}

/// Writes the downloaded file `file_id` to `out` with `files.read`, chunk by chunk.
fn copy_to(engines: &mut [Engine], t: &Target, file_id: &str, out: &mut impl Write) -> Result<u64> {
    let mut offset = 0;
    loop {
        let chunk: FilesReadResult = call(
            engines,
            t,
            method::FILES_READ,
            json!({ "accountId": t.account_id, "fileId": file_id, "offset": offset,
                    "len": FILES_READ_MAX_BYTES }),
        )?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(chunk.data.as_bytes())
            .map_err(|e| CliError::failed(format!("files.read sent bad base64: {e}")))?;
        out.write_all(&bytes)
            .map_err(|e| CliError::failed(format!("could not write the attachment: {e}")))?;
        offset += bytes.len() as u64;
        if chunk.eof || bytes.is_empty() {
            return Ok(offset);
        }
    }
}

/// Saves `file_id` at `path`, through a temporary file beside it, so a failed download
/// leaves nothing behind.
fn save_file(engines: &mut [Engine], t: &Target, file_id: &str, path: &Path) -> Result<u64> {
    let tmp = path.with_file_name(format!(
        ".{}.tern-partial",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let write = |engines: &mut [Engine]| -> Result<u64> {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| CliError::failed(format!("could not create {}: {e}", tmp.display())))?;
        let n = copy_to(engines, t, file_id, &mut f)?;
        f.sync_all()
            .map_err(|e| CliError::failed(format!("could not write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| CliError::failed(format!("could not write {}: {e}", path.display())))?;
        Ok(n)
    };
    let r = write(engines);
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn not_downloaded(name: &str) -> CliError {
    CliError::failed(format!(
        "{name} hasn't downloaded; check the account is online with `tern status` and try again"
    ))
}

fn default_name(filename: &str) -> String {
    tern_client::files::safe_file_name(if filename.is_empty() {
        "attachment"
    } else {
        filename
    })
}

pub fn attachment_get(
    ctx: &Ctx,
    engines: &mut [Engine],
    file_id: &str,
    output_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let (t, got) = locate(engines, &targets, file_id)?;
    if !got.downloaded {
        return Err(not_downloaded(&format!("attachment {file_id}")));
    }
    if output_path.is_some_and(|p| p.as_os_str() == "-") {
        let mut out = std::io::stdout().lock();
        copy_to(engines, &t, file_id, &mut out)?;
        let _ = out.flush();
        return Ok(());
    }
    let path = match output_path {
        Some(p) if p.is_dir() => p.join(default_name(&got.filename)),
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(default_name(&got.filename)),
    };
    if path.exists() && !force {
        return Err(exists_error(&path));
    }
    let size = save_file(engines, &t, file_id, &path)?;
    let path = std::path::absolute(&path).unwrap_or(path);
    if ctx.json {
        output::json(&json!({
            "accountId": t.account_id,
            "fileId": file_id,
            "filename": got.filename,
            "contentType": got.content_type,
            "size": size,
            "path": path.display().to_string(),
        }));
    } else {
        output::line(&path.display().to_string());
    }
    Ok(())
}

/// `name`, or `stem (2).ext` and so on when an earlier file of the same message took it.
fn unique_name(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == name) {
        return name.to_owned();
    }
    let (stem, ext) = match name.rfind('.').filter(|i| *i > 0) {
        Some(i) => (&name[..i], &name[i..]),
        None => (name, ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|c| !taken.iter().any(|t| t == c))
        .unwrap_or_default()
}

pub fn attachment_save(
    ctx: &Ctx,
    engines: &mut [Engine],
    message_id: &str,
    dir: Option<&Path>,
    force: bool,
) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let ids = [message_id.to_owned()];
    let groups = by_account(engines, &targets, &ids, method::MESSAGES_GET, "message")?;
    let t = groups[0].0.clone();
    require(engines, &t, feature::FILES, "read attachments")?;
    let r: Value = call(
        engines,
        &t,
        method::MESSAGES_GET,
        json!({ "accountId": t.account_id, "id": message_id }),
    )?;
    let files = r["files"].as_array().cloned().unwrap_or_default();
    if files.is_empty() {
        return Err(CliError::failed(format!(
            "message {message_id} has no attachments"
        )));
    }
    let dir = dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut names: Vec<String> = Vec::new();
    for f in &files {
        let name = unique_name(&default_name(str_of(f, "filename")), &names);
        names.push(name);
    }
    let paths: Vec<PathBuf> = names.iter().map(|n| dir.join(n)).collect();
    if !force {
        if let Some(p) = paths.iter().find(|p| p.exists()) {
            return Err(exists_error(p));
        }
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| CliError::failed(format!("could not make {}: {e}", dir.display())))?;
    let mut saved = Vec::new();
    for (f, path) in files.iter().zip(&paths) {
        let file_id = str_of(f, "id");
        let got: FilesGetResult = call(
            engines,
            &t,
            method::FILES_GET,
            json!({ "accountId": t.account_id, "fileId": file_id }),
        )?;
        if !got.downloaded {
            return Err(not_downloaded(&format!(
                "{} ({file_id})",
                str_of(f, "filename")
            )));
        }
        let size = save_file(engines, &t, file_id, path)?;
        let path = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        if !ctx.json {
            output::line(&path.display().to_string());
        }
        saved.push(json!({
            "fileId": file_id,
            "filename": str_of(f, "filename"),
            "contentType": str_of(f, "contentType"),
            "size": size,
            "path": path.display().to_string(),
        }));
    }
    if ctx.json {
        output::json(&json!({
            "accountId": t.account_id,
            "messageId": message_id,
            "files": saved,
        }));
    }
    Ok(())
}

// -- tern export ---------------------------------------------------------------------------

/// The output path, absolute (the engine writes it), refused when it exists without
/// `--force`.
fn output_path(path: &Path, force: bool) -> Result<PathBuf> {
    if path.is_dir() {
        return Err(CliError::usage(format!(
            "{} is a directory; give a file name",
            path.display()
        )));
    }
    if path.exists() && !force {
        return Err(exists_error(path));
    }
    std::path::absolute(path).map_err(|e| CliError::usage(format!("{}: {e}", path.display())))
}

/// Queues `task` for `t` and waits until it has finished; its final JSON, or its error.
fn run_task(
    mut engines: Vec<Engine>,
    t: &Target,
    mut task: Value,
    timeout: Option<u64>,
) -> Result<Value> {
    stamp(&mut task);
    let params = tern_ipc::SubscribeParams {
        accounts: Some(vec![t.account_id.clone()]),
        classes: Some(vec![TASK_CLASS.to_owned()]),
    };
    engines[t.engine]
        .client
        .subscribe(&params)
        .map_err(|e| CliError::call(&t.email, method::SUBSCRIBE, e))?;
    queue(&mut engines, t, &task)?;
    let id = str_of(&task, "id").to_owned();
    let engine = engines.swap_remove(t.engine);
    let subs = Subscriptions::start(vec![(engine, true)], None, None)?;
    let done = wait_all(subs, std::slice::from_ref(&id), timeout)?
        .pop()
        .unwrap_or(Value::Null);
    if !done["error"].is_null() {
        return Err(CliError::failed(format!(
            "{} {id} failed: {}",
            str_of(&task, "__cls"),
            describe_error(&done["error"])
        )));
    }
    if done["status"] == "cancelled" {
        return Err(CliError::failed(format!("task {id} was cancelled")));
    }
    Ok(done)
}

pub fn export_message(
    ctx: &Ctx,
    mut engines: Vec<Engine>,
    message_id: &str,
    output: &Path,
    force: bool,
    timeout: Option<u64>,
) -> Result<()> {
    let path = output_path(output, force)?;
    let targets = discovery::targets(&engines, ctx.account())?;
    let ids = [message_id.to_owned()];
    let groups = by_account(
        &mut engines,
        &targets,
        &ids,
        method::MESSAGES_GET,
        "message",
    )?;
    let t = groups[0].0.clone();
    let task =
        builders::get_message_rfc2822(&t.account_id, message_id, &path.display().to_string());
    let done = run_task(
        engines,
        &t,
        task,
        Some(timeout.unwrap_or(MESSAGE_EXPORT_WAIT)),
    )?;
    // The engine logs a failed write and still completes the task.
    let size = std::fs::metadata(&path)
        .map_err(|_| {
            CliError::failed(format!(
                "the engine could not write {}; see `tern logs`",
                path.display()
            ))
        })?
        .len();
    if ctx.json {
        output::json(&json!({
            "accountId": t.account_id,
            "messageId": message_id,
            "path": path.display().to_string(),
            "size": size,
            "taskId": done["id"],
        }));
    } else {
        output::line(&path.display().to_string());
    }
    Ok(())
}

/// A folder (or Gmail label) of `t` by id, path, the path as a person writes it, or role.
fn find_folder(engines: &mut [Engine], t: &Target, name: &str) -> Result<Value> {
    let (mut list, _) = list_categories(engines, t, Kind::Folder)?;
    list.extend(list_categories(engines, t, Kind::Label)?.0);
    let lower = name.to_lowercase();
    list.iter()
        .find(|c| str_of(c, "id") == name)
        .or_else(|| {
            list.iter()
                .find(|c| str_of(c, "path").eq_ignore_ascii_case(name))
        })
        .or_else(|| {
            list.iter()
                .find(|c| display_name(c).to_lowercase() == lower)
        })
        .or_else(|| list.iter().find(|c| str_of(c, "role") == lower))
        .cloned()
        .ok_or_else(|| {
            CliError::failed(format!(
                "{}: no folder {name}; `tern folders` lists them",
                t.email
            ))
        })
}

pub fn export_mbox(
    ctx: &Ctx,
    mut engines: Vec<Engine>,
    folder_name: &str,
    output: &Path,
    force: bool,
    timeout: Option<u64>,
) -> Result<()> {
    let path = output_path(output, force)?;
    let t = discovery::one_target(&engines, ctx.account())?;
    let folder = find_folder(&mut engines, &t, folder_name)?;
    // Per export, as the desktop names them, so a lingering earlier export can't share them.
    let token = format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        std::process::id()
    );
    let staging = PathBuf::from(format!("{}.{token}.partial", path.display()));
    let working = PathBuf::from(format!("{}.{token}.incomplete", path.display()));
    std::fs::create_dir_all(&staging)
        .map_err(|e| CliError::failed(format!("could not make {}: {e}", staging.display())))?;
    let task = builders::get_many_rfc2822(&t.account_id, &folder, &staging.display().to_string());
    let done = match run_task(engines, &t, task, timeout) {
        Ok(d) => d,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let result = &done["result"];
    let (total, exported, failed) = (
        result["total"].as_u64().unwrap_or(0),
        result["exported"].as_u64().unwrap_or(0),
        result["failed"].as_u64().unwrap_or(0),
    );
    if done["progress"]["cancelled"] == true || failed > 0 || exported != total {
        return Err(CliError::failed(format!(
            "the export stopped after {exported} of {total} messages ({failed} failed); {} \
             holds the ones exported, and {} is unchanged",
            staging.display(),
            path.display()
        )));
    }
    let assembled = (|| -> std::io::Result<usize> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&working)?);
        let n = crate::mbox::assemble(&staging, &mut out)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(&working, &path)?;
        Ok(n)
    })();
    let n = assembled.map_err(|e| {
        let _ = std::fs::remove_file(&working);
        CliError::failed(format!(
            "could not write {}: {e}; the exported messages are in {}",
            path.display(),
            staging.display()
        ))
    })?;
    let _ = std::fs::remove_dir_all(&staging);
    if ctx.json {
        output::json(&json!({
            "accountId": t.account_id,
            "folder": display_name(&folder),
            "folderId": str_of(&folder, "id"),
            "path": path.display().to_string(),
            "messages": n,
            "taskId": done["id"],
        }));
    } else {
        output::line(&format!(
            "{} ({n} message{})",
            path.display(),
            if n == 1 { "" } else { "s" }
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_one_message_stay_apart() {
        let taken = vec!["a.pdf".to_owned(), "a (2).pdf".to_owned()];
        assert_eq!(unique_name("a.pdf", &taken), "a (3).pdf");
        assert_eq!(unique_name("b.pdf", &taken), "b.pdf");
        assert_eq!(unique_name(".rc", &[".rc".to_owned()]), ".rc (2)");
        assert_eq!(default_name(""), "attachment");
        assert_eq!(default_name("../x"), "..-x");
    }
}
