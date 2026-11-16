// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::repo::RepoError;

/// Errors from API handlers. 401 carries no `WWW-Authenticate`, so Electron never pops up a
/// credentials dialog.
#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    NotFound,
    BadRequest(String),
    Conflict(String),
    PayloadTooLarge,
    TooManyRequests,
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_owned()),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_owned()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m),
            ApiError::PayloadTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too large".to_owned()),
            // Both engines treat 429 as retryable; the body stays empty (3.5).
            ApiError::TooManyRequests => return StatusCode::TOO_MANY_REQUESTS.into_response(),
            ApiError::Internal(m) => {
                tracing::error!(error = %m, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_owned(),
                )
            }
        };
        (status, Json(serde_json::json!({ "message": message }))).into_response()
    }
}

impl From<RepoError> for ApiError {
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::AccountLimit => ApiError::BadRequest(e.to_string()),
            other => ApiError::Internal(other.to_string()),
        }
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> Self {
        ApiError::Internal(e.to_string())
    }
}
