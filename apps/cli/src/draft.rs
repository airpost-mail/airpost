// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern draft new|reply|reply-all|forward|send|list`: the engine's `draft.create`,
//! `draft.validate` and `draft.send` (serve only, `draft-pipeline`), and a `query` for the
//! list.
//!
//! The text given goes above what the engine writes into a draft of that kind (the
//! signature, the quoted message). It is built once unsaved to read that part, then saved
//! with the text on top, so the engine's own save marks it written and pushes it. A reply
//! that finds the draft already answering the message keeps that draft's quote and
//! signature and replaces the text above them.

use std::io::Read;

use serde_json::{json, Map, Value};
use tern_client::tasks::TASK_CLASS;
use tern_ipc::{draft_kind, feature, method, DraftSendResult, READ_DEFAULT_LIMIT};

use crate::app::{call, call_raw, is_not_found, Ctx};
use crate::cli::{DraftCommand, DraftContentArgs, DraftReplyArgs};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::mail_actions::require;
use crate::output::{self, Table};
use crate::read::{find_shown, wait_for_body, Shown};

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

/// Flags come as booleans or as 0/1.
fn flag(v: &Value) -> bool {
    v.as_bool().unwrap_or_else(|| v.as_i64().unwrap_or(0) != 0)
}

pub fn run(ctx: &Ctx, mut engines: Vec<Engine>, command: DraftCommand) -> Result<()> {
    match command {
        DraftCommand::New { content } => new(ctx, &mut engines, &content),
        DraftCommand::Reply(args) => reply(ctx, &mut engines, draft_kind::REPLY, &args),
        DraftCommand::ReplyAll(args) => reply(ctx, &mut engines, draft_kind::REPLY_ALL, &args),
        DraftCommand::Forward { id, content } => {
            let targets = discovery::targets(&engines, ctx.account())?;
            let (t, message_id) = answered(&mut engines, &targets, &id)?;
            let spec = Call {
                kind: draft_kind::FORWARD,
                account_id: Some(&t.account_id),
                message_id: Some(&message_id),
                behavior: None,
            };
            write(ctx, &mut engines, &t, &spec, &content)
        }
        DraftCommand::Send {
            id,
            archive,
            wait,
            timeout,
        } => send(ctx, engines, &id, archive, wait, timeout),
        DraftCommand::List { limit } => list(ctx, &mut engines, limit),
    }
}

/// One `draft.create`, before its fields.
struct Call<'a> {
    kind: &'a str,
    account_id: Option<&'a str>,
    message_id: Option<&'a str>,
    behavior: Option<&'a str>,
}

fn new(ctx: &Ctx, engines: &mut [Engine], content: &DraftContentArgs) -> Result<()> {
    // Without --account the engine writes from the default sending account.
    let t = match ctx.account() {
        Some(_) => discovery::one_target(engines, ctx.account())?,
        None => {
            let targets = discovery::targets(engines, None)?;
            targets
                .iter()
                .find(|t| engines[t.engine].has_feature(feature::DRAFT_PIPELINE))
                .unwrap_or(&targets[0])
                .clone()
        }
    };
    let spec = Call {
        kind: draft_kind::NEW,
        account_id: ctx.account().map(|_| t.account_id.as_str()),
        message_id: None,
        behavior: None,
    };
    write(ctx, engines, &t, &spec, content)
}

fn reply(ctx: &Ctx, engines: &mut [Engine], kind: &str, args: &DraftReplyArgs) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let (t, message_id) = answered(engines, &targets, &args.id)?;
    let spec = Call {
        kind,
        account_id: Some(&t.account_id),
        message_id: Some(&message_id),
        behavior: (!args.no_prefer_existing).then_some("prefer-existing"),
    };
    write(ctx, engines, &t, &spec, &args.content)
}

/// The message a reply or forward answers: `id` itself, or the newest message of thread `id`
/// that isn't a draft; with its body downloaded, which the quote needs.
fn answered(engines: &mut [Engine], targets: &[Target], id: &str) -> Result<(Target, String)> {
    let (shown, t) = find_shown(engines, targets, id, false)?;
    let message = match shown {
        Shown::Message(r) => r["message"].clone(),
        Shown::Thread(r) => r["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| !flag(&m["draft"]))
            .max_by_key(|m| m["date"].as_i64().unwrap_or(0))
            .cloned()
            .ok_or_else(|| {
                CliError::failed(format!("thread {id} has only drafts; nothing to answer"))
            })?,
    };
    if flag(&message["draft"]) {
        return Err(CliError::usage(format!(
            "{id} is a draft; answer a message you received, or send it with `tern draft send`"
        )));
    }
    require(engines, &t, feature::DRAFT_PIPELINE, "write drafts")?;
    let message_id = str_of(&message, "id").to_owned();
    let r: Value = call(
        engines,
        &t,
        method::MESSAGES_GET,
        json!({ "accountId": t.account_id, "id": message_id, "body": true }),
    )?;
    if r["body"].is_null() && wait_for_body(engines, &t, &message_id)?.is_none() {
        return Err(CliError::failed(format!(
            "the body of {message_id} hasn't downloaded; check the account is online with \
             `tern status`"
        )));
    }
    Ok((t, message_id))
}

fn read_text(content: &DraftContentArgs) -> Result<Option<String>> {
    let text = match (&content.body, &content.body_file) {
        (Some(b), _) => b.clone(),
        (None, Some(path)) if path.as_os_str() == "-" => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).map_err(|e| {
                CliError::failed(format!("could not read the body from stdin: {e}"))
            })?;
            s
        }
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| {
            CliError::failed(format!(
                "could not read the body from {}: {e}",
                path.display()
            ))
        })?,
        (None, None) => return Ok(None),
    };
    Ok(Some(text.trim_end().to_owned()))
}

/// The --to/--cc/--bcc values: comma- or semicolon-separated, outside quotes and `<>`.
fn split_addresses(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for v in values {
        let mut cur = String::new();
        let (mut quoted, mut angle) = (false, false);
        for c in v.chars() {
            match c {
                '"' => quoted = !quoted,
                '<' if !quoted => angle = true,
                '>' if !quoted => angle = false,
                ',' | ';' if !quoted && !angle => {
                    out.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            }
            cur.push(c);
        }
        out.push(cur);
    }
    out.into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `address` or `Name <address>` as a contact.
fn parse_address(s: &str) -> Option<Value> {
    let (name, email) = match s.rfind('<') {
        Some(open) => {
            let close = open + s[open..].find('>')?;
            (
                s[..open].trim().trim_matches('"').trim(),
                s[open + 1..close].trim(),
            )
        }
        None => ("", s.trim()),
    };
    let valid = email.contains('@') && !email.contains(char::is_whitespace);
    valid.then(|| json!({ "name": name, "email": email }))
}

fn contacts(flag: &str, values: &[String]) -> Result<Value> {
    split_addresses(values)
        .iter()
        .map(|s| {
            parse_address(s).ok_or_else(|| {
                CliError::usage(format!(
                    "--{flag} {s:?} isn't an address; give name@example.com or \
                     \"Name <name@example.com>\""
                ))
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(Value::Array)
}

/// Plain text as the HTML a composer writes for it.
fn text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br/>"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Where the text written above a draft's signature and quoted message ends: the blank
/// lines before them stay with them. The markers are the ones the draft builders and the
/// desktop's composer write (`<signature>`, `gmail_quote`; in plain text the attribution
/// line above the first `>` line, or the forwarded-message line).
fn text_end(body: &str, plaintext: bool) -> usize {
    if plaintext {
        let mut offset = 0;
        let mut previous: Option<(usize, &str)> = None;
        for line in body.split_inclusive('\n') {
            if line.starts_with('>') {
                let start = match previous {
                    Some((at, prev)) if !prev.trim().is_empty() => at,
                    _ => offset,
                };
                return body[..start].trim_end().len();
            }
            if line.starts_with("---------- ") {
                return body[..offset].trim_end().len();
            }
            previous = Some((offset, line));
            offset += line.len();
        }
        return body.trim_end().len();
    }
    let lower = body.to_ascii_lowercase();
    let signature = lower.find("<signature");
    let quote = lower
        .find("gmail_quote")
        .and_then(|at| lower[..at].rfind('<'));
    let end = signature
        .into_iter()
        .chain(quote)
        .min()
        .unwrap_or(body.len());
    let mut head = &body[..end];
    loop {
        let trimmed = head.trim_end();
        let lower = trimmed.to_ascii_lowercase();
        match ["<br/>", "<br>", "<br />"]
            .iter()
            .find(|br| lower.ends_with(*br))
        {
            Some(br) => head = &trimmed[..trimmed.len() - br.len()],
            None => {
                head = trimmed;
                break;
            }
        }
    }
    head.len()
}

fn write(
    ctx: &Ctx,
    engines: &mut [Engine],
    t: &Target,
    spec: &Call,
    content: &DraftContentArgs,
) -> Result<()> {
    require(engines, t, feature::DRAFT_PIPELINE, "write drafts")?;
    let text = read_text(content)?;
    let mut fields = Map::new();
    for (key, values) in [
        ("to", &content.to),
        ("cc", &content.cc),
        ("bcc", &content.bcc),
    ] {
        if !values.is_empty() {
            fields.insert(key.into(), contacts(key, values)?);
        }
    }
    if let Some(s) = &content.subject {
        fields.insert("subject".into(), json!(s));
    }
    let mut params = json!({ "kind": spec.kind });
    if let Some(a) = spec.account_id {
        params["accountId"] = json!(a);
    }
    if let Some(m) = spec.message_id {
        params["messageId"] = json!(m);
    }
    if let Some(b) = spec.behavior {
        params["behavior"] = json!(b);
    }
    if content.html {
        params["html"] = json!(true);
    } else if content.plain {
        params["html"] = json!(false);
    }
    if let Some(text) = text {
        let mut unsaved = params.clone();
        unsaved["save"] = json!(false);
        let built: Value = call(engines, t, method::DRAFT_CREATE, unsaved)?;
        let plaintext = flag(&built["plaintext"]);
        let body = str_of(&built, "body");
        let kept = if is_stored(&built) {
            &body[text_end(body, plaintext)..]
        } else {
            body
        };
        let text = if plaintext || content.html {
            text
        } else {
            text_to_html(&text)
        };
        fields.insert("body".into(), json!(format!("{text}{kept}")));
        // Saved as the draft just built: its Message-ID, account and format.
        fields.insert("hMsgId".into(), built["hMsgId"].clone());
        params["accountId"] = built["aid"].clone();
        params["html"] = json!(!plaintext);
    }
    params["fields"] = Value::Object(fields);
    let draft: Value = call(engines, t, method::DRAFT_CREATE, params)?;
    let updated = is_stored(&draft);
    if ctx.json {
        output::json(&json!({ "draft": draft, "updated": updated }));
        return Ok(());
    }
    let to = output::contacts(&draft["to"]);
    output::line(&format!(
        "{} draft {} in thread {}{}: {}",
        if updated { "Updated" } else { "Saved" },
        str_of(&draft, "id"),
        str_of(&draft, "threadId"),
        if to.is_empty() {
            String::new()
        } else {
            format!(" to {}", output::one_line(&to))
        },
        output::one_line(str_of(&draft, "subject")),
    ));
    Ok(())
}

/// Whether `draft.create` answered with a draft already stored (a reply draft it found)
/// rather than one it built: the store's version starts at 1, a built draft's is 0.
fn is_stored(draft: &Value) -> bool {
    draft["v"].as_i64().unwrap_or(0) > 0
}

/// The stored draft `id` names, by its id or its Message-ID, and its Message-ID.
fn find_draft(engines: &mut [Engine], targets: &[Target], id: &str) -> Result<(Target, String)> {
    let hmid = id.trim_start_matches('<').trim_end_matches('>');
    for t in targets {
        match call_raw::<Value>(
            engines,
            t,
            method::MESSAGES_GET,
            json!({ "accountId": t.account_id, "id": id }),
        ) {
            Ok(r) if flag(&r["message"]["draft"]) => {
                return Ok((t.clone(), str_of(&r["message"], "hMsgId").to_owned()))
            }
            Ok(_) => {
                return Err(CliError::usage(format!(
                    "{id} isn't a draft; `tern draft list` lists them"
                )))
            }
            Err(e) if is_not_found(&e) => {}
            Err(e) => return Err(CliError::call(&t.email, method::MESSAGES_GET, e)),
        }
        if !hmid.contains('@') || !engines[t.engine].has_feature(feature::QUERY) {
            continue;
        }
        let r: Value = call(
            engines,
            t,
            method::QUERY,
            json!({
                "class": "Message",
                "accounts": [t.account_id],
                "where": [
                    {"attr": "headerMessageId", "op": "=", "value": hmid},
                    {"attr": "draft", "op": "=", "value": true},
                ],
                "limit": 1,
            }),
        )?;
        if r["rows"].as_array().is_some_and(|rows| !rows.is_empty()) {
            return Ok((t.clone(), hmid.to_owned()));
        }
    }
    Err(CliError::failed(format!(
        "no draft {id} in {}; `tern draft list` lists them",
        match targets {
            [one] => one.email.as_str(),
            _ => "any account",
        }
    )))
}

fn send(
    ctx: &Ctx,
    mut engines: Vec<Engine>,
    id: &str,
    archive: bool,
    wait: bool,
    timeout: Option<u64>,
) -> Result<()> {
    let targets = discovery::targets(&engines, ctx.account())?;
    let (t, hmid) = find_draft(&mut engines, &targets, id)?;
    require(&engines, &t, feature::DRAFT_PIPELINE, "send drafts")?;
    let checked: Value = call(
        &mut engines,
        &t,
        method::DRAFT_VALIDATE,
        json!({ "accountId": t.account_id, "headerMessageId": hmid }),
    )?;
    let messages = |key: &str| -> Vec<String> {
        checked[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| str_of(p, "message").to_owned())
            .collect()
    };
    let errors = messages("errors");
    if !errors.is_empty() {
        return Err(CliError::failed(format!(
            "draft {id} can't be sent: {}",
            errors.join("; ")
        )));
    }
    let warnings = messages("warnings");
    // `--json` without `--wait` puts them in its output; `--wait` prints the task.
    if !ctx.json || wait {
        for w in &warnings {
            output::warn(w);
        }
    }
    if wait {
        // Before the send, on the same connection, so the task's last delta isn't missed.
        let params = tern_ipc::SubscribeParams {
            accounts: Some(vec![t.account_id.clone()]),
            classes: Some(vec![TASK_CLASS.to_owned()]),
        };
        engines[t.engine]
            .client
            .subscribe(&params)
            .map_err(|e| CliError::call(&t.email, method::SUBSCRIBE, e))?;
    }
    let after = if archive { "send-and-archive" } else { "send" };
    let r: DraftSendResult = call(
        &mut engines,
        &t,
        method::DRAFT_SEND,
        json!({ "accountId": t.account_id, "headerMessageId": hmid, "after": after }),
    )?;
    let task_id = r.task_id.unwrap_or_default();
    if wait {
        let engine = engines.swap_remove(t.engine);
        let subs = crate::watch::Subscriptions::start(vec![(engine, true)], None, None)?;
        return crate::tasks::wait_on(ctx, subs, &task_id, timeout);
    }
    if ctx.json {
        output::json(&json!({
            "accountId": t.account_id,
            "headerMessageId": hmid,
            "taskId": task_id,
            "warnings": warnings,
        }));
    } else {
        output::line(&format!(
            "Sending draft {hmid} (task {task_id}; `tern task wait {task_id}` waits for it)"
        ));
    }
    Ok(())
}

fn list(ctx: &Ctx, engines: &mut [Engine], limit: Option<usize>) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let limit = limit.unwrap_or(READ_DEFAULT_LIMIT);
    let mut drafts: Vec<(Value, String)> = Vec::new();
    for t in &targets {
        require(engines, t, feature::QUERY, "list drafts")?;
        let mut r: Value = call(
            engines,
            t,
            method::QUERY,
            json!({
                "class": "Message",
                "accounts": [t.account_id],
                "where": {"attr": "draft", "op": "=", "value": true},
                "order": [{"attr": "date", "dir": "desc"}],
                "limit": limit,
            }),
        )?;
        if let Value::Array(rows) = r["rows"].take() {
            drafts.extend(rows.into_iter().map(|d| (d, t.email.clone())));
        }
    }
    drafts.sort_by_key(|(d, _)| std::cmp::Reverse(d["date"].as_i64().unwrap_or(0)));
    if ctx.json {
        let drafts: Vec<Value> = drafts.into_iter().map(|(d, _)| d).collect();
        output::json(&json!({ "drafts": drafts }));
        return Ok(());
    }
    if drafts.is_empty() {
        output::line("No drafts.");
        return Ok(());
    }
    let multi = targets.len() > 1;
    let mut headers = vec!["DATE", "TO", "SUBJECT", "THREAD", "ID"];
    if multi {
        headers.insert(0, "ACCOUNT");
    }
    let mut table = Table::new(headers, usize::from(multi) + 2);
    for (d, email) in &drafts {
        let to = d["to"]
            .as_array()
            .map(|a| a.iter().map(output::contact_short).collect::<Vec<_>>())
            .unwrap_or_default()
            .join(", ");
        let mut cells = vec![
            output::list_date(d["date"].as_i64().unwrap_or(0)),
            output::one_line(&to),
            output::one_line(str_of(d, "subject")),
            str_of(d, "threadId").to_owned(),
            str_of(d, "id").to_owned(),
        ];
        if multi {
            cells.insert(0, email.clone());
        }
        table.row(cells, false);
    }
    table.print(&ctx.style);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_split_outside_quotes_and_brackets() {
        let values = [
            "ann@example.net, \"Doe, John\" <john@example.net>".to_owned(),
            "bob@example.net;carol@example.net".to_owned(),
        ];
        let split = split_addresses(&values);
        assert_eq!(
            split,
            [
                "ann@example.net",
                "\"Doe, John\" <john@example.net>",
                "bob@example.net",
                "carol@example.net"
            ]
        );
        assert_eq!(
            parse_address(&split[1]),
            Some(json!({"name": "Doe, John", "email": "john@example.net"}))
        );
        assert_eq!(
            parse_address("ann@example.net"),
            Some(json!({"name": "", "email": "ann@example.net"}))
        );
        assert!(parse_address("Ann").is_none());
        assert!(parse_address("Ann <ann@example.net").is_none());
        assert!(contacts("to", &["nobody".to_owned()]).is_err());
    }

    #[test]
    fn text_becomes_html() {
        assert_eq!(
            text_to_html("a < b & c\r\nnext \"line\""),
            "a &lt; b &amp; c<br/>next &quot;line&quot;"
        );
    }

    #[test]
    fn the_text_above_the_signature_and_quote_is_found() {
        let reply = "<p>Old text</p>\n        <br/>\n        <br/><signature id=\"s1\">Me</signature>\n        <div class=\"gmail_quote_attribution\">On Monday</div><blockquote class=\"gmail_quote\">x</blockquote>";
        let end = text_end(reply, false);
        assert_eq!(&reply[..end], "<p>Old text</p>");
        assert!(reply[end..].trim_start().starts_with("<br/>"));
        let no_sig = "Hi<br><div class=\"gmail_quote\">q</div>";
        assert_eq!(&no_sig[..text_end(no_sig, false)], "Hi");
        assert_eq!(text_end("<br/>", false), 0);
        assert_eq!(text_end("Just text", false), "Just text".len());

        let plain = "Old text\n\nOn Monday, Bob wrote:\n> Lunch?\n";
        assert_eq!(&plain[..text_end(plain, true)], "Old text");
        let forward = "Note\n\n---------- Forwarded Message ---------\n\nFrom: Bob\n";
        assert_eq!(&forward[..text_end(forward, true)], "Note");
        assert_eq!(text_end("\n\nOn Monday, Bob wrote:\n> x", true), 0);
        assert_eq!(text_end("Only\n", true), 4);
    }

    #[test]
    fn stored_drafts_have_a_version() {
        assert!(is_stored(&json!({"v": 3})));
        assert!(!is_stored(&json!({"v": 0})));
        assert!(!is_stored(&json!({})));
    }
}
