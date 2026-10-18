// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The HTTP server: state, the device extractor, the Host/Origin guard, headers and routes
//! (docs/web-gateway-plan.md 2.2).

mod files;
mod session;
mod ws;

use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, COOKIE, HOST, ORIGIN, REFERRER_POLICY, RETRY_AFTER,
    X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use governor::{DefaultDirectRateLimiter, DefaultKeyedRateLimiter, Quota, RateLimiter};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

pub use ws::close;

use crate::config::Config;
use crate::devices::{Device, DeviceFile, DevicesError, Snapshot};
use crate::engine::{EngineInfo, ProbeCache};

/// Pairing attempts per minute per client address (IPv4, or IPv6 /64).
const PAIR_PER_IP_PER_MINUTE: u32 = 5;
/// Pairing attempts per hour from everyone, counted only while a code is pending and never
/// for loopback clients, so strangers can't lock out pairing when nobody is pairing.
const PAIR_GLOBAL_PER_HOUR: u32 = 30;
const UPGRADES_PER_DEVICE_PER_MINUTE: u32 = 10;
pub const MAX_SOCKETS_PER_DEVICE: usize = 8;
pub const MAX_SOCKETS: usize = 64;
/// 400 days, the longest lifetime browsers keep; refreshed on every `/session`.
const COOKIE_MAX_AGE_SECS: u64 = 400 * 24 * 3600;
/// Last-used times and idle expiry are written back to `devices.json` this often.
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(60);

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

struct Inner {
    config: Config,
    devices: DeviceFile,
    snapshot: RwLock<Arc<Snapshot>>,
    /// Bumped on every reload, so open WebSockets re-check their device.
    generation: watch::Sender<u64>,
    last_used: Mutex<HashMap<String, i64>>,
    pair_by_ip: DefaultKeyedRateLimiter<IpAddr>,
    pair_global: DefaultDirectRateLimiter,
    upgrades: DefaultKeyedRateLimiter<String>,
    sockets: Mutex<Sockets>,
    csp: HeaderValue,
    probe: ProbeCache,
    /// Set when the server stops, so open WebSockets close with 1001.
    shutdown: watch::Sender<bool>,
    uploads: Arc<tokio::sync::Semaphore>,
}

/// Uploads streaming at once, gateway-wide: each holds up to 25 MiB on disk.
const MAX_UPLOADS: usize = 8;

#[derive(Default)]
struct Sockets {
    per_device: HashMap<String, usize>,
    total: usize,
}

impl AppState {
    pub fn new(config: Config) -> Result<AppState, DevicesError> {
        let devices = DeviceFile::open(&config.state_dir)?;
        let snapshot = devices.snapshot(now_ms())?;
        let per_minute = |n: u32| Quota::per_minute(NonZeroU32::new(n).expect("non-zero"));
        let csp = HeaderValue::from_str(&app_csp(&config)).expect("ascii CSP");
        files::sweep_incoming(&config.state_dir);
        Ok(AppState(Arc::new(Inner {
            devices,
            snapshot: RwLock::new(Arc::new(snapshot)),
            generation: watch::channel(0).0,
            last_used: Mutex::default(),
            pair_by_ip: RateLimiter::keyed(per_minute(PAIR_PER_IP_PER_MINUTE)),
            pair_global: RateLimiter::direct(Quota::per_hour(
                NonZeroU32::new(PAIR_GLOBAL_PER_HOUR).expect("non-zero"),
            )),
            upgrades: RateLimiter::keyed(per_minute(UPGRADES_PER_DEVICE_PER_MINUTE)),
            sockets: Mutex::default(),
            csp,
            probe: ProbeCache::default(),
            shutdown: watch::channel(false).0,
            uploads: Arc::new(tokio::sync::Semaphore::new(MAX_UPLOADS)),
            config,
        })))
    }

    pub fn config(&self) -> &Config {
        &self.0.config
    }

    pub fn devices(&self) -> &DeviceFile {
        &self.0.devices
    }

    fn snapshot(&self) -> Arc<Snapshot> {
        self.0.snapshot.read().expect("snapshot lock").clone()
    }

    pub fn generation(&self) -> watch::Receiver<u64> {
        self.0.generation.subscribe()
    }

    fn upload_permit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.0.uploads.clone().try_acquire_owned().ok()
    }

    fn shutdown(&self) -> watch::Receiver<bool> {
        self.0.shutdown.subscribe()
    }

    async fn engine(&self) -> EngineInfo {
        self.0.probe.get(&self.0.config.config_dir).await
    }

    async fn engine_lost(&self) {
        self.0.probe.invalidate().await;
    }

    /// Re-reads `devices.json`; open WebSockets of devices that are gone then close.
    pub fn reload(&self) -> Result<(), DevicesError> {
        let snap = self.0.devices.snapshot(now_ms())?;
        *self.0.snapshot.write().expect("snapshot lock") = Arc::new(snap);
        self.0.generation.send_modify(|g| *g += 1);
        Ok(())
    }

    /// The paired device for this cookie, if it is still live. Records the use.
    fn authenticate(&self, headers: &HeaderMap) -> Option<(Device, String)> {
        let token = cookie(headers, self.cookie_name())?;
        let snap = self.snapshot();
        let mut device = snap.authenticate(&token)?.clone();
        let now = now_ms();
        let mut used = self.0.last_used.lock().expect("last_used lock");
        if let Some(&t) = used.get(&device.id) {
            device.last_used_at = device.last_used_at.max(t);
        }
        if device.expired(now) {
            return None;
        }
        used.insert(device.id.clone(), now);
        Some((device, token))
    }

    /// Whether `id` is still paired, for open WebSockets after a reload.
    pub fn device_live(&self, id: &str) -> bool {
        self.snapshot().device(id).is_some()
    }

    pub fn cookie_name(&self) -> &'static str {
        if self.0.config.secure_cookie() {
            "__Host-tern_device"
        } else {
            "tern_device"
        }
    }

    fn set_cookie(&self, token: &str, max_age: u64) -> HeaderValue {
        let secure = if self.0.config.secure_cookie() {
            "; Secure"
        } else {
            ""
        };
        HeaderValue::from_str(&format!(
            "{}={token}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}",
            self.cookie_name()
        ))
        .expect("cookie is ascii")
    }

    /// The client's address: the peer, or for a trusted proxy the nearest `X-Forwarded-For`
    /// hop that is not itself a trusted proxy. Proxies append, so the right end is the only
    /// part not under the client's control.
    fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        let trusted = |ip: IpAddr| self.0.config.trusted_proxies.iter().any(|n| n.contains(ip));
        let mut client = peer.to_canonical();
        if !trusted(client) {
            return client;
        }
        for value in headers.get_all("x-forwarded-for").iter().rev() {
            let Ok(value) = value.to_str() else {
                return client;
            };
            for hop in value.rsplit(',') {
                let Some(ip) = parse_forwarded_ip(hop.trim()) else {
                    return client;
                };
                client = ip;
                if !trusted(ip) {
                    return ip;
                }
            }
        }
        client
    }

    /// Takes one pairing attempt from the client's budget, then from the global one when
    /// it applies. `Err` carries the seconds to wait.
    fn pair_allowed(&self, client: IpAddr) -> Result<(), u64> {
        let clock = governor::clock::DefaultClock::default();
        use governor::clock::Clock;
        let wait = |e: governor::NotUntil<_>| e.wait_time_from(clock.now()).as_secs().max(1);
        self.0
            .pair_by_ip
            .check_key(&rate_limit_key(client))
            .map_err(wait)?;
        if !client.is_loopback() && self.snapshot().code_pending(now_ms()) {
            self.0.pair_global.check().map_err(wait)?;
        }
        Ok(())
    }

    /// Takes a WebSocket slot for `device`, or None when it or the gateway is at its cap or
    /// the device upgrades too often.
    fn take_socket(&self, device: &str) -> Option<SocketSlot> {
        if self.0.upgrades.check_key(&device.to_owned()).is_err() {
            return None;
        }
        let mut s = self.0.sockets.lock().expect("sockets lock");
        let n = s.per_device.get(device).copied().unwrap_or(0);
        if n >= MAX_SOCKETS_PER_DEVICE || s.total >= MAX_SOCKETS {
            return None;
        }
        s.per_device.insert(device.to_owned(), n + 1);
        s.total += 1;
        Some(SocketSlot {
            state: self.clone(),
            device: device.to_owned(),
        })
    }

    /// Writes last-used times back, re-reads the file (idle expiry, edits made while the
    /// control socket was down) and forgets idle rate-limit keys.
    async fn housekeeping(&self) {
        let used = std::mem::take(&mut *self.0.last_used.lock().expect("last_used lock"));
        let state = self.clone();
        let res = tokio::task::spawn_blocking(move || {
            state.0.devices.touch(&used, now_ms())?;
            state.reload()
        })
        .await;
        match res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "devices housekeeping failed"),
            Err(e) => tracing::warn!(error = %e, "devices housekeeping panicked"),
        }
        self.0.pair_by_ip.retain_recent();
        self.0.upgrades.retain_recent();
        let dir = self.0.config.state_dir.clone();
        let _ = tokio::task::spawn_blocking(move || files::sweep_incoming(&dir)).await;
    }
}

/// Releases a WebSocket slot when dropped.
pub struct SocketSlot {
    state: AppState,
    device: String,
}

impl Drop for SocketSlot {
    fn drop(&mut self) {
        let mut s = self.state.0.sockets.lock().expect("sockets lock");
        if let Some(n) = s.per_device.get_mut(&self.device) {
            *n -= 1;
            if *n == 0 {
                s.per_device.remove(&self.device);
            }
        }
        s.total -= 1;
    }
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
        .filter(|v| !v.is_empty())
}

/// An `X-Forwarded-For` element: an address, or `addr:port` / `[v6]:port`.
fn parse_forwarded_ip(s: &str) -> Option<IpAddr> {
    s.parse::<IpAddr>()
        .or_else(|_| s.parse::<SocketAddr>().map(|a| a.ip()))
        .ok()
        .map(|ip| ip.to_canonical())
}

/// One budget per IPv4 address and per IPv6 /64.
fn rate_limit_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6((u128::from(v6) & !(u128::from(u64::MAX))).into()),
        v4 => v4,
    }
}

/// The app's CSP. Email bodies render in a sandboxed srcdoc frame with its own, stricter
/// CSP; a srcdoc frame also inherits this one, so remote and gateway images (rewritten
/// `cid:` parts) must pass both.
///
/// CSP host sources can't spell an IPv6 literal (Chrome drops `ws://[::1]:5180` with a
/// console error), so those origins get no `ws:` entry; `'self'` covers the page's own
/// WebSocket in current browsers.
fn app_csp(config: &Config) -> String {
    let ws: Vec<String> = config
        .allowed_origins
        .iter()
        .filter(|o| !o.host.starts_with('['))
        .map(|o| format!("{}://{}", if o.https { "wss" } else { "ws" }, o.host))
        .collect();
    [
        "default-src 'self'".to_owned(),
        "script-src 'self'".to_owned(),
        "style-src 'self' 'unsafe-inline'".to_owned(),
        "img-src 'self' data: blob: https: http:".to_owned(),
        "font-src 'self' data:".to_owned(),
        "media-src 'self' data: blob:".to_owned(),
        format!("connect-src 'self' {}", ws.join(" ")),
        "object-src 'none'".to_owned(),
        "base-uri 'none'".to_owned(),
        "form-action 'self'".to_owned(),
        "frame-ancestors 'none'".to_owned(),
    ]
    .join("; ")
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest(&'static str),
    Unpaired,
    Forbidden(&'static str),
    RateLimited {
        retry_after_secs: u64,
    },
    NotFound,
    TooLarge(&'static str),
    /// The engine is down, too old, or couldn't produce the file.
    Unavailable(&'static str),
    Internal,
}

impl From<DevicesError> for ApiError {
    fn from(e: DevicesError) -> Self {
        tracing::error!(error = %e, "devices file");
        ApiError::Internal
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> Self {
        tracing::error!(error = %e, "blocking task");
        ApiError::Internal
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad-request", m),
            ApiError::Unpaired => (
                StatusCode::UNAUTHORIZED,
                "unpaired",
                "This browser isn't paired with the gateway.",
            ),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m),
            ApiError::RateLimited { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate-limited",
                "Too many attempts. Wait a minute.",
            ),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not-found", "Not found."),
            ApiError::TooLarge(m) => (StatusCode::PAYLOAD_TOO_LARGE, "too-large", m),
            ApiError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", m),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "The gateway failed; see its log.",
            ),
        };
        let mut res = (
            status,
            Json(serde_json::json!({"error": code, "message": message})),
        )
            .into_response();
        if let ApiError::RateLimited { retry_after_secs } = self {
            res.headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(retry_after_secs));
        }
        res
    }
}

/// A request from a paired device (by its cookie).
pub struct Paired {
    pub device: Device,
    pub token: String,
}

impl FromRequestParts<AppState> for Paired {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let (device, token) = state
            .authenticate(&parts.headers)
            .ok_or(ApiError::Unpaired)?;
        Ok(Paired { device, token })
    }
}

const WS_PATH: &str = "/gateway/v1/ws";
/// Liveness for container healthchecks, which call it on the loopback address whatever the
/// public host is, so it skips the Host check and says nothing about the engine.
pub const HEALTH_PATH: &str = "/healthz";

/// Host on every request; Origin on every state-changing one. The WebSocket route checks
/// both itself, so that it can answer with a close code the browser can read.
async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    if path != WS_PATH && path != HEALTH_PATH {
        let host = req.headers().get(HOST).and_then(|v| v.to_str().ok());
        if !host.is_some_and(|h| state.config().host_allowed(h)) {
            return ApiError::Forbidden("Host not allowed.").into_response();
        }
        let safe = matches!(*req.method(), Method::GET | Method::HEAD);
        if !safe {
            let origin = req.headers().get(ORIGIN).and_then(|v| v.to_str().ok());
            if !origin.is_some_and(|o| state.config().origin_allowed(o)) {
                return ApiError::Forbidden("Origin not allowed.").into_response();
            }
        }
    }
    let mut res = next.run(req).await;
    let ok = res.status().is_success();
    let h = res.headers_mut();
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    if path.starts_with("/gateway/") || path == HEALTH_PATH {
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    } else {
        h.insert(CONTENT_SECURITY_POLICY, state.0.csp.clone());
        h.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        let cache = if path.starts_with("/assets/") && ok {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        h.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    }
    res
}

/// Logs method, path and status, never the query.
fn trace_layer() -> TraceLayer<
    tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
    impl Fn(&Request) -> tracing::Span + Clone,
> {
    TraceLayer::new_for_http()
        .make_span_with(|req: &Request| {
            tracing::info_span!("http", method = %req.method(), path = %req.uri().path())
        })
        .on_response(DefaultOnResponse::new().level(Level::INFO))
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true}))
}

async fn api_not_found() -> ApiError {
    ApiError::NotFound
}

async fn no_web_app() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_FOUND,
        "The web app isn't installed here. Start tern-gateway with --web-root <apps/web/dist>.\n",
    )
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/gateway/v1/session", get(session::session))
        .route("/gateway/v1/pair", post(session::pair))
        .route("/gateway/v1/unpair", post(session::unpair))
        .route(WS_PATH, get(ws::upgrade))
        .route(
            "/gateway/v1/files/{account_id}/{file_id}",
            get(files::download),
        )
        .route("/gateway/v1/files/{account_id}", post(files::upload))
        .route(HEALTH_PATH, get(healthz))
        .route("/gateway/{*rest}", any(api_not_found));
    let app = match &state.config().web_root {
        Some(root) => api.fallback_service(
            ServeDir::new(root)
                .append_index_html_on_directories(true)
                .fallback(ServeFile::new(root.join("index.html"))),
        ),
        None => api.fallback(no_web_app),
    };
    app.layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(trace_layer())
        .with_state(state)
}

/// A bound, not yet running server.
pub struct Server {
    listener: TcpListener,
    state: AppState,
}

impl Server {
    /// On a listener bound already, so the config can name the port it got.
    pub fn with_listener(listener: TcpListener, state: AppState) -> Server {
        Server { listener, state }
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Serves until `shutdown` resolves, then drains open connections.
    pub async fn run(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let state = self.state.clone();
        let housekeeping = tokio::spawn(async move {
            let mut tick = tokio::time::interval(HOUSEKEEPING_INTERVAL);
            tick.tick().await;
            loop {
                tick.tick().await;
                state.housekeeping().await;
            }
        });
        let app = router(self.state.clone()).into_make_service_with_connect_info::<SocketAddr>();
        let state = self.state.clone();
        let out = axum::serve(self.listener, app)
            .with_graceful_shutdown(async move {
                shutdown.await;
                state.0.shutdown.send_replace(true);
            })
            .await;
        housekeeping.abort();
        self.state.housekeeping().await;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies() {
        let mut h = HeaderMap::new();
        h.append(
            COOKIE,
            HeaderValue::from_static("a=1; tern_device=tok; b=2"),
        );
        assert_eq!(cookie(&h, "tern_device").as_deref(), Some("tok"));
        assert_eq!(cookie(&h, "__Host-tern_device"), None);
        let mut h = HeaderMap::new();
        h.append(COOKIE, HeaderValue::from_static("tern_device="));
        assert_eq!(cookie(&h, "tern_device"), None);
    }

    #[test]
    fn csp_has_no_ipv6_literal_sources() {
        let config = Config::new(crate::config::ConfigInput {
            bind: "127.0.0.1:5180".parse().unwrap(),
            public_url: None,
            insecure_public_url: false,
            allow_origins: &[],
            trusted_proxies: &[],
            web_root: None,
            config_dir: "/tmp/x".into(),
        })
        .unwrap();
        let csp = app_csp(&config);
        let connect = csp
            .split("; ")
            .find(|d| d.starts_with("connect-src"))
            .unwrap();
        assert!(connect.contains("ws://127.0.0.1:5180"), "{connect}");
        assert!(connect.contains("ws://localhost:5180"), "{connect}");
        assert!(!connect.contains('['), "{connect}");
    }

    #[test]
    fn rate_limit_keys() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(rate_limit_key(ip("::ffff:192.0.2.7")), ip("192.0.2.7"));
        assert_eq!(
            rate_limit_key(ip("2001:db8:1:2:aaaa::1")),
            ip("2001:db8:1:2::")
        );
    }
}
