// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The tasks behind `threads.snooze`, `threads.unsnooze` and `messages.unsubscribe`'s mailto:
//! what the desktop queues for the same actions, built
//! from the store so a client needs neither the categories nor the task JSON.
//!
//! - snooze: `snooze-utils.ts` `moveThreads` with `snooze: true` (Gmail swaps the inbox label
//!   for Snoozed's; elsewhere the inbox copy moves to Snoozed), then `snooze-store.ts`'s
//!   `thread-snooze` metadata `{expiration}` per thread. The expiration executor
//!   (`ms_sync::expiration_executor`) or a client brings the thread back when it is due.
//! - unsnooze: the metadata cleared and, for the threads still in Snoozed, the move back that
//!   the executor makes when a snooze comes due. Nothing is resurfaced: the user asked for
//!   the thread now.
//! - unsubscribe by mail: the message a `mailto:` URI describes, from the account address the
//!   list wrote to, sent as a `SendDraftTask`.

use ms_core::{Account, Error};
use ms_net::unsubscribe::Mailto;
use ms_store::model::json_str;
use ms_store::{Folder, Label, Message, Model, Query, Store, Task, Thread};
use ms_sync::expiration_executor::{RETRY_WHEN_ONLINE_KEY, THREAD_SNOOZE};
use serde_json::{json, Value};

#[derive(Debug)]
pub enum ActionError {
    /// A thread or message the account doesn't have.
    NotFound(String),
    /// The account can't do it, e.g. it has no Snoozed folder yet.
    Unavailable(String),
    Database(Error),
}

impl From<Error> for ActionError {
    fn from(e: Error) -> Self {
        ActionError::Database(e)
    }
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionError::NotFound(s) | ActionError::Unavailable(s) => f.write_str(s),
            ActionError::Database(e) => write!(f, "{e}"),
        }
    }
}

type ActionResult<T> = std::result::Result<T, ActionError>;

/// The snooze: the move to Snoozed, then one `thread-snooze` metadata task per thread.
pub fn snooze_tasks(
    store: &Store,
    account_id: &str,
    thread_ids: &[String],
    until: i64,
) -> ActionResult<Vec<Task>> {
    let threads = find_threads(store, account_id, thread_ids)?;
    let mut tasks = vec![snooze_move(store, account_id, &threads)?];
    tasks.extend(
        threads
            .iter()
            .map(|t| metadata_task(account_id, t, json!({ "expiration": until }))),
    );
    Ok(tasks)
}

/// The unsnooze: the move back for the threads still in Snoozed, then the metadata cleared
/// on every thread.
pub fn unsnooze_tasks(
    store: &Store,
    account_id: &str,
    thread_ids: &[String],
) -> ActionResult<Vec<Task>> {
    let threads = find_threads(store, account_id, thread_ids)?;
    let snoozed: Vec<&Thread> = threads
        .iter()
        .filter(|t| has_role(t.folders(), "snoozed") || has_role(t.labels(), "snoozed"))
        .collect();
    let mut tasks = Vec::new();
    if !snoozed.is_empty() {
        tasks.push(unsnooze_move(store, account_id, &snoozed)?);
    }
    tasks.extend(
        threads
            .iter()
            .map(|t| metadata_task(account_id, t, json!({ "expiration": null }))),
    );
    Ok(tasks)
}

fn find_threads(store: &Store, account_id: &str, ids: &[String]) -> ActionResult<Vec<Thread>> {
    if ids.is_empty() {
        return Err(ActionError::NotFound("no threads given".into()));
    }
    let mut out: Vec<Thread> = Vec::with_capacity(ids.len());
    for id in ids {
        if out.iter().any(|t| t.id() == id) {
            continue;
        }
        let thread = store
            .find::<Thread>(
                &Query::new()
                    .equal("id", id.as_str())
                    .equal("accountId", account_id),
            )?
            .ok_or_else(|| ActionError::NotFound(format!("no thread {id}")))?;
        out.push(thread);
    }
    Ok(out)
}

fn label(store: &Store, account_id: &str, role: &str) -> ms_core::Result<Option<Label>> {
    store.find::<Label>(
        &Query::new()
            .equal("accountId", account_id)
            .equal("role", role),
    )
}

fn folder(store: &Store, account_id: &str, role: &str) -> ms_core::Result<Option<Folder>> {
    store.find::<Folder>(
        &Query::new()
            .equal("accountId", account_id)
            .equal("role", role),
    )
}

fn no_snoozed_folder() -> ActionError {
    ActionError::Unavailable(
        "the account has no Snoozed folder yet; it is created by the first sync".into(),
    )
}

fn move_data(threads: &[&Thread], description: &str, undoable: bool) -> Value {
    let ids: Vec<&str> = threads.iter().map(|t| t.id()).collect();
    json!({
        "threadIds": ids, "source": "Snooze Move", "taskDescription": description,
        "canBeUndone": undoable,
    })
}

fn snooze_move(store: &Store, account_id: &str, threads: &[Thread]) -> ActionResult<Task> {
    let threads: Vec<&Thread> = threads.iter().collect();
    let mut data = move_data(&threads, "Snoozed", true);
    if let Some(snoozed) = label(store, account_id, "snoozed")? {
        let inbox: Vec<Value> = label(store, account_id, "inbox")?
            .iter()
            .map(Model::to_json)
            .collect();
        data["labelsToAdd"] = json!([snoozed.to_json()]);
        data["labelsToRemove"] = json!(inbox);
        return Ok(Task::create("ChangeLabelsTask", account_id, &data));
    }
    let snoozed = folder(store, account_id, "snoozed")?.ok_or_else(no_snoozed_folder)?;
    // Only the Inbox copy moves; a Sent copy of the same message stays where it is.
    let source: Vec<String> = folder(store, account_id, "inbox")?
        .iter()
        .map(|f| f.id().to_owned())
        .collect();
    data["folder"] = snoozed.to_json();
    data["sourceFolderIds"] = json!(source);
    Ok(Task::create("ChangeFolderTask", account_id, &data))
}

fn unsnooze_move(store: &Store, account_id: &str, threads: &[&Thread]) -> ActionResult<Task> {
    let mut data = move_data(threads, "Unsnoozed", false);
    if let Some(snoozed) = label(store, account_id, "snoozed")? {
        let inbox: Vec<Value> = label(store, account_id, "inbox")?
            .iter()
            .map(Model::to_json)
            .collect();
        data["labelsToAdd"] = json!(inbox);
        data["labelsToRemove"] = json!([snoozed.to_json()]);
        return Ok(Task::create("ChangeLabelsTask", account_id, &data));
    }
    let snoozed = folder(store, account_id, "snoozed")?.ok_or_else(no_snoozed_folder)?;
    let inbox = folder(store, account_id, "inbox")?
        .ok_or_else(|| ActionError::Unavailable("the account has no Inbox folder".into()))?;
    data["folder"] = inbox.to_json();
    data["sourceFolderIds"] = json!([snoozed.id()]);
    Ok(Task::create("ChangeFolderTask", account_id, &data))
}

/// `SyncbackMetadataTask.forSaving` for a thread.
fn metadata_task(account_id: &str, thread: &Thread, value: Value) -> Task {
    Task::create(
        "SyncbackMetadataTask",
        account_id,
        &json!({
            "modelId": thread.id(),
            "modelClassName": "thread",
            "pluginId": THREAD_SNOOZE,
            "value": value,
        }),
    )
}

fn has_role(categories: &Value, role: &str) -> bool {
    categories
        .as_array()
        .is_some_and(|list| list.iter().any(|c| json_str(c, "role") == role))
}

/// The subject and body sent when the mailto URI gives none: list servers act on the
/// address, and some on the word.
pub const UNSUBSCRIBE_DEFAULT_TEXT: &str = "unsubscribe";

/// The `SendDraftTask` for an unsubscribe by mail: to the URI's recipients, from the
/// account's address (or alias) that the message `message_id` was sent to, in plain text.
pub fn unsubscribe_send_task(
    store: &Store,
    account: &Account,
    message_id: &str,
    mailto: &Mailto,
) -> ActionResult<Task> {
    let message = store
        .find::<Message>(
            &Query::new()
                .equal("id", message_id)
                .equal("accountId", account.id()),
        )?
        .ok_or_else(|| ActionError::NotFound(format!("no message {message_id}")))?;
    let (name, email) = sender_for(account, &message);
    let header_message_id = new_header_message_id(&email);
    let contacts = |list: &[String]| -> Vec<Value> {
        list.iter()
            .map(|e| json!({"name": "", "email": e}))
            .collect()
    };
    let text = |s: &str| {
        if s.trim().is_empty() {
            UNSUBSCRIBE_DEFAULT_TEXT.to_owned()
        } else {
            s.to_owned()
        }
    };
    let draft = json!({
        "__cls": "Message",
        "aid": account.id(),
        "v": 0,
        "hMsgId": header_message_id,
        "subject": text(&mailto.subject),
        "from": [{"name": name, "email": email}],
        "to": contacts(&mailto.to),
        "cc": contacts(&mailto.cc),
        "bcc": [], "replyTo": [], "files": [],
        "body": text(&mailto.body),
        "plaintext": true,
        "draft": true,
        "pristine": false,
        "unread": false,
        "starred": false,
        "rthMsgId": "",
        "fwdMsgId": "",
        "date": store.now(),
    });
    Ok(Task::create(
        "SendDraftTask",
        account.id(),
        &json!({
            "draft": draft,
            "headerMessageId": header_message_id,
            "perRecipientBodies": null,
            "silent": false,
            RETRY_WHEN_ONLINE_KEY: true,
        }),
    ))
}

/// The account's identity (name, address) that received `message`: the alias among its To
/// and Cc, else the account's own address.
fn sender_for(account: &Account, message: &Message) -> (String, String) {
    let account_name = json_str(account.as_json(), "name").to_owned();
    let mut identities = vec![(
        account_name.clone(),
        account.email_address().trim().to_owned(),
    )];
    if let Some(aliases) = account.as_json().get("aliases").and_then(Value::as_array) {
        for alias in aliases.iter().filter_map(Value::as_str) {
            identities.push(match (alias.rfind('<'), alias.rfind('>')) {
                (Some(lo), Some(hi)) if lo < hi => (
                    alias[..lo].trim().trim_matches('"').to_owned(),
                    alias[lo + 1..hi].trim().to_owned(),
                ),
                _ => (account_name.clone(), alias.trim().to_owned()),
            });
        }
    }
    let received: Vec<String> = [&message.data["to"], &message.data["cc"]]
        .into_iter()
        .filter_map(Value::as_array)
        .flatten()
        .map(|c| json_str(c, "email").to_ascii_lowercase())
        .collect();
    identities
        .iter()
        .find(|(_, e)| received.contains(&e.to_ascii_lowercase()))
        .unwrap_or(&identities[0])
        .clone()
}

/// `generateHeaderMessageId` (`draft-factory.ts`): a random UUID at the sender's domain.
fn new_header_message_id(from_email: &str) -> String {
    let domain = from_email
        .rsplit_once('@')
        .map(|(_, d)| d.trim())
        .filter(|d| {
            !d.is_empty()
                && d.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        })
        .unwrap_or("localhost");
    let id = ms_core::ids::random_id();
    format!("{}@{domain}", id.to_ascii_uppercase())
}
