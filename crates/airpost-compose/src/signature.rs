// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Signatures in a draft body (`composer-signature/lib/signature-utils.ts`): a
//! `<signature id="…">` block, put before the quoted text.

use std::sync::LazyLock;

use regex::Regex;

use crate::settings::Signature;

/// The desktop's signature regex (`RegExpUtils`): from the first signature to the last
/// `</signature>`.
static SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<signature id="([A-Za-z0-9\-/\\]+)">.*</signature>"#)
        .expect("signature regex")
});

/// `RegExpUtils.nativeQuoteStartRegex()`: the first element whose tag mentions `gmail_quote`.
pub static QUOTE_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<[A-Za-z0-9_]+[^>]*gmail_quote").expect("quote regex"));

fn number_of_trailing_brs(text: &str) -> usize {
    let mut count = 0;
    let mut text = text.trim();
    loop {
        if let Some(t) = text.strip_suffix("<br/>") {
            text = t;
        } else if let Some(t) = text.strip_suffix("<div></div>") {
            text = t;
        } else {
            break;
        }
        text = text.trim();
        count += 1;
    }
    count
}

/// `currentSignatureId`: the signature above the quoted text, if any.
pub fn current_signature_id(body: &str) -> Option<String> {
    let reply_end = QUOTE_START.find(body).map_or(body.len(), |m| m.start());
    SIGNATURE
        .captures(&body[..reply_end])
        .map(|c| c[1].to_owned())
}

/// `applySignature`: replaces the current signature with `signature`, or removes it.
pub fn apply_signature(body: &str, signature: Option<&Signature>) -> String {
    let mut additional_whitespace = "<br/>";
    let mut new_body = body.to_owned();
    if current_signature_id(body).is_some() {
        new_body = SIGNATURE.replace(&new_body, "").into_owned();
        additional_whitespace = "";
    }
    let Some(signature) = signature else {
        return new_body;
    };
    let insertion = QUOTE_START
        .find(&new_body)
        .map_or(new_body.len(), |m| m.start());
    let (before, after) = new_body.split_at(insertion);
    if number_of_trailing_brs(before) > 1 {
        additional_whitespace = "";
    }
    format!(
        "{before}{additional_whitespace}<signature id=\"{}\">{}</signature>{after}",
        signature.id, signature.body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_and_replaces() {
        let sig = Signature {
            id: "s1".into(),
            body: "<b>S</b>".into(),
        };
        assert_eq!(
            apply_signature("<br/>", Some(&sig)),
            "<br/><br/><signature id=\"s1\"><b>S</b></signature>"
        );
        let with = apply_signature(
            "<div>Hi</div><div class=\"gmail_quote\">q</div>",
            Some(&sig),
        );
        assert_eq!(with, "<div>Hi</div><br/><signature id=\"s1\"><b>S</b></signature><div class=\"gmail_quote\">q</div>");
        assert_eq!(current_signature_id(&with).as_deref(), Some("s1"));
        assert_eq!(
            apply_signature(&with, None),
            "<div>Hi</div><br/><div class=\"gmail_quote\">q</div>"
        );
    }
}
