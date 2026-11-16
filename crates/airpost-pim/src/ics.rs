// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! A lossless line model of an iCalendar object (RFC 5545 section 3.1): components holding
//! properties and child components, each property's parameters and raw value kept as written,
//! so an edit changes only the properties it touches and a server's own properties survive.
//! Values are interpreted only where an edit needs them ([`crate::time`], [`unescape_text`]).

/// One parameter: an uppercased name and its values, unquoted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub values: Vec<String>,
}

/// One content line: `NAME;PARAM=v:value`, the name uppercased, the value as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub params: Vec<Param>,
    pub value: String,
}

impl Property {
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Property {
            name: name.to_ascii_uppercase(),
            params: Vec::new(),
            value: value.into(),
        }
    }

    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.set_param(name, value);
        self
    }

    /// The first value of parameter `name`.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .and_then(|p| p.values.first())
            .map(String::as_str)
    }

    pub fn has_param(&self, name: &str) -> bool {
        self.params
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Replaces parameter `name`, keeping its position when present.
    pub fn set_param(&mut self, name: &str, value: &str) {
        let name = name.to_ascii_uppercase();
        match self.params.iter_mut().find(|p| p.name == name) {
            Some(p) => p.values = vec![value.to_owned()],
            None => self.params.push(Param {
                name,
                values: vec![value.to_owned()],
            }),
        }
    }

    pub fn remove_param(&mut self, name: &str) {
        self.params.retain(|p| !p.name.eq_ignore_ascii_case(name));
    }

    /// The value split on unescaped commas (multi-valued properties: EXDATE, RDATE, CATEGORIES).
    pub fn values(&self) -> Vec<String> {
        split_unescaped(&self.value, ',')
    }

    /// The decoded text of a TEXT property (SUMMARY, DESCRIPTION, LOCATION, COMMENT).
    pub fn text(&self) -> String {
        unescape_text(&self.value)
    }

    fn write(&self, out: &mut String) {
        let mut line = self.name.clone();
        for p in &self.params {
            line.push(';');
            line.push_str(&p.name);
            line.push('=');
            let values: Vec<String> = p.values.iter().map(|v| quote_param(v)).collect();
            line.push_str(&values.join(","));
        }
        line.push(':');
        line.push_str(&self.value);
        fold(&line, out);
    }
}

/// A component: VCALENDAR, VEVENT, VTIMEZONE, STANDARD, VALARM…
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub props: Vec<Property>,
    pub children: Vec<Component>,
}

impl Component {
    pub fn new(name: &str) -> Self {
        Component {
            name: name.to_ascii_uppercase(),
            props: Vec::new(),
            children: Vec::new(),
        }
    }

    /// A VCALENDAR with Airpost's PRODID and VERSION 2.0.
    pub fn new_calendar() -> Self {
        let mut c = Component::new("VCALENDAR");
        c.props
            .push(Property::new("PRODID", "-//Airpost//Calendar//EN"));
        c.props.push(Property::new("VERSION", "2.0"));
        c
    }

    pub fn prop(&self, name: &str) -> Option<&Property> {
        self.props
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn prop_mut(&mut self, name: &str) -> Option<&mut Property> {
        self.props
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn props_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> + 'a {
        self.props
            .iter()
            .filter(move |p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn has(&self, name: &str) -> bool {
        self.prop(name).is_some()
    }

    /// The raw value of the first `name` property.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.prop(name).map(|p| p.value.as_str())
    }

    /// The decoded text of the first `name` property, `""` when absent.
    pub fn text(&self, name: &str) -> String {
        self.prop(name).map(Property::text).unwrap_or_default()
    }

    pub fn remove_all(&mut self, name: &str) {
        self.props.retain(|p| !p.name.eq_ignore_ascii_case(name));
    }

    /// Replaces the first `name` property's value (parameters kept), adding the property at
    /// the end when missing; any further `name` properties go.
    pub fn set_value(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        let mut seen = false;
        self.props.retain_mut(|p| {
            if !p.name.eq_ignore_ascii_case(name) {
                return true;
            }
            if seen {
                return false;
            }
            seen = true;
            p.value = value.clone();
            true
        });
        if !seen {
            self.props.push(Property::new(name, value));
        }
    }

    /// Replaces every `name` property with `prop`, at the first one's position.
    pub fn replace(&mut self, prop: Property) {
        match self
            .props
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(&prop.name))
        {
            Some(i) => {
                let name = prop.name.clone();
                self.props[i] = prop;
                let mut index = 0;
                self.props.retain(|p| {
                    let keep = index == i || !p.name.eq_ignore_ascii_case(&name);
                    index += 1;
                    keep
                });
            }
            None => self.props.push(prop),
        }
    }

    /// Sets a TEXT property to `text`, escaped.
    pub fn set_text(&mut self, name: &str, text: &str) {
        self.set_value(name, escape_text(text));
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> + 'a {
        self.children
            .iter()
            .filter(move |c| c.name.eq_ignore_ascii_case(name))
    }

    /// The text form with CRLF line ends, lines folded at 75 octets.
    pub fn to_ics(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        fold(&format!("BEGIN:{}", self.name), out);
        for p in &self.props {
            p.write(out);
        }
        for c in &self.children {
            c.write(out);
        }
        fold(&format!("END:{}", self.name), out);
    }
}

/// Why a text is not an iCalendar object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parses the first top-level component (a VCALENDAR, or a bare VEVENT). Line ends may be
/// CRLF or LF; folded lines are unfolded; a component left open at the end is closed.
pub fn parse(ics: &str) -> Result<Component, ParseError> {
    let mut stack: Vec<Component> = Vec::new();
    let mut root: Option<Component> = None;
    for line in unfold(ics) {
        if line.trim().is_empty() {
            continue;
        }
        let prop = parse_line(&line);
        if prop.name == "BEGIN" {
            stack.push(Component::new(prop.value.trim()));
            continue;
        }
        if prop.name == "END" {
            let Some(done) = stack.pop() else { continue };
            match stack.last_mut() {
                Some(parent) => parent.children.push(done),
                None => {
                    root = Some(done);
                    break;
                }
            }
            continue;
        }
        if let Some(current) = stack.last_mut() {
            current.props.push(prop);
        }
    }
    while let Some(done) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(done),
            None => root = Some(done),
        }
    }
    root.ok_or_else(|| ParseError("no BEGIN:VCALENDAR or BEGIN:VEVENT".into()))
}

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_owned());
    }
    lines
}

fn parse_line(line: &str) -> Property {
    let mut quoted = false;
    let mut colon = None;
    for (i, b) in line.bytes().enumerate() {
        match b {
            b'"' => quoted = !quoted,
            b':' if !quoted => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let (head, value) = match colon {
        Some(i) => (&line[..i], &line[i + 1..]),
        None => (line, ""),
    };
    let mut parts = split_quoted(head, ';').into_iter();
    let name = parts.next().unwrap_or_default().trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| {
            let (n, v) = p.split_once('=')?;
            Some(Param {
                name: n.trim().to_ascii_uppercase(),
                values: split_quoted(v, ',')
                    .into_iter()
                    .map(|v| v.trim_matches('"').to_owned())
                    .collect(),
            })
        })
        .collect();
    Property {
        name,
        params,
        value: value.to_owned(),
    }
}

fn split_quoted(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c == sep && !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn split_unescaped(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
        } else if c == '\\' {
            cur.push(c);
            escaped = true;
        } else if c == sep {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn quote_param(v: &str) -> String {
    if v.contains([':', ';', ',']) {
        format!("\"{}\"", v.replace('"', ""))
    } else {
        v.to_owned()
    }
}

/// Folds a line at 75 octets, never inside a UTF-8 sequence (RFC 5545 section 3.1).
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
}

/// TEXT escaping (RFC 5545 section 3.3.11).
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let normalized = text.replace("\r\n", "\n");
    for c in normalized.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

pub fn unescape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_folds() {
        let long = "x".repeat(100);
        let ics = format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:a\nATTENDEE;CN=\"Doe, Jane\";PARTSTAT=ACCEPTED:mailto:j@x\nDESCRIPTION:{long}\nX-FOO;X-P=a,b:v\nEND:VEVENT\nEND:VCALENDAR"
        );
        let cal = parse(&ics).unwrap();
        let vevent = &cal.children[0];
        let att = vevent.prop("ATTENDEE").unwrap();
        assert_eq!(att.param("CN"), Some("Doe, Jane"));
        assert_eq!(att.value, "mailto:j@x");
        assert_eq!(vevent.prop("X-FOO").unwrap().params[0].values, ["a", "b"]);
        let out = cal.to_ics();
        assert!(out.lines().all(|l| l.len() <= 76));
        assert!(out.contains("ATTENDEE;CN=\"Doe, Jane\";PARTSTAT=ACCEPTED:mailto:j@x\r\n"));
        assert_eq!(parse(&out).unwrap(), cal);
    }

    #[test]
    fn text_escapes() {
        let t = "a, b; c\\d\nnext";
        assert_eq!(escape_text(t), "a\\, b\\; c\\\\d\\nnext");
        assert_eq!(unescape_text(&escape_text(t)), t);
    }

    #[test]
    fn multibyte_fold_stays_valid() {
        let mut out = String::new();
        fold(&"é".repeat(60), &mut out);
        let joined = out.replace("\r\n ", "");
        assert_eq!(joined.trim_end(), "é".repeat(60));
    }
}
