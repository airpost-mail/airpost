// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Content-Transfer-Encoding decoders.
//!
//! Derived from mailcore2 `Data::decodedDataUsingEncoding` and libetpan's
//! `mailmime_part_parse` (Copyright © 2001-2013 MailCore team and © 2001-2005 DINH Viet
//! Hoa, BSD-3-Clause; see NOTICE.md, "mailcore2" and "libEtPan!"). All three decoders are
//! lenient the way libetpan is: bytes outside the alphabet are skipped and a malformed
//! escape is kept as text, so a damaged part still yields most of its content.

/// The transfer encodings mailcore distinguishes (`Encoding` in `MCMessageConstants.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferEncoding {
    /// 7bit, 8bit, binary and anything unrecognised: the bytes are used as they are.
    Identity,
    QuotedPrintable,
    Base64,
    UuEncode,
}

impl TransferEncoding {
    /// `Attachment::encodingForMIMEEncoding`. A missing header is 8bit.
    pub(crate) fn from_header(value: Option<&str>) -> Self {
        let Some(v) = value else {
            return TransferEncoding::Identity;
        };
        let v = v.trim().trim_matches('"');
        if v.eq_ignore_ascii_case("base64") {
            TransferEncoding::Base64
        } else if v.eq_ignore_ascii_case("quoted-printable") {
            TransferEncoding::QuotedPrintable
        } else if v.eq_ignore_ascii_case("x-uuencode") || v.eq_ignore_ascii_case("x-uue") {
            TransferEncoding::UuEncode
        } else {
            TransferEncoding::Identity
        }
    }

    pub(crate) fn decode(self, data: &[u8]) -> Vec<u8> {
        match self {
            TransferEncoding::Identity => data.to_vec(),
            TransferEncoding::QuotedPrintable => decode_quoted_printable(data),
            TransferEncoding::Base64 => decode_base64(data),
            TransferEncoding::UuEncode => decode_uuencode(data),
        }
    }
}

/// Base64, skipping every byte outside the alphabet. Padding ends a quantum, and decoding
/// resumes after it, so concatenated base64 blocks decode in full.
pub(crate) fn decode_base64(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in data {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                acc = 0;
                bits = 0;
                continue;
            }
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

/// Quoted-printable (RFC 2045 section 6.7). `=` followed by optional blanks and a line
/// break is a soft break; `=` not followed by two hex digits is kept literally.
pub(crate) fn decode_quoted_printable(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        if c != b'=' {
            out.push(c);
            i += 1;
            continue;
        }
        if let (Some(h), Some(l)) = (
            data.get(i + 1).and_then(|b| hex(*b)),
            data.get(i + 2).and_then(|b| hex(*b)),
        ) {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        let mut j = i + 1;
        while j < data.len() && (data[j] == b' ' || data[j] == b'\t') {
            j += 1;
        }
        if j == data.len() {
            i = j;
        } else if data[j] == b'\n' {
            i = j + 1;
        } else if data[j] == b'\r' {
            i = if data.get(j + 1) == Some(&b'\n') {
                j + 2
            } else {
                j + 1
            };
        } else {
            out.push(b'=');
            i += 1;
        }
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

/// uuencode: the lines between `begin <mode> <name>` and `end`. Without a `begin` line
/// every line is decoded.
pub(crate) fn decode_uuencode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    let mut started = !data
        .split(|b| *b == b'\n')
        .any(|l| l.starts_with(b"begin "));
    for line in data.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if !started {
            started = line.starts_with(b"begin ");
            continue;
        }
        if line == b"end" {
            break;
        }
        let Some((&len_char, rest)) = line.split_first() else {
            continue;
        };
        let len = (len_char.wrapping_sub(b' ') & 0x3f) as usize;
        if len == 0 {
            continue;
        }
        let mut decoded = Vec::with_capacity(len + 2);
        for chunk in rest.chunks(4) {
            let v: Vec<u32> = (0..4)
                .map(|k| (chunk.get(k).copied().unwrap_or(b' ').wrapping_sub(b' ') & 0x3f) as u32)
                .collect();
            let n = (v[0] << 18) | (v[1] << 12) | (v[2] << 6) | v[3];
            decoded.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
        }
        decoded.truncate(len);
        out.extend_from_slice(&decoded);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_lenient() {
        assert_eq!(decode_base64(b"SGVs\r\nbG8=\r\n"), b"Hello");
        assert_eq!(decode_base64(b"SGVsbG8"), b"Hello");
        assert_eq!(decode_base64(b"SGk=SGk="), b"HiHi");
        assert_eq!(decode_base64(b"S G\tV*sbG8="), b"Hello");
    }

    #[test]
    fn quoted_printable_lenient() {
        assert_eq!(decode_quoted_printable(b"caf=C3=A9"), "café".as_bytes());
        assert_eq!(decode_quoted_printable(b"soft=\r\nbreak"), b"softbreak");
        assert_eq!(decode_quoted_printable(b"soft= \nbreak"), b"softbreak");
        assert_eq!(decode_quoted_printable(b"a=ZZb"), b"a=ZZb");
        assert_eq!(decode_quoted_printable(b"x=3d=3D"), b"x==");
        assert_eq!(decode_quoted_printable(b"end="), b"end");
        assert_eq!(decode_quoted_printable(b"l1\r\nl2"), b"l1\r\nl2");
    }

    #[test]
    fn uuencode() {
        let enc = b"begin 644 cat.txt\n#0V%T\n`\nend\n";
        assert_eq!(decode_uuencode(enc), b"Cat");
    }

    #[test]
    fn encoding_names() {
        assert_eq!(
            TransferEncoding::from_header(Some(" BASE64 ")),
            TransferEncoding::Base64
        );
        assert_eq!(
            TransferEncoding::from_header(Some("Quoted-Printable")),
            TransferEncoding::QuotedPrintable
        );
        assert_eq!(
            TransferEncoding::from_header(Some("7bit")),
            TransferEncoding::Identity
        );
        assert_eq!(
            TransferEncoding::from_header(None),
            TransferEncoding::Identity
        );
    }
}
