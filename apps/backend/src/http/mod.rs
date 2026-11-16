// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The HTTP server: state, the auth extractor, layers and routes (docs/backend-plan.md 3.4).

mod api;
mod error;
mod hub;
mod login;
mod metadata;
mod stream;

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, FromRequestParts, Request};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method};
use axum::routing::{delete, get, post};
use axum::serve::ListenerExt;
use axum::Router;
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Semaphore};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

pub use error::ApiError;

use hub::Hub;

use crate::auth;
use crate::config::{Config, ConfigError, IpNet, Signup};
use crate::ids;
use crate::repo::{Authenticated, Repo, RepoError};
use crate::wire::{self, Features};

/// Features this build implements. `/airpost/v1/server` and `/airpost/v1/me` advertise a feature
/// only when it is also enabled in the config. Tracking arrives in B2-T, sharing in B2-S.
pub const IMPLEMENTED_FEATURES: Features = Features {
    tracking: false,
    sharing: false,
};

/// Body limit for forms and small JSON. Metadata pushes and backfills raise it per route
/// (3.5); B2 uploads will too.
const BODY_LIMIT: usize = wire::MAX_VALUE_BYTES;
/// `tokens.last_used_at` is refreshed at most this often, to keep reads off the writer.
const TOUCH_INTERVAL_MS: i64 = 60_000;
/// Login and sign-up attempts per minute, per IP and per email (section 5).
const LOGIN_ATTEMPTS_PER_MINUTE: u32 = 5;
/// Password hashes computed at once. argon2id at the default cost holds about 19 MiB each.
const MAX_CONCURRENT_PASSWORD_HASHES: usize = 4;
/// Dead peers: an idle connection is probed after this long (TCP keepalive), and on Linux
/// unacknowledged data (a stream's heartbeats to a vanished client) ends it after
/// [`TCP_USER_TIMEOUT`] instead of the kernel's ~15 min of retransmissions.
const TCP_KEEPALIVE_IDLE: Duration = Duration::from_secs(60);
const TCP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
#[cfg(any(target_os = "linux", target_os = "android"))]
const TCP_USER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

struct Inner {
    repo: Arc<dyn Repo>,
    server_id: String,
    public_url: String,
    signup: Signup,
    features: Features,
    trusted_proxies: Vec<IpNet>,
    login_by_ip: DefaultKeyedRateLimiter<IpAddr>,
    login_by_email: DefaultKeyedRateLimiter<String>,
    password_hashes: Semaphore,
    hub: Arc<Hub>,
    /// Set when the server stops, so open streams end and graceful shutdown can finish.
    shutdown: watch::Sender<bool>,
}

impl AppState {
    pub fn new(config: &Config, repo: Arc<dyn Repo>) -> Result<AppState, StateError> {
        let server_id = repo.server_id()?;
        let public_url = config.public_url()?;
        let quota =
            Quota::per_minute(NonZeroU32::new(LOGIN_ATTEMPTS_PER_MINUTE).expect("non-zero"));
        Ok(AppState(Arc::new(Inner {
            repo,
            server_id,
            public_url,
            signup: config.signup,
            features: Features {
                tracking: config.features.tracking && IMPLEMENTED_FEATURES.tracking,
                sharing: config.features.sharing && IMPLEMENTED_FEATURES.sharing,
            },
            trusted_proxies: config.trusted_proxies.clone(),
            login_by_ip: RateLimiter::keyed(quota),
            login_by_email: RateLimiter::keyed(quota),
            password_hashes: Semaphore::new(MAX_CONCURRENT_PASSWORD_HASHES),
            hub: Arc::default(),
            shutdown: watch::channel(false).0,
        })))
    }

    fn hub(&self) -> &Arc<Hub> {
        &self.0.hub
    }

    fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.0.shutdown.subscribe()
    }

    /// Ends every open stream.
    fn begin_shutdown(&self) {
        self.0.shutdown.send_replace(true);
    }

    /// Open streams of `(user, aid)`.
    pub fn open_streams(&self, user_id: &str, aid: &str) -> usize {
        self.0.hub.open_streams(user_id, aid)
    }

    async fn purge_old_claims(&self) {
        let before = ids::now_ms() - wire::CLAIM_RETENTION_DAYS * 24 * 3600 * 1000;
        match self.blocking(move |r| r.purge_claims(before)).await {
            Ok(n) if n > 0 => tracing::info!(claims = n, "purged old claims"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = ?e, "could not purge old claims"),
        }
    }

    pub fn repo(&self) -> &Arc<dyn Repo> {
        &self.0.repo
    }

    /// Runs a repo call on the blocking pool.
    pub async fn blocking<T, F>(&self, f: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&dyn Repo) -> Result<T, RepoError> + Send + 'static,
    {
        let repo = self.0.repo.clone();
        Ok(tokio::task::spawn_blocking(move || f(repo.as_ref())).await??)
    }

    /// The client's address: the peer, or for a trusted proxy the nearest `X-Forwarded-For`
    /// hop that is not itself a trusted proxy. Proxies append, so the right end is the only
    /// part not under the client's control.
    pub fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        let trusted = |ip: IpAddr| self.0.trusted_proxies.iter().any(|n| n.contains(ip));
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

    /// Takes one login attempt from the IP's budget and then the email's. An IP over its
    /// budget spends nothing from the email's, and creates no per-email entry. `email` must
    /// be at most [`auth::MAX_EMAIL_BYTES`].
    fn login_allowed(&self, ip: IpAddr, email: &str) -> bool {
        self.0.login_by_ip.check_key(&rate_limit_key(ip)).is_ok()
            && self
                .0
                .login_by_email
                .check_key(&crate::repo::normalize_email(email))
                .is_ok()
    }

    /// Waits for a turn to hash or verify a password.
    async fn password_hash_permit(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.0
            .password_hashes
            .acquire()
            .await
            .expect("the semaphore is never closed")
    }

    fn forget_idle_rate_limits(&self) {
        self.0.login_by_ip.retain_recent();
        self.0.login_by_email.retain_recent();
    }
}

/// An `X-Forwarded-For` element: an address, or `addr:port` / `[v6]:port`.
fn parse_forwarded_ip(s: &str) -> Option<IpAddr> {
    s.parse::<IpAddr>()
        .or_else(|_| s.parse::<SocketAddr>().map(|a| a.ip()))
        .ok()
        .map(|ip| ip.to_canonical())
}

/// One rate-limit budget per IPv4 address and per IPv6 /64, the smallest block a host is
/// usually given; per address, one host would get 2^64 budgets.
fn rate_limit_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6((u128::from(v6) & !(u128::from(u64::MAX))).into()),
        v4 => v4,
    }
}

/// Lets the kernel notice peers that vanished (3.5): keepalive for idle connections, and on
/// Linux a bound on how long sent data may stay unacknowledged.
fn tune_socket(tcp: &mut TcpStream) {
    let sock = socket2::SockRef::from(&*tcp);
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(TCP_KEEPALIVE_IDLE)
        .with_interval(TCP_KEEPALIVE_INTERVAL);
    if let Err(e) = sock.set_tcp_keepalive(&keepalive) {
        tracing::debug!(error = %e, "could not set TCP keepalive");
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if let Err(e) = sock.set_tcp_user_timeout(Some(TCP_USER_TIMEOUT)) {
        tracing::debug!(error = %e, "could not set TCP_USER_TIMEOUT");
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// An authenticated API caller: a live device token of an enabled user.
pub struct AuthUser(pub Authenticated);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(auth::token_from_header)
            .ok_or(ApiError::Unauthorized)?;
        let hash = ids::hash_token(&token);
        let found = state
            .blocking(move |r| r.authenticate(&hash))
            .await?
            .ok_or(ApiError::Unauthorized)?;
        let now = ids::now_ms();
        if found
            .token_last_used_at
            .is_none_or(|t| now - t >= TOUCH_INTERVAL_MS)
        {
            let repo = state.repo().clone();
            let id = found.token_id;
            tokio::task::spawn_blocking(move || {
                if let Err(e) = repo.touch_token(id, now) {
                    tracing::warn!(error = %e, "could not update token last_used_at");
                }
            });
        }
        Ok(AuthUser(found))
    }
}

/// Echoes `Origin` for the app's `file://` and `null` origins only, never `*` (3.4).
fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _| {
            let o = origin.as_bytes();
            o == b"null" || o.starts_with(b"file://")
        }))
        .allow_credentials(true)
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .max_age(Duration::from_secs(600))
}

/// Logs method, path and status, never the query. Tracking paths are reduced to their prefix
/// (section 5: access logs drop the path and query for `/t/`).
fn trace_layer() -> TraceLayer<
    tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
    impl Fn(&Request) -> tracing::Span + Clone,
> {
    TraceLayer::new_for_http()
        .make_span_with(|req: &Request| {
            let path = req.uri().path();
            let path = if path.starts_with("/t/") {
                "/t/…"
            } else {
                path
            };
            tracing::info_span!("http", method = %req.method(), path = %path)
        })
        .on_response(DefaultOnResponse::new().level(Level::INFO))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/ping", get(api::ping))
        .route("/airpost/v1/server", get(api::server_info))
        .route("/airpost/v1/login", get(login::form).post(login::submit))
        .route("/airpost/v1/me", get(api::me))
        .route(
            "/airpost/v1/tokens/current",
            delete(api::revoke_current_token),
        )
        .route("/airpost/v1/accounts/{aid}", delete(api::delete_account))
        .route(
            "/metadata/{aid}/{object_id}/{plugin_id}",
            post(metadata::push).layer(DefaultBodyLimit::max(metadata::PUSH_BODY_LIMIT)),
        )
        .route("/metadata/{aid}", get(metadata::page))
        .route("/deltas/{aid}/head", get(metadata::head))
        .route("/deltas/{aid}/streaming", get(stream::stream))
        .route(
            "/airpost/v1/backfill/{aid}",
            post(metadata::backfill).layer(DefaultBodyLimit::max(wire::BACKFILL_MAX_BODY_BYTES)),
        )
        .route(
            "/airpost/v1/claim/{aid}/{object_id}/{plugin_id}/{expiration}",
            post(metadata::claim),
        )
        .fallback(api::not_found)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(trace_layer())
        .layer(cors_layer())
        .with_state(state)
}

/// A bound, not yet running server.
pub struct Server {
    listener: TcpListener,
    state: AppState,
}

impl Server {
    pub async fn bind(config: &Config, repo: Arc<dyn Repo>) -> Result<Server, BindError> {
        let state = AppState::new(config, repo)?;
        let listener = TcpListener::bind(config.bind).await?;
        Ok(Server { listener, state })
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
        let cleanup = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            for minute in 0u64.. {
                tick.tick().await;
                state.forget_idle_rate_limits();
                if minute % 60 == 0 {
                    state.purge_old_claims().await;
                }
            }
        });
        let state = self.state.clone();
        let app = router(self.state).into_make_service_with_connect_info::<SocketAddr>();
        let out = axum::serve(self.listener.tap_io(tune_socket), app)
            .with_graceful_shutdown(async move {
                shutdown.await;
                state.begin_shutdown();
            })
            .await;
        cleanup.abort();
        out
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error("binding: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_keys() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(rate_limit_key(ip("192.0.2.7")), ip("192.0.2.7"));
        assert_eq!(rate_limit_key(ip("::ffff:192.0.2.7")), ip("192.0.2.7"));
        assert_eq!(
            rate_limit_key(ip("2001:db8:1:2:aaaa::1")),
            ip("2001:db8:1:2::")
        );
        assert_eq!(
            rate_limit_key(ip("2001:db8:1:2:ffff:ffff:ffff:ffff")),
            rate_limit_key(ip("2001:db8:1:2::9"))
        );
        assert_ne!(rate_limit_key(ip("2001:db8:1:3::1")), ip("2001:db8:1:2::"));
    }

    #[test]
    fn forwarded_ips() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(parse_forwarded_ip("203.0.113.5"), Some(ip("203.0.113.5")));
        assert_eq!(
            parse_forwarded_ip("203.0.113.5:4711"),
            Some(ip("203.0.113.5"))
        );
        assert_eq!(
            parse_forwarded_ip("[2001:db8::1]:443"),
            Some(ip("2001:db8::1"))
        );
        assert_eq!(parse_forwarded_ip("2001:db8::1"), Some(ip("2001:db8::1")));
        assert_eq!(parse_forwarded_ip("unknown"), None);
    }
}
