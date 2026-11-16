// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The plaintext composer's helpers (`composer-editor/plaintext.ts`): quote deepening, the
//! 72-column soft wrap with its zero-width-space markers, and plaintext shown as HTML. HTML to
//! plaintext uses the engine's own flattener, where the desktop goes through its editor
//! (`convertToPlainText(convertFromHTML(html))`).

pub const ZERO_WIDTH_SPACE: char = '\u{200B}';
/// A newline the wrap inserted, which the next wrap may remove.
pub const SOFT_NEWLINE: &str = "\n\u{200B}";
pub const DEFAULT_LINE_WIDTH: usize = 72;

/// `deepenPlaintextQuote`: one more `>` on every line, with a space after the last.
pub fn deepen_plaintext_quote(text: &str) -> String {
    // `\n${text}`.replace(/(\n>*) ?/g, '$1> ')
    let input = format!("\n{text}");
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\n' {
            out.push('\n');
            i += 1;
            while i < chars.len() && chars[i] == '>' {
                out.push('>');
                i += 1;
            }
            if i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            out.push_str("> ");
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `wrapPlaintext`: lines longer than `line_width` (UTF-16 units, as in the desktop) are
/// broken at spaces with [`SOFT_NEWLINE`], keeping a `> ` quote prefix; soft newlines from an
/// earlier wrap are undone first.
pub fn wrap_plaintext(text: &str, line_width: usize) -> String {
    let value: Vec<char> = text.chars().collect();
    let mut result = String::new();
    let mut line = String::new(); // the result's current line
    let mut word = String::new();

    let flush_word = |result: &mut String, line: &mut String, word: &mut String| {
        // A word too long for a line with nothing on it but a quote prefix stays there:
        // wrapping it would leave a blank line above it and drop the space before it.
        let line_has_text = line.chars().any(|c| c != '>' && !c.is_whitespace());
        if line_has_text && js_len(line) + js_len(word) > line_width {
            let quote_prefix = if word.starts_with('>') {
                String::new()
            } else {
                let gts = line.chars().take_while(|&c| c == '>').count();
                if gts > 0 && line.chars().nth(gts) == Some(' ') {
                    format!("{} ", ">".repeat(gts))
                } else {
                    String::new()
                }
            };
            let new_line = format!("{quote_prefix}{}", word.trim());
            result.push_str(SOFT_NEWLINE);
            result.push_str(&new_line);
            *line = new_line;
        } else {
            result.push_str(word);
            line.push_str(word);
        }
        word.clear();
    };

    let mut i = 0;
    while i < value.len() {
        let mut c = value[i];
        if c == '\n' && value.get(i + 1) == Some(&ZERO_WIDTH_SPACE) {
            c = ' ';
        }
        if c == ZERO_WIDTH_SPACE {
            while value.get(i + 1) == Some(&'>') {
                i += 1;
            }
            if value.get(i + 1) == Some(&' ') {
                i += 1;
            }
        } else if c == '\n' {
            result.push_str(&word);
            result.push('\n');
            line.clear();
            word.clear();
        } else if c == ' ' {
            flush_word(&mut result, &mut line, &mut word);
            word.push(c);
        } else {
            word.push(c);
        }
        i += 1;
    }
    flush_word(&mut result, &mut line, &mut word);
    result
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{A0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
    out
}

/// `convertPlaintextToHTML`: a `pre-wrap` div with the text escaped and newlines as `<br>`,
/// as `innerText` writes it.
pub fn plaintext_to_html(plain: &str) -> String {
    let lines: Vec<String> = plain
        .replace("\r\n", "\n")
        .split(['\n', '\r'])
        .map(escape_text)
        .collect();
    format!(
        "<div style=\"white-space: pre-wrap;\">{}</div>",
        lines.join("<br>")
    )
}

/// HTML as plaintext for a plaintext reply: the engine's flattener (blockquotes `> `-prefixed,
/// links without their URL), trimmed.
pub fn html_to_plaintext(html: &str) -> String {
    let text = airpost_mime::flatten_html_with(html, true, false);
    // The flattener ends paragraphs with a blank line; the editor's conversion separates them
    // with one.
    text.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepens_quotes() {
        assert_eq!(deepen_plaintext_quote("a\n> b\n>> c"), "\n> a\n>> b\n>>> c");
    }

    #[test]
    fn wraps_and_rewraps() {
        let text = format!("{}dolor", "lorem ipsum ".repeat(10));
        let wrapped = wrap_plaintext(&text, DEFAULT_LINE_WIDTH);
        assert_eq!(
            wrapped,
            "lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem ipsum\n\u{200b}lorem ipsum lorem ipsum lorem ipsum lorem ipsum dolor"
        );
        assert_eq!(wrap_plaintext(&wrapped, DEFAULT_LINE_WIDTH), wrapped);
        assert_eq!(wrap_plaintext("short\nlines", 72), "short\nlines");
    }

    #[test]
    fn plaintext_as_html() {
        assert_eq!(
            plaintext_to_html("a <b>\nc & d"),
            "<div style=\"white-space: pre-wrap;\">a &lt;b&gt;<br>c &amp; d</div>"
        );
    }
}
