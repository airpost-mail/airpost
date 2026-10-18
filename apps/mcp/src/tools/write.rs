// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The tools that queue tasks: drafts, sending, and thread changes.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{
    failed, opt_recipients, opt_str, opt_str_list, recipients_schema, req_bool, req_str,
    req_str_list, schema, string_list_schema, Ctx, Output, ToolDef, ToolError, ToolResult,
};
use crate::discovery::AccountRef;
use crate::drafts::{self, NewDraft};
use crate::model::{self, array_of, i64_of, str_of};
use crate::policy::Category;
use crate::tasks;

const SEND_LATER_ID: &str = "send-later";
const DEFAULT_SEND_ACTION_KEY: &str = "send";
/// The after-send actions the engine's expiration executor performs itself
/// (ms-sync `expiration_executor::SEND_ACTIONS`).
const ENGINE_SEND_ACTIONS: [&str; 2] = ["send", "send-and-archive"];
/// Draft threads `find_draft` looks through when a Message-ID isn't the id the engine
/// gives drafts created on this device.
const DRAFT_SCAN: usize = 100;

pub(super) fn definitions() -> Vec<ToolDef> {
    let draft_ref = json!({
        "id": {"type": "string", "description": "The draft's message id (from get_thread, or the result of create_draft / reply_to_thread)"},
        "headerMessageId": {"type": "string", "description": "The draft's headerMessageId; used when id is not given"},
        "accountId": {"type": "string", "description": "Optional account ID the draft belongs to"}
    });
    let mut update = draft_ref.clone();
    for (k, v) in [
        (
            "subject",
            json!({"type": "string", "description": "New subject"}),
        ),
        (
            "body",
            json!({"type": "string", "description": "New body (HTML). Replaces the whole body, including any quoted text, so include the quote when keeping it"}),
        ),
        (
            "to",
            recipients_schema("New recipients; replaces the current ones"),
        ),
        (
            "cc",
            recipients_schema("New CC recipients; replaces the current ones"),
        ),
        (
            "bcc",
            recipients_schema("New BCC recipients; replaces the current ones"),
        ),
    ] {
        update[k] = v;
    }
    vec![
        ToolDef {
            name: "create_draft",
            description: "Create a new email draft (does not send it)",
            schema: schema(
                json!({
                    "to": recipients_schema("Recipients"),
                    "cc": recipients_schema("CC recipients"),
                    "bcc": recipients_schema("BCC recipients"),
                    "subject": {"type": "string", "description": "Email subject"},
                    "body": {"type": "string", "description": "Email body (HTML)"},
                    "accountId": {"type": "string", "description": "Account ID to send from (uses default if omitted)"}
                }),
                &["to", "subject", "body"],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "reply_to_thread",
            description: "Create a reply draft in a thread (does not send it). It replies to the newest message in the thread that is not a draft (or to `messageId`), quoting it and addressing its sender (and, for reply-all, its other recipients). The result lists the thread's existing unsent drafts: to change a reply you already drafted, call update_draft on it instead of creating another.",
            schema: schema(
                json!({
                    "threadId": {"type": "string", "description": "Thread ID to reply to"},
                    "body": {"type": "string", "description": "Reply body (HTML)"},
                    "type": {"type": "string", "enum": ["reply", "reply-all"], "default": "reply", "description": "Reply type (default: reply)"},
                    "messageId": {"type": "string", "description": "Optional ID of the message in the thread to reply to (not a draft); defaults to the newest received or sent message"}
                }),
                &["threadId", "body"],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "update_draft",
            description: "Edit an existing draft: change its subject, body, or To/Cc/Bcc. The draft keeps its identity (id, headerMessageId, thread, the message it replies to, attachments). Use this to revise a reply drafted with reply_to_thread. Drafts that are being sent can't be edited.",
            schema: schema(update, &[]),
            category: Category::Write,
        },
        ToolDef {
            name: "delete_draft",
            description: "Delete an unsent draft",
            schema: schema(draft_ref, &[]),
            category: Category::Write,
        },
        ToolDef {
            name: "archive_threads",
            description: "Archive one or more threads",
            schema: schema(json!({"threadIds": string_list_schema("Thread IDs to archive")}), &["threadIds"]),
            category: Category::Write,
        },
        ToolDef {
            name: "trash_threads",
            description: "Move one or more threads to trash",
            schema: schema(json!({"threadIds": string_list_schema("Thread IDs to trash")}), &["threadIds"]),
            category: Category::Write,
        },
        ToolDef {
            name: "move_to_folder",
            description: "Move threads to a specific folder",
            schema: schema(
                json!({
                    "threadIds": string_list_schema("Thread IDs to move"),
                    "folderId": {"type": "string", "description": "Target folder ID"}
                }),
                &["threadIds", "folderId"],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "set_labels",
            description: "Add or remove labels from threads (Gmail only)",
            schema: schema(
                json!({
                    "threadIds": string_list_schema("Thread IDs"),
                    "labelsToAdd": string_list_schema("Label IDs to add"),
                    "labelsToRemove": string_list_schema("Label IDs to remove")
                }),
                &["threadIds"],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "set_unread",
            description: "Mark threads as read or unread",
            schema: schema(
                json!({
                    "threadIds": string_list_schema("Thread IDs"),
                    "unread": {"type": "boolean", "description": "true to mark unread, false to mark read"}
                }),
                &["threadIds", "unread"],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "set_starred",
            description: "Star or unstar threads",
            schema: schema(
                json!({
                    "threadIds": string_list_schema("Thread IDs"),
                    "starred": {"type": "boolean", "description": "true to star, false to unstar"}
                }),
                &["threadIds", "starred"],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "send_draft",
            description: "Send an existing draft by its headerMessageId",
            schema: schema(
                json!({"headerMessageId": {"type": "string", "description": "The headerMessageId of the draft to send"}}),
                &["headerMessageId"],
            ),
            category: Category::Send,
        },
        ToolDef {
            name: "schedule_send",
            description: "Schedule an existing draft to be sent automatically at a future date/time, equivalent to the \"Send Later\" composer button. The Tern engine sends it at that time with no window open (the result's note says when the desktop app must send it instead: an older engine, or an actionKey other than send / send-and-archive). Use cancel_scheduled_send to undo, and list_scheduled_sends to see whether it went out or failed.",
            schema: schema(
                json!({
                    "headerMessageId": {"type": "string", "description": "The headerMessageId of the draft to schedule"},
                    "sendAt": {"type": "string", "description": "ISO 8601 date-time in the future to send the draft at; without an offset it is local time"},
                    "actionKey": {"type": "string", "description": "Optional send action key to use when the draft is sent (defaults to send). The engine performs send and send-and-archive; other keys are plugin actions the desktop app performs"}
                }),
                &["headerMessageId", "sendAt"],
            ),
            category: Category::Send,
        },
        ToolDef {
            name: "cancel_scheduled_send",
            description: "Cancel a pending \"Send Later\" schedule on a draft so it will not be sent automatically. Errors if the draft has no active schedule.",
            schema: schema(
                json!({"headerMessageId": {"type": "string", "description": "The headerMessageId of the draft"}}),
                &["headerMessageId"],
            ),
            category: Category::Write,
        },
    ]
}

/// The id the engine gives a draft created on this device (`ms_core::ids::draft_id`):
/// base58 of the first 30 bytes of sha256(`accountId:headerMessageId`).
pub(crate) fn draft_id(account_id: &str, header_message_id: &str) -> String {
    let digest = Sha256::digest(format!("{account_id}:{header_message_id}").as_bytes());
    bs58::encode(&digest[..30])
        .with_alphabet(bs58::Alphabet::BITCOIN)
        .into_string()
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

// ── Drafts ──────────────────────────────────────────────────────────────────

const DEFAULT_SEND_ACCOUNT: &str = "core.sending.defaultAccountIdForSend";

/// `core.sending.defaultAccountIdForSend`: from the engine's settings when an engine keeps
/// them, which then hold it for every client (unset means none), else from `config.json`.
fn default_send_account(ctx: &mut Ctx<'_>) -> Option<String> {
    match ctx.backend.settings(&[DEFAULT_SEND_ACCOUNT]) {
        Some(values) => values
            .get(DEFAULT_SEND_ACCOUNT)
            .and_then(Value::as_str)
            .map(str::to_owned),
        None => ctx.desktop.default_send_account(),
    }
}

/// `DraftFactory._accountForNewDraft` without a UI: the default sending account when it is
/// running, else the first running account.
fn account_for_new_draft(ctx: &mut Ctx<'_>) -> Result<AccountRef, ToolError> {
    let all = ctx.backend.accounts();
    if all.is_empty() {
        return Err(failed(ctx.no_engine.clone()));
    }
    let default = default_send_account(ctx);
    Ok(default
        .and_then(|d| all.iter().find(|a| a.account_id == d).cloned())
        .unwrap_or_else(|| all[0].clone()))
}

fn draft_result(task: &Value, extra: Value) -> Value {
    let draft = &task["draft"];
    let mut out = json!({
        "id": str_of(draft, "id"),
        "headerMessageId": str_of(draft, "hMsgId"),
        "accountId": str_of(draft, "aid"),
        "threadId": draft.get("threadId").filter(|t| t.as_str().is_some_and(|s| !s.is_empty())).cloned().unwrap_or(Value::Null),
        "subject": str_of(draft, "subject"),
        "draft": true,
    });
    if let (Value::Object(o), Value::Object(e)) = (&mut out, extra) {
        o.extend(e);
    }
    out
}

pub(super) fn create_draft(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let to = opt_recipients(args, "to")?.ok_or_else(|| {
        failed("Invalid arguments: 'to' must be an array of {name?, email} objects")
    })?;
    let cc = opt_recipients(args, "cc")?.unwrap_or_default();
    let bcc = opt_recipients(args, "bcc")?.unwrap_or_default();
    let subject = req_str(args, "subject")?;
    let body = req_str(args, "body")?;
    // Checked after the default is picked, so omitting accountId can't reach a disabled
    // default account.
    let account = match opt_str(args, "accountId")? {
        Some(id) => ctx.account(&id)?,
        None => {
            let a = account_for_new_draft(ctx)?;
            if !ctx.policy.account_allowed(&a.account_id) {
                return Err(ToolError::Denied(format!(
                    "Access denied: account '{}' is not enabled for MCP access.",
                    a.account_id
                )));
            }
            a
        }
    };
    let sender = ctx.sender(&account);
    let draft = drafts::new_draft_json(
        &sender,
        NewDraft {
            to,
            cc,
            bcc,
            subject,
            body,
            from: sender.default_me(),
            thread_id: None,
            reply_to_header_message_id: None,
        },
        now_secs(),
    );
    let task = ctx.queue(&account.account_id, tasks::syncback_draft(draft))?;
    Ok(Output::text(&draft_result(&task, json!({}))).with_account(&account.account_id))
}

/// The message a reply answers: `messageId` when given (never a draft), else the newest
/// message that is neither a draft nor hidden. Drafts are the user's own unsent text, so
/// replying to one would quote and address the user.
fn reply_target<'a>(
    ctx: &Ctx<'_>,
    messages: &'a [Value],
    message_id: Option<&str>,
) -> Result<&'a Value, ToolError> {
    if let Some(id) = message_id {
        let m = messages
            .iter()
            .find(|m| str_of(m, "id") == id && ctx.policy.message_allowed(m))
            .ok_or_else(|| failed(format!("Message '{id}' is not in this thread")))?;
        if model::is_draft(m) {
            return Err(failed(format!(
                "Message '{id}' is an unsent draft, not a message to reply to; edit it with \
                 update_draft instead"
            )));
        }
        if model::is_hidden(m) {
            return Err(failed(format!("Message '{id}' is not in this thread")));
        }
        return Ok(m);
    }
    messages
        .iter()
        .rev()
        .find(|m| !model::is_draft(m) && !model::is_hidden(m) && ctx.policy.message_allowed(m))
        .ok_or_else(|| failed("No message to reply to in this thread (it has only drafts)"))
}

pub(super) fn reply_to_thread(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let thread_id = req_str(args, "threadId")?;
    let body = req_str(args, "body")?;
    let kind = opt_str(args, "type")?.unwrap_or_else(|| "reply".into());
    if kind != "reply" && kind != "reply-all" {
        return Err(failed(
            "Invalid arguments: 'type' must be \"reply\" or \"reply-all\"",
        ));
    }
    let message_id = opt_str(args, "messageId")?;
    let (account, thread, messages) = ctx.find_thread(&thread_id)?;
    let target = reply_target(ctx, &messages, message_id.as_deref())?.clone();
    let existing: Vec<Value> = messages
        .iter()
        .filter(|m| model::is_draft(m) && !model::is_hidden(m) && ctx.policy.message_allowed(m))
        .map(model::draft_summary)
        .collect();

    let quoted_body = ctx.body(&account.account_id, str_of(&target, "id"))?;
    let me = ctx.me();
    let (to, cc) = if kind == "reply-all" {
        drafts::participants_for_reply_all(&target, &me)
    } else {
        drafts::participants_for_reply(&target, &me)
    };
    let sender = ctx.sender(&account);
    let quote = drafts::reply_quote(&target, quoted_body.as_deref());
    let draft = drafts::new_draft_json(
        &sender,
        NewDraft {
            to,
            cc,
            bcc: Vec::new(),
            subject: drafts::subject_with_prefix(str_of(&target, "subject"), "Re:"),
            body: format!("{body}\n\n{quote}"),
            from: sender.reply_from(&target),
            thread_id: Some(str_of(&thread, "id").to_owned()),
            reply_to_header_message_id: Some(str_of(&target, "hMsgId").to_owned()),
        },
        now_secs(),
    );
    let task = ctx.queue(&account.account_id, tasks::syncback_draft(draft))?;
    let mut note = Value::Null;
    if !existing.is_empty() {
        note = Value::from(
            "This thread already had unsent drafts (existingDrafts). If one of them was the reply \
             you meant to change, update it with update_draft and delete this new one with \
             delete_draft.",
        );
    }
    let result = draft_result(
        &task,
        json!({
            "repliedTo": {
                "id": str_of(&target, "id"),
                "headerMessageId": str_of(&target, "hMsgId"),
                "subject": str_of(&target, "subject"),
                "from": array_of(&target, "from").iter().map(|c| json!({"name": c.get("name"), "email": c.get("email")})).collect::<Vec<_>>(),
                "date": model::iso_date(i64_of(&target, "date")),
                "draft": false,
            },
            "quoted": quoted_body.is_some(),
            "to": task["draft"]["to"].as_array().map(|l| l.iter().map(|c| json!({"name": c.get("name"), "email": c.get("email")})).collect::<Vec<_>>()),
            "cc": task["draft"]["cc"].as_array().map(|l| l.iter().map(|c| json!({"name": c.get("name"), "email": c.get("email")})).collect::<Vec<_>>()),
            "existingDrafts": existing,
            "note": note,
        }),
    );
    Ok(Output::text(&result).with_account(&account.account_id))
}

/// A draft by id or Message-ID, in `account_id` or any allowed account. Drafts created
/// on this device have an id derived from the Message-ID; others are found among the
/// threads in the account's Drafts folder.
fn find_draft(
    ctx: &mut Ctx<'_>,
    id: Option<&str>,
    header_message_id: Option<&str>,
    account_id: Option<&str>,
) -> Result<(AccountRef, Value), ToolError> {
    let not_found = || match (id, header_message_id) {
        (Some(id), _) => failed(format!("Draft '{id}' not found")),
        (None, Some(h)) => failed(format!("Draft with headerMessageId '{h}' not found")),
        (None, None) => failed("Invalid arguments: give the draft's id or headerMessageId"),
    };
    if id.is_none() && header_message_id.is_none() {
        return Err(not_found());
    }
    let accounts = ctx.accounts_for(account_id)?;
    let is_match = |m: &Value| {
        model::is_draft(m)
            && id.is_none_or(|i| str_of(m, "id") == i)
            && header_message_id.is_none_or(|h| str_of(m, "hMsgId") == h)
    };
    for account in &accounts {
        let aid = account.account_id.as_str();
        let candidate = match (id, header_message_id) {
            (Some(i), _) => i.to_owned(),
            (None, Some(h)) => draft_id(aid, h),
            _ => unreachable!("checked above"),
        };
        match ctx.call(aid, ms_ipc::method::MESSAGES_GET, json!({"id": candidate})) {
            Ok(r) => {
                let m = r.get("message").cloned().unwrap_or(Value::Null);
                if is_match(&m) && ctx.policy.message_allowed(&m) {
                    return Ok((account.clone(), m));
                }
            }
            Err(e) if e.is_not_found() => {}
            Err(e) => return Err(e.into()),
        }
    }
    if id.is_some() {
        return Err(not_found());
    }
    for account in &accounts {
        let aid = account.account_id.as_str();
        let page = match ctx.call(
            aid,
            ms_ipc::method::THREADS_LIST,
            json!({"role": "drafts", "limit": DRAFT_SCAN}),
        ) {
            Ok(p) => p,
            Err(e) if e.is_not_found() => continue,
            Err(e) => return Err(e.into()),
        };
        for t in array_of(&page, "threads").to_vec() {
            let r = ctx.call(
                aid,
                ms_ipc::method::THREADS_GET,
                json!({"id": str_of(&t, "id")}),
            )?;
            if let Some(m) = array_of(&r, "messages").iter().find(|m| is_match(m)) {
                if ctx.policy.message_allowed(m) {
                    return Ok((account.clone(), m.clone()));
                }
            }
        }
    }
    Err(not_found())
}

/// Refuses a draft whose send is under way: inside the desktop's Undo Send window (a
/// `send-later` schedule marked `isUndoSend`), or with a `SendDraftTask` that hasn't
/// finished. An engine with `tasks.list` knows every queued send, whoever queued it; with an
/// older one only the sends this session queued are known.
fn ensure_not_sending(ctx: &mut Ctx<'_>, account: &str, draft: &Value) -> Result<(), ToolError> {
    let hmid = str_of(draft, "hMsgId").to_owned();
    let busy = || {
        failed(format!(
            "Draft '{hmid}' is being sent and can't be changed; wait for the send to finish (or \
             undo it in Tern) and try again"
        ))
    };
    let send_later = array_of(draft, "metadata")
        .iter()
        .find(|m| str_of(m, "pluginId") == SEND_LATER_ID)
        .map(|m| &m["value"]);
    if let Some(v) = send_later {
        if model::bool_of(v, "isUndoSend") && i64_of(v, "expiration") > now_secs() {
            return Err(busy());
        }
    }
    if ctx
        .backend
        .has_feature(account, ms_ipc::feature::TASKS_LIST)
    {
        let r = ctx.call(
            account,
            ms_ipc::method::TASKS_LIST,
            json!({"class": tasks::SEND_DRAFT_TASK, "headerMessageId": hmid,
                   "status": ["local", "remote"], "limit": 1}),
        )?;
        if !array_of(&r, "tasks").is_empty() {
            return Err(busy());
        }
        ctx.session.sends.lock().expect("sends lock").remove(&hmid);
        return Ok(());
    }
    let pending = ctx
        .session
        .sends
        .lock()
        .expect("sends lock")
        .get(&hmid)
        .cloned();
    if let Some((task_account, task_id)) = pending.filter(|(a, _)| a == account) {
        match ctx.call(
            &task_account,
            ms_ipc::method::TASKS_GET,
            json!({"id": task_id}),
        ) {
            Ok(r) if matches!(str_of(&r["task"], "status"), "local" | "remote") => {
                return Err(busy())
            }
            Ok(_) => {
                ctx.session.sends.lock().expect("sends lock").remove(&hmid);
            }
            Err(e) if e.is_not_found() => {
                ctx.session.sends.lock().expect("sends lock").remove(&hmid);
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// The draft and its body, read again once the body is local. A draft synced from another
/// device gets `plaintext` and `files` only when its body is downloaded, so a task built
/// from the header-only copy would fail to send or drop the attachments.
fn loaded_draft(
    ctx: &mut Ctx<'_>,
    account: &str,
    draft: &Value,
) -> Result<(Value, String), ToolError> {
    let id = str_of(draft, "id").to_owned();
    let not_local = || failed("The draft's body isn't downloaded yet; try again in a moment");
    ctx.body(account, &id)?.ok_or_else(not_local)?;
    let r = match ctx.call(
        account,
        ms_ipc::method::MESSAGES_GET,
        json!({"id": id, "body": true}),
    ) {
        Ok(r) => r,
        Err(e) if e.is_not_found() => return Err(failed(format!("Draft '{id}' not found"))),
        Err(e) => return Err(e.into()),
    };
    let fresh = r.get("message").cloned().unwrap_or(Value::Null);
    if !model::is_draft(&fresh) || !ctx.policy.message_allowed(&fresh) {
        return Err(failed(format!("Draft '{id}' not found")));
    }
    let body = r
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(not_local)?;
    Ok((fresh, body.to_owned()))
}

pub(super) fn update_draft(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let id = opt_str(args, "id")?;
    let hmid = opt_str(args, "headerMessageId")?;
    let account_id = opt_str(args, "accountId")?;
    let subject = opt_str(args, "subject")?;
    let body = opt_str(args, "body")?;
    let to = opt_recipients(args, "to")?;
    let cc = opt_recipients(args, "cc")?;
    let bcc = opt_recipients(args, "bcc")?;
    if subject.is_none() && body.is_none() && to.is_none() && cc.is_none() && bcc.is_none() {
        return Err(failed(
            "Nothing to change: give at least one of subject, body, to, cc or bcc",
        ));
    }
    let (account, draft) = find_draft(ctx, id.as_deref(), hmid.as_deref(), account_id.as_deref())?;
    ensure_not_sending(ctx, &account.account_id, &draft)?;
    let (mut draft, _) = loaded_draft(ctx, &account.account_id, &draft)?;
    if let Some(s) = subject {
        draft["subject"] = Value::from(s);
    }
    for (key, list) in [("to", to), ("cc", cc), ("bcc", bcc)] {
        if let Some(l) = list {
            draft[key] = drafts::recipients_json(&l);
        }
    }
    draft["pristine"] = Value::Bool(false);
    // Without a new body the stored one stays: an absent `body` isn't written.
    let json = drafts::desktop_message_json(&draft, body.as_deref());
    let task = ctx.queue(&account.account_id, tasks::syncback_draft(json))?;
    Ok(Output::text(&draft_result(&task, json!({"updated": true})))
        .with_account(&account.account_id))
}

pub(super) fn delete_draft(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let id = opt_str(args, "id")?;
    let hmid = opt_str(args, "headerMessageId")?;
    let account_id = opt_str(args, "accountId")?;
    let (account, draft) = find_draft(ctx, id.as_deref(), hmid.as_deref(), account_id.as_deref())?;
    ensure_not_sending(ctx, &account.account_id, &draft)?;
    let draft_id = str_of(&draft, "id").to_owned();
    ctx.queue(
        &account.account_id,
        tasks::destroy_draft(&account.account_id, std::slice::from_ref(&draft_id)),
    )?;
    Ok(Output::text(&json!({
        "deleted": true,
        "id": draft_id,
        "headerMessageId": str_of(&draft, "hMsgId"),
    }))
    .with_account(&account.account_id))
}

// ── Sending ─────────────────────────────────────────────────────────────────

pub(super) fn send_draft(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let hmid = req_str(args, "headerMessageId")?;
    let (account, draft) = find_draft(ctx, None, Some(&hmid), None)?;
    ensure_not_sending(ctx, &account.account_id, &draft)?;
    // SendDraftTask.willBeQueued: the From address must be one of this account's.
    let from = array_of(&draft, "from")
        .first()
        .map(|c| str_of(c, "email").to_owned());
    let me = drafts::Me::new(&[ctx.sender(&account)]);
    if !from.as_deref().is_some_and(|f| me.is_me(f)) {
        return Err(failed(
            "The draft's From address isn't one of this account's addresses; fix the draft in \
             Tern and send it again",
        ));
    }
    let (draft, body) = loaded_draft(ctx, &account.account_id, &draft)?;
    let task = tasks::send_draft(drafts::desktop_message_json(&draft, Some(&body)));
    let task_id = str_of(&task, "id").to_owned();
    ctx.queue(&account.account_id, task)?;
    ctx.session
        .sends
        .lock()
        .expect("sends lock")
        .insert(hmid.clone(), (account.account_id.clone(), task_id));
    Ok(
        Output::text(&json!({"sent": true, "headerMessageId": hmid}))
            .with_account(&account.account_id),
    )
}

pub(super) fn schedule_send(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let hmid = req_str(args, "headerMessageId")?;
    let send_at = req_str(args, "sendAt")?;
    let action_key = opt_str(args, "actionKey")?;
    let when = parse_send_at(&send_at, &chrono::Local)
        .ok_or_else(|| failed(format!("Invalid sendAt date: '{send_at}'")))?;
    if when.timestamp() <= now_secs() {
        return Err(failed("sendAt must be in the future"));
    }
    let (account, draft) = find_draft(ctx, None, Some(&hmid), None)?;
    let action_key = action_key.unwrap_or_else(|| DEFAULT_SEND_ACTION_KEY.into());
    let mut value = json!({"expiration": when.timestamp(), "actionKey": action_key});
    // A send action only a desktop plugin performs: the engine leaves the send to the app.
    let engine_action = ENGINE_SEND_ACTIONS.contains(&action_key.as_str());
    if !engine_action {
        value["clientFinalize"] = json!(true);
    }
    let task = tasks::syncback_message_metadata(
        &draft,
        SEND_LATER_ID,
        value,
        Some(json!({"expiration": null})),
    );
    ctx.queue(&account.account_id, task)?;
    let note = if engine_action
        && ctx
            .backend
            .has_feature(&account.account_id, ms_ipc::feature::EXPIRATION_EXECUTOR)
    {
        SCHEDULE_NOTE
    } else {
        SCHEDULE_NOTE_DESKTOP
    };
    Ok(Output::text(&json!({
        "scheduled": true,
        "headerMessageId": hmid,
        "sendAt": when.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "note": note,
    }))
    .with_account(&account.account_id))
}

/// An engine with the expiration executor sends a due draft itself.
const SCHEDULE_NOTE: &str = "The Tern engine sends the draft at sendAt by itself; if it isn't \
                             running then, the draft goes out when it next starts.";
/// An older engine only announces a due send-later, and a newer one leaves a send action it
/// doesn't perform to the app; the desktop's send-later plugin sends it.
const SCHEDULE_NOTE_DESKTOP: &str = "This Tern engine leaves scheduled drafts to the desktop \
                                     app: it must be running at sendAt, or the draft goes out \
                                     when the app next opens.";

/// `sendAt` the way the plugin's `new Date(sendAt)` read it: a date-time without an offset
/// is local time (to the minute or with seconds), a date alone is midnight UTC.
pub(super) fn parse_send_at<Tz: chrono::TimeZone>(
    s: &str,
    local: &Tz,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let s = s.trim();
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&chrono::Utc));
    }
    for f in ["%Y-%m-%dT%H:%M%#z", "%Y-%m-%dT%H:%M:%S%.f%#z"] {
        if let Ok(d) = chrono::DateTime::parse_from_str(s, f) {
            return Some(d.with_timezone(&chrono::Utc));
        }
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(n) = chrono::NaiveDateTime::parse_from_str(s, f) {
            // A time skipped by a DST change doesn't exist; an ambiguous one takes the first.
            return local
                .from_local_datetime(&n)
                .earliest()
                .map(|d| d.with_timezone(&chrono::Utc));
        }
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|n| n.and_utc())
}

pub(super) fn cancel_scheduled_send(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let hmid = req_str(args, "headerMessageId")?;
    let (account, draft) = find_draft(ctx, None, Some(&hmid), None)?;
    let scheduled = array_of(&draft, "metadata")
        .iter()
        .find(|m| str_of(m, "pluginId") == SEND_LATER_ID)
        .is_some_and(|m| i64_of(&m["value"], "expiration") > 0);
    if !scheduled {
        return Err(failed(format!(
            "Draft '{hmid}' does not have a pending scheduled send"
        )));
    }
    let task =
        tasks::syncback_message_metadata(&draft, SEND_LATER_ID, json!({"expiration": null}), None);
    ctx.queue(&account.account_id, task)?;
    Ok(
        Output::text(&json!({"cancelled": true, "headerMessageId": hmid}))
            .with_account(&account.account_id),
    )
}

// ── Threads ─────────────────────────────────────────────────────────────────

/// The threads, grouped by account in first-seen order. Fails closed: any id that is
/// missing or blocked fails the whole call.
fn threads_by_account(
    ctx: &mut Ctx<'_>,
    ids: &[String],
) -> Result<Vec<(AccountRef, Vec<Value>)>, ToolError> {
    if ids.is_empty() {
        return Err(failed("No threads given"));
    }
    let mut groups: Vec<(AccountRef, Vec<Value>)> = Vec::new();
    for id in ids {
        let mut found = None;
        for account in ctx.allowed_accounts()? {
            match ctx.call(
                &account.account_id,
                ms_ipc::method::THREADS_GET,
                json!({"id": id}),
            ) {
                Ok(r) => {
                    found = Some((account, r["thread"].clone()));
                    break;
                }
                Err(e) if e.is_not_found() => {}
                Err(e) => return Err(e.into()),
            }
        }
        let Some((account, thread)) = found else {
            return Err(failed(format!("Thread '{id}' not found")));
        };
        if !ctx.policy.thread_allowed(&thread) {
            return Err(ToolError::Denied(format!(
                "Access denied: thread '{id}' is not accessible (its account or folder is \
                 excluded from MCP access)."
            )));
        }
        match groups
            .iter_mut()
            .find(|(a, _)| a.account_id == account.account_id)
        {
            Some((_, list)) => list.push(thread),
            None => groups.push((account, vec![thread])),
        }
    }
    Ok(groups)
}

fn ids_of(threads: &[Value]) -> Vec<String> {
    threads.iter().map(|t| str_of(t, "id").to_owned()).collect()
}

fn single_account(groups: &[(AccountRef, Vec<Value>)]) -> Option<String> {
    match groups {
        [(a, _)] => Some(a.account_id.clone()),
        _ => None,
    }
}

fn with_single_account(o: Output, groups: &[(AccountRef, Vec<Value>)]) -> Output {
    match single_account(groups) {
        Some(a) => o.with_account(&a),
        None => o,
    }
}

/// `CategoryStore.getCategoryByRole`: folders first, then labels.
fn by_role(categories: &[Value], role: &str) -> Option<Value> {
    categories
        .iter()
        .find(|c| str_of(c, "role") == role)
        .cloned()
}

fn is_label(c: &Value) -> bool {
    str_of(c, "__cls") == "Label"
}

pub(super) fn archive_threads(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let ids = req_str_list(args, "threadIds")?;
    let groups = threads_by_account(ctx, &ids)?;
    for (account, threads) in &groups {
        let aid = &account.account_id;
        let cats = ctx.categories(aid)?;
        let thread_ids = ids_of(threads);
        // TaskFactory.tasksForArchiving: Gmail drops the Inbox label; folders move to
        // Archive, else All Mail.
        let task = match by_role(&cats, "inbox") {
            Some(inbox) if is_label(&inbox) => {
                tasks::change_labels(aid, &thread_ids, vec![], vec![inbox])
            }
            _ => {
                let archive = by_role(&cats, "archive")
                    .or_else(|| by_role(&cats, "all"))
                    .ok_or_else(|| failed(format!("Account '{aid}' has no Archive folder")))?;
                tasks::change_folder(aid, &thread_ids, archive)
            }
        };
        ctx.queue(aid, task)?;
    }
    Ok(with_single_account(
        Output::text(&json!({"archived": ids.len()})),
        &groups,
    ))
}

pub(super) fn trash_threads(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let ids = req_str_list(args, "threadIds")?;
    let groups = threads_by_account(ctx, &ids)?;
    for (account, threads) in &groups {
        let aid = &account.account_id;
        let trash = by_role(&ctx.categories(aid)?, "trash")
            .ok_or_else(|| failed(format!("Account '{aid}' has no Trash folder")))?;
        ctx.queue(aid, tasks::change_folder(aid, &ids_of(threads), trash))?;
    }
    Ok(with_single_account(
        Output::text(&json!({"trashed": ids.len()})),
        &groups,
    ))
}

pub(super) fn move_to_folder(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let ids = req_str_list(args, "threadIds")?;
    let folder_id = req_str(args, "folderId")?;
    let groups = threads_by_account(ctx, &ids)?;
    let mut moved = 0;
    let mut skipped = Vec::new();
    for (account, threads) in &groups {
        let aid = &account.account_id;
        // An account where the destination is excluded or isn't a folder is skipped, as
        // the plugin skipped it.
        let folder = if ctx.policy.folder_allowed(aid, &folder_id) {
            ctx.categories(aid)?
                .into_iter()
                .find(|c| str_of(c, "id") == folder_id && !is_label(c))
        } else {
            None
        };
        let Some(folder) = folder else {
            skipped.extend(ids_of(threads));
            continue;
        };
        ctx.queue(aid, tasks::change_folder(aid, &ids_of(threads), folder))?;
        moved += threads.len();
    }
    if moved == 0 {
        return Err(failed(format!("Folder '{folder_id}' not found")));
    }
    let mut out = json!({"moved": moved, "folderId": folder_id});
    if !skipped.is_empty() {
        out["skippedThreadIds"] = json!(skipped);
    }
    Ok(with_single_account(Output::text(&out), &groups))
}

pub(super) fn set_labels(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let ids = req_str_list(args, "threadIds")?;
    let add = opt_str_list(args, "labelsToAdd")?.unwrap_or_default();
    let remove = opt_str_list(args, "labelsToRemove")?.unwrap_or_default();
    let groups = threads_by_account(ctx, &ids)?;
    let [(account, threads)] = groups.as_slice() else {
        return Err(failed(
            "set_labels works on threads of one account at a time",
        ));
    };
    let aid = account.account_id.clone();
    if !ctx.uses_labels(account)? {
        return Err(failed(format!(
            "Account '{aid}' does not use labels (not Gmail)"
        )));
    }
    for id in add.iter().chain(&remove) {
        if !ctx.policy.folder_allowed(&aid, id) {
            return Err(ToolError::Denied(format!(
                "Access denied: label '{id}' is excluded from MCP access."
            )));
        }
    }
    let cats = ctx.categories(&aid)?;
    let resolve = |list: &[String]| -> Result<Vec<Value>, ToolError> {
        list.iter()
            .map(|id| {
                cats.iter()
                    .find(|c| str_of(c, "id") == id && is_label(c))
                    .cloned()
                    .ok_or_else(|| failed(format!("Label '{id}' not found")))
            })
            .collect()
    };
    let task = tasks::change_labels(&aid, &ids_of(threads), resolve(&add)?, resolve(&remove)?);
    ctx.queue(&aid, task)?;
    Ok(Output::text(&json!({"updated": ids.len()})).with_account(&aid))
}

pub(super) fn set_unread(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let ids = req_str_list(args, "threadIds")?;
    let unread = req_bool(args, "unread")?;
    let groups = threads_by_account(ctx, &ids)?;
    for (account, threads) in &groups {
        let aid = &account.account_id;
        ctx.queue(aid, tasks::change_unread(aid, &ids_of(threads), unread))?;
    }
    Ok(with_single_account(
        Output::text(&json!({"updated": ids.len(), "unread": unread})),
        &groups,
    ))
}

pub(super) fn set_starred(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let ids = req_str_list(args, "threadIds")?;
    let starred = req_bool(args, "starred")?;
    let groups = threads_by_account(ctx, &ids)?;
    for (account, threads) in &groups {
        let aid = &account.account_id;
        ctx.queue(aid, tasks::change_starred(aid, &ids_of(threads), starred))?;
    }
    Ok(with_single_account(
        Output::text(&json!({"updated": ids.len(), "starred": starred})),
        &groups,
    ))
}
