// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Attachments by id ([`feature::FILES`](crate::feature::FILES)):
//! `files.get`, `files.read` and `files.put`. Local clients only
//! ([`policy`](crate::policy)): `files.get` hands out paths and `files.put` reads one, so
//! browsers reach files through `tern-gateway`'s HTTP routes instead.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most bytes one `files.read` returns; a larger `len` is clamped to it.
pub const FILES_READ_MAX_BYTES: u64 = 4 << 20;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilesGetParams {
    pub account_id: String,
    pub file_id: String,
}

/// `files.get`: the file as it is now. The engine asks for the message's body when the file
/// isn't on disk and answers once it is there, or with `downloaded: false` (and no `path`)
/// when it doesn't appear in time.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilesGetResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Bytes on disk when downloaded, else the size the File records.
    pub size: u64,
    pub content_type: String,
    pub downloaded: bool,
    /// The File's `filename`.
    #[serde(default)]
    pub filename: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilesReadParams {
    pub account_id: String,
    pub file_id: String,
    #[serde(default)]
    pub offset: u64,
    /// At most [`FILES_READ_MAX_BYTES`]; the default.
    #[serde(default)]
    pub len: Option<u64>,
}

/// `files.read`: one chunk of a downloaded file, base64 (standard alphabet, padded).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilesReadResult {
    pub data: String,
    pub offset: u64,
    /// The file's whole size.
    pub size: u64,
    /// The chunk ends at the end of the file.
    pub eof: bool,
}

/// `files.put`: one of `data` (base64) or `path` (a file the engine copies; the caller may
/// delete it once answered). Refused on relayed connections even with the admin scope, so a
/// remote device can't read the engine's file system.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilesPutParams {
    pub account_id: String,
    pub name: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// `files.put`'s answer: the new File's id, and the File JSON a draft's `files` carries.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilesPutResult {
    pub file_id: String,
    #[schemars(with = "crate::models::FileJson")]
    pub file: Value,
}
