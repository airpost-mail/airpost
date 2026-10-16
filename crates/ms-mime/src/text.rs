// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! String helpers from mailcore2's `MCString.cpp` (Copyright © 2001-2013 MailCore team,
//! BSD-3-Clause; see NOTICE.md, "mailcore2"): `stripWhitespace`, `htmlEncodedString`,
//! `htmlMessageContent` and `substringToIndex`.

/// mailcore's `isWhitespace` set.
pub(crate) fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '\u{c}' | '\r' | '\u{a0}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `String::stripWhitespace`: every whitespace character becomes a space, runs of spaces
/// collapse to one, and leading and trailing spaces go.
pub fn strip_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if is_whitespace(c) {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    out
}

/// `htmlEncodedString`: `&`, `<` and `>` escaped, line breaks as `<br/>`. libxml2 also
/// turned non-ASCII characters into entities; they are kept as UTF-8 here, which renders
/// the same.
pub(crate) fn html_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("<br/>"),
            _ => out.push(c),
        }
    }
    out
}

/// `String::htmlMessageContent`: plain text as HTML. Lines starting with `>` are gathered
/// into `<blockquote type="cite">` (recursively, so nested quotes nest), other lines are
/// escaped and end with `<br/>`, and trailing empty lines are dropped.
///
/// Note: mailcore split on `\n` and kept each line's `\r`, so a CRLF line is not empty for
/// the trailing-line rule; that is kept, but the `\r` itself is not copied into the HTML.
///
/// mailcore recursed once per quote level, so a line of tens of thousands of `>` overflowed
/// the stack. This walks the lines once and produces the same output: each recursion level
/// `d` sees the lines of quote depth `>= d`, and its trailing-line rule drops a depth-`d`
/// empty line when only depth-`d` empty lines follow it before the block closes.
pub(crate) fn html_message_content(s: &str) -> String {
    let lines: Vec<(usize, &str)> = s.split('\n').map(quote_depth).collect();

    let mut dropped = vec![false; lines.len()];
    for i in (0..lines.len()).rev() {
        let (depth, content) = lines[i];
        if !content.is_empty() {
            continue;
        }
        dropped[i] = match lines.get(i + 1) {
            None => true,
            Some(&(next_depth, next)) => {
                next_depth < depth || (next_depth == depth && next.is_empty() && dropped[i + 1])
            }
        };
    }

    let mut out = String::with_capacity(s.len() + s.len() / 4);
    let mut open = 0;
    for (&(depth, content), &drop) in lines.iter().zip(&dropped) {
        // Dropped lines still open and close quote levels: the levels above saw them as
        // non-empty lines.
        while open > depth {
            out.push_str("</blockquote>");
            open -= 1;
        }
        while open < depth {
            out.push_str("<blockquote type=\"cite\">");
            open += 1;
        }
        if !drop {
            out.push_str(&html_encode(content.strip_suffix('\r').unwrap_or(content)));
            out.push_str("<br/>");
        }
    }
    for _ in 0..open {
        out.push_str("</blockquote>");
    }
    out
}

/// A line's quote depth and its text at that depth: each level strips one `>` and then the
/// spaces after it, as each mailcore recursion level did.
fn quote_depth(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line;
    while let Some(r) = rest.strip_prefix('>') {
        rest = r.trim_start_matches(' ');
        depth += 1;
    }
    (depth, rest)
}

/// `substringToIndex` on UTF-16 code units, as the snippet and search lengths are counted.
/// A surrogate pair straddling the limit is dropped rather than split.
pub fn truncate_utf16(s: &str, units: usize) -> &str {
    let mut count = 0;
    for (idx, c) in s.char_indices() {
        let w = c.len_utf16();
        if count + w > units {
            return &s[..idx];
        }
        count += w;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_whitespace() {
        assert_eq!(strip_whitespace("  a \t\r\n b\u{a0}\u{a0}c  "), "a b c");
        assert_eq!(strip_whitespace("\n\n"), "");
        assert_eq!(strip_whitespace("x"), "x");
    }

    #[test]
    fn plain_to_html() {
        assert_eq!(
            html_message_content("a < b & c\nnext"),
            "a &lt; b &amp; c<br/>next<br/>"
        );
        assert_eq!(html_message_content("hi\n\n\n"), "hi<br/>");
        assert_eq!(
            html_message_content("reply\n> quoted\n>  more\nafter"),
            "reply<br/><blockquote type=\"cite\">quoted<br/>more<br/></blockquote>after<br/>"
        );
        assert_eq!(
            html_message_content("> a\n>> b\n"),
            "<blockquote type=\"cite\">a<br/><blockquote type=\"cite\">b<br/></blockquote></blockquote>"
        );
        // CRLF: the "\r" line is not empty, so it keeps its <br/>
        assert_eq!(html_message_content("hi\r\n\r\n"), "hi<br/><br/>");
    }

    /// The recursive mailcore version, as reference for the iterative one.
    fn reference(s: &str) -> String {
        let mut lines: Vec<&str> = s.split('\n').collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let mut out = String::new();
        let mut quoted: Option<String> = None;
        let flush = |out: &mut String, q: &str| {
            out.push_str("<blockquote type=\"cite\">");
            out.push_str(&reference(q));
            out.push_str("</blockquote>");
        };
        for line in lines {
            if let Some(rest) = line.strip_prefix('>') {
                let q = quoted.get_or_insert_with(String::new);
                q.push_str(rest.trim_start_matches(' '));
                q.push('\n');
                continue;
            }
            if let Some(q) = quoted.take() {
                flush(&mut out, &q);
            }
            out.push_str(&html_encode(line.strip_suffix('\r').unwrap_or(line)));
            out.push_str("<br/>");
        }
        if let Some(q) = quoted.take() {
            flush(&mut out, &q);
        }
        out
    }

    #[test]
    fn plain_to_html_matches_recursive_version() {
        // every string up to 8 characters over an alphabet that hits each rule
        let alphabet = ['>', ' ', 'a', '\n', '\r'];
        let mut buf = String::new();
        fn walk(buf: &mut String, alphabet: &[char], left: usize) {
            assert_eq!(html_message_content(buf), reference(buf), "input {buf:?}");
            if left == 0 {
                return;
            }
            for &c in alphabet {
                buf.push(c);
                walk(buf, alphabet, left - 1);
                buf.pop();
            }
        }
        walk(&mut buf, &alphabet, 8);
    }

    #[test]
    fn deep_quotes_do_not_recurse() {
        let line = ">".repeat(200_000);
        let html = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || html_message_content(&format!("{line}x\n")))
            .unwrap()
            .join()
            .unwrap();
        assert!(html.starts_with("<blockquote type=\"cite\"><blockquote"));
        assert_eq!(html.matches("</blockquote>").count(), 200_000);
    }

    #[test]
    fn utf16_truncation() {
        assert_eq!(truncate_utf16("abcdef", 3), "abc");
        assert_eq!(truncate_utf16("ab", 3), "ab");
        assert_eq!(truncate_utf16("a😀b", 2), "a");
        assert_eq!(truncate_utf16("a😀b", 3), "a😀");
        assert_eq!(truncate_utf16("éé", 1), "é");
    }
}
