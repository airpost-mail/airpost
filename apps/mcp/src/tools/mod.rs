// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The MCP tools: the desktop plugin's `mcp-tools.ts` tool for tool (names, input schemas,
//! output shapes) on the engine protocol, plus `update_draft` and `delete_draft`. Reads go
//! through the read API, changes through `task.queue` with the task JSON the desktop
//! queues. Every call checks the access level first, and every lookup skips accounts the
//! policy doesn't enable, so their ids read as unknown.

mod attachments;
mod followup;
mod pim;
mod read;
mod write;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub use attachments::read_attachment_resource;

pub fn attachment_uri_scheme() -> &'static str {
    attachments::URI_SCHEME
}

use crate::config::DesktopConfig;
use crate::discovery::{AccountRef, Backend, EngineError};
use crate::drafts::{Recipient, Sender};
use crate::model::{array_of, str_of};
use crate::policy::{Category, Policy};

/// How long a tool waits for a body the engine has to download first.
const BODY_WAIT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(250);
/// How long a tool waits for a task whose result it returns (a new label's id).
const TASK_WAIT: Duration = Duration::from_secs(20);

/// State that lives as long as one MCP session: the sends this session queued, so it can
/// refuse to edit a draft while its `SendDraftTask` runs.
#[derive(Default)]
pub struct Session {
    sends: Mutex<HashMap<String, (String, String)>>,
}

pub struct Ctx<'a> {
    pub backend: &'a mut dyn Backend,
    pub policy: &'a Policy,
    pub desktop: &'a DesktopConfig,
    pub config_dir: &'a Path,
    pub session: &'a Session,
    /// The error to give when no engine is running.
    pub no_engine: String,
    /// Serving `tern-mcp http`, whose client may be on another machine: tools that read
    /// files by path refuse.
    pub over_http: bool,
}

/// A tool's result: MCP content blocks, and whether it is an error.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub content: Vec<Value>,
    pub is_error: bool,
    /// Policy refusals, for the audit log's `denied` outcome.
    pub denied: bool,
    /// The account the call acted on, when there was one.
    pub account_id: Option<String>,
}

impl Output {
    pub fn text(data: &Value) -> Output {
        Output {
            content: vec![text_block(data)],
            is_error: false,
            denied: false,
            account_id: None,
        }
    }

    fn with_account(mut self, account: &str) -> Output {
        self.account_id = Some(account.to_owned());
        self
    }

    /// The first text block, for the audit log and tests.
    pub fn first_text(&self) -> &str {
        self.content
            .first()
            .and_then(|c| c.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
    }
}

pub fn text_block(data: &Value) -> Value {
    json!({"type": "text", "text": serde_json::to_string_pretty(data).unwrap_or_default()})
}

#[derive(Debug)]
pub enum ToolError {
    /// The policy refused it.
    Denied(String),
    Failed(String),
}

impl From<EngineError> for ToolError {
    fn from(e: EngineError) -> Self {
        ToolError::Failed(e.to_string())
    }
}

type ToolResult = Result<Output, ToolError>;

pub(crate) fn failed(msg: impl Into<String>) -> ToolError {
    ToolError::Failed(msg.into())
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub category: Category,
}

/// Every tool, in the order `tools/list` returns them.
pub fn definitions() -> Vec<ToolDef> {
    let mut defs = read::definitions();
    defs.extend(attachments::definitions());
    defs.extend(write::definitions());
    defs.extend(pim::definitions());
    defs.extend(followup::definitions());
    defs
}

/// Runs one tool call. Unknown tools and bad arguments are tool errors, so the model sees
/// the message.
pub fn call(ctx: &mut Ctx<'_>, name: &str, args: &Value) -> Output {
    let Some(def) = definitions().into_iter().find(|d| d.name == name) else {
        return error_output(&format!("Unknown tool '{name}'"), false);
    };
    if let Err(e) = ctx.policy.check_access(def.category) {
        return error_output(&e, true);
    }
    let result = match name {
        "list_accounts" => read::list_accounts(ctx),
        "list_folders" => read::list_folders(ctx, args),
        "search_mail" => read::search_mail(ctx, args),
        "list_threads" => read::list_threads(ctx, args),
        "get_thread" => read::get_thread(ctx, args),
        "get_message" => read::get_message(ctx, args),
        "get_message_tracking" => read::get_message_tracking(ctx, args),
        "list_tracked_messages" => read::list_tracked_messages(ctx, args),
        "get_top_links" => read::get_top_links(ctx, args),
        "list_scheduled_sends" => read::list_scheduled_sends(ctx, args),
        "list_templates" => read::list_templates(ctx, args),
        "get_attachment" => attachments::get_attachment(ctx, args),
        "create_draft" => write::create_draft(ctx, args),
        "reply_to_thread" => write::reply_to_thread(ctx, args),
        "update_draft" => write::update_draft(ctx, args),
        "delete_draft" => write::delete_draft(ctx, args),
        "archive_threads" => write::archive_threads(ctx, args),
        "trash_threads" => write::trash_threads(ctx, args),
        "move_to_folder" => write::move_to_folder(ctx, args),
        "set_labels" => write::set_labels(ctx, args),
        "create_label" => write::create_label(ctx, args),
        "set_unread" => write::set_unread(ctx, args),
        "set_starred" => write::set_starred(ctx, args),
        "send_draft" => write::send_draft(ctx, args),
        "schedule_send" => write::schedule_send(ctx, args),
        "cancel_scheduled_send" => write::cancel_scheduled_send(ctx, args),
        "snooze_threads" => write::snooze_threads(ctx, args),
        "unsnooze_threads" => write::unsnooze_threads(ctx, args),
        "unsubscribe" => write::unsubscribe(ctx, args),
        "list_events" => pim::list_events(ctx, args),
        "create_event" => pim::create_event(ctx, args),
        "update_event" => pim::update_event(ctx, args),
        "respond_to_event" => pim::respond_to_event(ctx, args),
        "get_contact" => pim::get_contact(ctx, args),
        "create_contact" => pim::create_contact(ctx, args),
        "update_contact" => pim::update_contact(ctx, args),
        "set_reminder" => followup::set_reminder(ctx, args),
        _ => Err(failed(format!("Unknown tool '{name}'"))),
    };
    match result {
        Ok(o) => o,
        Err(ToolError::Denied(m)) => error_output(&m, true),
        Err(ToolError::Failed(m)) => error_output(&m, false),
    }
}

fn error_output(msg: &str, denied: bool) -> Output {
    Output {
        content: vec![json!({"type": "text", "text": json!({"error": msg}).to_string()})],
        is_error: true,
        denied,
        account_id: None,
    }
}

// ── Arguments ───────────────────────────────────────────────────────────────

fn invalid(key: &str, what: &str) -> ToolError {
    failed(format!("Invalid arguments: '{key}' must be {what}"))
}

pub(crate) fn opt_str(args: &Value, key: &str) -> Result<Option<String>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(key, "a string")),
    }
}

pub(crate) fn req_str(args: &Value, key: &str) -> Result<String, ToolError> {
    opt_str(args, key)?.ok_or_else(|| invalid(key, "a string"))
}

pub(crate) fn opt_bool(args: &Value, key: &str) -> Result<Option<bool>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(invalid(key, "a boolean")),
    }
}

pub(crate) fn req_bool(args: &Value, key: &str) -> Result<bool, ToolError> {
    opt_bool(args, key)?.ok_or_else(|| invalid(key, "a boolean"))
}

/// A non-negative count; fractions are truncated, as the plugin's `.limit()` did.
pub(crate) fn opt_count(args: &Value, key: &str, default: usize) -> Result<usize, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f >= 0.0 && f.is_finite() => Ok(f as usize),
            _ => Err(invalid(key, "a non-negative number")),
        },
        Some(_) => Err(invalid(key, "a number")),
    }
}

pub(crate) fn opt_str_list(args: &Value, key: &str) -> Result<Option<Vec<String>>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid(key, "an array of strings"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(invalid(key, "an array of strings")),
    }
}

pub(crate) fn req_str_list(args: &Value, key: &str) -> Result<Vec<String>, ToolError> {
    opt_str_list(args, key)?.ok_or_else(|| invalid(key, "an array of strings"))
}

/// `[{name?, email}]`.
pub(crate) fn opt_recipients(args: &Value, key: &str) -> Result<Option<Vec<Recipient>>, ToolError> {
    let bad = || invalid(key, "an array of {name?, email} objects");
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a
            .iter()
            .map(|c| {
                let email = c.get("email").and_then(Value::as_str).ok_or_else(bad)?;
                let name = match c.get("name") {
                    None | Some(Value::Null) => "",
                    Some(Value::String(s)) => s.as_str(),
                    Some(_) => return Err(bad()),
                };
                Ok(Recipient {
                    name: name.to_owned(),
                    email: email.to_owned(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(bad()),
    }
}

// ── Schema helpers ──────────────────────────────────────────────────────────

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

fn recipients_schema(description: &str) -> Value {
    json!({
        "type": "array",
        "description": description,
        "items": {
            "type": "object",
            "properties": {"name": {"type": "string"}, "email": {"type": "string"}},
            "required": ["email"]
        }
    })
}

fn string_list_schema(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

// ── Lookups ─────────────────────────────────────────────────────────────────

impl Ctx<'_> {
    /// The accounts the policy lets the caller see. No engine at all is an error.
    pub(crate) fn allowed_accounts(&self) -> Result<Vec<AccountRef>, ToolError> {
        let all = self.backend.accounts();
        if all.is_empty() {
            return Err(failed(self.no_engine.clone()));
        }
        Ok(all
            .into_iter()
            .filter(|a| self.policy.account_allowed(&a.account_id))
            .collect())
    }

    /// The named account, if it is running and enabled; otherwise it doesn't exist as far
    /// as the caller can tell.
    pub(crate) fn account(&self, account_id: &str) -> Result<AccountRef, ToolError> {
        self.allowed_accounts()?
            .into_iter()
            .find(|a| a.account_id == account_id)
            .ok_or_else(|| failed(format!("Account '{account_id}' not found")))
    }

    /// `accountId` when given, else every allowed account.
    pub(crate) fn accounts_for(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<AccountRef>, ToolError> {
        match account_id {
            Some(id) => Ok(vec![self.account(id)?]),
            None => self.allowed_accounts(),
        }
    }

    pub(crate) fn call(
        &mut self,
        account: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, EngineError> {
        self.backend.call(account, method, params)
    }

    pub(crate) fn sender(&self, account: &AccountRef) -> Sender {
        let meta = self
            .desktop
            .accounts()
            .into_iter()
            .find(|m| m.id == account.account_id)
            .unwrap_or_else(|| crate::config::AccountMeta {
                id: account.account_id.clone(),
                email: account.email.clone(),
                ..Default::default()
            });
        Sender {
            account_id: account.account_id.clone(),
            meta,
        }
    }

    /// `Account.usesLabels()` (Gmail). Without the desktop's account record, an account
    /// with labels counts as one.
    pub(crate) fn uses_labels(&mut self, account: &AccountRef) -> Result<bool, ToolError> {
        if let Some(p) = self.sender(account).meta.provider {
            return Ok(p == "gmail");
        }
        let labels = self.call(
            &account.account_id,
            tern_ipc::method::LABELS_LIST,
            json!({}),
        )?;
        Ok(!array_of(&labels, "labels").is_empty())
    }

    /// The account's folders and labels, folders first, each with its `__cls`.
    pub(crate) fn categories(&mut self, account: &str) -> Result<Vec<Value>, ToolError> {
        let folders = self.call(account, tern_ipc::method::FOLDERS_LIST, json!({}))?;
        let labels = self.call(account, tern_ipc::method::LABELS_LIST, json!({}))?;
        let tagged = |list: &[Value], cls: &str| -> Vec<Value> {
            list.iter()
                .map(|c| {
                    let mut c = c.clone();
                    if c.get("__cls").is_none() {
                        c["__cls"] = Value::from(cls);
                    }
                    c
                })
                .collect()
        };
        let mut out = tagged(array_of(&folders, "folders"), "Folder");
        out.extend(tagged(array_of(&labels, "labels"), "Label"));
        Ok(out)
    }

    pub(crate) fn categories_by_id(
        &mut self,
        account: &str,
    ) -> Result<HashMap<String, Value>, ToolError> {
        Ok(self
            .categories(account)?
            .into_iter()
            .map(|c| (str_of(&c, "id").to_owned(), c))
            .collect())
    }

    /// The thread and its messages (oldest first, no bodies) in whichever allowed account
    /// has it. A thread the policy blocks reads as missing.
    pub(crate) fn find_thread(
        &mut self,
        thread_id: &str,
    ) -> Result<(AccountRef, Value, Vec<Value>), ToolError> {
        for account in self.allowed_accounts()? {
            match self.call(
                &account.account_id,
                tern_ipc::method::THREADS_GET,
                json!({"id": thread_id}),
            ) {
                Ok(r) => {
                    let thread = r.get("thread").cloned().unwrap_or(Value::Null);
                    if !self.policy.thread_allowed(&thread) {
                        break;
                    }
                    let messages = array_of(&r, "messages").to_vec();
                    return Ok((account, thread, messages));
                }
                Err(e) if e.is_not_found() => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(failed(format!("Thread '{thread_id}' not found")))
    }

    /// The message in `account`, or in whichever allowed account has it.
    pub(crate) fn find_message(
        &mut self,
        message_id: &str,
        account_id: Option<&str>,
    ) -> Result<(AccountRef, Value), ToolError> {
        for account in self.accounts_for(account_id)? {
            match self.call(
                &account.account_id,
                tern_ipc::method::MESSAGES_GET,
                json!({"id": message_id}),
            ) {
                Ok(r) => {
                    let message = r.get("message").cloned().unwrap_or(Value::Null);
                    if !self.policy.message_allowed(&message) {
                        break;
                    }
                    return Ok((account, message));
                }
                Err(e) if e.is_not_found() => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(failed(format!("Message '{message_id}' not found")))
    }

    /// The task once it has finished, polling `tasks.get` for up to `wait`; `None` while it
    /// is still pending.
    pub(crate) fn wait_for_task(
        &mut self,
        account: &str,
        task_id: &str,
        wait: Duration,
    ) -> Result<Option<Value>, ToolError> {
        let deadline = Instant::now() + wait;
        loop {
            let r = self.call(account, tern_ipc::method::TASKS_GET, json!({"id": task_id}))?;
            let task = r.get("task").cloned().unwrap_or(Value::Null);
            if crate::tasks::is_final(&task) {
                return Ok(Some(task));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(POLL);
        }
    }

    /// Bodies of `ids`, asking the engine to download the missing ones and waiting up to
    /// `wait` for them. A body that doesn't arrive is `None`.
    pub(crate) fn bodies(
        &mut self,
        account: &str,
        ids: &[String],
        wait: Duration,
    ) -> Result<HashMap<String, Option<String>>, ToolError> {
        let mut out = HashMap::new();
        let mut missing = Vec::new();
        for id in ids {
            let body = self.body_now(account, id)?;
            if body.is_none() {
                missing.push(id.clone());
            }
            out.insert(id.clone(), body);
        }
        if missing.is_empty() {
            return Ok(out);
        }
        self.call(
            account,
            tern_ipc::method::BODIES_FETCH,
            json!({"ids": missing}),
        )?;
        let deadline = Instant::now() + wait;
        while !missing.is_empty() && Instant::now() < deadline {
            std::thread::sleep(POLL);
            let mut still = Vec::new();
            for id in missing {
                match self.body_now(account, &id)? {
                    Some(b) => {
                        out.insert(id, Some(b));
                    }
                    None => still.push(id),
                }
            }
            missing = still;
        }
        Ok(out)
    }

    fn body_now(&mut self, account: &str, id: &str) -> Result<Option<String>, ToolError> {
        let r = self.call(
            account,
            tern_ipc::method::MESSAGES_GET,
            json!({"id": id, "body": true}),
        )?;
        Ok(r.get("body").and_then(Value::as_str).map(str::to_owned))
    }

    pub(crate) fn body(&mut self, account: &str, id: &str) -> Result<Option<String>, ToolError> {
        Ok(self
            .bodies(account, &[id.to_owned()], BODY_WAIT)?
            .remove(id)
            .flatten())
    }

    /// Queues a task and returns it after its local phase.
    pub(crate) fn queue(&mut self, account: &str, task: Value) -> Result<Value, ToolError> {
        self.call(account, tern_ipc::method::TASK_QUEUE, json!({"task": task}))
            .map_err(|e| failed(format!("The engine refused the task: {e}")))
    }
}

pub(crate) fn body_wait() -> Duration {
    BODY_WAIT
}

pub(crate) fn task_wait() -> Duration {
    TASK_WAIT
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests;
