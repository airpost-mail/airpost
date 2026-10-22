// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `get_attachment` and the `attachment://{messageId}/{fileId}` resource. The engine saves
//! attachments under `<config-dir>/files/` when it fetches a message's body, at the path
//! `AttachmentStore.pathForFile` computes; these read them from there. An engine with
//! `files` finds the file and downloads it first with `files.get`; for an older one these
//! ask for the body and poll the directory.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use super::{failed, req_str, schema, Ctx, Output, ToolDef, ToolError, ToolResult};
use crate::model::{file_display_name, str_of};
use crate::policy::Category;

/// Attachments up to this size come back inline (text, or an image block).
const MAX_INLINE_BYTES: u64 = 200 * 1024;
/// Waits for the file to appear: long when the body still has to be downloaded, short
/// when it is local already (the file should be there, or never will be).
const DOWNLOAD_WAIT: Duration = Duration::from_secs(30);
const RECHECK_WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(500);

pub const URI_SCHEME: &str = "attachment";

pub(super) fn definitions() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "get_attachment",
        description: "View the contents of an email attachment. Find the attachment's fileId in the `files` array returned by get_message or get_thread (use search_mail's has:attachment to find mail with attachments). Every result starts with a JSON metadata block (filename, contentType, size, plus the file's local `path` and its MCP `resourceUri` — this server only runs on the same machine, so the path is directly readable by local tools). Text-based attachments (text/*, JSON, XML, SVG, ICS, EML) up to 200KB include their content in the JSON as UTF-8 `data`; images (PNG/JPEG/GIF/WebP) up to 200KB are additionally returned as an MCP image content block so they can be viewed directly; any other type or size returns no inline payload — instead the result ends with a resource_link content block whose URI can be fetched via resources/read, or the file can be read straight from `path`. If the attachment hasn't been downloaded from the mail server yet it is fetched first, which can take several seconds.",
        schema: schema(
            json!({
                "messageId": {"type": "string", "description": "The ID of the message the attachment belongs to"},
                "fileId": {"type": "string", "description": "The attachment's file ID (from the message's `files` array)"}
            }),
            &["messageId", "fileId"],
        ),
        category: Category::Read,
    }]
}

fn extension_mime(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "yml" | "yaml" => "text/yaml",
        "ics" => "text/calendar",
        "eml" => "message/rfc822",
        "pdf" => "application/pdf",
        _ => return None,
    })
}

/// The declared type, else one from the extension (`mimeTypeForFile`).
pub fn mime_type(file: &Value) -> String {
    let declared = str_of(file, "contentType")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !declared.is_empty() && declared != "application/octet-stream" {
        return declared;
    }
    let name = safe_display_name(file).to_lowercase();
    let ext = Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    extension_mime(ext)
        .unwrap_or("application/octet-stream")
        .to_owned()
}

fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
        || [
            "application/json",
            "application/javascript",
            "application/x-javascript",
            "application/xml",
            "application/xhtml+xml",
            "image/svg+xml",
            "message/rfc822",
        ]
        .contains(&mime)
}

fn is_viewable_image(mime: &str) -> bool {
    ["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&mime)
}

/// `File.safeDisplayName()`.
pub fn safe_display_name(file: &Value) -> String {
    tern_client::files::safe_file_name(&file_display_name(file))
}

/// `AttachmentStore.pathForFile`: `files/<id[0..2]>/<id[2..4]>/<id>/<safe name>`, or None
/// for an id that could leave the files directory.
pub fn path_for_file(config_dir: &Path, file: &Value) -> Option<PathBuf> {
    let id = str_of(file, "id").to_lowercase();
    if id.len() < 4 || !id.is_ascii() || id.contains(['/', '\\']) || id.contains("..") {
        return None;
    }
    Some(
        config_dir
            .join("files")
            .join(&id[..2])
            .join(&id[2..4])
            .join(&id)
            .join(safe_display_name(file)),
    )
}

/// Where the engine saved the file, if it has. As in `AttachmentStore`, a directory with
/// exactly one entry holds the file under an altered name.
fn path_on_disk(config_dir: &Path, file: &Value) -> Option<PathBuf> {
    let p = path_for_file(config_dir, file)?;
    if p.is_file() {
        return Some(p);
    }
    let dir = p.parent()?;
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != ".DS_Store")
        .map(|e| e.path())
        .collect();
    match entries.as_slice() {
        [only] if only.is_file() => Some(only.clone()),
        _ => None,
    }
}

pub fn resource_uri(message_id: &str, file_id: &str) -> String {
    format!(
        "{URI_SCHEME}://{}/{}",
        percent_encode(message_id),
        percent_encode(file_id)
    )
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn find_file(message: &Value, file_id: &str) -> Option<Value> {
    crate::model::array_of(message, "files")
        .iter()
        .find(|f| str_of(f, "id") == file_id)
        .cloned()
}

pub(super) fn get_attachment(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let message_id = req_str(args, "messageId")?;
    let file_id = req_str(args, "fileId")?;
    let (account, message) = ctx.find_message(&message_id, None)?;
    let aid = account.account_id.clone();
    let file = find_file(&message, &file_id).ok_or_else(|| {
        failed(format!(
            "Message '{message_id}' has no attachment with id '{file_id}'"
        ))
    })?;
    if path_for_file(ctx.config_dir, &file).is_none() {
        return Err(failed(format!(
            "Attachment '{file_id}' has an unsafe filename and cannot be resolved on disk"
        )));
    }
    let mut path = if ctx.backend.has_feature(&aid, tern_ipc::feature::FILES) {
        engine_path(ctx, &aid, &file_id)?
    } else {
        path_on_disk(ctx.config_dir, &file)
    };
    if path.is_none() && !ctx.backend.has_feature(&aid, tern_ipc::feature::FILES) {
        let r = ctx.call(
            &aid,
            tern_ipc::method::MESSAGES_GET,
            json!({"id": message_id, "body": true}),
        )?;
        let has_body = r.get("body").is_some_and(|b| !b.is_null());
        ctx.call(
            &aid,
            tern_ipc::method::BODIES_FETCH,
            json!({"ids": [message_id]}),
        )?;
        let deadline = Instant::now()
            + if has_body {
                RECHECK_WAIT
            } else {
                DOWNLOAD_WAIT
            };
        while path.is_none() && Instant::now() < deadline {
            std::thread::sleep(POLL);
            path = path_on_disk(ctx.config_dir, &file);
        }
    }
    let Some(path) = path else {
        return Err(failed(format!(
            "Attachment '{}' is not available locally and could not be downloaded. Open the \
             message in Tern to download it, then try again.",
            file_display_name(&file)
        )));
    };
    let size = std::fs::metadata(&path)
        .map_err(|e| failed(format!("Could not read the attachment: {e}")))?
        .len();
    let mime = mime_type(&file);
    let mut metadata = json!({
        "id": str_of(&file, "id"),
        "filename": file_display_name(&file),
        "contentType": mime,
        "size": size,
        "isInline": !str_of(&file, "contentId").is_empty(),
        "messageId": str_of(&message, "id"),
        "threadId": str_of(&message, "threadId"),
        "accountId": aid,
        "path": path.display().to_string(),
        "resourceUri": resource_uri(str_of(&message, "id"), str_of(&file, "id")),
    });
    let read =
        || std::fs::read(&path).map_err(|e| failed(format!("Could not read the attachment: {e}")));
    if size <= MAX_INLINE_BYTES {
        if is_text(&mime) {
            metadata["data"] = Value::from(String::from_utf8_lossy(&read()?).into_owned());
            return Ok(Output::text(&metadata).with_account(&aid));
        }
        if is_viewable_image(&mime) {
            let data = base64::engine::general_purpose::STANDARD.encode(read()?);
            metadata["note"] =
                Value::from("The image follows this JSON as an image content block.");
            let mut o = Output::text(&metadata).with_account(&aid);
            o.content
                .push(json!({"type": "image", "data": data, "mimeType": mime}));
            return Ok(o);
        }
    }
    metadata["note"] = Value::from(
        "Content is not returned inline for this type/size. Read the file at `path` with local \
         tools, or fetch `resourceUri` via resources/read.",
    );
    let mut o = Output::text(&metadata).with_account(&aid);
    o.content.push(json!({
        "type": "resource_link",
        "uri": metadata["resourceUri"],
        "name": file_display_name(&file),
        "mimeType": mime,
    }));
    Ok(o)
}

/// Where `files.get` says the file is, downloading it first; None when it couldn't be.
fn engine_path(
    ctx: &mut Ctx<'_>,
    account: &str,
    file_id: &str,
) -> Result<Option<PathBuf>, ToolError> {
    let got: tern_ipc::FilesGetResult = serde_json::from_value(ctx.call(
        account,
        tern_ipc::method::FILES_GET,
        json!({ "fileId": file_id }),
    )?)
    .map_err(|e| failed(format!("Unexpected files.get answer: {e}")))?;
    Ok(got.path.filter(|_| got.downloaded).map(PathBuf::from))
}

/// `resources/read` of an attachment URI: `(contents, accountId)`. An engine with `files`
/// downloads a missing attachment first; with an older one it must be on disk already
/// (get_attachment downloads it).
pub fn read_attachment_resource(
    ctx: &mut Ctx<'_>,
    uri: &str,
) -> Result<(Value, Option<String>), ToolError> {
    let rest = uri
        .strip_prefix(&format!("{URI_SCHEME}://"))
        .ok_or_else(|| failed(format!("Unknown resource '{uri}'")))?;
    let (m, f) = rest
        .split_once('/')
        .ok_or_else(|| failed(format!("Unknown resource '{uri}'")))?;
    let (message_id, file_id) = percent_decode(m)
        .zip(percent_decode(f))
        .ok_or_else(|| failed(format!("Unknown resource '{uri}'")))?;
    let (account, message) = ctx.find_message(&message_id, None)?;
    let file = find_file(&message, &file_id).ok_or_else(|| {
        failed(format!(
            "Message '{message_id}' has no attachment with id '{file_id}'"
        ))
    })?;
    let aid = account.account_id.clone();
    let path = if ctx.backend.has_feature(&aid, tern_ipc::feature::FILES) {
        engine_path(ctx, &aid, &file_id)?
    } else {
        path_on_disk(ctx.config_dir, &file)
    };
    let path = path.ok_or_else(|| {
        failed(format!(
            "Attachment '{}' has not been downloaded — call the get_attachment tool first.",
            file_display_name(&file)
        ))
    })?;
    let bytes =
        std::fs::read(&path).map_err(|e| failed(format!("Could not read the attachment: {e}")))?;
    Ok((
        json!({"contents": [{
            "uri": uri,
            "mimeType": mime_type(&file),
            "blob": base64::engine::general_purpose::STANDARD.encode(bytes),
        }]}),
        Some(account.account_id),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_names_and_types() {
        let f = json!({"id": "AbCdEf", "filename": "re\u{202E}port:1.txt. ", "contentType": "application/octet-stream"});
        assert_eq!(safe_display_name(&f), "report-1.txt");
        let p = path_for_file(Path::new("/cfg"), &f).unwrap();
        assert_eq!(p, Path::new("/cfg/files/ab/cd/abcdef/report-1.txt"));
        assert_eq!(mime_type(&f), "text/plain");
        assert_eq!(
            safe_display_name(&json!({"id": "x", "filename": "CON.txt"})),
            "_CON.txt"
        );
        assert!(path_for_file(Path::new("/cfg"), &json!({"id": "../../etc"})).is_none());
        assert_eq!(
            mime_type(&json!({"filename": "a", "contentType": "Image/PNG; x=y"})),
            "image/png"
        );
        let uri = resource_uri("m/1", "f 2");
        assert_eq!(uri, "attachment://m%2F1/f%202");
        assert_eq!(percent_decode("m%2F1").as_deref(), Some("m/1"));
    }
}
