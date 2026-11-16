// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Draft builders (`DraftFactory`, `draft-factory.ts`), each returning the Message JSON a
//! `SyncbackDraftTask` saves: new drafts, replies and reply-alls, forwards, `mailto:` drafts
//! and the resurfacing message of a snooze or reminder. New, reply, forward and mailto drafts
//! then go through the built-in `prepareNewDraft` hooks ([`prepare_new_draft`]): the
//! default signature of the From address, and tracking images out of the quoted text.

use serde_json::{json, Map, Value};

use crate::contacts::Contact;
use crate::dates::format_quote_date;
use crate::identity::{Identities, Identity};
use crate::mailto::parse_mailto;
use crate::participants::{
    for_reply, for_reply_all, subject_with_prefix, update_for_reply, Replied,
};
use crate::pixels::remove_tracking_pixels;
use crate::plaintext::{wrap_plaintext, DEFAULT_LINE_WIDTH};
use crate::quote::prepare_body_for_quoting;
use crate::sanitize::sanitize;
use crate::settings::ComposeSettings;
use crate::signature::apply_signature;

/// What every builder needs.
pub struct Ctx<'a> {
    pub settings: &'a ComposeSettings,
    pub accounts: &'a Identities,
    /// HTML or plaintext, `core.composing.html` unless the client says otherwise.
    pub html: bool,
    /// Epoch seconds.
    pub now: i64,
}

/// The Message JSON keys a client may set on a new draft (`draft.create` `fields`).
pub const FIELD_KEYS: [&str; 14] = [
    "aid",
    "to",
    "cc",
    "bcc",
    "from",
    "subject",
    "body",
    "files",
    "threadId",
    "hMsgId",
    "rthMsgId",
    "fwdMsgId",
    "plaintext",
    "pristine",
];

/// The [`FIELD_KEYS`] that carry what the user writes.
const CONTENT_KEYS: [&str; 6] = ["to", "cc", "bcc", "subject", "body", "files"];

/// Whether a client's `fields` write any of the draft's content (a non-empty recipient list,
/// subject, body or file list), which makes the draft one the user has touched.
pub fn sets_content(fields: &Map<String, Value>) -> bool {
    CONTENT_KEYS.iter().any(|k| match fields.get(*k) {
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    })
}

/// A client's `fields` over a draft that is already built, as [`create_draft`] sets them over
/// the defaults: contacts normalised, `file_ids` following `files`. `aid` and `hMsgId` stay,
/// so the draft keeps its account and identity.
pub fn set_fields(draft: &mut Value, fields: &Map<String, Value>) {
    for key in FIELD_KEYS {
        if key == "aid" || key == "hMsgId" {
            continue;
        }
        let Some(v) = fields.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        draft[key] = match key {
            "to" | "cc" | "bcc" | "from" => normalise_contacts(v),
            _ => v.clone(),
        };
    }
    if fields.get("files").is_some_and(|f| !f.is_null()) {
        draft["file_ids"] = file_ids(&draft["files"]);
    }
}

fn file_ids(files: &Value) -> Value {
    Value::Array(
        files
            .as_array()
            .map(|f| f.iter().map(|f| json!(str_of(f, "id"))).collect())
            .unwrap_or_default(),
    )
}

fn contact_json(c: &Contact) -> Value {
    json!({"name": c.name, "email": c.email, "__cls": "Contact"})
}

fn contacts_json(list: &[Contact]) -> Value {
    Value::Array(list.iter().map(contact_json).collect())
}

/// Contacts as clients send them (`{name, email}`), with the model class added.
fn normalise_contacts(v: &Value) -> Value {
    contacts_json(&Contact::list_from_json(v))
}

/// `DOMUtils.escapeHTMLCharacters`.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#039;"),
            c => out.push(c),
        }
    }
    out
}

/// `generateHeaderMessageId`: `<UUID>@<domain of the From address>`, the domain in its ASCII
/// form; the machine's hostname, or `localhost`, when the address has no usable domain.
pub fn new_header_message_id(from_email: &str) -> String {
    let valid = |d: &str| {
        !d.is_empty()
            && d.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    };
    let mut domain = from_email
        .rfind('@')
        .map(|at| from_email[at + 1..].trim().to_owned())
        .unwrap_or_default();
    if !domain.is_ascii() {
        domain = idna::domain_to_ascii(&domain).unwrap_or_default();
    }
    if !valid(&domain) {
        domain = hostname::get()
            .ok()
            .map(|h| {
                h.to_string_lossy()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                    .collect::<String>()
            })
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "localhost".into());
    }
    format!(
        "{}@{domain}",
        uuid::Uuid::new_v4().to_string().to_ascii_uppercase()
    )
}

/// `DraftFactory._accountForNewDraft` without a window to look at: the default sending
/// account, else the first.
pub fn account_for_new_draft<'a>(ctx: &Ctx<'a>) -> Option<&'a Identity> {
    ctx.settings
        .default_account_id
        .as_deref()
        .and_then(|id| ctx.accounts.get(id))
        .or_else(|| ctx.accounts.0.first())
}

fn str_of<'v>(v: &'v Value, k: &str) -> &'v str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// `DraftFactory.createDraft`: the defaults for `account` with `fields` over them, a new
/// Message-ID unless one is given, the account's auto-cc or auto-bcc added, and plaintext
/// wrapped.
pub fn create_draft(ctx: &Ctx, account: &Identity, fields: &Map<String, Value>) -> Value {
    let mut m = json!({
        "aid": account.account_id,
        "v": 0,
        "metadata": [],
        "to": [],
        "cc": [],
        "bcc": [],
        "from": [contact_json(&account.default_me())],
        "replyTo": [],
        "date": ctx.now,
        "body": if ctx.html { "<br/>" } else { "" },
        "files": [],
        "unread": false,
        "events": [],
        "starred": false,
        "subject": "",
        "draft": true,
        "pristine": true,
        "plaintext": !ctx.html,
        "object": "draft",
        "__cls": "Message",
    });
    for key in FIELD_KEYS {
        let Some(v) = fields.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        m[key] = match key {
            "to" | "cc" | "bcc" | "from" => normalise_contacts(v),
            _ => v.clone(),
        };
    }
    m["aid"] = json!(account.account_id);
    if str_of(&m, "hMsgId").is_empty() {
        let from = Contact::list_from_json(&m["from"]);
        m["hMsgId"] = json!(new_header_message_id(
            from.first().map_or("", |c| c.email.as_str())
        ));
    }
    if let Some((kind, auto)) = account.auto_contacts() {
        if kind == "cc" || kind == "bcc" {
            let mut list = Contact::list_from_json(&m[kind]);
            list.extend(auto);
            m[kind] = contacts_json(&list);
        }
    }
    if m["plaintext"].as_bool() == Some(true) {
        let body = wrap_plaintext(str_of(&m, "body"), DEFAULT_LINE_WIDTH);
        m["body"] = json!(body);
    }
    m["file_ids"] = file_ids(&m["files"]);
    m
}

/// The built-in `prepareNewDraft` hooks, in the order the desktop registers them: the From
/// address's default signature (HTML drafts only), then tracking images removed.
pub fn prepare_new_draft(ctx: &Ctx, draft: &mut Value) {
    let mut body = str_of(draft, "body").to_owned();
    if draft["plaintext"].as_bool() != Some(true) {
        let from = Contact::list_from_json(&draft["from"]);
        if let Some(sig) = from
            .first()
            .and_then(|f| ctx.settings.signature_for_email(&f.email))
        {
            body = apply_signature(&body, Some(sig));
        }
    }
    body = remove_tracking_pixels(&body);
    draft["body"] = json!(body);
}

/// A new draft: `fields` over the defaults of their `aid` (else the default account).
pub fn new_draft(ctx: &Ctx, fields: &Map<String, Value>) -> Result<Value, String> {
    let account = match fields.get("aid").and_then(Value::as_str) {
        Some(id) => ctx
            .accounts
            .get(id)
            .ok_or_else(|| format!("No account {id}"))?,
        None => account_for_new_draft(ctx).ok_or("There is no account to write from")?,
    };
    let mut d = create_draft(ctx, account, fields);
    prepare_new_draft(ctx, &mut d);
    Ok(d)
}

fn account_of<'a>(ctx: &Ctx<'a>, message: &Value) -> Result<&'a Identity, String> {
    let id = str_of(message, "aid");
    ctx.accounts
        .get(id)
        .ok_or_else(|| format!("The message's account {id} is not one the engine holds"))
}

fn quoted_body(ctx: &Ctx, message: &Value) -> String {
    let plaintext = message["plaintext"].as_bool() == Some(true);
    prepare_body_for_quoting(str_of(message, "body"), plaintext, ctx.html)
}

fn quote_date(ctx: &Ctx, message: &Value) -> String {
    let at = message["date"]
        .as_i64()
        .or_else(|| message["date"].as_f64().map(|d| d as i64))
        .unwrap_or(0);
    format_quote_date(at, &ctx.settings.quote_texts, ctx.settings.tz)
}

/// `Message.replyAttributionLine()`.
pub fn attribution_line(ctx: &Ctx, message: &Value) -> String {
    let from = Contact::list_from_json(&message["from"])
        .into_iter()
        .next()
        .unwrap_or_else(|| Contact::new("Unknown", "Unknown"));
    ctx.settings
        .quote_texts
        .attribution
        .replacen("{date}", &quote_date(ctx, message), 1)
        .replacen("{from}", &from.display(), 1)
}

/// `DraftFactory.createDraftForReply` then `prepareNewDraft`. `extra` (the client's `fields`)
/// goes over what the reply sets.
pub fn reply_draft(
    ctx: &Ctx,
    message: &Value,
    reply_all: bool,
    extra: &Map<String, Value>,
) -> Result<Value, String> {
    let account = account_of(ctx, message)?;
    let replied = Replied::from_message_json(message);
    let (to, cc) = if reply_all {
        for_reply_all(&replied, ctx.accounts)
    } else {
        for_reply(&replied, ctx.accounts)
    };
    let prev = quoted_body(ctx, message);
    let attribution = attribution_line(ctx, message);
    let body = if ctx.html {
        format!(
            "\n        <br/>\n        <br/>\n        <div class=\"gmail_quote_attribution\">{}</div>\n        <blockquote class=\"gmail_quote\"\n          style=\"margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex;\">\n          {prev}\n          <br/>\n        </blockquote>\n        ",
            escape_html(&attribution)
        )
    } else {
        format!("\n\n{attribution}\n{prev}")
    };
    let from = account.reply_from(&replied.to, &replied.cc);
    let mut fields = Map::new();
    fields.insert(
        "subject".into(),
        json!(subject_with_prefix(str_of(message, "subject"), "Re:")),
    );
    fields.insert("to".into(), contacts_json(&to));
    fields.insert("cc".into(), contacts_json(&cc));
    fields.insert("from".into(), json!([contact_json(&from)]));
    fields.insert("threadId".into(), json!(str_of(message, "threadId")));
    fields.insert("rthMsgId".into(), json!(str_of(message, "hMsgId")));
    fields.insert("body".into(), json!(body));
    fields.extend(
        extra
            .iter()
            .filter(|(k, _)| *k != "aid")
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    let mut d = create_draft(ctx, account, &fields);
    prepare_new_draft(ctx, &mut d);
    Ok(d)
}

/// `DraftFactory.updateDraftForReply`: an existing reply draft's recipients switched between
/// reply and reply-all.
pub fn update_reply_draft(ctx: &Ctx, draft: &mut Value, message: &Value, reply_all: bool) {
    let replied = Replied::from_message_json(message);
    let (to, cc) = update_for_reply(
        &Contact::list_from_json(&draft["to"]),
        &Contact::list_from_json(&draft["cc"]),
        &replied,
        ctx.accounts,
        reply_all,
    );
    draft["to"] = contacts_json(&to);
    draft["cc"] = contacts_json(&cc);
}

/// `DraftFactory.createDraftForForward` then `prepareNewDraft`. The message's files go with
/// the draft.
pub fn forward_draft(
    ctx: &Ctx,
    message: &Value,
    extra: &Map<String, Value>,
) -> Result<Value, String> {
    let account = account_of(ctx, message)?;
    let t = &ctx.settings.quote_texts;
    let format_contacts = |k: &str| {
        let text = Contact::list_from_json(&message[k])
            .iter()
            .map(Contact::display)
            .collect::<Vec<_>>()
            .join(", ");
        if ctx.html {
            escape_html(&text)
        } else {
            text
        }
    };
    let has = |k: &str| message[k].as_array().is_some_and(|a| !a.is_empty());
    let subject = str_of(message, "subject");
    let mut lines = Vec::new();
    if has("from") {
        lines.push(format!("{}: {}", t.from, format_contacts("from")));
    }
    lines.push(format!(
        "{}: {}",
        t.subject,
        if ctx.html {
            escape_html(subject)
        } else {
            subject.to_owned()
        }
    ));
    lines.push(format!("{}: {}", t.date, quote_date(ctx, message)));
    if has("to") {
        lines.push(format!("{}: {}", t.to, format_contacts("to")));
    }
    if has("cc") {
        lines.push(format!("{}: {}", t.cc, format_contacts("cc")));
    }
    let quoted = quoted_body(ctx, message);
    let body = if ctx.html {
        format!(
            "\n        <br/>\n        <div class=\"gmail_quote\">\n          <br>\n          ---------- {} ---------\n          <br><br>\n          {}\n          <br><br>\n          {quoted}\n          <br/>\n        </div>\n        ",
            t.forwarded_message,
            lines.join("<br>")
        )
    } else {
        format!(
            "\n\n---------- {} ---------\n\n{}\n\n{quoted}",
            t.forwarded_message,
            lines.join("\n")
        )
    };
    let from = account.reply_from(
        &Contact::list_from_json(&message["to"]),
        &Contact::list_from_json(&message["cc"]),
    );
    let mut fields = Map::new();
    fields.insert(
        "subject".into(),
        json!(subject_with_prefix(subject, "Fwd:")),
    );
    fields.insert("from".into(), json!([contact_json(&from)]));
    fields.insert(
        "files".into(),
        message.get("files").cloned().unwrap_or(json!([])),
    );
    fields.insert("threadId".into(), json!(str_of(message, "threadId")));
    fields.insert("fwdMsgId".into(), json!(str_of(message, "hMsgId")));
    fields.insert("body".into(), json!(body));
    fields.extend(
        extra
            .iter()
            .filter(|(k, _)| *k != "aid")
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    let mut d = create_draft(ctx, account, &fields);
    prepare_new_draft(ctx, &mut d);
    Ok(d)
}

/// `DraftFactory.createDraftForMailto` then `prepareNewDraft`, from `account` (else the
/// default account).
pub fn mailto_draft(ctx: &Ctx, account: Option<&Identity>, url: &str) -> Result<Value, String> {
    let link = parse_mailto(url).ok_or_else(|| format!("{url} is not a valid mailto URL."))?;
    let account = match account {
        Some(a) => a,
        None => account_for_new_draft(ctx).ok_or("There is no account to write from")?,
    };
    let mut fields = Map::new();
    fields.insert("to".into(), contacts_json(&link.to));
    if !link.cc.is_empty() {
        fields.insert("cc".into(), contacts_json(&link.cc));
    }
    if !link.bcc.is_empty() {
        fields.insert("bcc".into(), contacts_json(&link.bcc));
    }
    if let Some(s) = link.subject {
        fields.insert("subject".into(), json!(s));
    }
    if let Some(mut body) = link.body {
        if ctx.html {
            body = sanitize(&body.replace(['\n', '\r'], "<br/>"));
        }
        fields.insert("body".into(), json!(body));
    }
    let mut d = create_draft(ctx, account, &fields);
    prepare_new_draft(ctx, &mut d);
    Ok(d)
}

/// `DraftFactory.createDraftForResurfacing`: a message from the account to itself, named
/// "<account name> via Tern" so `Message.isHidden` hides it, answering `reply_to` so the thread
/// comes to the top of the inbox in every mail client. Unlike the desktop, the account's
/// auto-cc and auto-bcc are left off: nobody else should get it.
pub fn resurface_draft(
    account: &Identity,
    thread_id: &str,
    subject: &str,
    reply_to: &str,
    body: &str,
    now: i64,
) -> Value {
    let email = &account.email;
    let name = if account.name.is_empty() {
        email.as_str()
    } else {
        account.name.as_str()
    };
    let from = Contact::new(format!("{name} via Tern"), email.clone());
    json!({
        "__cls": "Message",
        "aid": account.account_id,
        "v": 0,
        "hMsgId": new_header_message_id(email),
        "subject": subject,
        "from": [contact_json(&from)],
        "to": [contact_json(&account.default_me())],
        "cc": [], "bcc": [], "replyTo": [], "files": [], "file_ids": [], "metadata": [], "events": [],
        "body": body,
        "plaintext": false,
        "draft": true,
        "pristine": false,
        "unread": false,
        "starred": false,
        "threadId": thread_id,
        "rthMsgId": reply_to,
        "fwdMsgId": "",
        "date": now,
        "object": "draft",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_message_ids() {
        let id = new_header_message_id("a@b.example");
        assert!(id.ends_with("@b.example") && id.len() == 36 + 10, "{id}");
        assert!(new_header_message_id("a@bücher.example").ends_with("@xn--bcher-kva.example"));
        assert!(!new_header_message_id("nobody").contains("@@"));
    }
}
