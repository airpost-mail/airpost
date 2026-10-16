// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A blocking client: one request at a time, notifications buffered while a call waits for
//! its response. The socket transport is Unix-only for now (plan S1); elsewhere
//! [`Client::connect`] returns [`ClientError::Unsupported`].

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::envelope::{parse_message, ErrorObject, Id, Message, Notification, Request};
use crate::frame::{encode, read_frame, write_frame, FrameError};
use crate::{method, HelloParams, HelloResult, SubscribeParams, MAX_LINE_BYTES, PROTOCOL_VERSION};

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    /// The engine sent a line over [`MAX_LINE_BYTES`].
    LineTooLong,
    /// The engine sent something that is not a v1 message, or a response to another request.
    Protocol(String),
    /// The engine answered with an error.
    Rpc(ErrorObject),
    /// The engine closed the connection.
    Closed,
    /// No socket transport on this platform.
    Unsupported,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "{e}"),
            ClientError::LineTooLong => write!(f, "the engine sent a line over the limit"),
            ClientError::Protocol(m) => write!(f, "protocol error: {m}"),
            ClientError::Rpc(e) => write!(f, "{e}"),
            ClientError::Closed => write!(f, "the engine closed the connection"),
            ClientError::Unsupported => {
                write!(f, "the engine socket is not supported on this platform")
            }
        }
    }
}

impl std::error::Error for ClientError {}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        ClientError::Io(e)
    }
}

impl From<FrameError> for ClientError {
    fn from(e: FrameError) -> Self {
        match e {
            FrameError::Io(e) => ClientError::Io(e),
            FrameError::TooLong => ClientError::LineTooLong,
        }
    }
}

impl ClientError {
    /// The engine's error code, for an [`ClientError::Rpc`].
    pub fn code(&self) -> Option<i64> {
        match self {
            ClientError::Rpc(e) => Some(e.code),
            _ => None,
        }
    }

    /// A read timeout set with [`Client::set_read_timeout`] passed.
    pub fn is_timeout(&self) -> bool {
        matches!(self, ClientError::Io(e)
            if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
    }
}

pub struct Client {
    reader: BufReader<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
    #[cfg(unix)]
    socket: Option<std::os::unix::net::UnixStream>,
    next_id: i64,
    pending: VecDeque<Notification>,
    line: Vec<u8>,
}

impl Client {
    /// Connects to the engine socket at `path`. Call [`Client::hello`] next.
    #[cfg(unix)]
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, ClientError> {
        let stream = std::os::unix::net::UnixStream::connect(path)?;
        let reader = stream.try_clone()?;
        let writer = stream.try_clone()?;
        let mut client = Client::from_streams(Box::new(reader), Box::new(writer));
        client.socket = Some(stream);
        Ok(client)
    }

    #[cfg(not(unix))]
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, ClientError> {
        let _ = path;
        Err(ClientError::Unsupported)
    }

    /// A client over any byte streams (tests, other transports).
    pub fn from_streams(reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>) -> Self {
        Client {
            reader: BufReader::new(reader),
            writer,
            #[cfg(unix)]
            socket: None,
            next_id: 1,
            pending: VecDeque::new(),
            line: Vec::new(),
        }
    }

    /// Bounds every read on a socket client; a read that times out returns an error for
    /// which [`ClientError::is_timeout`] is true, and the next read continues the line.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), ClientError> {
        #[cfg(unix)]
        if let Some(s) = &self.socket {
            s.set_read_timeout(timeout)?;
        }
        #[cfg(not(unix))]
        let _ = timeout;
        Ok(())
    }

    /// `hello` with [`PROTOCOL_VERSION`].
    pub fn hello(&mut self, client: &str) -> Result<HelloResult, ClientError> {
        let params = HelloParams {
            protocol: PROTOCOL_VERSION,
            client: client.to_owned(),
            relayed: false,
            scopes: Vec::new(),
            device: None,
        };
        self.call_as(method::HELLO, to_value(&params)?)
    }

    /// Sends a request and waits for its response. Notifications that arrive first are kept
    /// for [`Client::next_notification`].
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = Id::Num(self.next_id);
        self.next_id += 1;
        let request = Request::new(id.clone(), method, params);
        self.send_raw(&encode(&request))?;
        loop {
            match self.read_message()? {
                Message::Response(r) if r.id.as_ref() == Some(&id) => {
                    return r.into_result().map_err(ClientError::Rpc)
                }
                Message::Response(r) if r.id.is_none() => {
                    return Err(r
                        .error
                        .map(ClientError::Rpc)
                        .unwrap_or_else(|| ClientError::Protocol("a result with no id".into())))
                }
                Message::Response(r) => {
                    return Err(ClientError::Protocol(format!(
                        "a response to {:?} while waiting for {id:?}",
                        r.id
                    )))
                }
                Message::Notification(n) => self.pending.push_back(n),
                // The engine sends no requests in v1.
                Message::Request(_) => {}
            }
        }
    }

    /// [`Client::call`], with the result read as `T`.
    pub fn call_as<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<T, ClientError> {
        let v = self.call(method, params)?;
        serde_json::from_value(v).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// `subscribe`, then the notifications that follow.
    pub fn subscribe(
        &mut self,
        params: &SubscribeParams,
    ) -> Result<Notifications<'_>, ClientError> {
        self.call(method::SUBSCRIBE, to_value(params)?)?;
        Ok(self.notifications())
    }

    pub fn notifications(&mut self) -> Notifications<'_> {
        Notifications { client: self }
    }

    /// The next notification, buffered or read. [`ClientError::Closed`] at end of stream.
    pub fn next_notification(&mut self) -> Result<Notification, ClientError> {
        if let Some(n) = self.pending.pop_front() {
            return Ok(n);
        }
        loop {
            match self.read_message()? {
                Message::Notification(n) => return Ok(n),
                Message::Response(r) => {
                    return Err(ClientError::Protocol(format!(
                        "an unexpected response to {:?}",
                        r.id
                    )))
                }
                Message::Request(_) => {}
            }
        }
    }

    /// Writes one already framed line as is (tests send malformed lines with this).
    pub fn send_raw(&mut self, line: &[u8]) -> Result<(), ClientError> {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        write_frame(&mut self.writer, line)?;
        Ok(())
    }

    /// The next message, skipping blank lines.
    pub fn read_message(&mut self) -> Result<Message, ClientError> {
        loop {
            if !read_frame(&mut self.reader, &mut self.line, MAX_LINE_BYTES)? {
                return Err(ClientError::Closed);
            }
            let line = std::mem::take(&mut self.line);
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            return parse_message(&line).map_err(|e| ClientError::Protocol(e.to_string()));
        }
    }
}

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, ClientError> {
    serde_json::to_value(v).map_err(|e| ClientError::Protocol(e.to_string()))
}

/// Notifications until the engine closes the connection.
pub struct Notifications<'a> {
    client: &'a mut Client,
}

impl Iterator for Notifications<'_> {
    type Item = Result<Notification, ClientError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.client.next_notification() {
            Err(ClientError::Closed) => None,
            other => Some(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn client(engine_output: &str) -> (Client, Shared) {
        let sent = Shared::default();
        let c = Client::from_streams(
            Box::new(Cursor::new(engine_output.as_bytes().to_vec())),
            Box::new(sent.clone()),
        );
        (c, sent)
    }

    #[test]
    fn call_buffers_notifications_and_matches_ids() {
        let (mut c, sent) = client(concat!(
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocol":1,"engine":"0.1.0","pid":7,"accounts":[]}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"delta","params":{"accountId":"a","modelClass":"Thread","type":"persist","modelJSONs":[]}}"#,
            "\n\n",
            r#"{"jsonrpc":"2.0","id":2,"result":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"disconnect","params":{"reason":"bye"}}"#,
            "\n",
        ));
        let hello = c.hello("test/1").unwrap();
        assert_eq!(hello.pid, 7);
        let mut notes = c.subscribe(&SubscribeParams::default()).unwrap();
        let first = notes.next().unwrap().unwrap();
        assert_eq!(first.method, "delta");
        assert_eq!(notes.next().unwrap().unwrap().method, "disconnect");
        assert!(notes.next().is_none());

        let sent = String::from_utf8(sent.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<Value> = sent
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["method"], "hello");
        assert_eq!(lines[0]["params"]["protocol"], PROTOCOL_VERSION);
        assert_eq!(lines[1]["method"], "subscribe");
        assert_eq!(lines[1]["id"], 2);
    }

    #[test]
    fn errors() {
        let (mut c, _) = client(concat!(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"no"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":9,"result":{}}"#,
            "\n",
        ));
        let e = c.hello("t").unwrap_err();
        assert_eq!(e.code(), Some(code::UNSUPPORTED_PROTOCOL));
        assert_eq!(
            c.call("x", Value::Null).unwrap_err().code(),
            Some(code::PARSE_ERROR)
        );
        assert!(matches!(
            c.call("x", Value::Null).unwrap_err(),
            ClientError::Protocol(_)
        ));
        assert!(matches!(
            c.call("x", Value::Null).unwrap_err(),
            ClientError::Closed
        ));
    }
}
