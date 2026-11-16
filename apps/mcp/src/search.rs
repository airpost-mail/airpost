// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `search_mail` on an engine without the search grammar (feature `search-grammar`, which
//! parses the query itself): the query language on top of the plain-term `search`. Free text,
//! `from:`, `to:` and `subject:` values become search terms (the engine matches each term
//! anywhere in the thread's indexed text, so the field isn't enforced); `in:`, `is:`,
//! `has:attachment` and `before:`/`after:`/`since:` with exact dates filter the results.
//! Adjacent terms are ANDed; `AND` is accepted and ignored. `OR`, `NOT`, parentheses and
//! relative dates are refused, because the engine's search can't express them yet.

use chrono::NaiveDate;
use serde_json::Value;

use crate::model::{bool_of, i64_of};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Terms for the engine's `search`.
    pub terms: Vec<String>,
    /// `in:` values: a role or a folder/label name or path.
    pub folders: Vec<String>,
    pub unread: Option<bool>,
    pub starred: Option<bool>,
    pub has_attachment: bool,
    /// Epoch seconds: last message strictly before / at or after.
    pub before: Option<i64>,
    pub after: Option<i64>,
}

/// Splits on whitespace outside double quotes; quotes are removed.
fn tokens(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in q.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_date(s: &str) -> Result<i64, String> {
    for fmt in ["%Y/%m/%d", "%Y-%m-%d", "%Y.%m.%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Ok(d
                .and_hms_opt(0, 0, 0)
                .expect("midnight")
                .and_utc()
                .timestamp());
        }
    }
    Err(format!(
        "'{s}' is not a date this search understands: use YYYY/MM/DD or YYYY-MM-DD"
    ))
}

pub fn parse(q: &str) -> Result<Query, String> {
    let mut out = Query::default();
    for raw in tokens(q) {
        let tok = raw.trim_matches(|c| c == '(' || c == ')');
        if tok.len() != raw.len() || raw.starts_with('-') {
            return Err(
                "parentheses and negation aren't supported by Airpost's search yet; use plain terms \
                 and the in:/is:/has:/before:/after: filters"
                    .into(),
            );
        }
        match tok {
            "" | "AND" => continue,
            "OR" | "NOT" => {
                return Err(format!(
                    "{tok} isn't supported by Airpost's search yet: every term must match"
                ))
            }
            _ => {}
        }
        let Some((key, value)) = tok
            .split_once(':')
            .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        else {
            out.terms.push(tok.to_owned());
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "from" | "to" | "subject" => out.terms.push(value.to_owned()),
            "in" => out.folders.push(value.to_owned()),
            "is" => match value.to_ascii_lowercase().as_str() {
                "unread" => out.unread = Some(true),
                "read" => out.unread = Some(false),
                "starred" => out.starred = Some(true),
                "unstarred" => out.starred = Some(false),
                other => return Err(format!("unknown filter is:{other}")),
            },
            "has" if value.eq_ignore_ascii_case("attachment") => out.has_attachment = true,
            "before" => out.before = Some(parse_date(value)?),
            "after" | "since" => out.after = Some(parse_date(value)?),
            _ => out.terms.push(tok.to_owned()),
        }
    }
    Ok(out)
}

impl Query {
    /// The filters other than `in:`, which the caller resolves to category ids.
    pub fn matches(&self, thread: &Value) -> bool {
        let ts = i64_of(thread, "lmrt").max(i64_of(thread, "lmt"));
        self.unread.is_none_or(|u| bool_of(thread, "unread") == u)
            && self.starred.is_none_or(|s| bool_of(thread, "starred") == s)
            && (!self.has_attachment || i64_of(thread, "attachmentCount") > 0)
            && self.before.is_none_or(|b| ts < b)
            && self.after.is_none_or(|a| ts >= a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_filters_and_terms() {
        let q =
            parse("in:sent from:alice is:unread \"project alpha\" has:attachment after:2026/01/02")
                .unwrap();
        assert_eq!(q.terms, ["alice", "project alpha"]);
        assert_eq!(q.folders, ["sent"]);
        assert_eq!(q.unread, Some(true));
        assert!(q.has_attachment);
        assert_eq!(q.after, Some(1_767_312_000));
        assert_eq!(parse("a AND b").unwrap().terms, ["a", "b"]);
        assert!(parse("a OR b").is_err());
        assert!(parse("(to:bob)").is_err());
        assert!(parse("since:yesterday").unwrap_err().contains("YYYY/MM/DD"));
        assert!(parse("is:weird").is_err());
        assert_eq!(parse("http://x.test").unwrap().terms, ["http://x.test"]);
    }

    #[test]
    fn filters_threads() {
        let q = parse("is:starred before:2024-01-01").unwrap();
        assert!(q.matches(&json!({"starred": 1, "lmrt": 1_600_000_000})));
        assert!(!q.matches(&json!({"starred": 0, "lmrt": 1_600_000_000})));
        assert!(!q.matches(&json!({"starred": 1, "lmrt": 1_800_000_000})));
    }
}
