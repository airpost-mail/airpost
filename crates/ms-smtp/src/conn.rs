// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! One SMTP connection: the socket (plain or TLS), reply reading, the transcript, EHLO.
//!
//! Derived from libetpan's `mailsmtp.c` (`read_response`, `parse_response`,
//! `mailesmtp_parse_ehlo`, `mailesmtp_starttls`) and `mailsmtp_helper.c` (`mailsmtp_init`).
//! libetpan is Copyright © 2001-2005 DINH Viet Hoa, BSD-3-Clause; Mailspring-Sync's changes
//! to it are Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md ("libEtPan!").
//!
//! The connection is ours rather than lettre's `AsyncSmtpConnection` because the transcript
//! has to sit above TLS and mark AUTH payloads private, the AUTH list has to keep the
//! mechanisms lettre does not know (CRAM-MD5, DIGEST-MD5, GSSAPI…), and `mailsmtp_init`
//! falls back to HELO and to the address literal, which lettre's EHLO-only handshake cannot.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ms_core::protolog::{Direction, ProtocolLogger};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::ehlo;

/// Longest reply we accept; a server streaming garbage must not grow the buffer forever.
const MAX_REPLY_BYTES: usize = 1 << 20;

/// Largest slice handed to one timed socket write.
const WRITE_CHUNK: usize = 64 * 1024;

/// A reply as libetpan keeps it: the code of the last line and the text of every line (the
/// part after `NNN-` / `NNN `), each followed by `\n`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub code: u16,
    pub text: String,
}

impl Reply {
    pub fn first_line(&self) -> &str {
        self.text.lines().next().unwrap_or("")
    }
}

/// The socket failed, closed, timed out, or sent something that is not a reply
/// (`MAILSMTP_ERROR_STREAM`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamError(pub String);

#[derive(Debug)]
pub enum TlsError {
    /// The certificate was rejected (mailcore's `checkCertificate`).
    Certificate(String),
    /// The handshake itself failed; the text is what the TLS stack said.
    Handshake(String),
}

/// What EHLO advertised (`mailesmtp_parse_ehlo`). Absent after a HELO fallback.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extensions {
    pub starttls: bool,
    pub pipelining: bool,
    pub eight_bit_mime: bool,
    /// `SIZE` was advertised; the limit is 0 when none was given.
    pub size: Option<u64>,
    /// Upper-cased SASL mechanism names from `AUTH` / `AUTH=` lines.
    pub auth: Vec<String>,
}

impl Extensions {
    pub fn parse(reply: &Reply) -> Extensions {
        let mut ext = Extensions::default();
        for line in reply.text.lines() {
            let mut words = line.split_whitespace();
            let Some(first) = words.next() else { continue };
            let upper = first.to_ascii_uppercase();
            match upper.as_str() {
                "STARTTLS" => ext.starttls = true,
                "PIPELINING" => ext.pipelining = true,
                "8BITMIME" => ext.eight_bit_mime = true,
                "SIZE" => ext.size = Some(words.next().and_then(|n| n.parse().ok()).unwrap_or(0)),
                "AUTH" => ext.add_auth(words),
                // Some servers write "AUTH=LOGIN PLAIN".
                _ if upper.starts_with("AUTH=") => {
                    ext.add_auth(std::iter::once(&first[5..]).chain(words))
                }
                _ => {}
            }
        }
        ext
    }

    fn add_auth<'a>(&mut self, mechs: impl Iterator<Item = &'a str>) {
        for m in mechs {
            let m = m.to_ascii_uppercase();
            if !m.is_empty() && !self.auth.contains(&m) {
                self.auth.push(m);
            }
        }
    }

    pub fn has_auth(&self, mech: &str) -> bool {
        self.auth.iter().any(|m| m == mech)
    }
}

enum Transport {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl Transport {
    async fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Transport::Plain(s) => s.write(buf).await,
            Transport::Tls(s) => s.write(buf).await,
        }
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Transport::Plain(s) => s.flush().await,
            Transport::Tls(s) => s.flush().await,
        }
    }
}

pub struct Connection {
    transport: Option<Transport>,
    rbuf: Vec<u8>,
    logger: Option<Arc<dyn ProtocolLogger>>,
    timeout: Duration,
    local_addr: Option<SocketAddr>,
    /// `None` until EHLO succeeds, and after a HELO fallback.
    pub ext: Option<Extensions>,
}

impl Connection {
    pub async fn open(
        host: &str,
        port: u16,
        timeout: Duration,
        logger: Option<Arc<dyn ProtocolLogger>>,
    ) -> Result<Connection, StreamError> {
        let tcp = tokio::time::timeout(timeout, TcpStream::connect((host, port)))
            .await
            .map_err(|_| StreamError(format!("connecting to {host}:{port} timed out")))?
            .map_err(|e| StreamError(format!("connecting to {host}:{port}: {e}")))?;
        let _ = tcp.set_nodelay(true);
        let local_addr = tcp.local_addr().ok();
        Ok(Connection {
            transport: Some(Transport::Plain(tcp)),
            rbuf: Vec::new(),
            logger,
            timeout,
            local_addr,
            ext: None,
        })
    }

    pub fn note(&self, text: &str) {
        if let Some(l) = &self.logger {
            l.note(text);
        }
    }

    /// Wraps the socket in TLS. Any bytes the server sent ahead of the handshake must
    /// already have been rejected by the caller.
    pub async fn upgrade_tls(
        &mut self,
        config: Arc<rustls::ClientConfig>,
        host: &str,
    ) -> Result<(), TlsError> {
        let Some(Transport::Plain(tcp)) = self.transport.take() else {
            return Err(TlsError::Handshake(
                "connection is not a plain socket".into(),
            ));
        };
        // rustls sends SNI only for DNS names, which is what libetpan's ssl_callback does.
        let name = ServerName::try_from(host.to_owned())
            .map_err(|e| TlsError::Handshake(format!("invalid server name {host}: {e}")))?;
        let connect = TlsConnector::from(config).connect(name, tcp);
        let tls = tokio::time::timeout(self.timeout, connect)
            .await
            .map_err(|_| TlsError::Handshake("TLS handshake timed out".into()))?
            .map_err(classify_tls_error)?;
        self.transport = Some(Transport::Tls(Box::new(tls)));
        Ok(())
    }

    /// Bytes the server sent that have not been read as a reply yet.
    pub fn has_buffered_input(&self) -> bool {
        !self.rbuf.is_empty()
    }

    pub async fn write(&mut self, bytes: &[u8], private: bool) -> Result<(), StreamError> {
        if let Some(l) = &self.logger {
            let dir = if private {
                Direction::SentPrivate
            } else {
                Direction::Sent
            };
            l.data(dir, bytes);
        }
        // The timeout covers each socket write, not the whole buffer: libetpan polls with
        // `smtp_timeout` per write (`mailstream_socket.c`), so a slow DATA upload that keeps
        // making progress never times out.
        let timeout = self.timeout;
        let transport = self
            .transport
            .as_mut()
            .ok_or_else(|| StreamError("not connected".into()))?;
        let mut rest = bytes;
        while !rest.is_empty() {
            let chunk = &rest[..rest.len().min(WRITE_CHUNK)];
            match tokio::time::timeout(timeout, transport.write(chunk)).await {
                Ok(Ok(0)) => {
                    return self.fail(
                        Direction::ErrorSent,
                        "write failed: connection closed".into(),
                    )
                }
                Ok(Ok(n)) => rest = &rest[n..],
                Ok(Err(e)) => return self.fail(Direction::ErrorSent, format!("write failed: {e}")),
                Err(_) => return self.fail(Direction::ErrorSent, "write timed out".into()),
            }
        }
        match tokio::time::timeout(timeout, transport.flush()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => self.fail(Direction::ErrorSent, format!("write failed: {e}")),
            Err(_) => self.fail(Direction::ErrorSent, "write timed out".into()),
        }
    }

    fn fail<T>(&mut self, direction: Direction, why: String) -> Result<T, StreamError> {
        if let Some(l) = &self.logger {
            l.data(direction, format!("{why}\r\n").as_bytes());
        }
        self.transport = None;
        Err(StreamError(why))
    }

    async fn fill(&mut self) -> Result<(), StreamError> {
        let timeout = self.timeout;
        let Some(transport) = self.transport.as_mut() else {
            return Err(StreamError("not connected".into()));
        };
        let mut chunk = [0u8; 4096];
        let io = async {
            match transport {
                Transport::Plain(s) => s.read(&mut chunk).await,
                Transport::Tls(s) => s.read(&mut chunk).await,
            }
        };
        match tokio::time::timeout(timeout, io).await {
            Ok(Ok(0)) => self.fail(
                Direction::ErrorReceived,
                "connection closed by server".into(),
            ),
            Ok(Ok(n)) => {
                self.rbuf.extend_from_slice(&chunk[..n]);
                Ok(())
            }
            Ok(Err(e)) => self.fail(Direction::ErrorReceived, format!("read failed: {e}")),
            Err(_) => self.fail(Direction::ErrorReceived, "read timed out".into()),
        }
    }

    async fn read_line(&mut self) -> Result<Vec<u8>, StreamError> {
        loop {
            if let Some(p) = self.rbuf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.rbuf.drain(..=p).collect();
                if let Some(l) = &self.logger {
                    l.data(Direction::Received, &line);
                }
                return Ok(line);
            }
            if self.rbuf.len() > MAX_REPLY_BYTES {
                return self.fail(Direction::ErrorParse, "reply line too long".into());
            }
            self.fill().await?;
        }
    }

    /// `read_response`: reads lines until one without the `-` continuation mark.
    pub async fn read_reply(&mut self) -> Result<Reply, StreamError> {
        let mut text = String::new();
        loop {
            let raw = self.read_line().await?;
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\r', '\n']);
            let digits = line.bytes().take_while(u8::is_ascii_digit).count();
            // Note: libetpan reads a reply without a numeric code as code 0, a stream error.
            let code: u16 = match line[..digits].parse() {
                Ok(c) if c > 0 => c,
                _ => return self.fail(Direction::ErrorParse, format!("unparseable reply: {line}")),
            };
            let rest = &line[digits..];
            let (more, msg) = match rest.as_bytes().first() {
                Some(b'-') => (true, &rest[1..]),
                Some(b' ') => (false, &rest[1..]),
                _ => (false, rest),
            };
            text.push_str(msg);
            text.push('\n');
            if text.len() > MAX_REPLY_BYTES {
                return self.fail(Direction::ErrorParse, "reply too long".into());
            }
            if !more {
                return Ok(Reply { code, text });
            }
        }
    }

    pub async fn command(&mut self, line: &str, private: bool) -> Result<Reply, StreamError> {
        self.write(line.as_bytes(), private).await?;
        self.read_reply().await
    }

    /// `mailsmtp_init`: EHLO, then HELO, and when the FQDN was refused, both again with the
    /// address literal. A stream error stops the ladder. Returns the last refusal.
    pub async fn init(&mut self) -> Result<Result<(), Reply>, StreamError> {
        let first = ehlo::ehlo_name(false, self.local_addr);
        let mut refusal = match self.hello(&first).await? {
            Ok(()) => return Ok(Ok(())),
            Err(r) => r,
        };
        if ehlo::local_fqdn().is_some() {
            let literal = ehlo::ehlo_name(true, self.local_addr);
            if literal != first {
                match self.hello(&literal).await? {
                    Ok(()) => return Ok(Ok(())),
                    Err(r) => refusal = r,
                }
            }
        }
        Ok(Err(refusal))
    }

    async fn hello(&mut self, name: &str) -> Result<Result<(), Reply>, StreamError> {
        let reply = self.command(&format!("EHLO {name}\r\n"), false).await?;
        if reply.code == 250 {
            self.ext = Some(Extensions::parse(&reply));
            return Ok(Ok(()));
        }
        let reply = self.command(&format!("HELO {name}\r\n"), false).await?;
        if reply.code == 250 {
            self.ext = None;
            return Ok(Ok(()));
        }
        Ok(Err(reply))
    }

    /// The DATA payload after the 354, with the terminating dot line.
    pub async fn send_data(&mut self, stuffed: &[u8]) -> Result<Reply, StreamError> {
        self.write(stuffed, false).await?;
        self.read_reply().await
    }

    /// `mailsmtp_quit`: sends QUIT without waiting for the 221, then closes.
    pub async fn quit(mut self) {
        if self.transport.is_some() {
            let _ = self.write(b"QUIT\r\n", false).await;
        }
        if let Some(t) = self.transport.take() {
            let _ = tokio::time::timeout(Duration::from_secs(1), async move {
                match t {
                    Transport::Plain(mut s) => s.shutdown().await,
                    Transport::Tls(mut s) => s.shutdown().await,
                }
            })
            .await;
        }
    }
}

fn classify_tls_error(e: std::io::Error) -> TlsError {
    let text = e.to_string();
    let rustls_err = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>());
    match rustls_err {
        Some(rustls::Error::InvalidCertificate(_))
        | Some(rustls::Error::NoCertificatesPresented) => TlsError::Certificate(text),
        _ => TlsError::Handshake(text),
    }
}

/// The client configuration for TLS and STARTTLS: TLS 1.2 and 1.3 only,
/// certificates checked against the operating system's trust store.
pub fn platform_tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| e.to_string())?
        .with_platform_verifier()
        .map_err(|e| e.to_string())?
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ehlo_parsing() {
        let reply = Reply {
            code: 250,
            text: "mail.example.test\nsize 1000\nPIPELINING\nAUTH PLAIN login CRAM-MD5\nAUTH=DIGEST-MD5 PLAIN\nstarttls\n8BITMIME\n".into(),
        };
        let ext = Extensions::parse(&reply);
        assert!(ext.starttls && ext.pipelining && ext.eight_bit_mime);
        assert_eq!(ext.size, Some(1000));
        assert_eq!(ext.auth, vec!["PLAIN", "LOGIN", "CRAM-MD5", "DIGEST-MD5"]);
        let bare = Extensions::parse(&Reply {
            code: 250,
            text: "x\nSIZE\n".into(),
        });
        assert_eq!(bare.size, Some(0));
        assert!(bare.auth.is_empty());
    }
}
