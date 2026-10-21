// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Finding and probing the engine. The gateway relays only to
//! a `serve` engine (`run/engine.json`): per-account engines have no single socket for a
//! browser's every account.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ms_ipc::{feature, method, HelloResult, Request, Response, PROTOCOL_VERSION};
use serde::Serialize;
use serde_json::json;
use tern_client::discovery::{self, ENGINE_FILE};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

/// How long `/session` reuses a probe.
const PROBE_TTL: Duration = Duration::from_secs(2);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// The engine's `hello` answer is small; anything longer is not an engine.
const MAX_HELLO_LINE: u64 = 1 << 20;

/// The socket of the `serve` engine for `config_dir`, if one is running.
pub fn find(config_dir: &Path) -> Option<PathBuf> {
    discovery::scan(&config_dir.join("run"))
        .live
        .into_iter()
        .find(|(file, _)| file.file_name().is_some_and(|n| n == ENGINE_FILE))
        .map(|(_, entry)| entry.socket)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum EngineInfo {
    #[serde(rename_all = "camelCase")]
    Up { protocol: u32, engine: String },
    #[serde(rename_all = "camelCase")]
    Down { reason: &'static str },
}

pub const NOT_RUNNING: &str = "no engine is running for this profile";
pub const TOO_OLD: &str = "the engine doesn't support relayed connections; update it";

/// Connects to the engine's socket and says `hello` as a local client: whether there is an
/// engine the relay could use.
pub async fn probe(config_dir: &Path) -> EngineInfo {
    let Some(socket) = find(config_dir) else {
        return EngineInfo::Down {
            reason: NOT_RUNNING,
        };
    };
    let hello = async {
        let stream = UnixStream::connect(&socket).await.ok()?;
        let (r, mut w) = stream.into_split();
        let req = Request::new(
            1,
            method::HELLO,
            json!({ "protocol": PROTOCOL_VERSION, "client": concat!("tern-gateway/", env!("CARGO_PKG_VERSION"), " (probe)") }),
        );
        w.write_all(&ms_ipc::encode(&req)).await.ok()?;
        let mut line = Vec::new();
        BufReader::new(r)
            .take(MAX_HELLO_LINE)
            .read_until(b'\n', &mut line)
            .await
            .ok()?;
        let res: Response = serde_json::from_slice(&line).ok()?;
        serde_json::from_value::<HelloResult>(res.result?).ok()
    };
    match tokio::time::timeout(PROBE_TIMEOUT, hello).await {
        Ok(Some(h)) if h.features.iter().any(|f| f == feature::RELAYED) => EngineInfo::Up {
            protocol: h.protocol,
            engine: h.engine,
        },
        Ok(Some(_)) => EngineInfo::Down { reason: TOO_OLD },
        _ => EngineInfo::Down {
            reason: NOT_RUNNING,
        },
    }
}

/// Why [`call`] has no result.
#[derive(Debug)]
pub enum CallError {
    /// No engine, or it didn't answer in time.
    Down,
    /// The engine lacks the feature the call needs.
    TooOld,
    /// The engine answered with an error.
    Rpc(ms_ipc::ErrorObject),
}

/// Calls `method` on a connection of its own, as a local client: the gateway's file routes,
/// which may not go through a browser's relayed connection (`files.*` are local-only).
/// Answers that come before the response (there are none without `subscribe`) are skipped.
pub async fn call(
    config_dir: &Path,
    needs: &str,
    method: &str,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, CallError> {
    let socket = find(config_dir).ok_or(CallError::Down)?;
    let exchange = async {
        let stream = UnixStream::connect(&socket)
            .await
            .map_err(|_| CallError::Down)?;
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r);
        let send = |id: i64, method: &str, params: serde_json::Value| {
            ms_ipc::encode(&Request::new(id, method, params))
        };
        let hello = send(
            1,
            method::HELLO,
            json!({ "protocol": PROTOCOL_VERSION, "client": concat!("tern-gateway/", env!("CARGO_PKG_VERSION"), " (files)") }),
        );
        w.write_all(&hello).await.map_err(|_| CallError::Down)?;
        let hello: HelloResult = read_result(&mut lines, 1)
            .await?
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or(CallError::Down)?;
        if !hello.features.iter().any(|f| f == needs) {
            return Err(CallError::TooOld);
        }
        w.write_all(&send(2, method, params))
            .await
            .map_err(|_| CallError::Down)?;
        read_result(&mut lines, 2).await?.ok_or(CallError::Down)
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| CallError::Down)?
}

/// The result of the response with `id`; `Ok(None)` when the connection ends first.
async fn read_result(
    lines: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
    id: i64,
) -> Result<Option<serde_json::Value>, CallError> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = (&mut *lines)
            .take(ms_ipc::MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| CallError::Down)?;
        if n == 0 {
            return Ok(None);
        }
        let Ok(res) = serde_json::from_slice::<Response>(&line) else {
            continue;
        };
        if res.id != Some(ms_ipc::Id::Num(id)) {
            continue;
        }
        return match (res.result, res.error) {
            (_, Some(e)) => Err(CallError::Rpc(e)),
            (r, None) => Ok(r),
        };
    }
}

/// [`probe`] results, reused for [`PROBE_TTL`] so a page polling `/session` doesn't open an
/// engine connection per request.
#[derive(Default)]
pub struct ProbeCache(Mutex<Option<(Instant, EngineInfo)>>);

impl ProbeCache {
    pub async fn get(&self, config_dir: &Path) -> EngineInfo {
        let mut slot = self.0.lock().await;
        if let Some((at, info)) = slot.as_ref() {
            if at.elapsed() < PROBE_TTL {
                return info.clone();
            }
        }
        let info = probe(config_dir).await;
        *slot = Some((Instant::now(), info.clone()));
        info
    }

    /// Forgets the last probe, so the next `/session` after a relay lost the engine says
    /// so instead of repeating "up" for the rest of the TTL.
    pub async fn invalidate(&self) {
        *self.0.lock().await = None;
    }
}
