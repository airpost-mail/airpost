// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! [`Triage`]: the fixed vocabulary a triage answer is validated against.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::prompt::json_object;

/// Longest `reason` kept; the thread list shows it as a tooltip.
const MAX_REASON_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    NeedsReply,
    Important,
    Newsletter,
    Notification,
    Promotion,
    Other,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::NeedsReply,
        Category::Important,
        Category::Newsletter,
        Category::Notification,
        Category::Promotion,
        Category::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Category::NeedsReply => "needs-reply",
            Category::Important => "important",
            Category::Newsletter => "newsletter",
            Category::Notification => "notification",
            Category::Promotion => "promotion",
            Category::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Option<Category> {
        Category::ALL
            .into_iter()
            .find(|c| c.as_str().eq_ignore_ascii_case(value.trim()))
    }
}

/// Serializes as `{category, needsReply, reason}` with `reason` null when absent. `Debug`
/// leaves out the reason, which is model output.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Triage {
    pub category: Category,
    pub needs_reply: bool,
    pub reason: Option<String>,
}

impl fmt::Debug for Triage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Triage")
            .field("category", &self.category)
            .field("needs_reply", &self.needs_reply)
            .field("reason", &self.reason.is_some())
            .finish()
    }
}

impl Triage {
    /// What an answer outside the schema becomes.
    pub fn fallback() -> Triage {
        Triage {
            category: Category::Other,
            needs_reply: false,
            reason: None,
        }
    }

    /// The model's answer, validated: a known `category`, a boolean `needsReply` (taken from
    /// the category when missing) and a one-line `reason`. Anything else is
    /// [`Triage::fallback`].
    pub fn from_answer(text: &str) -> Triage {
        json_object(text)
            .and_then(|v| Triage::from_value(&v))
            .unwrap_or_else(Triage::fallback)
    }

    fn from_value(v: &Value) -> Option<Triage> {
        let category = Category::parse(v.get("category")?.as_str()?)?;
        let needs_reply = match v.get("needsReply") {
            None | Some(Value::Null) => category == Category::NeedsReply,
            Some(b) => b.as_bool()?,
        };
        let reason = match v.get("reason") {
            None | Some(Value::Null) => None,
            Some(r) => one_line(r.as_str()?),
        };
        Some(Triage {
            category,
            needs_reply: needs_reply || category == Category::NeedsReply,
            reason,
        })
    }
}

/// Whitespace collapsed to single spaces and cut to [`MAX_REASON_CHARS`]; `None` if empty.
fn one_line(text: &str) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    Some(match line.char_indices().nth(MAX_REASON_CHARS) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_answers() {
        let t = Triage::from_answer(
            r#"{"category":"needs-reply","needsReply":true,"reason":"Ann asks for a date."}"#,
        );
        assert_eq!(t.category, Category::NeedsReply);
        assert!(t.needs_reply);
        assert_eq!(t.reason.as_deref(), Some("Ann asks for a date."));
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            json!({"category": "needs-reply", "needsReply": true, "reason": "Ann asks for a date."})
        );

        let t = Triage::from_answer("```json\n{\"category\": \"Newsletter\", \"needsReply\": false, \"reason\": \"  weekly\\n digest \"}\n```");
        assert_eq!(t.category, Category::Newsletter);
        assert_eq!(t.reason.as_deref(), Some("weekly digest"));

        let t = Triage::from_answer(r#"{"category":"needs-reply"}"#);
        assert!(t.needs_reply);
        assert_eq!(t.reason, None);
    }

    #[test]
    fn anything_else_is_other() {
        for answer in [
            "",
            "needs-reply",
            "{\"category\": \"urgent\", \"needsReply\": true, \"reason\": \"x\"}",
            "{\"category\": \"important\", \"needsReply\": \"yes\"}",
            "{\"category\": 3}",
            "{\"category\": \"important\", \"reason\": 5}",
            "{\"category\": \"important\",",
        ] {
            assert_eq!(
                Triage::from_answer(answer),
                Triage::fallback(),
                "{answer:?}"
            );
        }
        assert_eq!(
            serde_json::to_value(Triage::fallback()).unwrap(),
            json!({"category": "other", "needsReply": false, "reason": null})
        );
    }

    #[test]
    fn long_reasons_are_cut() {
        let long = "word ".repeat(100);
        let t = Triage::from_answer(
            &json!({"category": "important", "needsReply": false, "reason": long}).to_string(),
        );
        assert_eq!(t.reason.unwrap().chars().count(), MAX_REASON_CHARS + 1);
    }
}
