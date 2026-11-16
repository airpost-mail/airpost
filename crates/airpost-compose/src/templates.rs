// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Templates as the desktop's `composer-templates` plugin keeps them: one HTML file per
//! template in `<config>/templates`, named `<name>.html`, its subject in a leading
//! `<meta name="subject" content="…"/>` line (`lib/file.ts`). Tagged variables are the
//! composer's `<span data-tvar="name" …>text</span>` (`template-plugins.tsx`). Inserting one
//! into a draft (`TemplateStore._onInsertTemplateId`) replaces the text above the signature
//! and quote, and sets the subject unless the draft is a reply or a forward.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Value};

use crate::plaintext::html_to_plaintext;

/// The folder under the config dir.
pub const TEMPLATES_DIR: &str = "templates";
/// The extension the desktop saves templates with.
pub const TEMPLATE_EXTENSION: &str = "html";

static SUBJECT_META: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\A\s*<meta\s+name=["']?subject["']?\s+content=(?:"([^"]*)"|'([^']*)')\s*/?>[^\S\n]*\n?"#,
    )
    .expect("subject meta regex")
});
/// `INVALID_TEMPLATE_NAME_REGEX`: letters (with accents), digits, `_`, `-` and spaces only.
static INVALID_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[^a-zA-Z\x{00C0}-\x{017F}0-9_\- ]+").expect("template name regex")
});
static VARIABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<[a-z]+\b[^>]*\sdata-tvar=(?:"([^"]*)"|'([^']*)')"#).expect("tvar regex")
});
static SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<signature id="([A-Za-z0-9\-/\\]+)">.*</signature>"#)
        .expect("signature regex")
});
static PLAINTEXT_QUOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(?:[^\n]*\n)?>|^-{10} ").expect("plaintext quote regex"));

/// Where `_onInsertTemplateId` puts a template: before the first of these.
const INSERT_BEFORE: [&str; 3] = [
    "<signature",
    "<div class=\"gmail_quote_attribution\"",
    "<blockquote class=\"gmail_quote\"",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Template {
    /// Empty when the file has none.
    pub subject: String,
    pub body: String,
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn unescape_attribute(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// `parseTemplate`: a file's subject and body. Without the meta tag the subject is empty and
/// the body the whole file.
pub fn parse(contents: &str) -> Template {
    match SUBJECT_META.captures(contents) {
        None => Template {
            subject: String::new(),
            body: contents.to_owned(),
        },
        Some(c) => {
            let raw = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
            Template {
                subject: unescape_attribute(raw).trim().to_owned(),
                body: contents[c.get(0).map_or(0, |m| m.end())..].to_owned(),
            }
        }
    }
}

/// `stringifyTemplate`: the file for a subject and body; no meta tag without a subject.
pub fn stringify(template: &Template) -> String {
    let subject = template.subject.trim();
    if subject.is_empty() {
        return template.body.clone();
    }
    format!(
        "<meta name=\"subject\" content=\"{}\"/>\n{}",
        escape_attribute(subject),
        template.body
    )
}

/// The name a template file is listed under: the file name without its extension
/// (`path.basename(filename, path.extname(filename))`). None for dot files, which the
/// desktop skips.
pub fn name_of_file(filename: &str) -> Option<&str> {
    if filename.starts_with('.') {
        return None;
    }
    Some(match filename.rfind('.') {
        Some(i) if i > 0 => &filename[..i],
        _ => filename,
    })
}

/// The file a new template is saved as.
pub fn file_name(name: &str) -> String {
    format!("{name}.{TEMPLATE_EXTENSION}")
}

/// Why `name` can't name a template, in the desktop's words.
pub fn name_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("You must provide a template name.")
    } else if INVALID_NAME.is_match(name) {
        Some("Invalid template name! Names can only contain letters, numbers, spaces, dashes, and underscores.")
    } else {
        None
    }
}

/// `saveNewTemplate`'s name for a template called `name` when `taken` names exist: `name`,
/// else `name 1`, `name 2`, …
pub fn unused_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut resolved = name.to_owned();
    let mut n = 1;
    while taken(&resolved) {
        resolved = format!("{name} {n}");
        n += 1;
    }
    resolved
}

/// The tagged variables in a body, in order, each once.
pub fn variables(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in VARIABLE.captures_iter(body) {
        let name = unescape_attribute(c.get(1).or(c.get(2)).map_or("", |m| m.as_str()));
        // `if (name)`: the editor drops an empty one.
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// The body as one line of plain text, cut to about `max` characters.
pub fn preview(body: &str, max: usize) -> String {
    let text = html_to_plaintext(body);
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let cut: String = line.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", cut.trim_end())
}

/// `_onCreateTemplateFromDraft`: a draft's text as a template body, without the quoted text
/// and from the signature on. The desktop removes quotes with a DOM walk; this cuts at the
/// first quote element, as [`crate::validate`] does.
pub fn body_from_draft(body: &str) -> String {
    let mut end = body.len();
    if let Some(m) = SIGNATURE.find(body) {
        end = m.start();
    }
    let lower = body.to_ascii_lowercase();
    if let Some(at) = lower
        .find("gmail_quote")
        .and_then(|at| lower[..at].rfind('<'))
    {
        end = end.min(at);
    }
    if let Some(at) = lower.find("<blockquote") {
        end = end.min(at);
    }
    body[..end].to_owned()
}

/// The template name a draft's subject gives (the subject without the characters a name
/// can't have).
pub fn name_from_subject(subject: &str) -> String {
    INVALID_NAME.replace_all(subject.trim(), "").into_owned()
}

/// Whether a draft is a reply or a forward, whose subject a template leaves alone.
pub fn answers_a_message(draft: &Value) -> bool {
    ["rthMsgId", "fwdMsgId"].iter().any(|k| {
        draft
            .get(*k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    })
}

/// `_onInsertTemplateId` on a draft's Message JSON: the template's body in place of the text
/// above the signature and quote, and its subject unless the draft answers a message or
/// `keep_subject`. A plaintext draft gets the body as plain text above its quote.
pub fn apply(draft: &mut Value, template: &Template, keep_subject: bool) {
    let current = draft.get("body").and_then(Value::as_str).unwrap_or("");
    let plaintext = draft
        .get("plaintext")
        .is_some_and(|p| p.as_bool().unwrap_or_else(|| p.as_i64().unwrap_or(0) != 0));
    let body = if plaintext {
        let text = html_to_plaintext(&template.body);
        match PLAINTEXT_QUOTE.find(current) {
            Some(m) => format!("{text}\n\n{}", &current[m.start()..]),
            None => text,
        }
    } else {
        let insertion = INSERT_BEFORE
            .iter()
            .filter_map(|s| current.find(s))
            .min()
            .unwrap_or(current.len());
        format!("{}{}", template.body, &current[insertion..])
    };
    draft["body"] = json!(body);
    if !keep_subject && !template.subject.is_empty() && !answers_a_message(draft) {
        draft["subject"] = json!(template.subject);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/template-file.test.ts
    #[test]
    fn parses_the_desktop_file_format() {
        let body = "<div>Hello world</div>";
        assert_eq!(
            parse(body),
            Template {
                subject: String::new(),
                body: body.into()
            }
        );
        assert_eq!(parse(""), Template::default());
        let t = parse("<meta name=\"subject\" content=\"Following up\"/>\n<div>Hello world</div>");
        assert_eq!(t.subject, "Following up");
        assert_eq!(t.body, "<div>Hello world</div>");
        let t = parse("<meta name=subject content='Following up'><div>Hello</div>");
        assert_eq!(t.subject, "Following up");
        assert_eq!(t.body, "<div>Hello</div>");
        let t = parse(
            "<meta name=\"subject\" content=\"Ben &amp; Jerry&#39;s &quot;best&quot; &lt;flavors&gt;\"/>",
        );
        assert_eq!(t.subject, "Ben & Jerry's \"best\" <flavors>");
        let body = "<div>Hi</div><meta name=\"subject\" content=\"Nope\"/>";
        assert_eq!(parse(body).subject, "");
        assert_eq!(parse(body).body, body);
    }

    #[test]
    fn writes_the_desktop_file_format() {
        let t = |subject: &str, body: &str| Template {
            subject: subject.into(),
            body: body.into(),
        };
        assert_eq!(stringify(&t("", "<div>Hi</div>")), "<div>Hi</div>");
        assert_eq!(stringify(&t("   ", "<div>Hi</div>")), "<div>Hi</div>");
        assert_eq!(
            stringify(&t("Following up", "<div>Hi</div>")),
            "<meta name=\"subject\" content=\"Following up\"/>\n<div>Hi</div>"
        );
        assert_eq!(
            stringify(&t("Ben & Jerry's \"best\"", "")),
            "<meta name=\"subject\" content=\"Ben &amp; Jerry's &quot;best&quot;\"/>\n"
        );
        let round = t("A <b>bold</b> & \"quoted\" subject", "<div>Hi</div>");
        assert_eq!(parse(&stringify(&round)), round);
    }

    #[test]
    fn names_variables_and_previews() {
        assert_eq!(
            name_of_file("Welcome to Templates.html"),
            Some("Welcome to Templates")
        );
        assert_eq!(name_of_file("notes"), Some("notes"));
        assert_eq!(name_of_file("a.b.html"), Some("a.b"));
        assert_eq!(name_of_file(".DS_Store"), None);
        assert_eq!(name_error("Follow up 2_x-y"), None);
        assert_eq!(name_error("Café"), None);
        assert!(name_error("../etc").is_some());
        assert!(name_error("").is_some());
        assert_eq!(name_from_subject(" Re: Q3 / plan! "), "Re Q3  plan");
        assert_eq!(unused_name("A", |n| n == "A" || n == "A 1"), "A 2");

        // The start of the desktop's `assets/Welcome to Templates.html`.
        let welcome = "<meta name=\"subject\" content=\"Welcome to Templates!\"/>\n\
            <div>Hey, <span data-tvar=\"first_name\" class=\"template-variable undefined\" title=\"first_name\">First Name</span>!</div>\n\
            <div><br/></div>\n<div>Templates are pre-written emails with\n\
            <span data-tvar=\"content\" class=\"template-variable undefined\" title=\"content\">tagged variables</span> that make it easy to\n\
            reply quickly. The <span data-tvar=\"highlighting\" class=\"template-variable undefined\" title=\"highlighting\">highlighting</span> \
            is always removed.</div>";
        let t = parse(welcome);
        assert_eq!(t.subject, "Welcome to Templates!");
        assert_eq!(
            variables(&t.body),
            ["first_name", "content", "highlighting"]
        );
        assert!(preview(&t.body, 40).starts_with("Hey, First Name!"));
        assert!(preview(&t.body, 40).ends_with('…'));
        assert_eq!(preview("<p>Short</p>", 40), "Short");
    }

    #[test]
    fn applies_above_the_signature_and_quote() {
        let t = Template {
            subject: "Hello".into(),
            body: "<div>Hi <span data-tvar=\"name\">Name</span></div>".into(),
        };
        let mut new = json!({"subject": "", "body": "<br/><br/><signature id=\"s\">S</signature>"});
        apply(&mut new, &t, false);
        assert_eq!(
            new["body"],
            "<div>Hi <span data-tvar=\"name\">Name</span></div><signature id=\"s\">S</signature>"
        );
        assert_eq!(new["subject"], "Hello");

        let mut reply = json!({
            "subject": "Re: x", "rthMsgId": "<a@b>",
            "body": "<p>old</p><div class=\"gmail_quote_attribution\">On Mon</div><blockquote class=\"gmail_quote\">q</blockquote>",
        });
        apply(&mut reply, &t, false);
        assert_eq!(reply["subject"], "Re: x");
        assert!(reply["body"]
            .as_str()
            .unwrap()
            .starts_with("<div>Hi <span data-tvar=\"name\">Name</span></div><div class=\"gmail_quote_attribution\">"));

        let mut kept = json!({"subject": "Mine", "body": ""});
        apply(&mut kept, &t, true);
        assert_eq!(kept["subject"], "Mine");

        let mut plain = json!({"plaintext": true, "body": "old\n\nOn Mon, Bob wrote:\n> q\n"});
        apply(&mut plain, &t, false);
        assert_eq!(plain["body"], "Hi Name\n\nOn Mon, Bob wrote:\n> q\n");
    }

    #[test]
    fn validate_flags_variables_above_the_quote() {
        let me = crate::Identities::from_accounts_json(&[
            json!({"id": "a", "emailAddress": "me@x.test"}),
        ]);
        let codes = |body: &str| -> Vec<&'static str> {
            let draft = json!({"to": [{"email": "b@y.test"}], "from": [{"email": "me@x.test"}],
                               "subject": "s", "body": body});
            crate::validate::validate(&draft, &me, &[])
                .misc_warnings
                .iter()
                .map(|p| p.code)
                .collect()
        };
        let filled = codes("<div>Hi Ann</div>");
        assert!(!filled.contains(&"template-variable"));
        let draft = json!({"to": [{"email": "b@y.test"}], "from": [{"email": "me@x.test"}],
                           "subject": "s", "body": "<div>Hi <span data-tvar=\"first_name\">First</span></div>"});
        let v = crate::validate::validate(&draft, &me, &[]);
        assert_eq!(
            v.misc_warnings.last().map(|p| p.message.as_str()),
            Some("The message has template variables to fill in: first_name.")
        );
        let quoted = codes("<div>Hi</div><blockquote class=\"gmail_quote\"><span data-tvar=\"x\">x</span></blockquote>");
        assert!(!quoted.contains(&"template-variable"));
    }

    #[test]
    fn a_draft_body_without_its_signature_and_quote() {
        assert_eq!(
            body_from_draft("<div>Text</div><br/><signature id=\"s\">S</signature><div class=\"gmail_quote\">q</div>"),
            "<div>Text</div><br/>"
        );
        assert_eq!(body_from_draft("<p>Only</p>"), "<p>Only</p>");
        assert_eq!(
            body_from_draft("<p>A</p><blockquote>q</blockquote>"),
            "<p>A</p>"
        );
    }
}
