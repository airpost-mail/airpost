// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Reading reply suggestions out of a model's answer.

use std::fmt;

use serde::Serialize;
use serde_json::Value;

use crate::prompt::json_object;

const MAX_LABEL_CHARS: usize = 40;
const MAX_TEXT_CHARS: usize = 300;

/// A one-line reply the user can pick; picking it asks for a full draft.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub label: String,
    pub text: String,
}

impl fmt::Debug for Suggestion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Suggestion(***)")
    }
}

/// The suggestions in `answer`, at most `count`: the JSON the prompt asks for (an object
/// with `suggestions`, or a bare array), else one per non-empty line. Empty when neither
/// yields any.
pub(crate) fn parse(answer: &str, count: usize) -> Vec<Suggestion> {
    let from_json = json_object(answer)
        .and_then(|v| v.get("suggestions").cloned())
        .or_else(|| json_array(answer));
    let mut out: Vec<Suggestion> = match from_json {
        Some(Value::Array(items)) => items.iter().filter_map(from_value).collect(),
        _ => answer.lines().filter_map(from_line).collect(),
    };
    out.truncate(count);
    out
}

fn json_array(text: &str) -> Option<Value> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

fn from_value(v: &Value) -> Option<Suggestion> {
    let text = clean(v.get("text")?.as_str()?, MAX_TEXT_CHARS)?;
    let label = v
        .get("label")
        .and_then(Value::as_str)
        .and_then(|l| clean(l, MAX_LABEL_CHARS))
        .unwrap_or_else(|| label_for(&text));
    Some(Suggestion { label, text })
}

fn from_line(line: &str) -> Option<Suggestion> {
    let line = line
        .trim()
        .trim_start_matches(['-', '*', '•'])
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(['.', ')'])
        .trim();
    if line.starts_with(['{', '}', '[', ']', '`']) {
        return None;
    }
    let text = clean(line, MAX_TEXT_CHARS)?;
    Some(Suggestion {
        label: label_for(&text),
        text,
    })
}

/// The first four words.
fn label_for(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().take(4).collect();
    let label = words.join(" ");
    let label = label.trim_end_matches(['.', ',', '!', '?', ';', ':']);
    clean(label, MAX_LABEL_CHARS).unwrap_or_default()
}

fn clean(text: &str, max_chars: usize) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    Some(match line.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: &[Suggestion]) -> Vec<(&str, &str)> {
        v.iter()
            .map(|s| (s.label.as_str(), s.text.as_str()))
            .collect()
    }

    #[test]
    fn json_answers() {
        let answer = r#"{"suggestions":[{"label":"Accept","text":"Tuesday works for me."},{"label":"","text":"Could we do Wednesday instead?"},{"label":"x"},{"label":"Decline","text":"Sorry, I can't make it."}]}"#;
        assert_eq!(
            texts(&parse(answer, 3)),
            [
                ("Accept", "Tuesday works for me."),
                ("Could we do Wednesday", "Could we do Wednesday instead?"),
                ("Decline", "Sorry, I can't make it."),
            ]
        );
        assert_eq!(parse(answer, 1).len(), 1);
        let bare = r#"[{"label":"Yes","text":"Yes."}]"#;
        assert_eq!(texts(&parse(bare, 3)), [("Yes", "Yes.")]);
    }

    #[test]
    fn line_answers() {
        let answer = "1. Sounds good, see you then.\n\n- I need to check my calendar first.\n";
        assert_eq!(
            texts(&parse(answer, 3)),
            [
                ("Sounds good, see you", "Sounds good, see you then."),
                ("I need to check", "I need to check my calendar first."),
            ]
        );
        assert!(parse("", 3).is_empty());
    }
}
