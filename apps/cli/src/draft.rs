// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern draft new|reply|reply-all|forward|edit|delete|validate|send|cancel-send|scheduled|
//! list`: the engine's `draft.create`, `draft.validate`, `draft.send` and `draft.cancelSend`
//! (serve only, `draft-pipeline`), `draft.update` and `draft.delete` (`draft-editing`), a
//! `query` for the list and `metadata.list` for the scheduled sends.
//!
//! The text given goes above what the engine writes into a draft of that kind (the
//! signature, the quoted message). It is built once unsaved to read that part, then saved
//! with the text on top, so the engine's own save marks it written and pushes it. A reply
//! that finds the draft already answering the message keeps that draft's quote and
//! signature and replaces the text above them, and so does `edit`. Files are attached by
//! path: the engine copies them in.
//!
//! `send --at` writes the `send-later` metadata the desktop's Send Later writes, which the
//! engine's expiration executor sends when due; `send --undo` is `draft.send` with a delay,
//! the desktop's undo window.

use std::io::Read;
use std::path::PathBuf;

use serde_json::{json, Map, Value};
use tern_client::metadata::SEND_LATER;
use tern_client::tasks::TASK_CLASS;
use tern_ipc::{
    draft_kind, feature, method, DraftSendResult, DraftValidateResult, READ_DEFAULT_LIMIT,
};

use crate::app::{call, call_raw, is_not_found, Ctx};
use crate::cli::{DraftCommand, DraftContentArgs, DraftFieldArgs, DraftReplyArgs, FollowUpArgs};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::followup::{self, FollowUp};
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
        DraftCommand::Edit { id, fields, detach } => edit(ctx, &mut engines, &id, &fields, &detach),
        DraftCommand::Delete { id } => delete(ctx, &mut engines, &id),
        DraftCommand::Validate { id } => validate(ctx, &mut engines, &id),
        DraftCommand::Send {
            id,
            archive,
            wait,
            timeout,
            at,
            undo,
            follow_up,
        } => {
            let how = SendHow {
                archive,
                wait,
                timeout,
                at,
                undo,
            };
            send(ctx, engines, &id, &how, &follow_up)
        }
        DraftCommand::CancelSend { id } => cancel_send(ctx, &mut engines, &id),
        DraftCommand::Scheduled => scheduled(ctx, &mut engines),
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

fn read_text(content: &DraftFieldArgs) -> Result<Option<String>> {
    read_body(content.body.as_ref(), content.body_file.as_ref())
}

/// `--body`, else `--body-file` (`-` for stdin), trailing whitespace trimmed.
pub(crate) fn read_body(
    body: Option<&String>,
    body_file: Option<&PathBuf>,
) -> Result<Option<String>> {
    let text = match (body, body_file) {
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
pub(crate) fn text_to_html(text: &str) -> String {
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
    let follow_up = FollowUp::parse(&content.fields.follow_up)?;
    if follow_up.tracks() && content.plain {
        return Err(CliError::usage(
            "--track-opens and --track-links need an HTML draft; leave out --plain",
        ));
    }
    follow_up.check(engines, t)?;
    let attach = attach_paths(&content.fields.attach)?;
    if !attach.is_empty() {
        require(engines, t, feature::DRAFT_EDITING, "attach files")?;
    }
    let text = read_text(&content.fields)?;
    let mut fields = header_fields(&content.fields)?;
    let mut params = json!({ "kind": spec.kind });
    if let Some(name) = &content.template {
        require(
            engines,
            t,
            feature::TEMPLATES,
            "write drafts from templates",
        )?;
        params["template"] = json!(name);
    }
    if let Some(a) = spec.account_id {
        params["accountId"] = json!(a);
    }
    if let Some(m) = spec.message_id {
        params["messageId"] = json!(m);
    }
    if let Some(b) = spec.behavior {
        params["behavior"] = json!(b);
    }
    let html = content.fields.html;
    if html || follow_up.tracks() {
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
        fields.insert(
            "body".into(),
            json!(text_over(&text, kept, plaintext, html)),
        );
        // Saved as the draft just built: its Message-ID, account and format.
        fields.insert("hMsgId".into(), built["hMsgId"].clone());
        params["accountId"] = built["aid"].clone();
        params["html"] = json!(!plaintext);
    }
    params["fields"] = Value::Object(fields);
    if !attach.is_empty() {
        params["attach"] = json!(attach);
    }
    let draft: Value = call(engines, t, method::DRAFT_CREATE, params)?;
    let updated = is_stored(&draft);
    follow_up.apply(engines, t, &draft)?;
    if ctx.json {
        let mut out = json!({ "draft": draft, "updated": updated });
        follow_up.extend_json(&mut out);
        output::json(&out);
        return Ok(());
    }
    print_saved(&draft, if updated { "Updated" } else { "Saved" });
    follow_up.print();
    Ok(())
}

/// `--to`, `--cc`, `--bcc` and `--subject` as draft fields.
fn header_fields(content: &DraftFieldArgs) -> Result<Map<String, Value>> {
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
    Ok(fields)
}

/// The `--attach` paths as the absolute ones the engine copies from.
fn attach_paths(paths: &[PathBuf]) -> Result<Vec<String>> {
    paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .map(|p| p.display().to_string())
                .map_err(|e| CliError::usage(format!("--attach {}: {e}", p.display())))
        })
        .collect()
}

/// The new text on top of what a draft keeps below its text (signature, quote), as HTML in
/// an HTML draft unless it is HTML already.
fn text_over(text: &str, kept: &str, plaintext: bool, html: bool) -> String {
    if plaintext || html {
        format!("{text}{kept}")
    } else {
        format!("{}{kept}", text_to_html(text))
    }
}

/// `filename (size)` for each of a draft's files.
fn file_names(draft: &Value) -> Vec<String> {
    draft["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| match f["size"].as_u64() {
            Some(n) => format!("{} ({})", str_of(f, "filename"), output::bytes(n)),
            None => str_of(f, "filename").to_owned(),
        })
        .collect()
}

fn print_saved(draft: &Value, verb: &str) {
    let to = output::contacts(&draft["to"]);
    output::line(&format!(
        "{verb} draft {} in thread {}{}: {}",
        str_of(draft, "id"),
        str_of(draft, "threadId"),
        if to.is_empty() {
            String::new()
        } else {
            format!(" to {}", output::one_line(&to))
        },
        output::one_line(str_of(draft, "subject")),
    ));
    let files = file_names(draft);
    if !files.is_empty() {
        output::line(&format!("Attached: {}", files.join(", ")));
    }
}

fn edit(
    ctx: &Ctx,
    engines: &mut [Engine],
    id: &str,
    content: &DraftFieldArgs,
    detach: &[String],
) -> Result<()> {
    let attach = attach_paths(&content.attach)?;
    let text = read_text(content)?;
    let mut fields = header_fields(content)?;
    let follow_up = FollowUp::parse(&content.follow_up)?;
    let changes_draft =
        text.is_some() || !fields.is_empty() || !attach.is_empty() || !detach.is_empty();
    if !changes_draft && follow_up.is_empty() {
        return Err(CliError::usage(
            "nothing to change: give --to, --cc, --bcc, --subject, --body, --body-file, \
             --attach, --detach, --remind-in, --track-opens or --track-links",
        ));
    }
    let targets = discovery::targets(engines, ctx.account())?;
    let (t, hmid, draft_id) = find_draft(engines, &targets, id)?;
    follow_up.check(engines, &t)?;
    if !changes_draft {
        let draft = stored_draft(engines, &t, &draft_id)?;
        follow_up.apply(engines, &t, &draft)?;
        if ctx.json {
            let mut out = json!({ "draft": draft, "updated": true });
            follow_up.extend_json(&mut out);
            output::json(&out);
        } else {
            print_saved(&draft, "Updated");
            follow_up.print();
        }
        return Ok(());
    }
    require(engines, &t, feature::DRAFT_EDITING, "edit drafts")?;
    if let Some(text) = text {
        let r: Value = call(
            engines,
            &t,
            method::MESSAGES_GET,
            json!({ "accountId": t.account_id, "id": draft_id, "body": true }),
        )?;
        let body = match r["body"].as_str() {
            Some(b) => b.to_owned(),
            None => wait_for_body(engines, &t, &draft_id)?.ok_or_else(|| {
                CliError::failed(format!(
                    "the body of draft {id} hasn't downloaded; check the account is online \
                     with `tern status`"
                ))
            })?,
        };
        let plaintext = flag(&r["message"]["plaintext"]);
        if plaintext && content.html {
            return Err(CliError::usage(format!(
                "draft {id} is plain text; give the text without --html"
            )));
        }
        let kept = &body[text_end(&body, plaintext)..];
        fields.insert(
            "body".into(),
            json!(text_over(&text, kept, plaintext, content.html)),
        );
    }
    let draft: Value = call(
        engines,
        &t,
        method::DRAFT_UPDATE,
        json!({
            "accountId": t.account_id,
            "headerMessageId": hmid,
            "fields": fields,
            "attach": attach,
            "detach": detach,
        }),
    )?;
    follow_up.apply(engines, &t, &draft)?;
    if ctx.json {
        let mut out = json!({ "draft": draft, "updated": true });
        follow_up.extend_json(&mut out);
        output::json(&out);
    } else {
        print_saved(&draft, "Updated");
        follow_up.print();
    }
    Ok(())
}

/// The stored draft's JSON, as `messages.get` has it.
fn stored_draft(engines: &mut [Engine], t: &Target, draft_id: &str) -> Result<Value> {
    let r: Value = call(
        engines,
        t,
        method::MESSAGES_GET,
        json!({ "accountId": t.account_id, "id": draft_id }),
    )?;
    Ok(r["message"].clone())
}

fn delete(ctx: &Ctx, engines: &mut [Engine], id: &str) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let (t, hmid, draft_id) = find_draft(engines, &targets, id)?;
    require(engines, &t, feature::DRAFT_EDITING, "delete drafts")?;
    let r: tern_ipc::DraftDeleteResult = call(
        engines,
        &t,
        method::DRAFT_DELETE,
        json!({ "accountId": t.account_id, "headerMessageId": hmid }),
    )?;
    if ctx.json {
        output::json(&json!({
            "accountId": t.account_id,
            "id": draft_id,
            "headerMessageId": hmid,
            "taskId": r.task_id,
            "cancelledSend": r.cancelled_send,
        }));
    } else {
        output::line(&format!(
            "Deleted draft {draft_id}{}",
            if r.cancelled_send {
                ", and cancelled its send"
            } else {
                ""
            }
        ));
    }
    Ok(())
}

/// Whether `draft.create` answered with a draft already stored (a reply draft it found)
/// rather than one it built: the store's version starts at 1, a built draft's is 0.
fn is_stored(draft: &Value) -> bool {
    draft["v"].as_i64().unwrap_or(0) > 0
}

/// The stored draft `id` names, by its id or its Message-ID: its account, Message-ID and id.
pub(crate) fn find_draft(
    engines: &mut [Engine],
    targets: &[Target],
    id: &str,
) -> Result<(Target, String, String)> {
    let hmid = id.trim_start_matches('<').trim_end_matches('>');
    for t in targets {
        match call_raw::<Value>(
            engines,
            t,
            method::MESSAGES_GET,
            json!({ "accountId": t.account_id, "id": id }),
        ) {
            Ok(r) if flag(&r["message"]["draft"]) => {
                let m = &r["message"];
                return Ok((
                    t.clone(),
                    str_of(m, "hMsgId").to_owned(),
                    str_of(m, "id").to_owned(),
                ));
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
        if let Some(row) = r["rows"].as_array().and_then(|rows| rows.first()) {
            return Ok((t.clone(), hmid.to_owned(), str_of(row, "id").to_owned()));
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

fn check(engines: &mut [Engine], t: &Target, hmid: &str) -> Result<DraftValidateResult> {
    call(
        engines,
        t,
        method::DRAFT_VALIDATE,
        json!({ "accountId": t.account_id, "headerMessageId": hmid }),
    )
}

fn problems(list: &[tern_ipc::DraftProblem]) -> Vec<String> {
    list.iter().map(|p| p.message.clone()).collect()
}

fn cant_send(id: &str, checked: &DraftValidateResult) -> CliError {
    CliError::failed(format!(
        "draft {id} can't be sent: {}",
        problems(&checked.errors).join("; ")
    ))
}

/// `tern draft validate`: the checks `send` runs, without sending. Errors exit 1.
fn validate(ctx: &Ctx, engines: &mut [Engine], id: &str) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let (t, hmid, draft_id) = find_draft(engines, &targets, id)?;
    require(engines, &t, feature::DRAFT_PIPELINE, "check drafts")?;
    let checked = check(engines, &t, &hmid)?;
    if ctx.json {
        output::json(&json!({
            "accountId": t.account_id,
            "id": draft_id,
            "headerMessageId": hmid,
            "ok": checked.errors.is_empty(),
            "errors": checked.errors,
            "warnings": checked.warnings,
        }));
    }
    if !checked.errors.is_empty() {
        return Err(cant_send(id, &checked));
    }
    if !ctx.json {
        for w in problems(&checked.warnings) {
            output::warn(&w);
        }
        output::line(&format!("Draft {draft_id} can be sent"));
    }
    Ok(())
}

/// How `draft send` sends.
struct SendHow {
    archive: bool,
    wait: bool,
    timeout: Option<u64>,
    at: Option<String>,
    undo: Option<u64>,
}

fn send(
    ctx: &Ctx,
    mut engines: Vec<Engine>,
    id: &str,
    how: &SendHow,
    follow_up_args: &FollowUpArgs,
) -> Result<()> {
    let (wait, timeout) = (how.wait, how.timeout);
    let at = how
        .at
        .as_deref()
        .map(|s| followup::future("--at", s))
        .transpose()?;
    let follow_up = FollowUp::parse(follow_up_args)?;
    let targets = discovery::targets(&engines, ctx.account())?;
    let (t, hmid, draft_id) = find_draft(&mut engines, &targets, id)?;
    require(&engines, &t, feature::DRAFT_PIPELINE, "send drafts")?;
    follow_up.check(&engines, &t)?;
    let checked = check(&mut engines, &t, &hmid)?;
    if !checked.errors.is_empty() {
        return Err(cant_send(id, &checked));
    }
    let warnings = problems(&checked.warnings);
    // `--json` without `--wait` puts them in its output; `--wait` prints the task.
    if !ctx.json || wait {
        for w in &warnings {
            output::warn(w);
        }
    }
    let after = if how.archive {
        "send-and-archive"
    } else {
        "send"
    };
    if !follow_up.is_empty() {
        let draft = stored_draft(&mut engines, &t, &draft_id)?;
        follow_up.apply(&mut engines, &t, &draft)?;
    }
    let pending = Pending {
        t: &t,
        draft_id: &draft_id,
        hmid: &hmid,
        warnings: &warnings,
    };
    if let Some(at) = at {
        return send_later(ctx, &mut engines, &pending, at, after);
    }
    if let Some(secs) = how.undo {
        let r: DraftSendResult = call(
            &mut engines,
            &t,
            method::DRAFT_SEND,
            json!({ "accountId": t.account_id, "headerMessageId": hmid, "after": after,
                    "delayMs": secs * 1000 }),
        )?;
        let at = r.scheduled_at.unwrap_or_default();
        pending.report(ctx, at, json!({ "undo": true }));
        return Ok(());
    }
    if wait {
        // Before the send, on the same connection, so the task's last delta isn't missed.
        let params = tern_ipc::SubscribeParams {
            accounts: Some(vec![t.account_id.clone()]),
            classes: Some(vec![TASK_CLASS.to_owned()]),
            since: None,
        };
        engines[t.engine]
            .client
            .subscribe(&params)
            .map_err(|e| CliError::call(&t.email, method::SUBSCRIBE, e))?;
    }
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

/// A draft whose send waits: for `send --at` or `send --undo`.
struct Pending<'a> {
    t: &'a Target,
    draft_id: &'a str,
    hmid: &'a str,
    warnings: &'a [String],
}

impl Pending<'_> {
    fn report(&self, ctx: &Ctx, at: i64, extra: Value) {
        let id = self.draft_id;
        if ctx.json {
            let mut out = json!({
                "accountId": self.t.account_id,
                "id": id,
                "headerMessageId": self.hmid,
                "sendAt": at,
                "warnings": self.warnings,
            });
            if let (Some(o), Value::Object(e)) = (out.as_object_mut(), extra) {
                o.extend(e);
            }
            output::json(&out);
        } else {
            output::line(&format!(
                "Draft {id} sends {} (`tern draft cancel-send {id}` stops it until then)",
                output::full_date(at)
            ));
        }
    }
}

/// `send --at`: `SyncbackMetadataTask` with the `send-later` value the desktop's Send Later
/// and tern-mcp's `schedule_send` write, so Tern shows the draft as scheduled.
fn send_later(ctx: &Ctx, engines: &mut [Engine], p: &Pending, at: i64, after: &str) -> Result<()> {
    let draft = json!({ "id": p.draft_id, "aid": p.t.account_id, "hMsgId": p.hmid });
    let mut task = tern_client::tasks::syncback_message_metadata(
        &draft,
        SEND_LATER,
        json!({ "expiration": at, "actionKey": after }),
        Some(json!({ "expiration": null })),
    );
    crate::organize::stamp(&mut task);
    crate::organize::queue(engines, p.t, &task)?;
    if !engines[p.t.engine].has_feature(feature::EXPIRATION_EXECUTOR) {
        output::warn(
            "this engine leaves scheduled sends to the desktop app: Tern must be open then, or \
             the draft goes out when it next opens",
        );
    }
    p.report(ctx, at, json!({ "taskId": task["id"] }));
    Ok(())
}

fn cancel_send(ctx: &Ctx, engines: &mut [Engine], id: &str) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let (t, hmid, draft_id) = find_draft(engines, &targets, id)?;
    require(engines, &t, feature::DRAFT_PIPELINE, "cancel sends")?;
    let r: tern_ipc::DraftCancelSendResult = call(
        engines,
        &t,
        method::DRAFT_CANCEL_SEND,
        json!({ "accountId": t.account_id, "headerMessageId": hmid }),
    )?;
    if !r.cancelled {
        return Err(CliError::failed(format!(
            "draft {id} has no pending send, or its send has started and can't be stopped"
        )));
    }
    if ctx.json {
        output::json(&json!({
            "accountId": t.account_id,
            "id": draft_id,
            "headerMessageId": hmid,
            "cancelled": true,
        }));
    } else {
        output::line(&format!(
            "Cancelled the send of draft {draft_id}; it stays a draft"
        ));
    }
    Ok(())
}

/// One row of `draft scheduled`: `None` for a draft with nothing pending.
fn scheduled_row(d: &Value, account_id: &str) -> Option<Value> {
    let value = tern_client::metadata::plugin_value(d, SEND_LATER)?;
    let at = value["expiration"]
        .as_i64()
        .or_else(|| value["expiration"].as_f64().map(|f| f as i64));
    let error = value["error"].as_str().filter(|e| !e.is_empty());
    let state = match (at, error) {
        (Some(_), _) if flag(&value["isUndoSend"]) => "undo",
        (Some(_), _) => "scheduled",
        (None, Some(_)) => "failed",
        (None, None) => return None,
    };
    Some(json!({
        "accountId": account_id,
        "id": str_of(d, "id"),
        "headerMessageId": str_of(d, "hMsgId"),
        "threadId": str_of(d, "threadId"),
        "subject": str_of(d, "subject"),
        "to": d["to"],
        "state": state,
        "sendAt": at,
        "actionKey": value["actionKey"],
        "error": error,
    }))
}

fn scheduled(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    const PAGE: usize = 200;
    let targets = discovery::targets(engines, ctx.account())?;
    let mut rows: Vec<Value> = Vec::new();
    for t in &targets {
        require(engines, t, feature::METADATA_LIST, "list scheduled sends")?;
        let mut offset = 0;
        loop {
            let page: Value = call(
                engines,
                t,
                method::METADATA_LIST,
                json!({ "accounts": [t.account_id], "pluginId": SEND_LATER, "draft": true,
                        "limit": PAGE, "offset": offset }),
            )?;
            let models = page["models"].as_array().cloned().unwrap_or_default();
            offset += models.len();
            rows.extend(
                models
                    .iter()
                    .filter_map(|d| scheduled_row(d, &t.account_id)),
            );
            if models.len() < PAGE {
                break;
            }
        }
    }
    // Failed ones first, then soonest first.
    rows.sort_by_key(|r| r["sendAt"].as_i64().unwrap_or(i64::MIN));
    if ctx.json {
        output::json(&json!({ "scheduled": rows }));
        return Ok(());
    }
    if rows.is_empty() {
        output::line("No scheduled sends.");
        return Ok(());
    }
    let mut table = Table::new(vec!["SEND AT", "STATE", "TO", "SUBJECT", "ID"], 3);
    for r in &rows {
        let to = r["to"]
            .as_array()
            .map(|a| a.iter().map(output::contact_short).collect::<Vec<_>>())
            .unwrap_or_default()
            .join(", ");
        let state = match r["error"].as_str() {
            Some(e) => format!("failed: {}", output::one_line(e)),
            None => str_of(r, "state").to_owned(),
        };
        table.row(
            vec![
                r["sendAt"]
                    .as_i64()
                    .map(output::full_date)
                    .unwrap_or_else(|| "-".into()),
                state,
                output::one_line(&to),
                output::one_line(str_of(r, "subject")),
                str_of(r, "id").to_owned(),
            ],
            r["error"].is_string(),
        );
    }
    table.print(&ctx.style);
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
    let mut headers = vec!["DATE", "TO", "SUBJECT", "FILES", "THREAD", "ID"];
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
            d["files"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| str_of(f, "filename"))
                .collect::<Vec<_>>()
                .join(", "),
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
    fn scheduled_rows_say_what_is_pending() {
        let row = |value: Value| {
            let d = json!({"id": "d1", "hMsgId": "h@x", "subject": "S", "to": [],
                           "metadata": [{"pluginId": "send-later", "v": 2, "value": value}]});
            scheduled_row(&d, "a1")
        };
        let at = row(json!({"expiration": 100, "actionKey": "send"})).unwrap();
        assert_eq!(
            (&at["state"], &at["sendAt"]),
            (&json!("scheduled"), &json!(100))
        );
        let undo = row(json!({"expiration": 100.0, "isUndoSend": true})).unwrap();
        assert_eq!(
            (&undo["state"], &undo["sendAt"]),
            (&json!("undo"), &json!(100))
        );
        let failed = row(json!({"expiration": null, "error": "no body"})).unwrap();
        assert_eq!(
            (&failed["state"], &failed["error"]),
            (&json!("failed"), &json!("no body"))
        );
        assert!(row(json!({"expiration": null, "isUndoSend": true})).is_none());
        assert!(scheduled_row(&json!({"id": "d2"}), "a1").is_none());
    }

    #[test]
    fn stored_drafts_have_a_version() {
        assert!(is_stored(&json!({"v": 3})));
        assert!(!is_stored(&json!({"v": 0})));
        assert!(!is_stored(&json!({})));
    }
}
