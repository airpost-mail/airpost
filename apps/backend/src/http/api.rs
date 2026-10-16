// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! `/ping`, server info, `/tern/v1/me`, sign-out and account removal (docs/backend-plan.md 3.4).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use super::{ApiError, AppState, AuthUser};
use crate::config::Signup;
use crate::ids;
use crate::wire::{Me, ServerInfo, SignupMode};

pub async fn ping() -> Json<Value> {
    Json(json!({}))
}

/// `GET /tern/v1/server`, no auth.
pub async fn server_info(State(state): State<AppState>) -> Json<ServerInfo> {
    let s = &state.0;
    Json(ServerInfo {
        server_id: s.server_id.clone(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        public_url: s.public_url.clone(),
        signup: match s.signup {
            Signup::Open => SignupMode::Open,
            Signup::Closed => SignupMode::Closed,
        },
        features: s.features,
    })
}

/// `GET /tern/v1/me`.
pub async fn me(State(state): State<AppState>, AuthUser(auth): AuthUser) -> Json<Me> {
    let s = &state.0;
    let user = auth.user;
    Json(Me {
        user_id: user.id,
        email: user.email,
        first_name: user.first_name,
        last_name: user.last_name,
        tracking_ns: user.tracking_ns,
        server_id: s.server_id.clone(),
        public_url: s.public_url.clone(),
        features: s.features,
    })
}

/// `DELETE /tern/v1/tokens/current`: sign-out. Streams of the token end at their next
/// heartbeat, when they re-check it.
pub async fn revoke_current_token(
    State(state): State<AppState>,
    AuthUser(auth): AuthUser,
) -> Result<StatusCode, ApiError> {
    let id = auth.token_id;
    let now = ids::now_ms();
    state.blocking(move |r| r.revoke_token(id, now)).await?;
    Ok(StatusCode::NO_CONTENT)
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
