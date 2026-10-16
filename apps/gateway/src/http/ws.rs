// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `/gateway/v1/ws` (docs/web-gateway-plan.md 2.4, 2.6). Every upgrade is accepted and a
//! refused one is closed with a code, because a browser only sees 1006 for an upgrade
//! answered with an HTTP error.

use axum::extract::ws::{CloseFrame, Message, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::{HOST, ORIGIN};
use axum::http::HeaderMap;
use axum::response::Response;

use super::{AppState, SocketSlot};
use crate::devices::Device;

/// Close codes (plan 2.6).
pub mod close {
    pub const FELL_BEHIND: u16 = 4008;
    pub const PROTOCOL: u16 = 4400;
    pub const UNPAIRED: u16 = 4401;
    pub const FORBIDDEN: u16 = 4403;
    pub const LIMITED: u16 = 4429;
    pub const ENGINE_UNAVAILABLE: u16 = 4502;
}

/// Largest frame a browser may send.
pub const MAX_FRAME_BYTES: usize = 8 << 20;

type Refusal = (u16, &'static str);

fn admit(state: &AppState, headers: &HeaderMap) -> Result<(Device, SocketSlot), Refusal> {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    if !header(HOST).is_some_and(|h| state.config().host_allowed(h)) {
        return Err((close::FORBIDDEN, "host not allowed"));
    }
    if !header(ORIGIN).is_some_and(|o| state.config().origin_allowed(o)) {
        return Err((close::FORBIDDEN, "origin not allowed"));
    }
    let (device, _) = state
        .authenticate(headers)
        .ok_or((close::UNPAIRED, "unpaired"))?;
    let slot = state
        .take_socket(&device.id)
        .ok_or((close::LIMITED, "too many connections"))?;
    Ok((device, slot))
}

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let admitted = admit(&state, &headers);
    upgrade
        .max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |mut socket| async move {
            let (code, reason) = match admitted {
                Err(refusal) => {
                    tracing::info!(code = refusal.0, "websocket refused");
                    refusal
                }
                Ok((device, _slot)) => {
                    tracing::info!(device = %device.id, "websocket closed: no engine relay yet");
                    (close::ENGINE_UNAVAILABLE, "engine relay not available")
                }
            };
            let frame = CloseFrame {
                code,
                reason: reason.into(),
            };
            let _ = socket.send(Message::Close(Some(frame))).await;
        })
}
