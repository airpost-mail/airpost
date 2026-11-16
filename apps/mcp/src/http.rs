// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `airpost-mcp http`: MCP over streamable HTTP on `<bind>:<port>/mcp` (`127.0.0.1` unless
//! `--bind` says otherwise), for clients that can't launch a stdio server, as the desktop
//! plugin's `mcp-http-server.ts` served it. Every request needs `Authorization: Bearer
//! <token>`, the token in `<config-dir>/mcp-token` (created owner-only on first use). rmcp
//! checks the `Host` header against the loopback names and `--allowed-host`, which stops DNS
//! rebinding from a browser. Run by the unit `airpost mcp-http install` writes, it serves on the
//! socket launchd or systemd holds for it ([`ACTIVATION_SOCKET`]) instead of binding one.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use sha2::{Digest, Sha256};

use crate::server::{App, Handler};

pub const TOKEN_FILE: &str = "mcp-token";

pub fn token_path(config_dir: &Path) -> PathBuf {
    config_dir.join(TOKEN_FILE)
}

/// The bearer token, created on first use.
pub fn load_or_create_token(config_dir: &Path) -> std::io::Result<String> {
    let path = token_path(config_dir);
    match std::fs::read_to_string(&path) {
        Ok(t) if !t.trim().is_empty() => return Ok(t.trim().to_owned()),
        Ok(_) => std::fs::remove_file(&path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    crate::platform::create_private(&path, token.as_bytes())?;
    Ok(token)
}

/// Compares fixed-length digests in constant time, so neither the token nor its length
/// leaks through timing.
pub fn token_matches(header: Option<&str>, token: &str) -> bool {
    let presented = header.and_then(|h| h.strip_prefix("Bearer ")).unwrap_or("");
    let (a, b) = (
        Sha256::digest(presented.as_bytes()),
        Sha256::digest(token.as_bytes()),
    );
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn json_response(status: StatusCode, body: &str) -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_owned())).boxed())
        .expect("a valid response")
}

/// The `Host` values rmcp accepts: its loopback defaults and `extra`.
fn allowed_hosts(extra: &[String]) -> Vec<String> {
    let mut hosts = StreamableHttpServerConfig::default().allowed_hosts;
    hosts.extend(
        extra
            .iter()
            .map(|h| h.trim().to_owned())
            .filter(|h| !h.is_empty()),
    );
    hosts
}

/// The launchd `Sockets` key and systemd `FileDescriptorName=` of the socket in the unit
/// `airpost mcp-http install` writes.
pub const ACTIVATION_SOCKET: &str = "mcp";

/// `<config-dir>/mcp-http.json`: the serving process, its URL and build, for `airpost status`.
pub const RUNNING_FILE: &str = "mcp-http.json";

/// Serves on `inherited`, the socket the service manager passed, else on `addr`.
pub async fn serve(
    app: Arc<App>,
    addr: SocketAddr,
    inherited: Option<std::net::TcpListener>,
    extra_hosts: &[String],
) -> Result<(), String> {
    let token = load_or_create_token(&app.config_dir).map_err(|e| {
        format!(
            "could not read or create {}: {e}",
            token_path(&app.config_dir).display()
        )
    })?;
    let port = addr.port();
    let listener = match inherited {
        Some(l) => l
            .set_nonblocking(true)
            .and_then(|()| tokio::net::TcpListener::from_std(l))
            .map_err(|e| format!("could not use the service manager's socket: {e}"))?,
        None => tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::AddrInUse => {
                    format!("port {port} is already in use; choose another with --port or http.port in mcp.json")
                }
                _ => format!("could not listen on {addr}: {e}"),
            })?,
    };
    let addr = listener.local_addr().unwrap_or(addr);
    let running = app.config_dir.join(RUNNING_FILE);
    let _ = std::fs::write(
        &running,
        serde_json::json!({
            "pid": std::process::id(),
            "url": format!("http://{addr}/mcp"),
            "port": addr.port(),
            "version": env!("CARGO_PKG_VERSION"),
            "build": airpost_activation::build_id(),
        })
        .to_string(),
    );
    let out = accept_loop(app, listener, addr, extra_hosts, token).await;
    let _ = std::fs::remove_file(&running);
    out
}

async fn accept_loop(
    app: Arc<App>,
    listener: tokio::net::TcpListener,
    addr: SocketAddr,
    extra_hosts: &[String],
    token: String,
) -> Result<(), String> {
    let handler_app = app.clone();
    let service = StreamableHttpService::new(
        move || Ok(Handler::new(handler_app.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts(extra_hosts)),
    );
    crate::log(&format!("listening on http://{addr}/mcp"));
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| format!("accept failed: {e}"))?;
        let service = service.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
                let service = service.clone();
                let token = token.clone();
                async move {
                    if req.uri().path() != "/mcp" {
                        return Ok::<_, Infallible>(json_response(
                            StatusCode::NOT_FOUND,
                            r#"{"error":"Not found"}"#,
                        ));
                    }
                    let auth = req
                        .headers()
                        .get(hyper::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok());
                    if !token_matches(auth, &token) {
                        return Ok(json_response(
                            StatusCode::UNAUTHORIZED,
                            r#"{"error":"Unauthorized"}"#,
                        ));
                    }
                    Ok(service.handle(req).await)
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), svc)
                .await;
        });
    }
}

/// `airpost-mcp healthcheck`: an unauthenticated `POST /mcp` on 127.0.0.1 must get the 401 a
/// running server gives it.
pub fn healthcheck(port: u16) -> Result<(), String> {
    use std::io::{Read, Write};
    let timeout = Duration::from_secs(5);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("nothing answers on {addr}: {e}"))?;
    let _ = stream.set_read_timeout(Some(timeout));
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| format!("{addr}: {e}"))?;
    let mut head = [0u8; 12];
    stream
        .read_exact(&mut head)
        .map_err(|e| format!("{addr}: {e}"))?;
    match &head[..] {
        b"HTTP/1.1 401" => Ok(()),
        other => Err(format!(
            "{addr} answered {:?}, not 401",
            String::from_utf8_lossy(other)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_hosts_keep_loopback() {
        let hosts = allowed_hosts(&["airpost.example.com".into(), " ".into()]);
        assert!(hosts.iter().any(|h| h == "127.0.0.1"));
        assert_eq!(
            hosts.last().map(String::as_str),
            Some("airpost.example.com")
        );
        assert!(!hosts.iter().any(|h| h.trim().is_empty()));
    }

    #[test]
    fn token_file_and_check() {
        let tmp = tempfile::tempdir().unwrap();
        let t = load_or_create_token(tmp.path()).unwrap();
        assert_eq!(t.len(), 64);
        assert_eq!(load_or_create_token(tmp.path()).unwrap(), t);
        assert!(token_matches(Some(&format!("Bearer {t}")), &t));
        assert!(!token_matches(Some(&format!("Bearer {t}x")), &t));
        assert!(!token_matches(Some(&t), &t));
        assert!(!token_matches(None, &t));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(tmp.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
