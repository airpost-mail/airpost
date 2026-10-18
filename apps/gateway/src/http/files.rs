// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `/gateway/v1/files/…`: attachments for paired browsers (docs/web-gateway-plan.md 2.5),
//! on the engine's `files.get` and `files.put` over a connection of the gateway's own.
//!
//! Downloads are served as attachments with `Content-Security-Policy: sandbox`, whatever the
//! sender called them. `?inline=1` serves inline only a PNG, JPEG, GIF or WebP recognised
//! from its first bytes, with the gateway's `Content-Type`, so an `<img>` can show a `cid:`
//! part and nothing a sender labelled `image/png` can run as a page. Uploads stream to
//! `<config>/gateway/incoming/` under a size cap, go to the engine by path and are deleted.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::header::{
    CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE,
};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use ms_ipc::{code, feature, method, FilesGetResult, FilesPutResult};
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use super::{ApiError, AppState, Paired};
use crate::engine::{self, CallError};

/// The largest upload, as most mail servers' message limit.
pub const MAX_UPLOAD_BYTES: u64 = 25 << 20;
/// `files.get` may wait out a body download; the engine gives up after 30 s.
const GET_TIMEOUT: Duration = Duration::from_secs(45);
const PUT_TIMEOUT: Duration = Duration::from_secs(60);
/// Uploads left behind (a crash between the write and the delete) are removed after this.
const INCOMING_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
const INCOMING_DIR: &str = "incoming";
/// Where the engine keeps attachments under the profile (`ms_core::constants::FILES_DIR`).
const FILES_DIR: &str = "files";

fn incoming_dir(state: &AppState) -> PathBuf {
    state.config().state_dir.join(INCOMING_DIR)
}

/// Removes uploads older than a day.
pub fn sweep_incoming(state_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(state_dir.join(INCOMING_DIR)) else {
        return;
    };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > INCOMING_MAX_AGE);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Ids are passed to the engine as JSON strings; this only keeps junk out of its log.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

fn engine_error(e: CallError) -> ApiError {
    match e {
        CallError::Down => ApiError::Unavailable("The engine isn't running."),
        CallError::TooOld => ApiError::Unavailable("The engine is too old to serve files."),
        CallError::Rpc(e) if e.code == code::NOT_FOUND || e.code == code::UNKNOWN_ACCOUNT => {
            ApiError::NotFound
        }
        CallError::Rpc(e) if e.code == code::INVALID_PARAMS => {
            tracing::info!(message = %e.message, "the engine refused a file request");
            ApiError::BadRequest("The engine refused the file.")
        }
        CallError::Rpc(e) => {
            tracing::warn!(code = e.code, message = %e.message, "files call failed");
            ApiError::Unavailable("The engine couldn't handle the file.")
        }
    }
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    #[serde(default)]
    inline: Option<String>,
}

/// The raster image type of `head`, if it is one the browser may render inline.
pub fn sniff_image(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// `Content-Disposition` with an ASCII `filename` and the exact name in `filename*`
/// (RFC 6266, RFC 8187).
pub fn disposition(kind: &str, name: &str) -> HeaderValue {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    HeaderValue::from_str(&format!(
        "{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

/// `GET` (and `HEAD`, which also makes the engine download the file) of one file.
pub async fn download(
    State(state): State<AppState>,
    paired: Paired,
    UrlPath((account_id, file_id)): UrlPath<(String, String)>,
    Query(q): Query<DownloadQuery>,
    req: Request,
) -> Result<Response, ApiError> {
    if !valid_id(&account_id) || !valid_id(&file_id) {
        return Err(ApiError::NotFound);
    }
    let got = engine::call(
        &state.config().config_dir,
        feature::FILES,
        method::FILES_GET,
        json!({ "accountId": account_id, "fileId": file_id }),
        GET_TIMEOUT,
    )
    .await
    .map_err(engine_error)?;
    let got: FilesGetResult = serde_json::from_value(got).map_err(|_| ApiError::Internal)?;
    let Some(path) = got.path.filter(|_| got.downloaded) else {
        return Err(ApiError::Unavailable(
            "The attachment couldn't be downloaded from the mail server.",
        ));
    };
    let path = inside_files_dir(&state.config().config_dir, Path::new(&path)).await?;

    let inline = q.inline.as_deref().is_some_and(|v| v == "1" || v == "true");
    let image = if inline { sniff(&path).await } else { None };
    let name = if got.filename.is_empty() {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        got.filename
    };

    let res = ServeFile::new(&path)
        .oneshot(req)
        .await
        .map_err(|_| ApiError::Internal)?;
    let mut res = res.map(Body::new);
    let h = res.headers_mut();
    let content_type = image.unwrap_or("application/octet-stream");
    h.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(
        CONTENT_DISPOSITION,
        disposition(
            if image.is_some() {
                "inline"
            } else {
                "attachment"
            },
            &name,
        ),
    );
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox"));
    tracing::info!(device = %paired.device.id, inline = image.is_some(), "served a file");
    Ok(res)
}

/// The canonical `path`, refused unless it is a file under the engine's files directory.
async fn inside_files_dir(config_dir: &Path, path: &Path) -> Result<PathBuf, ApiError> {
    let root = tokio::fs::canonicalize(config_dir.join(FILES_DIR))
        .await
        .map_err(|_| ApiError::NotFound)?;
    let real = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let is_file = tokio::fs::metadata(&real).await.is_ok_and(|m| m.is_file());
    if !real.starts_with(&root) || !is_file {
        tracing::warn!("the engine named a file outside its files directory");
        return Err(ApiError::Forbidden("Not a file the gateway serves."));
    }
    Ok(real)
}

async fn sniff(path: &Path) -> Option<&'static str> {
    let mut f = tokio::fs::File::open(path).await.ok()?;
    let mut head = [0u8; 16];
    let mut n = 0;
    while n < head.len() {
        match f.read(&mut head[n..]).await.ok()? {
            0 => break,
            k => n += k,
        }
    }
    sniff_image(&head[..n])
}

#[derive(Deserialize)]
pub struct UploadQuery {
    name: String,
}

/// Deletes the upload's temporary file however the request ends.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `POST /gateway/v1/files/{accountId}?name=<file name>` with the bytes as the body and
/// their type as `Content-Type`. Answers `files.put`'s `{fileId, file}`.
pub async fn upload(
    State(state): State<AppState>,
    paired: Paired,
    UrlPath(account_id): UrlPath<String>,
    Query(q): Query<UploadQuery>,
    req: Request,
) -> Result<Response, ApiError> {
    if !valid_id(&account_id) {
        return Err(ApiError::NotFound);
    }
    let name = q.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 255 {
        return Err(ApiError::BadRequest("The file needs a name."));
    }
    let too_large = || ApiError::TooLarge("Attachments can be at most 25 MB.");
    let declared = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_UPLOAD_BYTES) {
        return Err(too_large());
    }
    let content_type = req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    let _permit = state.upload_permit().ok_or(ApiError::RateLimited {
        retry_after_secs: 5,
    })?;

    let dir = incoming_dir(&state);
    create_private_dir(&dir).await?;
    let temp = TempFile(dir.join(format!("{:016x}.part", rand::random::<u64>())));
    let mut out = open_private(&temp.0).await?;
    let mut written: u64 = 0;
    let mut body = req.into_body().into_data_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|_| ApiError::BadRequest("The upload was cut short."))?;
        written += chunk.len() as u64;
        if written > MAX_UPLOAD_BYTES {
            return Err(too_large());
        }
        out.write_all(&chunk).await.map_err(io_error)?;
    }
    out.flush().await.map_err(io_error)?;
    drop(out);

    let mut params = json!({ "accountId": account_id, "name": name, "path": temp.0 });
    if let Some(t) = content_type {
        params["contentType"] = json!(t);
    }
    let put = engine::call(
        &state.config().config_dir,
        feature::FILES,
        method::FILES_PUT,
        params,
        PUT_TIMEOUT,
    )
    .await
    .map_err(engine_error)?;
    let put: FilesPutResult = serde_json::from_value(put).map_err(|_| ApiError::Internal)?;
    tracing::info!(device = %paired.device.id, bytes = written, "stored an upload");
    Ok((StatusCode::OK, Json(put)).into_response())
}

fn io_error(e: std::io::Error) -> ApiError {
    tracing::error!(error = %e, "writing an upload");
    ApiError::Internal
}

async fn create_private_dir(dir: &Path) -> Result<(), ApiError> {
    let mut b = tokio::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    b.mode(0o700);
    b.create(dir).await.map_err(io_error)
}

async fn open_private(path: &Path) -> Result<tokio::fs::File, ApiError> {
    let mut o = tokio::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    o.mode(0o600);
    o.open(path).await.map_err(io_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_sniffed() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"<svg xmlns="), None);
        assert_eq!(sniff_image(b"<html>"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn dispositions() {
        assert_eq!(
            disposition("attachment", "report.pdf"),
            "attachment; filename=\"report.pdf\"; filename*=UTF-8''report.pdf"
        );
        let h = disposition("inline", "résumé \"x\".png");
        let s = h.to_str().unwrap();
        assert!(s.starts_with("inline; filename=\"r_sum_ _x_.png\""), "{s}");
        assert!(
            s.ends_with("filename*=UTF-8''r%C3%A9sum%C3%A9%20%22x%22.png"),
            "{s}"
        );
        // Header injection: control characters never reach the header.
        let h = disposition("attachment", "a\r\nSet-Cookie: x");
        assert!(!h.to_str().unwrap().contains('\n'));
    }
}
