// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Metadata push, backfill, head and pages, and expiration claims
//! (3.8). The stream is in `stream.rs`.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::Json;
use serde_json::{json, Value};

use super::api::valid_aid;
use super::{ApiError, AppState, AuthUser};
use crate::ids;
use crate::repo::{ClaimOutcome, MetadataWrite};
use crate::wire::{
    BackfillRecord, BackfillResponse, HeadResponse, MetadataPush, MetadataRecord, PushResponse,
    BACKFILL_MAX_RECORDS, MAX_VALUE_BYTES, PAGE_SIZE,
};

/// Room in a push body for the fields around `value`.
pub const PUSH_BODY_LIMIT: usize = MAX_VALUE_BYTES + 4 * 1024;
const MAX_OBJECT_ID_BYTES: usize = 512;
const MAX_PLUGIN_ID_BYTES: usize = 200;
/// RFC 5322 caps a line at 998 characters, which bounds a Message-ID.
const MAX_HEADER_MESSAGE_ID_BYTES: usize = 998;

fn valid_key(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

fn check_aid(aid: &str) -> Result<(), ApiError> {
    if valid_aid(aid) {
        Ok(())
    } else {
        Err(ApiError::BadRequest("invalid account id".into()))
    }
}

/// Validates the parts of a write that come from the client.
fn metadata_write(
    object_type: crate::wire::ObjectType,
    object_id: String,
    plugin_id: String,
    version: Option<u64>,
    value: Value,
    header_message_id: Option<String>,
) -> Result<MetadataWrite, ApiError> {
    if !valid_key(&object_id, MAX_OBJECT_ID_BYTES) {
        return Err(ApiError::BadRequest("invalid object id".into()));
    }
    if !valid_key(&plugin_id, MAX_PLUGIN_ID_BYTES) {
        return Err(ApiError::BadRequest("invalid plugin id".into()));
    }
    if header_message_id
        .as_deref()
        .is_some_and(|h| !valid_key(h, MAX_HEADER_MESSAGE_ID_BYTES))
    {
        return Err(ApiError::BadRequest("invalid headerMessageId".into()));
    }
    let size = serde_json::to_string(&value).map_or(usize::MAX, |s| s.len());
    if size > MAX_VALUE_BYTES {
        return Err(ApiError::PayloadTooLarge);
    }
    Ok(MetadataWrite {
        object_type,
        object_id,
        plugin_id,
        version,
        value,
        header_message_id,
    })
}

fn parse_json<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|e| ApiError::BadRequest(format!("invalid body: {e}")))
}

/// `POST /metadata/<aid>/<objectId>/<pluginId>`.
pub async fn push(
    State(state): State<AppState>,
    AuthUser(auth): AuthUser,
    Path((aid, object_id, plugin_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<PushResponse>, ApiError> {
    check_aid(&aid)?;
    let p: MetadataPush = parse_json(&body)?;
    let w = metadata_write(
        p.object_type,
        object_id,
        plugin_id,
        p.version,
        p.value,
        p.header_message_id,
    )?;
    let user_id = auth.user.id;
    let hub = state.hub().clone();
    // Notified inside the blocking call: it runs to the end even when the client goes away
    // and the handler is dropped, and a commit nobody is told about would sit undelivered.
    let written = state
        .blocking(move |r| {
            let written = r.push_metadata(&user_id, &aid, &w, ids::now_ms())?;
            hub.notify(&user_id, &aid, written.seq);
            Ok(written)
        })
        .await?;
    Ok(Json(PushResponse { v: written.v }))
}

/// `POST /airpost/v1/backfill/<aid>`. Records that fail validation (a value over 64 KiB, bad ids,
/// an unknown object type) are skipped and their indices returned in `rejected`, so one bad
/// local value does not hold back the rest of the batch.
pub async fn backfill(
    State(state): State<AppState>,
    AuthUser(auth): AuthUser,
    Path(aid): Path<String>,
    body: Bytes,
) -> Result<Json<BackfillResponse>, ApiError> {
    check_aid(&aid)?;
    let records: Vec<Value> = parse_json(&body)?;
    if records.len() > BACKFILL_MAX_RECORDS {
        return Err(ApiError::BadRequest(format!(
            "at most {BACKFILL_MAX_RECORDS} records per backfill"
        )));
    }
    let mut writes = Vec::with_capacity(records.len());
    let mut rejected = Vec::new();
    for (i, record) in records.into_iter().enumerate() {
        let w = serde_json::from_value::<BackfillRecord>(record)
            .map_err(|e| ApiError::BadRequest(e.to_string()))
            .and_then(|r| {
                metadata_write(
                    r.object_type,
                    r.object_id,
                    r.plugin_id,
                    r.version,
                    r.value,
                    r.header_message_id,
                )
            });
        match w {
            Ok(w) => writes.push(w),
            Err(_) => rejected.push(i),
        }
    }
    let user_id = auth.user.id;
    let hub = state.hub().clone();
    let written = state
        .blocking(move |r| {
            let written = r.backfill_metadata(&user_id, &aid, &writes, ids::now_ms())?;
            if let Some(seq) = written.iter().map(|w| w.seq).max() {
                hub.notify(&user_id, &aid, seq);
            }
            Ok(written)
        })
        .await?;
    Ok(Json(BackfillResponse {
        written: written.len(),
        rejected,
    }))
}

/// `GET /deltas/<aid>/head`. The C++ engine sends a `null` JSON body on this GET, which is
/// never read.
pub async fn head(
    State(state): State<AppState>,
    AuthUser(auth): AuthUser,
    Path(aid): Path<String>,
) -> Result<Json<HeadResponse>, ApiError> {
    check_aid(&aid)?;
    let user_id = auth.user.id;
    let seq = state
        .blocking(move |r| r.metadata_head(&user_id, &aid))
        .await?;
    Ok(Json(HeadResponse {
        cursor: seq.to_string(),
    }))
}

fn query_number(q: &HashMap<String, String>, key: &str) -> Result<Option<u64>, ApiError> {
    match q.get(key).map(String::as_str) {
        None | Some("") => Ok(None),
        Some(s) => s
            .parse()
            .map(Some)
            .map_err(|_| ApiError::BadRequest(format!("invalid {key}"))),
    }
}

/// `GET /metadata/<aid>?limit=&offset=`: rows in rowid order, tombstones included. `limit`
/// is capped at the page size.
pub async fn page(
    State(state): State<AppState>,
    AuthUser(auth): AuthUser,
    Path(aid): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Vec<MetadataRecord>>, ApiError> {
    check_aid(&aid)?;
    let limit = query_number(&q, "limit")?.map_or(PAGE_SIZE, |l| (l as usize).clamp(1, PAGE_SIZE));
    let offset = query_number(&q, "offset")?.unwrap_or(0);
    let user_id = auth.user.id;
    let rows = state
        .blocking(move |r| r.metadata_page(&user_id, &aid, limit, offset))
        .await?;
    Ok(Json(rows))
}

/// `POST /airpost/v1/claim/<aid>/<objectId>/<pluginId>/<expiration>`: 200 for the device that
/// holds the claim, 409 for every other.
pub async fn claim(
    State(state): State<AppState>,
    AuthUser(auth): AuthUser,
    Path((aid, object_id, plugin_id, expiration)): Path<(String, String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    check_aid(&aid)?;
    if !valid_key(&object_id, MAX_OBJECT_ID_BYTES) || !valid_key(&plugin_id, MAX_PLUGIN_ID_BYTES) {
        return Err(ApiError::BadRequest("invalid object or plugin id".into()));
    }
    let expiration: i64 = expiration
        .parse()
        .ok()
        .filter(|e| *e >= 0)
        .ok_or_else(|| ApiError::BadRequest("invalid expiration".into()))?;
    let (user_id, token_id) = (auth.user.id, auth.token_id);
    let outcome = state
        .blocking(move |r| {
            r.claim_expiration(
                &user_id,
                &aid,
                &object_id,
                &plugin_id,
                expiration,
                token_id,
                ids::now_ms(),
            )
        })
        .await?;
    match outcome {
        ClaimOutcome::Granted => Ok(Json(json!({}))),
        ClaimOutcome::Taken => Err(ApiError::Conflict(
            "already claimed by another device".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::ObjectType;

    #[test]
    fn write_validation() {
        let w = |oid: &str, pid: &str, value: Value, hmid: Option<&str>| {
            metadata_write(
                ObjectType::Thread,
                oid.into(),
                pid.into(),
                None,
                value,
                hmid.map(str::to_owned),
            )
        };
        assert!(w("t:abc", "thread-snooze", json!({}), Some("<a@b>")).is_ok());
        assert!(matches!(
            w("", "p", json!({}), None),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            w("o", "", json!({}), None),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            w("o\n", "p", json!({}), None),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            w(&"o".repeat(MAX_OBJECT_ID_BYTES + 1), "p", json!({}), None),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            w("o", "p", json!({}), Some("")),
            Err(ApiError::BadRequest(_))
        ));
        // The value limit counts the serialized value: a string of N bytes is N + 2 quoted.
        assert!(w("o", "p", json!("x".repeat(MAX_VALUE_BYTES - 2)), None).is_ok());
        assert!(matches!(
            w("o", "p", json!("x".repeat(MAX_VALUE_BYTES - 1)), None),
            Err(ApiError::PayloadTooLarge)
        ));
    }
}
