// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! What a draft takes from the settings the engine keeps (`settings.get`): HTML or plaintext
//! composing, the signatures and which one each address uses, the default account, and the
//! localized quote texts the desktop mirrors as `composing.quoteTexts`.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

pub const HTML_KEY: &str = "core.composing.html";
pub const SIGNATURES_KEY: &str = "signatures";
pub const DEFAULT_SIGNATURES_KEY: &str = "defaultSignatures";
pub const DEFAULT_ACCOUNT_KEY: &str = "core.sending.defaultAccountIdForSend";
pub const QUOTE_TEXTS_KEY: &str = "composing.quoteTexts";

/// A saved signature (`signatures` values: `{id, title, body}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub id: String,
    pub body: String,
}

/// `composing.quoteTexts` (`date-format.ts` `QuoteTexts`): the reply attribution and forward
/// header in the user's language, and the date pattern of their locale and hour cycle.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct QuoteTexts {
    /// `{date}` and `{from}` are filled in.
    pub attribution: String,
    pub forwarded_message: String,
    pub from: String,
    pub subject: String,
    pub date: String,
    pub to: String,
    pub cc: String,
    /// See [`crate::dates::format_quote_date`].
    pub date_time: String,
    pub months: Vec<String>,
    pub day_periods: Vec<String>,
}

impl Default for QuoteTexts {
    /// English with a 12-hour clock, what the desktop writes for `en-US`.
    fn default() -> Self {
        QuoteTexts {
            attribution: "On {date}, {from} wrote:".into(),
            forwarded_message: "Forwarded Message".into(),
            from: "From".into(),
            subject: "Subject".into(),
            date: "Date".into(),
            to: "To".into(),
            cc: "Cc".into(),
            date_time: "{month} {day}, {year}, {hour}:{minute} {dayPeriod}".into(),
            months: [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ]
            .map(String::from)
            .to_vec(),
            day_periods: vec!["AM".into(), "PM".into()],
        }
    }
}

/// The settings a draft is built with.
#[derive(Clone, Debug)]
pub struct ComposeSettings {
    pub html: bool,
    pub signatures: HashMap<String, Signature>,
    /// Address to signature id.
    pub default_signatures: HashMap<String, String>,
    pub default_account_id: Option<String>,
    pub quote_texts: QuoteTexts,
    /// The zone dates in quotes are written in.
    pub tz: chrono_tz::Tz,
}

impl Default for ComposeSettings {
    fn default() -> Self {
        ComposeSettings {
            html: true,
            signatures: HashMap::new(),
            default_signatures: HashMap::new(),
            default_account_id: None,
            quote_texts: QuoteTexts::default(),
            tz: chrono_tz::UTC,
        }
    }
}

impl ComposeSettings {
    /// From the settings values (`key -> value`, absent or null for unset) and the zone to
    /// write dates in. Values of the wrong shape count as unset.
    pub fn from_values(get: impl Fn(&str) -> Option<Value>, tz: chrono_tz::Tz) -> ComposeSettings {
        let mut s = ComposeSettings {
            tz,
            ..ComposeSettings::default()
        };
        if let Some(html) = get(HTML_KEY).and_then(|v| v.as_bool()) {
            s.html = html;
        }
        if let Some(Value::Object(map)) = get(SIGNATURES_KEY) {
            for (id, sig) in map {
                let body = sig
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let sig_id = sig
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_owned();
                s.signatures.insert(id, Signature { id: sig_id, body });
            }
        }
        if let Some(Value::Object(map)) = get(DEFAULT_SIGNATURES_KEY) {
            for (email, id) in map {
                if let Some(id) = id.as_str() {
                    s.default_signatures.insert(email, id.to_owned());
                }
            }
        }
        s.default_account_id = get(DEFAULT_ACCOUNT_KEY)
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|id| !id.is_empty());
        if let Some(v) = get(QUOTE_TEXTS_KEY).filter(Value::is_object) {
            if let Ok(t) = serde_json::from_value::<QuoteTexts>(v) {
                s.quote_texts = t;
            }
        }
        s
    }

    /// `SignatureStore.signatureForEmail`: the default signature of an address.
    pub fn signature_for_email(&self, email: &str) -> Option<&Signature> {
        let id = self.default_signatures.get(email)?;
        self.signatures.get(id)
    }
}

/// The engine's own zone, for dates when the client names none: `TZ`, else the zone
/// `/etc/localtime` links to, else UTC.
pub fn local_tz() -> chrono_tz::Tz {
    let from_env = std::env::var("TZ")
        .ok()
        .and_then(|tz| tz.trim_start_matches(':').parse().ok());
    if let Some(tz) = from_env {
        return tz;
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            let p = p.to_string_lossy().into_owned();
            let i = p.find("zoneinfo/")?;
            p[i + "zoneinfo/".len()..].parse().ok()
        })
        .unwrap_or(chrono_tz::UTC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_settings() {
        let values = json!({
            "core.composing.html": false,
            "signatures": {"s1": {"id": "s1", "title": "W", "body": "<b>x</b>"}},
            "defaultSignatures": {"a@b.example": "s1", "c@d.example": "gone"},
            "composing.quoteTexts": {"attribution": "Am {date} schrieb {from}:"},
        });
        let s = ComposeSettings::from_values(|k| values.get(k).cloned(), chrono_tz::UTC);
        assert!(!s.html);
        assert_eq!(
            s.signature_for_email("a@b.example").unwrap().body,
            "<b>x</b>"
        );
        assert!(s.signature_for_email("c@d.example").is_none());
        assert_eq!(s.quote_texts.attribution, "Am {date} schrieb {from}:");
        assert_eq!(s.quote_texts.from, "From");
    }
}
