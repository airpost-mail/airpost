// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The socket under an IMAP session: TCP, TLS and STARTTLS on rustls, COMPRESS=DEFLATE,
//! response framing (a line plus its literals), and the protocol transcript.
//!
//! Plays the part of libetpan's `mailstream_*` and `mailimap_read_line` (Copyright ©
//! 2001-2005 DINH Viet Hoa, BSD-3-Clause) and mailcore2's `ConnectionLogger` hooks
//! (`MCIMAPSession.cpp`, Copyright © 2001-2013 MailCore team, BSD-3-Clause). See NOTICE.md
//! ("mailcore2", "libEtPan!").

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_compression::tokio::bufread::DeflateDecoder;
use async_compression::tokio::write::DeflateEncoder;
use ms_core::protolog::{Direction, ProtocolLogger};
use rustls::pki_types::ServerName;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// A literal larger than this is not a message; the stream is treated as broken.
const MAX_LITERAL: usize = 1 << 30;
/// Longest line between literals.
const MAX_LINE: usize = 16 << 20;
/// Literals are read in chunks so the read timeout bounds a stall, not a whole message.
const LITERAL_CHUNK: usize = 64 * 1024;

type Reader = Pin<Box<dyn AsyncBufRead + Send>>;
type Writer = Pin<Box<dyn AsyncWrite + Send>>;

enum Stream {
    /// Before TLS: kept as halves of one `TcpStream` so STARTTLS can reunite them.
    Plain {
        reader: BufReader<OwnedReadHalf>,
        writer: OwnedWriteHalf,
    },
    Boxed {
        reader: Reader,
        writer: Writer,
    },
}

/// Why a TLS handshake failed.
#[derive(Debug)]
pub(crate) enum TlsFailure {
    /// The certificate was rejected (mailcore's `checkCertificate`).
    Certificate(String),
    /// The negotiation itself failed; the text is what rustls said.
    Handshake(String),
}

pub(crate) struct Wire {
    stream: Stream,
    logger: Option<Arc<dyn ProtocolLogger>>,
    timeout: Duration,
    compressed: bool,
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "timed out")
}

impl Wire {
    pub async fn connect(
        host: &str,
        port: u16,
        timeout: Duration,
        logger: Option<Arc<dyn ProtocolLogger>>,
    ) -> io::Result<Wire> {
        let tcp = tokio::time::timeout(timeout, TcpStream::connect((host, port)))
            .await
            .map_err(|_| timed_out())??;
        let _ = tcp.set_nodelay(true);
        let (r, w) = tcp.into_split();
        Ok(Wire {
            stream: Stream::Plain {
                reader: BufReader::new(r),
                writer: w,
            },
            logger,
            timeout,
            compressed: false,
        })
    }

    pub fn note(&self, text: &str) {
        if let Some(l) = &self.logger {
            l.note(text);
        }
    }

    /// Bytes received but not yet parsed. After a STARTTLS OK this must be empty: anything
    /// already there was injected before the handshake (CVE-2011-0411 class).
    pub fn has_buffered_input(&self) -> bool {
        match &self.stream {
            Stream::Plain { reader, .. } => !reader.buffer().is_empty(),
            Stream::Boxed { .. } => false,
        }
    }

    /// Runs the TLS handshake on the plain socket (implicit TLS, or after STARTTLS).
    pub async fn upgrade_tls(
        &mut self,
        config: Arc<rustls::ClientConfig>,
        host: &str,
    ) -> Result<(), TlsFailure> {
        let Stream::Plain { .. } = &self.stream else {
            return Err(TlsFailure::Handshake("TLS is already active".into()));
        };
        let placeholder = Stream::Boxed {
            reader: Box::pin(BufReader::new(tokio::io::empty())),
            writer: Box::pin(tokio::io::sink()),
        };
        let Stream::Plain { reader, writer } = std::mem::replace(&mut self.stream, placeholder)
        else {
            unreachable!()
        };
        let tcp = reader
            .into_inner()
            .reunite(writer)
            .map_err(|e| TlsFailure::Handshake(e.to_string()))?;
        let name = ServerName::try_from(host.to_owned())
            .map_err(|e| TlsFailure::Handshake(format!("invalid server name {host}: {e}")))?;
        let connector = TlsConnector::from(config);
        let tls = tokio::time::timeout(self.timeout, connector.connect(name, tcp))
            .await
            .map_err(|_| TlsFailure::Handshake("TLS handshake timed out".into()))?
            .map_err(classify_tls_error)?;
        let (r, w) = tokio::io::split(tls);
        self.stream = Stream::Boxed {
            reader: Box::pin(BufReader::new(r)),
            writer: Box::pin(w),
        };
        Ok(())
    }

    /// COMPRESS=DEFLATE (RFC 4978): raw deflate both ways from the next byte on. Bytes the
    /// server already sent stay in the old buffer, which the decoder reads first.
    pub fn start_compression(&mut self) {
        let placeholder = Stream::Boxed {
            reader: Box::pin(BufReader::new(tokio::io::empty())),
            writer: Box::pin(tokio::io::sink()),
        };
        let (reader, writer): (Reader, Writer) =
            match std::mem::replace(&mut self.stream, placeholder) {
                Stream::Plain { reader, writer } => (Box::pin(reader), Box::pin(writer)),
                Stream::Boxed { reader, writer } => (reader, writer),
            };
        self.stream = Stream::Boxed {
            reader: Box::pin(BufReader::new(DeflateDecoder::new(reader))),
            writer: Box::pin(DeflateEncoder::new(writer)),
        };
        self.compressed = true;
    }

    pub fn is_compressed(&self) -> bool {
        self.compressed
    }

    fn reader(&mut self) -> Pin<&mut (dyn AsyncBufRead + Send)> {
        match &mut self.stream {
            Stream::Plain { reader, .. } => {
                let p: Pin<&mut BufReader<OwnedReadHalf>> = Pin::new(reader);
                p
            }
            Stream::Boxed { reader, .. } => reader.as_mut(),
        }
    }

    fn writer(&mut self) -> Pin<&mut (dyn AsyncWrite + Send)> {
        match &mut self.stream {
            Stream::Plain { writer, .. } => {
                let p: Pin<&mut OwnedWriteHalf> = Pin::new(writer);
                p
            }
            Stream::Boxed { writer, .. } => writer.as_mut(),
        }
    }

    /// Writes and flushes `bytes`. `private` keeps them out of the transcript.
    pub async fn write(&mut self, bytes: &[u8], private: bool) -> io::Result<()> {
        if let Some(l) = &self.logger {
            l.data(
                if private {
                    Direction::SentPrivate
                } else {
                    Direction::Sent
                },
                bytes,
            );
        }
        let timeout = self.timeout;
        let mut w = self.writer();
        tokio::time::timeout(timeout, async {
            w.write_all(bytes).await?;
            w.flush().await
        })
        .await
        .map_err(|_| timed_out())?
    }

    /// Waits until the server has sent something, without consuming it. Cancel-safe.
    pub async fn wait_readable(&mut self) -> io::Result<()> {
        let mut r = self.reader();
        let closed = r.fill_buf().await?.is_empty();
        if closed {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        Ok(())
    }

    /// One response: a line and, while it ends in a literal marker, the literal and the
    /// line after it. Returned with every CRLF, as received.
    pub async fn read_response(&mut self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        loop {
            let start = buf.len();
            self.read_line(&mut buf).await?;
            let line = &buf[start..];
            match literal_size(line) {
                Some(n) if n > MAX_LITERAL => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("literal of {n} bytes"),
                    ));
                }
                Some(n) => self.read_exact_into(&mut buf, n).await?,
                None => break,
            }
        }
        if let Some(l) = &self.logger {
            l.data(Direction::Received, &buf);
        }
        Ok(buf)
    }

    async fn read_line(&mut self, buf: &mut Vec<u8>) -> io::Result<()> {
        let timeout = self.timeout;
        let start = buf.len();
        loop {
            let mut r = self.reader();
            let n = tokio::time::timeout(timeout, r.read_until(b'\n', buf))
                .await
                .map_err(|_| timed_out())??;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed",
                ));
            }
            if buf.last() == Some(&b'\n') {
                return Ok(());
            }
            if buf.len() - start > MAX_LINE {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
            }
        }
    }

    async fn read_exact_into(&mut self, buf: &mut Vec<u8>, n: usize) -> io::Result<()> {
        let timeout = self.timeout;
        let mut left = n;
        while left > 0 {
            let chunk = left.min(LITERAL_CHUNK);
            let at = buf.len();
            buf.resize(at + chunk, 0);
            let mut r = self.reader();
            tokio::time::timeout(timeout, r.read_exact(&mut buf[at..]))
                .await
                .map_err(|_| timed_out())??;
            left -= chunk;
        }
        Ok(())
    }

    pub async fn shutdown(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(1), self.writer().shutdown()).await;
    }
}

/// The size of the literal a line announces (`… {123}` or `{123+}` before the CRLF).
fn literal_size(line: &[u8]) -> Option<usize> {
    let line = line.strip_suffix(b"\n")?;
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let inner = line.strip_suffix(b"}")?;
    let open = inner.iter().rposition(|b| *b == b'{')?;
    let digits = &inner[open + 1..];
    let digits = digits.strip_suffix(b"+").unwrap_or(digits);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

fn classify_tls_error(e: io::Error) -> TlsFailure {
    let text = e.to_string();
    let rustls_err = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>());
    match rustls_err {
        Some(rustls::Error::InvalidCertificate(_))
        | Some(rustls::Error::NoCertificatesPresented) => TlsFailure::Certificate(text),
        _ => TlsFailure::Handshake(text),
    }
}

/// The client configuration for TLS and STARTTLS: TLS 1.2 and 1.3 (rustls has nothing
/// older), certificates checked against the operating system's trust store.
pub(crate) fn platform_tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
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
    fn literal_markers() {
        assert_eq!(literal_size(b"* 1 FETCH (BODY[] {12}\r\n"), Some(12));
        assert_eq!(literal_size(b"* 1 FETCH (BODY[] {12+}\r\n"), Some(12));
        assert_eq!(literal_size(b"* 1 FETCH (BODY[] {}\r\n"), None);
        assert_eq!(literal_size(b"* OK done\r\n"), None);
        assert_eq!(literal_size(b"* OK {x}\n"), None);
        assert_eq!(literal_size(b"* LIST () \"/\" {3}\n"), Some(3));
    }
}
