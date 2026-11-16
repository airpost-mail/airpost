// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Newline-delimited JSON framing: one message per line, UTF-8, `\n` after each.
//! serde_json escapes newlines inside strings, so a serialized message never contains one.

use std::fmt;
use std::io::{self, BufRead, Write};

use serde::Serialize;

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    /// The line passed the limit. Part of it has been consumed, so the stream cannot be
    /// resynchronized; close it.
    TooLong,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Io(e) => write!(f, "{e}"),
            FrameError::TooLong => write!(f, "line exceeds the maximum length"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        FrameError::Io(e)
    }
}

/// Appends the next line, without its `\n`, to `buf`. Returns `Ok(false)` at end of stream
/// with nothing read; a final line without a newline is returned as a line.
///
/// `buf` is not cleared: the caller clears it after using a line. After an I/O error (a
/// read timeout) the partial line stays in `buf` and the next call continues it.
pub fn read_frame<R: BufRead + ?Sized>(
    r: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> Result<bool, FrameError> {
    loop {
        let available = match r.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FrameError::Io(e)),
        };
        if available.is_empty() {
            return Ok(!buf.is_empty());
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(i) => {
                let too_long = buf.len() + i > max;
                if !too_long {
                    buf.extend_from_slice(&available[..i]);
                }
                r.consume(i + 1);
                return if too_long {
                    Err(FrameError::TooLong)
                } else {
                    Ok(true)
                };
            }
            None => {
                let n = available.len();
                let too_long = buf.len() + n > max;
                if !too_long {
                    buf.extend_from_slice(available);
                }
                r.consume(n);
                if too_long {
                    return Err(FrameError::TooLong);
                }
            }
        }
    }
}

/// Writes `line` and a newline in one call, then flushes.
pub fn write_frame<W: Write + ?Sized>(w: &mut W, line: &[u8]) -> io::Result<()> {
    let mut framed = Vec::with_capacity(line.len() + 1);
    framed.extend_from_slice(line);
    framed.push(b'\n');
    w.write_all(&framed)?;
    w.flush()
}

/// `msg` as one framed line, newline included.
pub fn encode<T: Serialize + ?Sized>(msg: &T) -> Vec<u8> {
    let mut out = serde_json::to_vec(msg).unwrap_or_default();
    out.push(b'\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn lines(input: &[u8], max: usize) -> Vec<Result<Vec<u8>, String>> {
        // A tiny buffer makes lines span several fill_buf calls.
        let mut r = BufReader::with_capacity(3, Cursor::new(input.to_vec()));
        let mut out = Vec::new();
        let mut buf = Vec::new();
        loop {
            match read_frame(&mut r, &mut buf, max) {
                Ok(false) => return out,
                Ok(true) => out.push(Ok(std::mem::take(&mut buf))),
                Err(e) => {
                    out.push(Err(e.to_string()));
                    buf.clear();
                }
            }
        }
    }

    #[test]
    fn splits_lines() {
        assert_eq!(
            lines(b"{\"a\":1}\n\n{\"b\":2}\npartial", 64),
            vec![
                Ok(b"{\"a\":1}".to_vec()),
                Ok(Vec::new()),
                Ok(b"{\"b\":2}".to_vec()),
                Ok(b"partial".to_vec()),
            ]
        );
        assert!(lines(b"", 64).is_empty());
    }

    #[test]
    fn limit_is_inclusive_and_over_it_errors() {
        assert_eq!(lines(b"12345\n", 5), vec![Ok(b"12345".to_vec())]);
        let out = lines(b"123456\nok\n", 5);
        assert_eq!(out[0], Err("line exceeds the maximum length".to_owned()));
        let out = lines(b"1234567890", 5);
        assert!(out[0].is_err());
    }

    #[test]
    fn write_and_encode() {
        let mut out = Vec::new();
        write_frame(&mut out, b"{}").unwrap();
        assert_eq!(out, b"{}\n");
        assert_eq!(
            encode(&serde_json::json!({"a": "x\ny"})),
            b"{\"a\":\"x\\ny\"}\n"
        );
    }
}
