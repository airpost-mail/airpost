// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `/gateway/v1/session`, `/pair` and `/unpair`.

use std::net::SocketAddr;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::{now_ms, ApiError, AppState, Paired, COOKIE_MAX_AGE_SECS};
use crate::devices::Device;
use crate::engine::EngineInfo;

/// Largest pairing request body.
const PAIR_BODY_LIMIT: usize = 4096;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    id: String,
    name: String,
    scopes: Vec<String>,
    paired_at: i64,
}

impl From<&Device> for DeviceInfo {
    fn from(d: &Device) -> Self {
        DeviceInfo {
            id: d.id.clone(),
            name: d.name.clone(),
            scopes: d.scopes.clone(),
            paired_at: d.paired_at,
        }
    }
}

#[derive(Serialize)]
pub struct GatewayInfo {
    version: &'static str,
}

#[derive(Serialize)]
pub struct SessionInfo {
    device: DeviceInfo,
    engine: EngineInfo,
    gateway: GatewayInfo,
}

/// Who this browser is, and whether the engine is up. Refreshes the cookie's lifetime.
pub async fn session(State(state): State<AppState>, paired: Paired) -> Response {
    let body = SessionInfo {
        device: (&paired.device).into(),
        engine: state.engine().await,
        gateway: GatewayInfo {
            version: env!("CARGO_PKG_VERSION"),
        },
    };
    let cookie = state.set_cookie(&paired.token, COOKIE_MAX_AGE_SECS);
    ([(SET_COOKIE, cookie)], Json(body)).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairRequest {
    code: String,
    #[serde(default)]
    device_name: String,
}

#[derive(Serialize)]
struct PairResult {
    device: DeviceInfo,
}

pub async fn pair(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    if body.len() > PAIR_BODY_LIMIT {
        return Err(ApiError::BadRequest("Request too large."));
    }
    let req: PairRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest("Expected {code}."))?;
    let client = state.client_ip(peer.ip(), &headers);
    state
        .pair_allowed(client)
        .map_err(|retry_after_secs| ApiError::RateLimited { retry_after_secs })?;
    let devices = state.devices().clone();
    let redeemed =
        tokio::task::spawn_blocking(move || devices.redeem(&req.code, &req.device_name, now_ms()))
            .await??;
    let Some((device, token)) = redeemed else {
        tracing::info!(client = %client, "pairing refused: unknown or expired code");
        return Err(ApiError::Unpaired);
    };
    state.reload()?;
    tracing::info!(device = %device.id, admin = device.is_admin(), "paired a device");
    let cookie = state.set_cookie(&token, COOKIE_MAX_AGE_SECS);
    Ok((
        [(SET_COOKIE, cookie)],
        Json(PairResult {
            device: (&device).into(),
        }),
    )
        .into_response())
}

pub async fn unpair(State(state): State<AppState>, paired: Paired) -> Result<Response, ApiError> {
    let devices = state.devices().clone();
    let id = paired.device.id.clone();
    tokio::task::spawn_blocking(move || devices.revoke(&id, now_ms())).await??;
    state.reload()?;
    tracing::info!(device = %paired.device.id, "device unpaired itself");
    Ok((
        StatusCode::NO_CONTENT,
        [(SET_COOKIE, state.set_cookie("", 0))],
    )
        .into_response())
}
