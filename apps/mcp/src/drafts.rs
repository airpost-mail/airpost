// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Draft Message JSON as the desktop builds it: `DraftFactory.createDraft` and
//! `createDraftForReply` for new drafts, the reply participants and quote of
//! `Message.participantsForReply[All]` and `prepareBodyForQuoting`, and `Message.toJSON()`
//! for a stored draft sent back in a `SyncbackDraftTask` or `SendDraftTask`, the way the
//! desktop's draft session saves it. Bodies are HTML (`core.composing.html`'s default).

use chrono::TimeZone;
use serde_json::{json, Map, Value};

use crate::config::AccountMeta;
use crate::model::{array_of, bool_of, i64_of, str_of};

/// An account's identity for composing: what `config.json` says about it, or just the
/// address the engine reports when the desktop hasn't written one.
#[derive(Clone, Debug, Default)]
pub struct Sender {
    pub account_id: String,
    pub meta: AccountMeta,
}

/// A recipient as the tools take them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipient {
    pub name: String,
    pub email: String,
}

pub fn contact_json(name: &str, email: &str) -> Value {
    json!({"name": name, "email": email, "__cls": "Contact"})
}

fn contact_of(v: &Value) -> Recipient {
    Recipient {
        name: str_of(v, "name").to_owned(),
        email: str_of(v, "email").to_owned(),
    }
}

/// `Utils.toEquivalentEmailForm`: lowercased, `+tag` dropped.
pub fn equivalent_email(email: &str) -> String {
    let email = email.trim();
    match email.rfind('@') {
        Some(at) if at > 0 && at + 1 < email.len() => {
            let local = &email[..at];
            let user = local.split('+').next().unwrap_or(local);
            format!("{user}@{}", &email[at + 1..]).to_lowercase()
        }
        _ => String::new(),
    }
}

fn emails_equivalent(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
    a == b || (!a.is_empty() && equivalent_email(&a) == equivalent_email(&b))
}

/// `Contact.fromString` for `Name <email>`, `email`, or `"Name" <email>`.
pub fn parse_contact(s: &str) -> Recipient {
    let s = s.trim();
    if let (Some(lt), Some(gt)) = (s.rfind('<'), s.rfind('>')) {
        if lt < gt {
            let email = s[lt + 1..gt].trim().to_owned();
            let name = s[..lt]
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .trim();
            return Recipient {
                name: name.to_owned(),
                email,
            };
        }
    }
    if s.contains('@') {
        Recipient {
            name: String::new(),
            email: s.to_owned(),
        }
    } else {
        Recipient {
            name: s.to_owned(),
            email: String::new(),
        }
    }
}

/// `ContactStore.parseContactsInString`, for the auto-cc/bcc setting.
pub fn parse_contacts(s: &str) -> Vec<Recipient> {
    s.split([',', ';', '\n'])
        .map(parse_contact)
        .filter(|c| c.email.contains('@'))
        .collect()
}

/// The addresses that count as the user (`Contact.isMe`): every account and alias.
#[derive(Clone, Debug, Default)]
pub struct Me {
    emails: Vec<String>,
}

impl Me {
    pub fn new(senders: &[Sender]) -> Me {
        let mut emails = Vec::new();
        for s in senders {
            emails.push(s.meta.email.clone());
            emails.extend(s.meta.aliases.iter().map(|a| parse_contact(a).email));
        }
        emails.retain(|e| !e.is_empty());
        Me { emails }
    }

    pub fn is_me(&self, email: &str) -> bool {
        self.emails.iter().any(|m| emails_equivalent(m, email))
    }
}

impl Sender {
    /// `Account.me()`.
    fn me(&self) -> Recipient {
        Recipient {
            name: self.meta.name.clone(),
            email: self.meta.email.clone(),
        }
    }

    /// `Account.meUsingAlias`: an alias without an address keeps the account's.
    fn me_using_alias(&self, alias: &str) -> Recipient {
        let mut c = parse_contact(alias);
        if c.email.is_empty() {
            c.email = self.meta.email.clone();
            if c.name.is_empty() {
                c.name = self.meta.name.clone();
            }
        }
        c
    }

    /// `Account.defaultMe()`.
    pub fn default_me(&self) -> Recipient {
        match &self.meta.default_alias {
            Some(a) => self.me_using_alias(a),
            None => self.me(),
        }
    }

    /// `DraftFactory._fromContactForReply`: the alias the message was sent to, if any.
    pub fn reply_from(&self, message: &Value) -> Recipient {
        let default_me = self.default_me();
        let mut result = default_me.clone();
        let recipients: Vec<Recipient> = array_of(message, "to")
            .iter()
            .chain(array_of(message, "cc"))
            .map(contact_of)
            .collect();
        for alias_string in &self.meta.aliases {
            let alias = self.me_using_alias(alias_string);
            for r in &recipients {
                let email_not_default = alias.email != default_me.email;
                let emails_match = r.email == alias.email;
                let name_not_default = alias.name != default_me.name;
                let names_match = r.name == alias.name;
                if emails_match && email_not_default && names_match && name_not_default {
                    return alias;
                }
                if (emails_match && email_not_default) || (names_match && name_not_default) {
                    result = alias.clone();
                }
            }
        }
        result
    }
}

fn uniq(list: Vec<Recipient>) -> Vec<Recipient> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for c in list {
        let key = equivalent_email(&c.email);
        if !seen.contains(&key) {
            seen.push(key);
            out.push(c);
        }
    }
    out
}

/// `Message.participantsForReply()`: `(to, cc)`.
pub fn participants_for_reply(message: &Value, me: &Me) -> (Vec<Recipient>, Vec<Recipient>) {
    let reply_to: Vec<Recipient> = array_of(message, "replyTo")
        .iter()
        .map(contact_of)
        .collect();
    let from: Vec<Recipient> = array_of(message, "from").iter().map(contact_of).collect();
    let to: Vec<Recipient> = array_of(message, "to").iter().map(contact_of).collect();
    let to = if reply_to.first().is_some_and(|r| !me.is_me(&r.email)) {
        reply_to
    } else if from.first().is_some_and(|f| me.is_me(&f.email)) {
        to
    } else {
        from
    };
    (uniq(to), Vec::new())
}

/// `Message.participantsForReplyAll()`: `(to, cc)`.
pub fn participants_for_reply_all(message: &Value, me: &Me) -> (Vec<Recipient>, Vec<Recipient>) {
    let list =
        |k: &str| -> Vec<Recipient> { array_of(message, k).iter().map(contact_of).collect() };
    let (reply_to, from, to, cc) = (list("replyTo"), list("from"), list("to"), list("cc"));
    let excluded: Vec<String> = from.iter().map(|c| equivalent_email(&c.email)).collect();
    let exclude_me_and_froms = |l: Vec<Recipient>| -> Vec<Recipient> {
        l.into_iter()
            .filter(|p| !(me.is_me(&p.email) || excluded.contains(&equivalent_email(&p.email))))
            .collect()
    };
    let (new_to, new_cc) = if reply_to.first().is_some_and(|r| !me.is_me(&r.email)) {
        (reply_to, exclude_me_and_froms([to, cc].concat()))
    } else if from.first().is_some_and(|f| me.is_me(&f.email)) {
        (to, exclude_me_and_froms(cc))
    } else {
        (from, exclude_me_and_froms([to, cc].concat()))
    };
    (uniq(new_to), uniq(new_cc))
}

/// `Utils.subjectWithPrefix`.
pub fn subject_with_prefix(subject: &str, prefix: &str) -> String {
    let lower = subject.to_lowercase();
    for p in ["fwd:", "re:"] {
        if lower.starts_with(p) {
            return format!("{prefix}{}", &subject[p.len()..]);
        }
    }
    format!("{prefix} {subject}")
}

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

/// `Message.replyAttributionLine()`, in local time like `moment(date).format(...)`.
pub fn attribution_line(message: &Value) -> String {
    let date = chrono::Local
        .timestamp_opt(i64_of(message, "date"), 0)
        .single()
        .map(|d| d.format("%b %-d %Y, at %-I:%M %P").to_string())
        .unwrap_or_default();
    let from = array_of(message, "from")
        .first()
        .map(contact_of)
        .unwrap_or(Recipient {
            name: "Unknown".into(),
            email: "Unknown".into(),
        });
    let who = if !from.name.is_empty() && from.name != from.email {
        format!("{} <{}>", from.name, from.email)
    } else {
        from.email
    };
    format!("On {date}, {who} wrote:")
}

/// Removes every `<tag …>…</tag>` block, case-insensitively.
fn strip_blocks(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(start) = lower[i..].find(&open).map(|p| i + p) {
        out.push_str(&html[i..start]);
        i = match lower[start..].find(&close) {
            Some(p) => start + p + close.len(),
            None => html.len(),
        };
    }
    out.push_str(&html[i..]);
    out
}

/// Removes `<img>` tags that point at `cid:` parts, which the reply doesn't carry.
fn strip_cid_images(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(start) = lower[i..].find("<img").map(|p| i + p) {
        let end = lower[start..]
            .find('>')
            .map_or(html.len(), |p| start + p + 1);
        out.push_str(&html[i..start]);
        if !lower[start..end].contains("cid:") {
            out.push_str(&html[start..end]);
        }
        i = end;
    }
    out.push_str(&html[i..]);
    out
}

/// `prepareBodyForQuoting` for an HTML reply. The desktop also runs its sanitizer and
/// inlines `<style>` rules; here scripts, styles and embedded frames are dropped instead.
pub fn body_for_quoting(message: &Value, body: &str) -> String {
    if bool_of(message, "plaintext") {
        let lines: Vec<String> = body.split('\n').map(escape_html).collect();
        return format!(
            "<div style=\"white-space: pre-wrap;\">{}</div>",
            lines.join("<br>")
        );
    }
    let mut html = strip_cid_images(body);
    for tag in ["script", "style", "iframe", "object"] {
        html = strip_blocks(&html, tag);
    }
    html
}

/// The quoted part of `createDraftForReply`'s HTML body.
pub fn reply_quote(message: &Value, body: Option<&str>) -> String {
    let prev = body
        .map(|b| body_for_quoting(message, b))
        .unwrap_or_default();
    format!(
        "<br/><br/><div class=\"gmail_quote_attribution\">{}</div>\
         <blockquote class=\"gmail_quote\" \
         style=\"margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex;\">{prev}<br/></blockquote>",
        escape_html(&attribution_line(message))
    )
}

/// A new Message-ID: a random UUID at the sender's domain.
pub fn new_header_message_id(from_email: &str) -> String {
    let domain = from_email
        .rsplit_once('@')
        .map(|(_, d)| d.trim())
        .filter(|d| !d.is_empty() && !d.contains(['<', '>', ' ']))
        .unwrap_or("tern.local");
    format!(
        "{}@{domain}",
        uuid::Uuid::new_v4().hyphenated().to_string().to_uppercase()
    )
}

pub struct NewDraft {
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub bcc: Vec<Recipient>,
    pub subject: String,
    pub body: String,
    pub from: Recipient,
    pub thread_id: Option<String>,
    pub reply_to_header_message_id: Option<String>,
}

fn contacts_json(list: &[Recipient]) -> Value {
    Value::Array(
        list.iter()
            .map(|c| contact_json(&c.name, &c.email))
            .collect(),
    )
}

/// `DraftFactory.createDraft` merged with `fields`, then `Message.toJSON()`. No `id`: the
/// engine derives it from the account and Message-ID. `pristine` is false, as the plugin
/// set it before saving.
pub fn new_draft_json(sender: &Sender, d: NewDraft, now_secs: i64) -> Value {
    let mut cc = d.cc;
    let mut bcc = d.bcc;
    if let Some((kind, value)) = &sender.meta.autoaddress {
        let auto = parse_contacts(value);
        match kind.as_str() {
            "cc" => cc.extend(auto),
            "bcc" => bcc.extend(auto),
            _ => {}
        }
    }
    let mut m = json!({
        "aid": sender.account_id,
        "v": 0,
        "metadata": [],
        "to": contacts_json(&d.to),
        "cc": contacts_json(&cc),
        "bcc": contacts_json(&bcc),
        "from": [contact_json(&d.from.name, &d.from.email)],
        "replyTo": [],
        "date": now_secs,
        "body": d.body,
        "files": [],
        "unread": false,
        "events": [],
        "starred": false,
        "hMsgId": new_header_message_id(&d.from.email),
        "subject": d.subject,
        "draft": true,
        "pristine": false,
        "plaintext": false,
        "file_ids": [],
        "object": "draft",
        "__cls": "Message",
    });
    if let Some(t) = d.thread_id {
        m["threadId"] = Value::from(t);
    }
    if let Some(r) = d.reply_to_header_message_id {
        m["rthMsgId"] = Value::from(r);
    }
    m
}

/// `Message.toJSON()` of a stored draft: its model attributes only (the engine fills in
/// its own bookkeeping from the stored copy), plus `body` when known, `file_ids` and
/// `object`. A missing `body` leaves the stored one in place.
pub fn desktop_message_json(stored: &Value, body: Option<&str>) -> Value {
    const KEYS: &[&str] = &[
        "id",
        "aid",
        "v",
        "metadata",
        "to",
        "cc",
        "bcc",
        "from",
        "replyTo",
        "date",
        "files",
        "unread",
        "events",
        "starred",
        "snippet",
        "threadId",
        "hMsgId",
        "subject",
        "draft",
        "pristine",
        "plaintext",
        "rthMsgId",
        "fwdMsgId",
        "folders",
        "hListUnsub",
        "hListUnsubPost",
    ];
    let mut out = Map::new();
    for k in KEYS {
        if let Some(v) = stored.get(*k).filter(|v| !v.is_null()) {
            out.insert((*k).to_owned(), v.clone());
        }
    }
    for k in ["to", "cc", "bcc", "from", "replyTo", "files", "events"] {
        out.entry(k).or_insert_with(|| Value::Array(Vec::new()));
    }
    out.entry("subject").or_insert_with(|| Value::from(""));
    // The engine requires it in a send; the desktop's Message model defaults it to false.
    out.entry("plaintext").or_insert(Value::Bool(false));
    if let Some(b) = body {
        out.insert("body".into(), Value::from(b));
    }
    let file_ids: Vec<Value> = array_of(stored, "files")
        .iter()
        .map(|f| Value::from(str_of(f, "id")))
        .collect();
    out.insert("file_ids".into(), Value::Array(file_ids));
    if bool_of(stored, "draft") {
        out.insert("object".into(), Value::from("draft"));
    }
    out.insert("__cls".into(), Value::from("Message"));
    Value::Object(out)
}

pub fn recipients_json(list: &[Recipient]) -> Value {
    contacts_json(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender() -> Sender {
        Sender {
            account_id: "a1".into(),
            meta: AccountMeta {
                id: "a1".into(),
                name: "Me".into(),
                email: "me@x.test".into(),
                aliases: vec!["Work Me <work@x.test>".into()],
                ..Default::default()
            },
        }
    }

    #[test]
    fn reply_participants() {
        let me = Me::new(&[sender()]);
        let received = json!({"from": [{"name": "Bob", "email": "bob@y.test"}],
            "to": [{"email": "me@x.test"}, {"email": "carol@y.test"}],
            "cc": [{"email": "Bob+list@y.test"}, {"email": "dan@y.test"}], "replyTo": []});
        let (to, cc) = participants_for_reply(&received, &me);
        assert_eq!(
            to.iter().map(|c| c.email.as_str()).collect::<Vec<_>>(),
            ["bob@y.test"]
        );
        assert!(cc.is_empty());
        let (to, cc) = participants_for_reply_all(&received, &me);
        assert_eq!(
            to.iter().map(|c| c.email.as_str()).collect::<Vec<_>>(),
            ["bob@y.test"]
        );
        assert_eq!(
            cc.iter().map(|c| c.email.as_str()).collect::<Vec<_>>(),
            ["carol@y.test", "dan@y.test"]
        );

        // From me: reply goes to the same recipients; a non-me Reply-To wins.
        let sent = json!({"from": [{"email": "work@x.test"}], "to": [{"email": "bob@y.test"}],
                          "cc": [], "replyTo": []});
        assert_eq!(participants_for_reply(&sent, &me).0[0].email, "bob@y.test");
        let list = json!({"from": [{"email": "bob@y.test"}], "to": [], "cc": [],
                          "replyTo": [{"email": "list@y.test"}]});
        assert_eq!(participants_for_reply(&list, &me).0[0].email, "list@y.test");
    }

    #[test]
    fn reply_from_alias_and_subject() {
        let s = sender();
        let to_alias = json!({"to": [{"name": "Work Me", "email": "work@x.test"}], "cc": []});
        assert_eq!(s.reply_from(&to_alias).email, "work@x.test");
        assert_eq!(
            s.reply_from(&json!({"to": [{"email": "me@x.test"}]})).email,
            "me@x.test"
        );
        assert_eq!(subject_with_prefix("Hello", "Re:"), "Re: Hello");
        assert_eq!(subject_with_prefix("RE: Hello", "Re:"), "Re: Hello");
        assert_eq!(subject_with_prefix("Fwd: Hello", "Re:"), "Re: Hello");
    }

    #[test]
    fn quoting() {
        let msg = json!({"date": 1_700_000_000, "from": [{"name": "Bob", "email": "bob@y.test"}]});
        assert!(attribution_line(&msg).starts_with("On Nov 1"));
        assert!(attribution_line(&msg).ends_with(", Bob <bob@y.test> wrote:"));
        let q = reply_quote(
            &msg,
            Some("<p>hi</p><script>x()</script><img src=\"cid:abc\"><img src=\"https://a/b.png\">"),
        );
        assert!(
            q.contains("<p>hi</p>") && q.contains("https://a/b.png"),
            "{q}"
        );
        assert!(!q.contains("script") && !q.contains("cid:"), "{q}");
        assert!(q.contains("Bob &lt;bob@y.test&gt; wrote:"));
        let plain = json!({"plaintext": true, "date": 0, "from": []});
        assert!(reply_quote(&plain, Some("a <b>\nc"))
            .contains("<div style=\"white-space: pre-wrap;\">a &lt;b&gt;<br>c</div>"));
    }

    #[test]
    fn new_draft_and_stored_draft_json() {
        let mut s = sender();
        s.meta.autoaddress = Some(("bcc".into(), "Log <log@x.test>, junk".into()));
        let d = new_draft_json(
            &s,
            NewDraft {
                to: vec![Recipient {
                    name: String::new(),
                    email: "bob@y.test".into(),
                }],
                cc: vec![],
                bcc: vec![],
                subject: "Hi".into(),
                body: "<p>x</p>".into(),
                from: s.default_me(),
                thread_id: Some("t1".into()),
                reply_to_header_message_id: Some("<orig@y.test>".into()),
            },
            1_700_000_000,
        );
        assert!(d.get("id").is_none());
        assert!(d["hMsgId"].as_str().unwrap().ends_with("@x.test"));
        assert_eq!(d["bcc"], json!([contact_json("Log", "log@x.test")]));
        assert_eq!(d["from"], json!([contact_json("Me", "me@x.test")]));
        assert_eq!(
            (&d["threadId"], &d["rthMsgId"], &d["object"], &d["pristine"]),
            (
                &json!("t1"),
                &json!("<orig@y.test>"),
                &json!("draft"),
                &json!(false)
            )
        );

        let stored = json!({"id": "d1", "aid": "a1", "hMsgId": "H@x.test", "threadId": "t1",
            "rthMsgId": "<orig@y.test>", "draft": true, "_sa": 5, "_suc": 0, "gMsgId": "",
            "files": [{"id": "f1", "filename": "a.pdf"}], "folders": {"fd": 4}});
        let j = desktop_message_json(&stored, Some("<p>new</p>"));
        assert_eq!(j["body"], "<p>new</p>");
        assert_eq!(j["file_ids"], json!(["f1"]));
        assert!(j.get("_sa").is_none() && j.get("gMsgId").is_none());
        assert_eq!(j["to"], json!([]));
        assert_eq!(j["plaintext"], false);
        assert_eq!(j["__cls"], "Message");
        assert!(desktop_message_json(&stored, None).get("body").is_none());
    }

    #[test]
    fn header_message_ids() {
        let h = new_header_message_id("Me@Example.test");
        assert!(h.ends_with("@Example.test") && h.len() > 40, "{h}");
        assert!(new_header_message_id("nobody").ends_with("@tern.local"));
    }
}
