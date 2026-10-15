// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! `/ping`, `/api/me`, the compatibility stubs and account removal (docs/backend-plan.md 3.4).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use super::{ApiError, AppState, AuthUser};
use crate::wire::Identity;

pub async fn ping() -> Json<Value> {
    Json(json!({}))
}

pub async fn me(State(state): State<AppState>, AuthUser(auth): AuthUser) -> Json<Identity> {
    Json(state.identity(&auth.user, None))
}

/// No quotas, but `markUsed` still queues the task and C++ runs it (section 1).
pub async fn feature_usage_event(_auth: AuthUser) -> StatusCode {
    StatusCode::NO_CONTENT
}

pub async fn not_found() -> ApiError {
    ApiError::NotFound
}

/// The client's account id (`idForAccount`, 8 hex chars). Accepts any short id of safe
/// characters, since only the path segment's shape matters to the server.
pub fn valid_aid(aid: &str) -> bool {
    (1..=64).contains(&aid.len())
        && aid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Deletes the account's metadata, claims and tracking events, and closes its streams.
/// Idempotent.
pub async fn delete_account(
    State(state): State<AppState>,
    AuthUser(auth): AuthUser,
    Path(aid): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !valid_aid(&aid) {
        return Err(ApiError::BadRequest("invalid account id".into()));
    }
    let user_id = auth.user.id;
    let hub = state.hub().clone();
    // Closed inside the blocking call, which finishes even if the client goes away.
    state
        .blocking(move |r| {
            r.delete_account(&user_id, &aid)?;
            hub.close_account(&user_id, &aid);
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::valid_aid;

    #[test]
    fn aids() {
        assert!(valid_aid("0a1b2c3d"));
        assert!(valid_aid("acc_1"));
        assert!(!valid_aid(""));
        assert!(!valid_aid("a/b"));
        assert!(!valid_aid("a b"));
        assert!(!valid_aid(&"a".repeat(65)));
    }
}
