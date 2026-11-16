// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Modified UTF-7 mailbox names (RFC 3501 section 5.1.3): the encoding the desktop's
//! `imapUtf7` gives a new folder or label name before queueing `SyncbackCategoryTask`, and
//! the decoding that turns a stored `path` back into the name a person types. The same code
//! as `airpost_core::utf7`, which clients don't link (it brings SQLite).

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

/// Encodes a UTF-8 mailbox name. Printable ASCII passes through (`&` becomes `&-`); every
/// other run is UTF-16BE in modified base64 between `&` and `-`.
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

fn flush(pending: &mut Vec<u16>, out: &mut String) {
    if pending.is_empty() {
        return;
    }
    let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
    out.push('&');
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        let chars = match chunk.len() {
            1 => 2,
            2 => 3,
            _ => 4,
        };
        for i in 0..chars {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out.push('-');
    pending.clear();
}

/// Decodes a modified UTF-7 mailbox name, or `None` when it is not valid modified UTF-7
/// (servers that send raw UTF-8 names are common; callers keep those as they are).
pub fn decode(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut out = String::with_capacity(name.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'&' {
            if !(0x20..=0x7e).contains(&b) {
                return None;
            }
            out.push(b as char);
            i += 1;
            continue;
        }
        let end = i + 1 + bytes[i + 1..].iter().position(|c| *c == b'-')?;
        if end == i + 1 {
            out.push('&');
            i = end + 1;
            continue;
        }
        let mut acc: u32 = 0;
        let mut bits = 0;
        let mut units: Vec<u16> = Vec::new();
        let mut byte_buf: Vec<u8> = Vec::new();
        for &c in &bytes[i + 1..end] {
            let v = ALPHABET.iter().position(|a| *a == c)? as u32;
            acc = acc << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                byte_buf.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
        if !byte_buf.len().is_multiple_of(2) {
            return None;
        }
        for pair in byte_buf.chunks(2) {
            units.push(u16::from_be_bytes([pair[0], pair[1]]));
        }
        out.push_str(&String::from_utf16(&units).ok()?);
        i = end + 1;
    }
    Some(out)
}

/// [`decode`], falling back to the name as given.
pub fn decode_or_raw(name: &str) -> String {
    decode(name).unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3501_examples() {
        assert_eq!(
            decode("~peter/mail/&U,BTFw-/&ZeVnLIqe-").unwrap(),
            "~peter/mail/台北/日本語"
        );
        assert_eq!(
            encode("~peter/mail/台北/日本語"),
            "~peter/mail/&U,BTFw-/&ZeVnLIqe-"
        );
        assert_eq!(decode("Entw&APw-rfe").unwrap(), "Entwürfe");
        assert_eq!(encode("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(encode("A&B"), "A&-B");
        assert_eq!(decode("A&-B").unwrap(), "A&B");
        assert_eq!(encode("Отправленные"), "&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-");
        assert_eq!(
            decode("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-").unwrap(),
            "Отправленные"
        );
        assert_eq!(encode("😀"), "&2D3eAA-");
        assert_eq!(decode("&2D3eAA-").unwrap(), "😀");
    }

    #[test]
    fn invalid() {
        assert_eq!(decode("Entwürfe"), None);
        assert_eq!(decode("&Jjo"), None);
        assert_eq!(decode_or_raw("Entwürfe"), "Entwürfe");
    }
}
