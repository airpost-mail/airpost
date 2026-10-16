// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern-mcp http`: MCP over streamable HTTP on `127.0.0.1:<port>/mcp`, for clients that
//! can't launch a stdio server, as the desktop plugin's `mcp-http-server.ts` served it.
//! Every request needs `Authorization: Bearer <token>`, the token in `<config-dir>/mcp-token`
//! (created owner-only on first use). rmcp checks the `Host` header against loopback names,
//! which stops DNS rebinding from a browser.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

pub async fn serve(app: Arc<App>, port: u16) -> Result<(), String> {
    let token = load_or_create_token(&app.config_dir).map_err(|e| {
        format!(
            "could not read or create {}: {e}",
            token_path(&app.config_dir).display()
        )
    })?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AddrInUse => {
                format!("port {port} is already in use; choose another with --port or http.port in mcp.json")
            }
            _ => format!("could not listen on 127.0.0.1:{port}: {e}"),
        })?;
    let handler_app = app.clone();
    let service = StreamableHttpService::new(
        move || Ok(Handler::new(handler_app.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    crate::log(&format!("listening on http://127.0.0.1:{port}/mcp"));
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

#[cfg(test)]
mod tests {
    use super::*;

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
