// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Writing results: pipe-safe stdout, tables fitted to the terminal, ANSI styling only on a
//! terminal without `NO_COLOR`, and the small text helpers the tables use.

use std::io::{IsTerminal, Write};

use chrono::{DateTime, Datelike, Local, TimeZone};
use serde_json::Value;

/// Writes a line to stdout. A closed pipe (`tern threads | head`) ends the process quietly
/// with status 0 instead of panicking.
pub fn line(s: &str) {
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{s}").and_then(|()| out.flush()).is_err() {
        std::process::exit(0);
    }
}

/// [`line`] without the newline, for text that streams in.
pub fn text(s: &str) {
    let mut out = std::io::stdout().lock();
    if write!(out, "{s}").and_then(|()| out.flush()).is_err() {
        std::process::exit(0);
    }
}

/// One JSON document (or NDJSON record) on stdout.
pub fn json(v: &Value) {
    line(&v.to_string());
}

pub fn warn(msg: &str) {
    let _ = writeln!(std::io::stderr(), "tern: warning: {msg}");
}

/// How human output looks: colour, and the width to fit.
#[derive(Clone, Copy, Debug, Default)]
pub struct Style {
    pub color: bool,
    /// None: don't truncate (stdout isn't a terminal).
    pub width: Option<usize>,
}

impl Style {
    pub fn detect() -> Self {
        let tty = std::io::stdout().is_terminal();
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        // Piped output is never cut; COLUMNS only stands in when the terminal won't say.
        let width = tty
            .then(|| {
                crate::platform::terminal_width()
                    .or_else(|| std::env::var("COLUMNS").ok().and_then(|c| c.parse().ok()))
            })
            .flatten();
        Style {
            color: tty && !no_color,
            width,
        }
    }

    pub fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }

    pub fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }

    pub fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }

    fn paint(&self, sgr: &str, s: &str) -> String {
        if self.color && !s.is_empty() {
            format!("\x1b[{sgr}m{s}\x1b[0m")
        } else {
            s.to_owned()
        }
    }
}

/// Display width, counted in chars. Wide (CJK, emoji) characters count as one, so a
/// truncated line holding them can run past the terminal.
pub fn width(s: &str) -> usize {
    s.chars().count()
}

/// `s` cut to `max` columns, ending in `…` when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Control characters and line breaks as spaces, so one field stays on one line.
pub fn one_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() || c.is_control() {
            if !space && !out.is_empty() {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out.trim_end().to_owned()
}

/// A table whose `flex` column gives way when the rows are wider than the terminal. Cells
/// hold plain text; styling is applied after the layout, so escapes never count as width.
pub struct Table {
    headers: Vec<&'static str>,
    rows: Vec<Row>,
    flex: usize,
}

struct Row {
    cells: Vec<String>,
    strong: bool,
    /// Columns shown in red (a problem).
    alerts: Vec<usize>,
}

const GAP: &str = "  ";
const FLEX_MIN: usize = 12;

impl Table {
    pub fn new(headers: Vec<&'static str>, flex: usize) -> Self {
        Table {
            headers,
            rows: Vec::new(),
            flex,
        }
    }

    /// A row; `strong` rows are bold (unread threads).
    pub fn row(&mut self, cells: Vec<String>, strong: bool) {
        self.rows.push(Row {
            cells: cells.into_iter().map(|c| one_line(&c)).collect(),
            strong,
            alerts: Vec::new(),
        });
    }

    /// Shows column `col` of the last row in red.
    pub fn alert(&mut self, col: usize) {
        if let Some(r) = self.rows.last_mut() {
            r.alerts.push(col);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The lines to print: a header, then the rows. The last column isn't padded.
    pub fn render(&self, style: &Style) -> Vec<String> {
        let n = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|h| width(h)).collect();
        for r in &self.rows {
            for (w, c) in widths.iter_mut().zip(&r.cells) {
                *w = (*w).max(width(c));
            }
        }
        if let Some(total) = style.width {
            let used: usize = widths.iter().sum::<usize>() + GAP.len() * n.saturating_sub(1);
            if used > total && self.flex < n {
                let others = used - widths[self.flex];
                widths[self.flex] = total
                    .saturating_sub(others)
                    .max(FLEX_MIN.min(widths[self.flex]));
            }
        }
        let fmt = |cells: &[String], alerts: &[usize]| -> String {
            let mut out = String::new();
            for (i, w) in widths.iter().enumerate() {
                let cell = truncate(cells.get(i).map_or("", String::as_str), *w);
                let pad = w - width(&cell);
                if alerts.contains(&i) {
                    out.push_str(&style.red(&cell));
                } else {
                    out.push_str(&cell);
                }
                if i + 1 < n {
                    out.push_str(&" ".repeat(pad));
                    out.push_str(GAP);
                }
            }
            out.trim_end().to_owned()
        };
        let headers: Vec<String> = self.headers.iter().map(|h| (*h).to_owned()).collect();
        let mut lines = vec![style.bold(&fmt(&headers, &[]))];
        for r in &self.rows {
            let l = fmt(&r.cells, &r.alerts);
            lines.push(if r.strong { style.bold(&l) } else { l });
        }
        lines
    }

    pub fn print(&self, style: &Style) {
        for l in self.render(style) {
            line(&l);
        }
    }
}

/// A unix timestamp as a list shows it: the time today, the day this year, else the date.
pub fn short_date<Tz: TimeZone>(ts: i64, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Some(t) = now.timezone().timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    if ts <= 0 {
        return String::new();
    }
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if t.year() == now.year() {
        t.format("%b %d").to_string()
    } else {
        t.format("%Y-%m-%d").to_string()
    }
}

pub fn list_date(ts: i64) -> String {
    short_date(ts, &Local::now())
}

pub fn full_date(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M %Z").to_string())
        .unwrap_or_default()
}

pub fn clock_time() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

/// `Name <email>`, or whichever of the two a contact JSON has.
pub fn contact(v: &Value) -> String {
    let name = v["name"].as_str().unwrap_or("").trim();
    let email = v["email"].as_str().unwrap_or("").trim();
    match (name.is_empty(), email.is_empty()) {
        (false, false) if name != email => format!("{name} <{email}>"),
        (false, _) => name.to_owned(),
        _ => email.to_owned(),
    }
}

/// The name of a contact, else its address.
pub fn contact_short(v: &Value) -> String {
    let name = v["name"].as_str().unwrap_or("").trim();
    if name.is_empty() {
        v["email"].as_str().unwrap_or("").trim().to_owned()
    } else {
        name.to_owned()
    }
}

pub fn contacts(list: &Value) -> String {
    list.as_array()
        .map(|a| a.iter().map(contact).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

pub fn uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {}s", secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// A stored body as text. Bodies are HTML, except a plain-text part the engine kept as is.
pub fn body_text(body: &str) -> String {
    const TAGS: [&str; 10] = [
        "<html",
        "<body",
        "<div",
        "<p>",
        "<p ",
        "<br",
        "<span",
        "<table",
        "<blockquote",
        "<a ",
    ];
    let lower = body.to_ascii_lowercase();
    if TAGS.iter().any(|t| lower.contains(t)) {
        html_to_text(body)
    } else {
        body.replace("\r\n", "\n").trim_end().to_owned()
    }
}

/// Readable text from an HTML body: tags dropped, block ends as line breaks, the common
/// entities decoded, `<style>`/`<script>`/`<head>` content left out. Not a renderer; for a
/// faithful view open the message in Tern.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut skip_until: Option<&str> = None;
    while let Some(lt) = rest.find('<') {
        if skip_until.is_none() {
            out.push_str(&decode_entities(&rest[..lt]));
        }
        let Some(gt) = rest[lt..].find('>') else {
            if skip_until.is_none() {
                out.push_str(&decode_entities(&rest[lt..]));
            }
            rest = "";
            break;
        };
        let tag = rest[lt + 1..lt + gt].trim();
        rest = &rest[lt + gt + 1..];
        if tag.starts_with("!--") && !tag.ends_with("--") {
            match rest.find("-->") {
                Some(end) => rest = &rest[end + 3..],
                None => rest = "",
            }
            continue;
        }
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if let Some(until) = skip_until {
            if closing && name == until {
                skip_until = None;
            }
            continue;
        }
        match name.as_str() {
            "style" | "script" | "head" | "title" if !closing => {
                skip_until = Some(match name.as_str() {
                    "style" => "style",
                    "script" => "script",
                    "title" => "title",
                    _ => "head",
                })
            }
            "br" => out.push('\n'),
            "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote"
            | "table" | "ul" | "ol" | "hr" => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                if name == "li" && !closing {
                    out.push_str("- ");
                }
            }
            "td" | "th" if closing => out.push(' '),
            _ => {}
        }
    }
    if skip_until.is_none() {
        out.push_str(&decode_entities(rest));
    }
    tidy_lines(&out)
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return collapse_spaces(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let end = rest[1..].find(';').map(|i| i + 1).filter(|&i| i <= 10);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ if name.starts_with("#x") || name.starts_with("#X") => {
                    u32::from_str_radix(&name[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if name.starts_with('#') => name[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    collapse_spaces(&out)
}

/// HTML whitespace: runs of spaces and newlines in text are one space.
fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(if c == '\u{a0}' { ' ' } else { c });
            prev_space = false;
        }
    }
    out
}

/// Trims each line and keeps at most one blank line in a row.
fn tidy_lines(s: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for l in s.lines().map(str::trim) {
        if l.is_empty() && out.last().is_none_or(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use serde_json::json;

    #[test]
    fn truncates_with_an_ellipsis() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("añbcdé", 3), "añ…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(one_line("  a\r\n\tb  c \n"), "a b c");
    }

    #[test]
    fn table_fits_the_width_by_shrinking_the_flex_column() {
        let mut t = Table::new(vec!["DATE", "SUBJECT", "ID"], 1);
        t.row(
            vec![
                "09:00".into(),
                "A subject that is far too long".into(),
                "t1".into(),
            ],
            false,
        );
        t.row(vec!["Oct 01".into(), "Short".into(), "t22".into()], true);
        let plain = Style::default();
        let wide = t.render(&plain);
        assert_eq!(wide[0], "DATE    SUBJECT                         ID");
        assert_eq!(wide[1], "09:00   A subject that is far too long  t1");
        assert_eq!(wide[2], "Oct 01  Short                           t22");

        let narrow = Style {
            color: false,
            width: Some(30),
        };
        let lines = t.render(&narrow);
        assert!(lines.iter().all(|l| width(l) <= 30), "{lines:#?}");
        assert_eq!(lines[1], "09:00   A subject that i…  t1");
        // Never below the minimum, even when that overflows.
        let tiny = Style {
            color: false,
            width: Some(5),
        };
        assert!(t.render(&tiny)[1].contains("A subject t"));

        let colored = Style {
            color: true,
            width: None,
        };
        let lines = t.render(&colored);
        assert!(lines[0].starts_with("\x1b[1mDATE"));
        assert!(!lines[1].contains('\x1b'));
        assert!(lines[2].starts_with("\x1b[1m"));
    }

    #[test]
    fn alert_cells_are_painted_after_padding() {
        let mut t = Table::new(vec!["SERVER", "PID"], 0);
        t.row(vec!["offline".into(), "1".into()], false);
        t.alert(0);
        t.row(vec!["ok".into(), "2".into()], false);
        let plain = t.render(&Style::default());
        assert_eq!(plain[1..], ["offline  1", "ok       2"]);
        let colored = Style {
            color: true,
            width: Some(10),
        };
        let lines = t.render(&colored);
        assert_eq!(lines[1], "\x1b[31moffline\x1b[0m  1");
        assert_eq!(lines[2], "ok       2");
    }

    #[test]
    fn dates_shorten_by_age() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let now = tz.with_ymd_and_hms(2026, 10, 16, 15, 30, 0).unwrap();
        let ts = |y, m, d, h, mi| tz.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp();
        assert_eq!(short_date(ts(2026, 10, 16, 9, 5), &now), "09:05");
        assert_eq!(short_date(ts(2026, 3, 2, 9, 5), &now), "Mar 02");
        assert_eq!(short_date(ts(2025, 12, 31, 23, 0), &now), "2025-12-31");
        assert_eq!(short_date(0, &now), "");
    }

    #[test]
    fn contacts_and_sizes() {
        assert_eq!(
            contact(&json!({"name": "Ann", "email": "a@x"})),
            "Ann <a@x>"
        );
        assert_eq!(contact(&json!({"name": "", "email": "a@x"})), "a@x");
        assert_eq!(contact(&json!({"name": "a@x", "email": "a@x"})), "a@x");
        assert_eq!(
            contact_short(&json!({"name": "Ann", "email": "a@x"})),
            "Ann"
        );
        assert_eq!(
            contacts(&json!([{"email": "a@x"}, {"name": "B", "email": "b@x"}])),
            "a@x, B <b@x>"
        );
        assert_eq!(bytes(14), "14 B");
        assert_eq!(bytes(2048), "2.0 KB");
        assert_eq!(uptime(59), "59s");
        assert_eq!(uptime(3_700), "1h 1m");
        assert_eq!(uptime(90_000), "1d 1h");
    }

    #[test]
    fn html_becomes_text() {
        let html = "<html><head><style>p{color:red}</style><title>T</title></head><body>\
                    <p>Hello&nbsp;<b>there</b> &amp; welcome</p><!-- note --><div>Line\n  two<br>three</div>\
                    <ul><li>one</li><li>two &#233; &#x41;</li></ul><script>x()</script>&bogus; end</body></html>";
        assert_eq!(
            html_to_text(html),
            "Hello there & welcome\nLine two\nthree\n- one\n- two é A\n&bogus; end"
        );
        assert_eq!(html_to_text("plain text"), "plain text");
        assert_eq!(body_text("one\r\n\r\n-- \r\nAnn\r\n"), "one\n\n-- \nAnn");
        assert_eq!(body_text("a<br/>b<br/>"), "a\nb");
        assert_eq!(html_to_text("a < b"), "a < b");
    }
}
