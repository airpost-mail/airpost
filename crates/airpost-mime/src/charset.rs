// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Turning body and header bytes into text.
//!
//! Derived from mailcore2 `Data::stringWithDetectedCharset` and `normalizeCharset`
//! (`MCData.cpp`, Copyright © 2001-2013 MailCore team, BSD-3-Clause; see NOTICE.md,
//! "mailcore2"). mailcore asked ICU whether a declared charset that ICU can detect was
//! plausible and otherwise trusted it. Here the declared charset is trusted unless the
//! bytes contradict it, which is what mislabelled mail looks like in practice:
//! - non-ASCII bytes that are valid UTF-8 under a single-byte label are UTF-8;
//! - a label whose decoder hits malformed sequences falls back to UTF-8 when valid, then
//!   to the `chardetng` guess when that decodes cleanly;
//! - no label, or one `encoding_rs` does not know: ISO-2022-JP when its escape sequences
//!   are present, UTF-8 when valid, then `<meta charset>` for HTML, then the `chardetng`
//!   guess;
//! - text that is UTF-8 apart from a few broken sequences stays UTF-8, with U+FFFD for the
//!   broken bytes, rather than becoming a single-byte guess.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, ISO_2022_JP, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252};

enum Label {
    Known(&'static Encoding),
    Utf7,
}

/// Decodes text with an optional declared charset (`Content-Type; charset=`).
pub(crate) fn decode_text(data: &[u8], declared: Option<&str>, is_html: bool) -> String {
    match declared.and_then(resolve_label) {
        Some(Label::Utf7) => decode_utf7(data),
        Some(Label::Known(enc)) => decode_labelled(data, enc),
        None => decode_unlabelled(data, is_html),
    }
}

/// Decodes bytes that carry no label, such as raw 8-bit header text.
pub(crate) fn decode_unlabelled(data: &[u8], is_html: bool) -> String {
    // ISO-2022-JP is 7-bit, so it is also valid UTF-8; the escapes give it away.
    if looks_like_iso_2022_jp(data) {
        let (text, had_errors) = decode_iso_2022_jp(data);
        if !had_errors {
            return text;
        }
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return strip_bom(s).to_owned();
    }
    if mostly_utf8(data) {
        return strip_bom(&String::from_utf8_lossy(data)).to_owned();
    }
    if is_html {
        if let Some(enc) = meta_charset(data) {
            if enc != UTF_8 {
                return decode_labelled(data, enc);
            }
        }
    }
    let guess = detect(data);
    guess.decode_with_bom_removal(data).0.into_owned()
}

fn decode_labelled(data: &[u8], enc: &'static Encoding) -> String {
    if enc == UTF_8 {
        return decode_unlabelled(data, false);
    }
    if enc == UTF_16LE || enc == UTF_16BE {
        return enc.decode(data).0.into_owned();
    }
    if enc == ISO_2022_JP && data.is_ascii() {
        return decode_iso_2022_jp(data).0;
    }
    if enc.is_single_byte() {
        if looks_like_iso_2022_jp(data) {
            let (text, had_errors) = decode_iso_2022_jp(data);
            if !had_errors {
                return text;
            }
        }
        if !data.is_ascii() {
            if let Ok(s) = std::str::from_utf8(data) {
                return strip_bom(s).to_owned();
            }
        }
    }
    let (text, had_errors) = enc.decode_with_bom_removal(data);
    if !had_errors {
        return text.into_owned();
    }
    // Pure ASCII under a stateful label (ISO-2022-JP with a stray escape) is still best
    // read with that label.
    if !data.is_ascii() {
        if let Ok(s) = std::str::from_utf8(data) {
            return strip_bom(s).to_owned();
        }
        if mostly_utf8(data) {
            return strip_bom(&String::from_utf8_lossy(data)).to_owned();
        }
    }
    let guess = detect(data);
    if guess != enc {
        let (alt, alt_errors) = guess.decode_with_bom_removal(data);
        if !alt_errors {
            return alt.into_owned();
        }
    }
    text.into_owned()
}

/// `ESC $ @`, `ESC $ B` or `ESC ( J`: the JIS X 0208 / JIS X 0201 designations.
fn looks_like_iso_2022_jp(data: &[u8]) -> bool {
    data.is_ascii()
        && data
            .windows(3)
            .any(|w| matches!(w, b"\x1b$@" | b"\x1b$B" | b"\x1b(J"))
}

/// ISO-2022-JP, with each line break returning to ASCII. RFC 1468 requires lines to end in
/// ASCII; senders that forget are read the way other decoders read them, where the WHATWG
/// decoder `encoding_rs` implements would report an error.
fn decode_iso_2022_jp(data: &[u8]) -> (String, bool) {
    let mut fixed = Vec::with_capacity(data.len() + 16);
    let mut ascii = true;
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b == 0x1b && i + 2 < data.len() {
            match (data[i + 1], data[i + 2]) {
                (b'(', b'B') | (b'(', b'J') => ascii = true,
                (b'$', b'@') | (b'$', b'B') | (b'(', b'I') => ascii = false,
                _ => {}
            }
        }
        if (b == b'\r' || b == b'\n') && !ascii {
            fixed.extend_from_slice(b"\x1b(B");
            ascii = true;
        }
        fixed.push(b);
        i += 1;
    }
    let (text, had_errors) = ISO_2022_JP.decode_without_bom_handling(&fixed);
    (text.into_owned(), had_errors)
}

/// UTF-8 with at most one broken sequence per four valid non-ASCII characters, as when a
/// line was cut in the middle of a character.
fn mostly_utf8(data: &[u8]) -> bool {
    let mut valid = 0usize;
    let mut invalid = 0usize;
    for chunk in data.utf8_chunks() {
        valid += chunk.valid().chars().filter(|c| !c.is_ascii()).count();
        if !chunk.invalid().is_empty() {
            invalid += 1;
        }
    }
    valid > 0 && invalid * 4 <= valid
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// The `chardetng` guess for bytes that are not valid UTF-8.
pub(crate) fn detect(data: &[u8]) -> &'static Encoding {
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
    detector.feed(data, true);
    detector.guess(None, Utf8Detection::Deny)
}

/// `normalizeCharset` plus the aliases real mail uses that the WHATWG label table lacks.
fn resolve_label(raw: &str) -> Option<Label> {
    let label = raw.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    let lower = label.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "utf-7" | "utf7" | "unicode-1-1-utf-7" | "x-unicode-2-0-utf-7" => return Some(Label::Utf7),
        "iso-2022-jp-2" | "iso-2022-jp-1" | "iso-2022-jp-3" => "iso-2022-jp",
        "iso-8859-8-e" => "iso-8859-8",
        "gb_2312-80" | "cp936" | "ms936" | "x-cp936" => "gbk",
        "cp949" | "ms949" | "uhc" | "x-windows-949" | "ks_c_5601" => "euc-kr",
        "cp932" | "ms932" | "windows-31j" | "x-sjis" | "sjis" | "shift-jis" | "x-ms-cp932" => {
            "shift_jis"
        }
        "cp950" | "ms950" | "big5-hkscs" | "x-x-big5" => "big5",
        "cp850" | "ibm850" | "cp437" | "ibm437" | "cp858" => "windows-1252",
        "iso8859-1" | "iso8859_1" | "8859-1" | "latin-1" => "iso-8859-1",
        "utf-8-sig" | "utf8mb4" | "unicode-1-1-utf-8" => "utf-8",
        "unicode" | "ucs-2" | "iso-10646-ucs-2" => "utf-16le",
        _ => lower.as_str(),
    };
    if let Some(stripped) = mapped.strip_prefix("cp").filter(|r| r.starts_with("125")) {
        return Encoding::for_label(format!("windows-{stripped}").as_bytes()).map(Label::Known);
    }
    if let Some(rest) = mapped.strip_prefix("iso8859-") {
        return Encoding::for_label(format!("iso-8859-{rest}").as_bytes()).map(Label::Known);
    }
    Encoding::for_label(mapped.as_bytes())
        // `koi8_r`, `euc_kr`, `iso_2022_jp`: Python's codec names leak into mail.
        .or_else(|| Encoding::for_label(mapped.replace('_', "-").as_bytes()))
        .map(Label::Known)
}

/// The charset of an HTML document's `<meta charset>` or `http-equiv` Content-Type, looked
/// for in the first 2 KB.
fn meta_charset(data: &[u8]) -> Option<&'static Encoding> {
    let head = &data[..data.len().min(2048)];
    let lower: Vec<u8> = head.iter().map(|b| b.to_ascii_lowercase()).collect();
    let mut from = 0;
    while let Some(pos) = find(&lower[from..], b"<meta") {
        let start = from + pos;
        let end = lower[start..]
            .iter()
            .position(|b| *b == b'>')
            .map_or(lower.len(), |p| start + p);
        let tag = &lower[start..end];
        if let Some(cs) = find(tag, b"charset=") {
            let value = &tag[cs + 8..];
            let value: Vec<u8> = value
                .iter()
                .copied()
                .skip_while(|b| *b == b'"' || *b == b'\'' || *b == b' ')
                .take_while(|b| !matches!(b, b'"' | b'\'' | b' ' | b';' | b'/' | b'>'))
                .collect();
            if let Ok(label) = std::str::from_utf8(&value) {
                if let Some(Label::Known(enc)) = resolve_label(label) {
                    return Some(enc);
                }
            }
        }
        from = end.max(start + 1);
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// UTF-7 (RFC 2152). `encoding_rs` has no UTF-7 decoder; Outlook Express still sends it.
pub(crate) fn decode_utf7(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        if c != b'+' {
            out.push(if c < 0x80 { c as char } else { '\u{fffd}' });
            i += 1;
            continue;
        }
        i += 1;
        if data.get(i) == Some(&b'-') {
            out.push('+');
            i += 1;
            continue;
        }
        let mut units: Vec<u16> = Vec::new();
        let mut acc: u32 = 0;
        let mut bits = 0u32;
        while i < data.len() {
            let v = match data[i] {
                b @ b'A'..=b'Z' => b - b'A',
                b @ b'a'..=b'z' => b - b'a' + 26,
                b @ b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => break,
            };
            acc = (acc << 6) | v as u32;
            bits += 6;
            if bits >= 16 {
                bits -= 16;
                units.push((acc >> bits) as u16);
                acc &= (1 << bits) - 1;
            }
            i += 1;
        }
        if data.get(i) == Some(&b'-') {
            i += 1;
        }
        out.extend(char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')));
    }
    out
}

/// Windows-1252 decoding, for bytes that must become text without any guessing.
pub(crate) fn latin1(data: &[u8]) -> String {
    WINDOWS_1252
        .decode_without_bom_handling(data)
        .0
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_charsets() {
        assert_eq!(decode_text(b"caf\xe9", Some("iso-8859-1"), false), "café");
        assert_eq!(
            decode_text(b"caf\xe9", Some("\"ISO-8859-1\""), false),
            "café"
        );
        assert_eq!(
            decode_text(b"\xcf\xf0\xe8", Some("windows-1251"), false),
            "При"
        );
        assert_eq!(decode_text(b"\xcf\xf0\xe8", Some("cp1251"), false), "При");
        assert_eq!(
            decode_text(b"\x93\xfa\x96\x7b", Some("shift_jis"), false),
            "日本"
        );
        assert_eq!(
            decode_text(b"\x93\xfa\x96\x7b", Some("cp932"), false),
            "日本"
        );
        assert_eq!(
            decode_text(b"\xc4\xe3\xba\xc3", Some("gb2312"), false),
            "你好"
        );
        assert_eq!(
            decode_text(b"\x1b$BF|K\\\x1b(B", Some("iso-2022-jp"), false),
            "日本"
        );
        assert_eq!(
            decode_text(b"\xc7\xd1", Some("ks_c_5601-1987"), false),
            "한"
        );
    }

    #[test]
    fn mislabelled_utf8_under_latin1_label() {
        assert_eq!(
            decode_text("café".as_bytes(), Some("iso-8859-1"), false),
            "café"
        );
        assert_eq!(
            decode_text("Grüße".as_bytes(), Some("us-ascii"), false),
            "Grüße"
        );
    }

    #[test]
    fn mislabelled_latin1_under_utf8_label() {
        let s = decode_text(b"Il \xe9tait une fois une for\xeat", Some("utf-8"), false);
        assert_eq!(s, "Il était une fois une forêt");
    }

    #[test]
    fn unlabelled() {
        assert_eq!(decode_text("ok ✓".as_bytes(), None, false), "ok ✓");
        assert_eq!(
            decode_text(
                b"Le na\xeff gar\xe7on a d\xe9j\xe0 mang\xe9 la cr\xe8me br\xfbl\xe9e",
                Some("x-unknown"),
                false
            ),
            "Le naïf garçon a déjà mangé la crème brûlée"
        );
        let html = b"<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1251\"></head><body>\xcf\xf0\xe8\xe2\xe5\xf2</body></html>";
        assert!(decode_text(html, None, true).contains("Привет"));
        let html5 = b"<meta charset='koi8-r'><p>\xf0\xd2\xc9\xd7\xc5\xd4</p>";
        assert!(decode_text(html5, None, true).contains("Привет"));
    }

    #[test]
    fn unlabelled_iso_2022_jp() {
        assert_eq!(decode_text(b"\x1b$BF|K\\\x1b(B", None, false), "日本");
        assert_eq!(
            decode_text(b"\x1b$BF|K\\\x1b(B", Some("us-ascii"), false),
            "日本"
        );
    }

    #[test]
    fn iso_2022_jp_line_without_reset() {
        assert_eq!(
            decode_text(b"\x1b$BF|K\\\r\nok", Some("iso-2022-jp"), false),
            "日本\r\nok"
        );
    }

    #[test]
    fn mostly_utf8_stays_utf8() {
        let mut data = "Здравствуйте, Елена! Спасибо за ваше сообщение."
            .as_bytes()
            .to_vec();
        data.push(0xd0);
        data.extend_from_slice(b" end");
        let s = decode_text(&data, Some("utf-8"), false);
        assert!(s.starts_with("Здравствуйте"), "{s}");
        assert!(s.ends_with("\u{fffd} end"), "{s}");
    }

    #[test]
    fn python_codec_names() {
        assert_eq!(decode_text(b"\xf0\xd2\xc9", Some("koi8_r"), false), "При");
    }

    #[test]
    fn utf8_bom_removed() {
        assert_eq!(decode_text(b"\xef\xbb\xbfhi", Some("utf-8"), false), "hi");
        assert_eq!(decode_text(b"\xef\xbb\xbfhi", None, false), "hi");
    }

    #[test]
    fn utf7() {
        assert_eq!(
            decode_text(b"Hi Mom -+Jjo--!", Some("utf-7"), false),
            "Hi Mom -☺-!"
        );
        assert_eq!(decode_text(b"A+ImIDkQ.", Some("UTF-7"), false), "A≢Α.");
        assert_eq!(decode_text(b"1 +- 1", Some("utf-7"), false), "1 + 1");
    }
}
