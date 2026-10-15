//! One HTTP exchange, behind a trait so the protocol code runs against captured responses
//! in tests. The real transport is reqwest's blocking client with ms-net's error mapping
//! (libcurl's texts as keys), replacing the libcurl handles in Mailspring-Sync
//! `MailSync/DAVWorker.cpp` (Copyright © 2017 Foundry 376, GPL-3.0; see NOTICE.md).
//!
//! Redirects are never followed, as with curl's defaults: a 3xx is an answer like any
//! other, and `.well-known` probes read its Location.

use ms_core::{Error, Result};
use reqwest::blocking::Client;
use reqwest::Method;
use std::sync::Mutex;
use std::time::Duration;

/// `CURLOPT_CONNECTTIMEOUT` for DAV requests.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(40);
/// curl had no overall limit; this only stops a stalled server holding the calContacts
/// thread forever. Large initial multigets stay far below it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// `PerformExpectedRedirect`'s connect timeout, used as the probe's whole budget.
pub const REDIRECT_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Overrides the whole-request timeout.
    pub timeout: Option<Duration>,
    /// Negotiate HTTP/1.1 only ([`DavProfile::http1_only`](crate::DavProfile)).
    pub http1_only: bool,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        find_header(&self.headers, name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// The URL that answered, for error debuginfo (`CURLINFO_EFFECTIVE_URL`).
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpResponse {
    /// Header lookup ignoring case: some servers and proxies vary it (`extractHeader`).
    pub fn header(&self, name: &str) -> Option<&str> {
        find_header(&self.headers, name)
    }
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
}

/// Sends one request and returns whatever status came back. Only a request that got no
/// response is an error, an offline/retryable `SyncError` keyed like libcurl's.
pub trait Transport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse>;
}

/// reqwest's blocking client. It runs its own runtime thread and panics when used inside a
/// tokio runtime, so from inside one each call moves to a separate thread.
pub struct ReqwestTransport {
    default: Client,
    http1: Mutex<Option<Client>>,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self> {
        Ok(ReqwestTransport {
            default: crate::runtime::outside_runtime(|| build_client(false))?,
            http1: Mutex::new(None),
        })
    }

    fn client(&self, http1_only: bool) -> Result<Client> {
        if !http1_only {
            return Ok(self.default.clone());
        }
        let mut slot = self.http1.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            *slot = Some(build_client(true)?);
        }
        Ok(slot
            .as_ref()
            .cloned()
            .unwrap_or_else(|| self.default.clone()))
    }
}

fn build_client(http1_only: bool) -> Result<Client> {
    // reqwest panics building a rustls client with no process-wide provider; the bin
    // installs ring at startup, this covers tests and early callers.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let mut builder = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .tls_backend_rustls();
    if http1_only {
        builder = builder.http1_only();
    }
    builder
        .build()
        .map_err(|e| Error::other(format!("Could not create the HTTP client: {e}")))
}

impl Transport for ReqwestTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        crate::runtime::outside_runtime(|| self.send_blocking(req))
    }
}

impl ReqwestTransport {
    fn send_blocking(&self, req: &HttpRequest) -> Result<HttpResponse> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|_| Error::other(format!("Invalid HTTP method: {}", req.method)))?;
        let mut builder = self.client(req.http1_only)?.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body.clone());
        }
        if let Some(t) = req.timeout {
            builder = builder.timeout(t);
        }
        let resp = builder
            .send()
            .map_err(|e| ms_net::http::transport_error(&e, &req.url))?;
        let status = resp.status().as_u16();
        let url = resp.url().to_string();
        let headers = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let body = resp
            .text()
            .map_err(|e| ms_net::http::transport_error(&e, &req.url))?;
        Ok(HttpResponse {
            status,
            url,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Answers one request on a local port with a fixed 207.
    fn one_shot_server() -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/dav/", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            let body = "<d:multistatus xmlns:d=\"DAV:\"/>";
            let _ = write!(
                sock,
                "HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        (url, handle)
    }

    #[test]
    fn works_inside_a_current_thread_runtime() {
        let (url, server) = one_shot_server();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let resp = rt.block_on(async {
            let t = ReqwestTransport::new().unwrap();
            let resp = t.send(&HttpRequest {
                method: "PROPFIND".into(),
                url: url.clone(),
                ..Default::default()
            });
            drop(t);
            resp
        });
        server.join().unwrap();
        let resp = resp.unwrap();
        assert_eq!(resp.status, 207);
        assert!(resp.body.contains("multistatus"));
    }
}
