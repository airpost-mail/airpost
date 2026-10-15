//! Protocol transcripts for IMAP and SMTP sessions: mailcore's `ConnectionLogger`.
//!
//! With `--verbose` every line goes to the engine log as `sent <line>` / `recv <line>`,
//! which the harness parses (`IMAP_LINE` in `test/harness/mailsync.py`) and its hooks match
//! on (`^\S+ IDLE$`, `recv \* VANISHED`, `sent \d+ APPEND`). In `test` mode the transcript
//! is accumulated and returned to the client as `log`. Either way AUTH commands and
//! credential payloads are [`Direction::SentPrivate`] and never recorded.

use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Sent,
    /// AUTHENTICATE / LOGIN commands and their continuation payloads.
    SentPrivate,
    Received,
    ErrorParse,
    ErrorReceived,
    ErrorSent,
}

impl Direction {
    /// The log prefix, or `None` for data that must not be logged.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Direction::Sent => Some("sent"),
            Direction::SentPrivate => None,
            Direction::Received => Some("recv"),
            Direction::ErrorParse => Some("error-parse"),
            Direction::ErrorReceived => Some("error-received"),
            Direction::ErrorSent => Some("error-sent"),
        }
    }
}

pub trait ProtocolLogger: Send + Sync {
    /// Raw bytes as written to or read from the wire, CRLFs included.
    fn data(&self, direction: Direction, bytes: &[u8]);
    /// A free-form note from the session (connection attempts, TLS errors).
    fn note(&self, _text: &str) {}
}

/// Verbose logging to the engine log, one log line per protocol line so every line is
/// visible to the harness's line matcher.
#[derive(Clone, Copy, Debug, Default)]
pub struct TracingLogger;

impl ProtocolLogger for TracingLogger {
    fn data(&self, direction: Direction, bytes: &[u8]) {
        let Some(label) = direction.label() else {
            return;
        };
        let text = String::from_utf8_lossy(bytes);
        for line in text.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            tracing::info!("{}", crate::logmsg::protocol_line(label, line));
        }
    }

    fn note(&self, text: &str) {
        tracing::info!("{text}");
    }
}

/// The `test` mode transcript (`AccumulatorLogger` in main.cpp).
#[derive(Debug, Default)]
pub struct AccumulatingLogger {
    buf: Mutex<String>,
}

impl AccumulatingLogger {
    pub fn push(&self, text: &str) {
        self.buf
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_str(text);
    }

    pub fn contents(&self) -> String {
        self.buf.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl ProtocolLogger for AccumulatingLogger {
    fn data(&self, direction: Direction, bytes: &[u8]) {
        if direction != Direction::SentPrivate {
            self.push(&String::from_utf8_lossy(bytes));
        }
    }

    fn note(&self, text: &str) {
        self.push(text);
        self.push("\r\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_skips_private() {
        let a = AccumulatingLogger::default();
        a.data(Direction::Sent, b"1 CAPABILITY\r\n");
        a.data(Direction::SentPrivate, b"2 LOGIN user pass\r\n");
        a.data(Direction::Received, b"1 OK\r\n");
        assert_eq!(a.contents(), "1 CAPABILITY\r\n1 OK\r\n");
    }
}
