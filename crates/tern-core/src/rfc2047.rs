// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! RFC 2047 header decoding for the values that feed message ids.
//!
//! This is deliberately our own small decoder rather than the MIME library's: ids must not
//! change when the MIME crate's display decoding does (see [`crate::ids`]). It is lenient in
//! the ways real mail needs:
//! - adjacent encoded words are joined before charset decoding, so a multi-byte character
//!   split across two words (common with long UTF-8 B words) survives;
//! - whitespace between two encoded words is dropped (RFC 2047 section 6.2);
//! - Q text may contain spaces, B text may lack padding;
//! - bytes outside encoded words are UTF-8 when valid, otherwise windows-1252;
//! - an encoded word with an unknown charset is kept as literal text.

use encoding_rs::{Encoding, WINDOWS_1252};

enum Segment<'a> {
    Text(&'a [u8]),
    Encoded {
        charset: &'a [u8],
        bytes: Vec<u8>,
        raw: &'a [u8],
    },
}

/// Decodes an unfolded or folded header value to UTF-8. Line breaks are removed (folding
/// whitespace keeps its space); no other trimming is done.
pub fn decode(raw: &[u8]) -> String {
    let unfolded: Vec<u8> = raw
        .iter()
        .copied()
        .filter(|b| *b != b'\r' && *b != b'\n')
        .collect();
    let mut segments = split(&unfolded);

    // Whitespace between two encoded words is not part of the text.
    let mut k = 1;
    while k + 1 < segments.len() {
        let drop = matches!(&segments[k], Segment::Text(t) if t.iter().all(|b| b.is_ascii_whitespace()))
            && matches!(segments[k - 1], Segment::Encoded { .. })
            && matches!(segments[k + 1], Segment::Encoded { .. });
        if drop {
            segments.remove(k);
        } else {
            k += 1;
        }
    }

    let mut out = String::with_capacity(unfolded.len());
    let mut i = 0;
    while i < segments.len() {
        match &segments[i] {
            Segment::Text(t) => {
                out.push_str(&decode_text(t));
                i += 1;
            }
            Segment::Encoded { charset, .. } => {
                let label = charset_label(charset);
                let mut joined: Vec<u8> = Vec::new();
                let mut raws: Vec<&[u8]> = Vec::new();
                while let Some(Segment::Encoded {
                    charset: c,
                    bytes,
                    raw,
                }) = segments.get(i)
                {
                    if !charset_label(c).eq_ignore_ascii_case(label) {
                        break;
                    }
                    joined.extend_from_slice(bytes);
                    raws.push(raw);
                    i += 1;
                }
                match Encoding::for_label(label) {
                    Some(enc) => out.push_str(&enc.decode_without_bom_handling(&joined).0),
                    None => {
                        for r in raws {
                            out.push_str(&decode_text(r));
                        }
                    }
                }
            }
        }
    }
    out
}

/// RFC 2231 allows `charset*language`; only the charset matters here.
fn charset_label(charset: &[u8]) -> &[u8] {
    match charset.iter().position(|b| *b == b'*') {
        Some(p) => &charset[..p],
        None => charset,
    }
}

fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

fn split(input: &[u8]) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    while i + 1 < input.len() {
        if input[i] == b'=' && input[i + 1] == b'?' {
            if let Some((end, segment)) = parse_word(input, i) {
                if text_start < i {
                    segments.push(Segment::Text(&input[text_start..i]));
                }
                segments.push(segment);
                i = end;
                text_start = end;
                continue;
            }
        }
        i += 1;
    }
    if text_start < input.len() {
        segments.push(Segment::Text(&input[text_start..]));
    }
    segments
}

/// Parses `=?charset?X?text?=` starting at `start`; returns the end offset and segment.
fn parse_word(input: &[u8], start: usize) -> Option<(usize, Segment<'_>)> {
    let rest = &input[start + 2..];
    let q1 = rest.iter().position(|b| *b == b'?')?;
    let charset = &rest[..q1];
    if charset.is_empty() || charset.iter().any(|b| b.is_ascii_whitespace()) {
        return None;
    }
    let after_charset = &rest[q1 + 1..];
    if after_charset.len() < 2 || after_charset[1] != b'?' {
        return None;
    }
    let encoding = after_charset[0].to_ascii_uppercase();
    if encoding != b'B' && encoding != b'Q' {
        return None;
    }
    let text_and_rest = &after_charset[2..];
    let end_rel = text_and_rest.windows(2).position(|w| w == b"?=")?;
    let text = &text_and_rest[..end_rel];
    let bytes = if encoding == b'B' {
        decode_b(text)
    } else {
        decode_q(text)
    };
    let consumed = 2 + q1 + 1 + 2 + end_rel + 2;
    let end = start + consumed;
    Some((
        end,
        Segment::Encoded {
            charset,
            bytes,
            raw: &input[start..end],
        },
    ))
}

fn decode_q(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        match text[i] {
            b'_' => out.push(b' '),
            b'=' if i + 2 < text.len() => match (hex(text[i + 1]), hex(text[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
                _ => out.push(b'='),
            },
            b => out.push(b),
        }
        i += 1;
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_b(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::decode;

    fn d(s: &str) -> String {
        decode(s.as_bytes())
    }

    #[test]
    fn plain() {
        assert_eq!(d("Hello world"), "Hello world");
        assert_eq!(decode(b"caf\xe9"), "café");
        assert_eq!(d("caf\u{e9}"), "café");
    }

    #[test]
    fn encoded_words() {
        assert_eq!(d("=?UTF-8?B?Q2Fmw6k=?="), "Café");
        assert_eq!(d("=?iso-8859-1?q?caf=E9_cr=E8me?="), "café crème");
        assert_eq!(
            d("Re: =?utf-8?q?Gr=C3=BC=C3=9Fe?= aus Berlin"),
            "Re: Grüße aus Berlin"
        );
        // whitespace between words is dropped, a multibyte char split across words survives
        assert_eq!(d("=?UTF-8?B?w6k=?= =?UTF-8?B?w6k=?="), "éé");
        assert_eq!(d("=?UTF-8?Q?=C3?= =?UTF-8?Q?=A9?="), "é");
        assert_eq!(d("=?UTF-8?Q?a?=\r\n =?UTF-8?Q?b?="), "ab");
        // RFC 2231 language suffix
        assert_eq!(d("=?UTF-8*en?Q?hi?="), "hi");
        // unpadded base64
        assert_eq!(d("=?UTF-8?B?Q2Fmw6k?="), "Café");
        // text between words that is not only whitespace is kept
        assert_eq!(d("=?UTF-8?Q?a?= - =?UTF-8?Q?b?="), "a - b");
    }

    #[test]
    fn malformed() {
        assert_eq!(d("=?unknown-charset?Q?x?="), "=?unknown-charset?Q?x?=");
        assert_eq!(d("=?UTF-8?X?x?="), "=?UTF-8?X?x?=");
        assert_eq!(d("=?UTF-8?Q?unterminated"), "=?UTF-8?Q?unterminated");
        assert_eq!(d("=?UTF-8?Q?a=ZZ?="), "a=ZZ");
    }
}
