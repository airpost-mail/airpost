// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The checks before a draft is sent (`draft.validate`): `DraftEditingSession`'s
//! `validateDraftRecipients` and `validateDraftForSending`, the templates extension's
//! empty-area warning, and template variables left in the text. Errors stop the send;
//! warnings ask first. The messages are the desktop's English ones.
//!
//! The desktop removes quoted text with a DOM walk (`QuotedHTMLTransformer.removeQuotedHTML`)
//! before looking for "attach" and a salutation. Here the body is cut at the first quote
//! (a `gmail_quote` element, a `<blockquote>` or an "Original Message" line), and kept whole
//! when nothing but the quote is there. An inline reply below a quote is not looked at.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::contacts::{likely_non_human_email, Contact};
use crate::data::DESKTOP;
use crate::identity::Identities;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub code: &'static str,
    pub message: String,
}

/// The desktop's four lists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    pub recipient_errors: Vec<Problem>,
    pub recipient_warnings: Vec<Problem>,
    pub misc_errors: Vec<Problem>,
    pub misc_warnings: Vec<Problem>,
}

impl Validation {
    /// `{errors, warnings}` with each entry's `group` (`recipients` or `misc`), recipients first.
    pub fn to_json(&self) -> Value {
        let tag = |list: &[Problem], group: &str| -> Vec<Value> {
            list.iter()
                .map(|p| serde_json::json!({"code": p.code, "message": p.message, "group": group}))
                .collect()
        };
        let mut errors = tag(&self.recipient_errors, "recipients");
        errors.extend(tag(&self.misc_errors, "misc"));
        let mut warnings = tag(&self.recipient_warnings, "recipients");
        warnings.extend(tag(&self.misc_warnings, "misc"));
        serde_json::json!({"errors": errors, "warnings": warnings})
    }
}

static SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<signature id="([A-Za-z0-9\-/\\]+)">.*</signature>"#).unwrap()
});
static QUOTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)<[A-Za-z0-9_]+[^>]*gmail_quote|<blockquote[\s>]|-{2,}\s*Original Message\s*-{2,}",
    )
    .unwrap()
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>|&nbsp;").unwrap());
static SALUTATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:[y|Y]o|[h|H]ey|[h|H]i|[M|m]orning|[A|a]fternoon|[E|e]vening|[D|d]ear) ([A-Z][A-Za-zÀ-ÿ. \-]+)[!_—,.\n\r< ]").unwrap()
});
static EMPTY_TEMPLATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<code[^>]*empty[^>]*>").unwrap());

/// The body the checks read: without quoted text and signature.
fn unquoted_body(body: &str) -> String {
    let body = body.trim();
    let mut cleaned = match QUOTE.find(body) {
        Some(m) => {
            let before = &body[..m.start()];
            if TAGS.replace_all(before, "").trim().is_empty() {
                body.to_owned()
            } else {
                before.to_owned()
            }
        }
        None => body.to_owned(),
    };
    if let Some(m) = SIGNATURE.find(&cleaned) {
        cleaned.truncate(m.start());
    }
    if let Some(i) = cleaned.find("<signature>") {
        cleaned.truncate(i.saturating_sub(1));
    }
    cleaned
}

/// Validates a draft (its Message JSON, with `body`). `me` is every account; `skip_salutation_for`
/// are addresses the user said not to check names for (the desktop's
/// `recipientWarningBlacklist`).
pub fn validate(draft: &Value, me: &Identities, skip_salutation_for: &[String]) -> Validation {
    let mut v = Validation::default();
    let list = |k: &str| {
        draft
            .get(k)
            .map(Contact::list_from_json)
            .unwrap_or_default()
    };
    let recipients: Vec<Contact> = [list("to"), list("cc"), list("bcc")].concat();
    let body = draft.get("body").and_then(Value::as_str).unwrap_or("");
    let cleaned = unquoted_body(body);

    // validateDraftRecipients
    let mut names: Vec<String> = DESKTOP.salutations.clone();
    let mut unnamed = false;
    for c in &recipients {
        if !c.is_valid() {
            v.recipient_errors.push(Problem {
                code: "invalid-recipient",
                message: format!(
                    "{} is not a valid email address - please remove or edit it before sending.",
                    c.email
                ),
            });
        }
        let name = c.full_name();
        if !name.is_empty() && name != c.email && !skip_salutation_for.contains(&c.email) {
            let lower = name.to_lowercase();
            names.push(lower.clone());
            names.extend(lower.split(' ').map(str::to_owned));
            names.extend(lower.split('-').map(str::to_owned));
            names.push(c.name_abbreviation().to_lowercase());
            names.extend(lower.chars().next().map(String::from));
        } else {
            unnamed = true;
        }
        if likely_non_human_email(&c.email) {
            unnamed = true;
        }
    }
    if recipients.is_empty() {
        v.recipient_errors.push(Problem {
            code: "no-recipients",
            message: "You need to provide one or more recipients before sending the message."
                .into(),
        });
    }
    if v.recipient_errors.is_empty() && !unnamed {
        if let Some(c) = SALUTATION.captures(&cleaned) {
            let written = c[1].to_owned();
            let mut salutation = written.to_lowercase();
            if salutation.ends_with('-') {
                salutation.pop();
            }
            let known = names.iter().any(|n| {
                *n == salutation || (n.chars().count() > 1 && salutation.contains(n.as_str()))
            });
            if !known {
                v.recipient_warnings.push(Problem {
                    code: "salutation-mismatch",
                    message: format!(
                        "The message is addressed to a name that doesn't appear to be a recipient (\"{written}\")"
                    ),
                });
            }
        }
    }

    // validateDraftForSending
    let from = list("from");
    if from
        .first()
        .is_none_or(|f| me.account_for_email(&f.email).is_none())
    {
        v.misc_errors.push(Problem {
            code: "unsendable-from",
            message: "The From address is not one of your accounts or aliases. Choose a From address and try again.".into(),
        });
    }
    if draft
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or("")
        .is_empty()
    {
        v.misc_warnings.push(Problem {
            code: "blank-subject",
            message: "The subject field is blank.".into(),
        });
    }
    let has_attachment = draft
        .get("files")
        .and_then(Value::as_array)
        .is_some_and(|f| !f.is_empty());
    if cleaned.to_lowercase().contains("attach") && !has_attachment {
        v.misc_warnings.push(Problem {
            code: "attachment-mentioned",
            message: "The message mentions an attachment but none are attached.".into(),
        });
    }
    // composer-templates: `search(...) > 0`, so an area at the very start isn't flagged.
    if EMPTY_TEMPLATE.find(body).is_some_and(|m| m.start() > 0) {
        v.misc_warnings.push(Problem {
            code: "empty-template",
            message: "The message contains an empty template area.".into(),
        });
    }
    // Engine only: the desktop's composer shows its variables highlighted instead.
    let unfilled = crate::templates::variables(&cleaned);
    if !unfilled.is_empty() {
        v.misc_warnings.push(Problem {
            code: "template-variable",
            message: format!(
                "The message has template variables to fill in: {}.",
                unfilled.join(", ")
            ),
        });
    }
    v
}
