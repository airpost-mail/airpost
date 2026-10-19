// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `DraftFactory.prepareBodyForQuoting`: the body of the message being replied to or forwarded
//! as the draft quotes it.

use std::sync::LazyLock;

use regex::Regex;

use crate::inline_style::inline_styles;
use crate::plaintext::{deepen_plaintext_quote, html_to_plaintext, plaintext_to_html};
use crate::sanitize::sanitize;

/// Inline images (`cid:`) are left out of quotes; the desktop has the same TODO.
static CID_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<img.*src=['"]cid:([^'"]*)['"][\s\S]*?>"#).expect("cid regex")
});

/// `body` is the quoted message's, `plaintext` whether it is a plaintext message, `html`
/// whether the draft is HTML.
pub fn prepare_body_for_quoting(body: &str, plaintext: bool, html: bool) -> String {
    if !html {
        let content = if plaintext {
            body.to_owned()
        } else {
            html_to_plaintext(body)
        };
        return deepen_plaintext_quote(&content);
    }
    let content = if plaintext {
        plaintext_to_html(body)
    } else {
        body.to_owned()
    };
    let transformed = CID_IMAGE.replace_all(&content, "");
    inline_styles(&sanitize(&transformed))
}
