// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `threads.*` and `categories.*` intents and the `undo.*` methods, on the socket (plan
//! sections 4.2 and 4.3). An intent finds the account of each thread, message or category,
//! has each account's `main` thread build its tasks (all of them before any is queued, so a
//! name an account doesn't know fails the call with nothing done), queues them, and answers
//! with the rows they changed and, when there is something to undo, an entry of the
//! [`UndoLog`](crate::undo::UndoLog). `threads.snooze` and a `draft.send` that waits out its
//! undo window add an entry too.

use std::collections::BTreeSet;

use serde_json::{json, Value};
use tern_ipc::{
    code, method, policy, task_status, CategoriesCreateParams, CategoriesRenameParams,
    CategoryIdParams, DraftCancelSendParams, ErrorObject, IntentResult, MailTargets, Perspective,
    Request as RpcRequest, ThreadsActionParams, ThreadsLabelParams, ThreadsMarkReadParams,
    ThreadsMoveParams, ThreadsRemoveFromViewParams, ThreadsStarParams, UndoApplyParams,
    UndoApplyResult, UndoEntry, UndoListParams, UndoListResult, UndoRecordParams, UndoRecordResult,
};
use tern_store::model::json_str;
use tern_store::read::{IdTable, ReadStore};
use tern_tasks::intents::{Plan, Selection, Verb, View, IDS_PER_TASK};
use tern_tasks::undo::{has_undo, undoable_class};

use crate::dispatch::Request;
use crate::socket::Session;
use crate::undo::{Kind, NewEntry};

pub fn is_intent_method(name: &str) -> bool {
    matches!(
        name,
        method::THREADS_ARCHIVE
            | method::THREADS_TRASH
            | method::THREADS_SPAM
            | method::THREADS_NOT_SPAM
            | method::THREADS_MOVE
            | method::THREADS_LABEL
            | method::THREADS_MARK_READ
            | method::THREADS_STAR
            | method::THREADS_REMOVE_FROM_VIEW
            | method::CATEGORIES_MARK_READ
            | method::CATEGORIES_CREATE
            | method::CATEGORIES_RENAME
            | method::CATEGORIES_DELETE
            | method::CATEGORIES_EMPTY
            | method::UNDO_APPLY
            | method::UNDO_REDO
            | method::UNDO_LIST
            | method::UNDO_RECORD
    )
}

/// Who calls: the undo log's client name, and a relayed device's scopes.
pub struct Caller<'a> {
    pub client: &'a str,
    pub scopes: Option<&'a [String]>,
}

impl Caller<'_> {
    fn may_queue(&self, task: &Value) -> bool {
        self.scopes
            .is_none_or(|s| policy::relayed_task_allowed(task, s))
    }
}

/// An answer, and the tasks behind it for `notify`.
pub struct Outcome {
    pub value: Value,
    /// By account, as their local phases left them.
    pub tasks: Vec<(String, Value)>,
    pub notify: bool,
}

/// The most rows an answer carries; past it a client waits for its live queries.
const MODELS_MAX: usize = IDS_PER_TASK;

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

fn invalid(why: impl Into<String>) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, why.into())
}

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, ErrorObject> {
    serde_json::to_value(v).map_err(internal)
}

pub fn call(
    session: &mut Session,
    caller: &Caller<'_>,
    req: &RpcRequest,
) -> Result<Outcome, ErrorObject> {
    match req.method.as_str() {
        method::UNDO_APPLY => undo_apply(session, caller, req.params_as()?, false),
        method::UNDO_REDO => undo_apply(session, caller, req.params_as()?, true),
        method::UNDO_LIST => {
            let p: UndoListParams = req.params_as()?;
            let entries = session
                .server
                .hub
                .undo()
                .list(caller.client, p.mine, p.limit);
            Ok(Outcome {
                value: to_value(&UndoListResult { entries })?,
                tasks: Vec::new(),
                notify: false,
            })
        }
        method::UNDO_RECORD => {
            let p: UndoRecordParams = req.params_as()?;
            let refs: Vec<(String, String)> = p
                .tasks
                .into_iter()
                .map(|t| (t.account_id, t.task_id))
                .collect();
            let tasks = stored_tasks(session, caller, &refs)?;
            let count = tasks.iter().map(|(_, t)| targets_count(t)).sum::<usize>();
            let label = p.label.unwrap_or_else(|| {
                tasks
                    .iter()
                    .map(|(_, t)| json_str(t, "taskDescription"))
                    .find(|d| !d.is_empty())
                    .map_or_else(|| format!("Changed {}", items(count)), str::to_owned)
            });
            let token = record(session, caller, "tasks", label, count, tasks);
            Ok(Outcome {
                value: to_value(&UndoRecordResult { token })?,
                tasks: Vec::new(),
                notify: false,
            })
        }
        method::CATEGORIES_CREATE => {
            let p: CategoriesCreateParams = req.params_as()?;
            session.server.accounts.check(&p.account_id)?;
            let plan = Plan::CategoryCreate { name: p.name };
            category_intent(session, caller, &p.account_id, plan, p.notify)
        }
        method::CATEGORIES_RENAME => {
            let p: CategoriesRenameParams = req.params_as()?;
            let account = category_account(session, &p.category_id)?;
            let plan = Plan::CategoryRename {
                category_id: p.category_id,
                name: p.name,
            };
            category_intent(session, caller, &account, plan, p.notify)
        }
        method::CATEGORIES_MARK_READ | method::CATEGORIES_DELETE | method::CATEGORIES_EMPTY => {
            let p: CategoryIdParams = req.params_as()?;
            let account = category_account(session, &p.category_id)?;
            let category_id = p.category_id;
            let plan = match req.method.as_str() {
                method::CATEGORIES_MARK_READ => Plan::CategoryMarkRead {
                    category_id,
                    source: caller.client.to_owned(),
                },
                method::CATEGORIES_DELETE => Plan::CategoryDelete { category_id },
                _ => Plan::CategoryEmpty { category_id },
            };
            category_intent(session, caller, &account, plan, p.notify)
        }
        _ => mail(session, caller, mail_intent(req)?),
    }
}

/// What a `threads.*` call asks for.
struct MailCall {
    verb: Verb,
    targets: MailTargets,
    perspective: Option<Perspective>,
    notify: bool,
    dry_run: bool,
}

/// A `threads.*` call's verb, targets, perspective and flags.
fn mail_intent(req: &RpcRequest) -> Result<MailCall, ErrorObject> {
    Ok(match req.method.as_str() {
        method::THREADS_MOVE => {
            let p: ThreadsMoveParams = req.params_as()?;
            if p.to.trim().is_empty() {
                return Err(invalid("invalid params: to names no folder or label"));
            }
            mail_call(
                Verb::Move(p.to),
                p.targets,
                p.perspective,
                p.notify,
                p.dry_run,
            )
        }
        method::THREADS_LABEL => {
            let p: ThreadsLabelParams = req.params_as()?;
            if p.add.is_empty() && p.remove.is_empty() {
                return Err(invalid("invalid params: add or remove a label"));
            }
            let verb = Verb::Label {
                add: p.add,
                remove: p.remove,
            };
            mail_call(verb, p.targets, None, p.notify, p.dry_run)
        }
        method::THREADS_MARK_READ => {
            let p: ThreadsMarkReadParams = req.params_as()?;
            let verb = Verb::MarkRead { unread: p.unread };
            mail_call(verb, p.targets, None, p.notify, p.dry_run)
        }
        method::THREADS_STAR => {
            let p: ThreadsStarParams = req.params_as()?;
            let verb = Verb::Star { starred: p.starred };
            mail_call(verb, p.targets, None, p.notify, p.dry_run)
        }
        method::THREADS_REMOVE_FROM_VIEW => {
            let p: ThreadsRemoveFromViewParams = req.params_as()?;
            let verb = Verb::RemoveFromView {
                prefer_trash: p.prefer_trash,
            };
            mail_call(verb, p.targets, Some(p.perspective), p.notify, p.dry_run)
        }
        name => {
            let p: ThreadsActionParams = req.params_as()?;
            let verb = match name {
                method::THREADS_ARCHIVE => Verb::Archive,
                method::THREADS_TRASH => Verb::Trash,
                method::THREADS_SPAM => Verb::Spam,
                _ => Verb::NotSpam,
            };
            mail_call(verb, p.targets, p.perspective, p.notify, p.dry_run)
        }
    })
}

fn mail_call(
    verb: Verb,
    targets: MailTargets,
    perspective: Option<Perspective>,
    notify: bool,
    dry_run: bool,
) -> MailCall {
    MailCall {
        verb,
        targets,
        perspective,
        notify,
        dry_run,
    }
}

fn dedup(ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.into_iter().filter(|i| seen.insert(i.clone())).collect()
}

fn mail(session: &mut Session, caller: &Caller<'_>, c: MailCall) -> Result<Outcome, ErrorObject> {
    let MailCall {
        verb,
        targets,
        perspective,
        notify,
        dry_run,
    } = c;
    let by_messages = targets.thread_ids.is_empty();
    if by_messages == targets.message_ids.is_empty() {
        return Err(invalid(
            "invalid params: give threadIds or messageIds, not both",
        ));
    }
    let ids = dedup(if by_messages {
        targets.message_ids
    } else {
        targets.thread_ids
    });
    let table = if by_messages {
        IdTable::Message
    } else {
        IdTable::Thread
    };
    let groups = group_by_account(session.reads.store()?, table, &ids)?;
    let view = perspective.map(|p| View {
        category_ids: p.category_ids,
        starred: p.starred,
        unread: p.unread,
    });
    let mut plans = Vec::new();
    for (account, ids) in groups {
        session.server.accounts.check(&account)?;
        let selection = if by_messages {
            Selection {
                message_ids: ids,
                ..Default::default()
            }
        } else {
            Selection {
                thread_ids: ids,
                ..Default::default()
            }
        };
        let plan = Plan::Mail {
            verb: verb.clone(),
            selection,
            view: view.clone(),
            source: caller.client.to_owned(),
        };
        plans.push((account, plan));
    }
    if dry_run {
        plan(session, caller, plans)?;
        return Ok(Outcome {
            value: to_value(&IntentResult::default())?,
            tasks: Vec::new(),
            notify: false,
        });
    }
    let tasks = plan_and_queue(session, caller, plans)?;

    let reads = session.reads.store()?;
    let changed = Changed::of(reads, &tasks)?;
    let wanted: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let (count, unchanged) = match &changed.messages {
        Some(msgs) if by_messages => (
            msgs.len(),
            ids.iter().filter(|i| !msgs.contains(*i)).cloned().collect(),
        ),
        Some(_) => (
            changed.threads.len(),
            ids.iter()
                .filter(|i| !changed.threads.iter().any(|(_, t)| t == *i))
                .cloned()
                .collect(),
        ),
        None => (wanted.len(), Vec::new()),
    };
    let models = changed.models(reads, by_messages)?;
    let label = mail_label(&verb, &items_of(count, by_messages));
    let undoable: Vec<(String, Value)> =
        tasks.iter().filter(|(_, t)| has_undo(t)).cloned().collect();
    let undo = if count == 0 {
        None
    } else {
        record(session, caller, verb.action(), label, count, undoable)
    };
    Ok(Outcome {
        value: to_value(&IntentResult {
            task_ids: task_ids(&tasks),
            undo,
            models,
            unchanged,
        })?,
        tasks,
        notify,
    })
}

fn category_intent(
    session: &mut Session,
    caller: &Caller<'_>,
    account: &str,
    plan: Plan,
    notify: bool,
) -> Result<Outcome, ErrorObject> {
    let mark_read = matches!(plan, Plan::CategoryMarkRead { .. });
    let tasks = plan_and_queue(session, caller, vec![(account.to_owned(), plan)])?;
    let mut result = IntentResult {
        task_ids: task_ids(&tasks),
        ..Default::default()
    };
    if mark_read {
        let reads = session.reads.store()?;
        let changed = Changed::of(reads, &tasks)?;
        result.models = changed.models(reads, false)?;
        let count = changed.threads.len();
        if count > 0 {
            let label = format!("Marked {} as read", items_of(count, false));
            let undoable = tasks.iter().filter(|(_, t)| has_undo(t)).cloned().collect();
            result.undo = record(session, caller, "categoryMarkRead", label, count, undoable);
        }
    }
    Ok(Outcome {
        value: to_value(&result)?,
        tasks,
        notify,
    })
}

/// Builds every account's tasks, then queues them account by account.
fn plan_and_queue(
    session: &Session,
    caller: &Caller<'_>,
    plans: Vec<(String, Plan)>,
) -> Result<Vec<(String, Value)>, ErrorObject> {
    let built = plan(session, caller, plans)?;
    let mut out = Vec::new();
    for (account, tasks) in built {
        if tasks.is_empty() {
            continue;
        }
        let v = session.dispatch(&account, Request::QueueTasks(tasks))?;
        out.extend(tasks_of(v).into_iter().map(|t| (account.clone(), t)));
    }
    Ok(out)
}

/// Every account's tasks, built and checked against what the caller may queue.
fn plan(
    session: &Session,
    caller: &Caller<'_>,
    plans: Vec<(String, Plan)>,
) -> Result<Vec<(String, Vec<Value>)>, ErrorObject> {
    let mut built = Vec::new();
    for (account, plan) in plans {
        let v = session.dispatch(&account, Request::Plan(Box::new(plan)))?;
        let tasks = tasks_of(v);
        if let Some(t) = tasks.iter().find(|t| !caller.may_queue(t)) {
            return Err(crate::socket::forbidden_relayed(json_str(t, "__cls")));
        }
        built.push((account, tasks));
    }
    Ok(built)
}

fn tasks_of(mut v: Value) -> Vec<Value> {
    match v["tasks"].take() {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}

fn task_ids(tasks: &[(String, Value)]) -> Vec<String> {
    tasks
        .iter()
        .map(|(_, t)| json_str(t, "id").to_owned())
        .collect()
}

/// The ids by account, accounts in the order their first id came; an id the engine doesn't
/// hold fails the call.
fn group_by_account(
    reads: &ReadStore,
    table: IdTable,
    ids: &[String],
) -> Result<Vec<(String, Vec<String>)>, ErrorObject> {
    let account_of: std::collections::HashMap<String, String> = reads
        .accounts_of(table, ids)
        .map_err(internal)?
        .into_iter()
        .collect();
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for id in ids {
        let Some(account) = account_of.get(id) else {
            return Err(ErrorObject::new(
                code::NOT_FOUND,
                format!(
                    "no {} {id}",
                    if table == IdTable::Thread {
                        "thread"
                    } else {
                        "message"
                    }
                ),
            ));
        };
        match out.iter_mut().find(|(a, _)| a == account) {
            Some((_, list)) => list.push(id.clone()),
            None => out.push((account.clone(), vec![id.clone()])),
        }
    }
    Ok(out)
}

fn category_account(session: &mut Session, category_id: &str) -> Result<String, ErrorObject> {
    let reads = session.reads.store()?;
    let account = reads.category_account(category_id).map_err(internal)?;
    let account = account.ok_or_else(|| {
        ErrorObject::new(code::NOT_FOUND, format!("no folder or label {category_id}"))
    })?;
    session.server.accounts.check(&account)?;
    Ok(account)
}

/// The messages and threads queued tasks changed.
struct Changed {
    /// The messages, when every task recorded the ones it changed.
    messages: Option<BTreeSet<String>>,
    /// (account, thread id), in order.
    threads: Vec<(String, String)>,
    message_rows: Vec<(String, String)>,
}

/// The messages a flag, label or move task's local phase changed.
fn changed_messages(task: &Value) -> Option<Vec<String>> {
    let key = match json_str(task, "__cls") {
        "ChangeUnreadTask" | "ChangeStarredTask" => "undoFlags",
        "ChangeLabelsTask" => "undoLabels",
        "ChangeFolderTask" => "undoPlacements",
        _ => return None,
    };
    Some(
        task[key]
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default(),
    )
}

fn strings(v: &Value) -> impl Iterator<Item = String> + '_ {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
}

impl Changed {
    fn of(reads: &ReadStore, tasks: &[(String, Value)]) -> Result<Changed, ErrorObject> {
        let mut all_recorded = true;
        let mut messages: Vec<(String, String)> = Vec::new();
        let mut threads: Vec<(String, String)> = Vec::new();
        for (account, t) in tasks {
            match changed_messages(t) {
                Some(ids) => messages.extend(ids.into_iter().map(|m| (account.clone(), m))),
                None => {
                    all_recorded = false;
                    threads.extend(strings(&t["threadIds"]).map(|i| (account.clone(), i)));
                    messages.extend(strings(&t["messageIds"]).map(|i| (account.clone(), i)));
                    if json_str(t, "modelClassName") == "Thread" {
                        threads.push((account.clone(), json_str(t, "modelId").to_owned()));
                    }
                }
            }
        }
        dedup_pairs(&mut messages);
        // A message's thread, for the rows and for which threads changed.
        let ids: Vec<String> = messages.iter().map(|(_, m)| m.clone()).collect();
        threads.extend(reads.message_threads(&ids).map_err(internal)?);
        dedup_pairs(&mut threads);
        Ok(Changed {
            messages: all_recorded.then(|| messages.iter().map(|(_, m)| m.clone()).collect()),
            threads,
            message_rows: messages,
        })
    }

    /// The rows as they are now: with `messages`, the Message rows first. None past
    /// [`MODELS_MAX`].
    fn models(&self, reads: &ReadStore, messages: bool) -> Result<Vec<Value>, ErrorObject> {
        let rows = self.threads.len() + if messages { self.message_rows.len() } else { 0 };
        if rows > MODELS_MAX {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(rows);
        if messages {
            for (account, id) in &self.message_rows {
                out.extend(reads.message(account, id).map_err(internal)?);
            }
        }
        for (account, id) in &self.threads {
            out.extend(reads.thread(account, id).map_err(internal)?);
        }
        Ok(out)
    }
}

fn dedup_pairs(v: &mut Vec<(String, String)>) {
    let mut seen = BTreeSet::new();
    v.retain(|p| seen.insert(p.clone()));
}

fn items_of(n: usize, messages: bool) -> String {
    match (n, messages) {
        (1, false) => "1 thread".into(),
        (1, true) => "1 message".into(),
        (n, false) => format!("{n} threads"),
        (n, true) => format!("{n} messages"),
    }
}

fn items(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}

/// The entry's English `label`.
fn mail_label(verb: &Verb, things: &str) -> String {
    match verb {
        Verb::Archive => format!("Archived {things}"),
        Verb::Trash => format!("Trashed {things}"),
        Verb::Spam => format!("Marked {things} as spam"),
        Verb::NotSpam => format!("Marked {things} as not spam"),
        Verb::Move(to) => format!("Moved {things} to {to}"),
        Verb::Label { .. } => format!("Changed labels of {things}"),
        Verb::MarkRead { unread: false } => format!("Marked {things} as read"),
        Verb::MarkRead { unread: true } => format!("Marked {things} as unread"),
        Verb::Star { starred: true } => format!("Starred {things}"),
        Verb::Star { starred: false } => format!("Unstarred {things}"),
        Verb::RemoveFromView { .. } => format!("Removed {things}"),
    }
}

fn targets_count(task: &Value) -> usize {
    let n = strings(&task["threadIds"]).count() + strings(&task["messageIds"]).count();
    n.max(1)
}

/// Adds the entry for `tasks` and sends `undo.changed`; none when no task has anything to
/// undo.
fn record(
    session: &Session,
    caller: &Caller<'_>,
    action: &'static str,
    label: String,
    count: usize,
    tasks: Vec<(String, Value)>,
) -> Option<String> {
    if tasks.is_empty() {
        return None;
    }
    let hub = &session.server.hub;
    let (token, changed) = hub.undo().record(NewEntry {
        client: caller.client.to_owned(),
        action,
        label,
        count,
        kind: Kind::Tasks {
            tasks,
            undo: Vec::new(),
        },
    });
    hub.undo_changed(changed);
    Some(token)
}

/// The stored tasks `refs` names, for an entry: each must have run its local phase
/// without error and be of a class the engine can undo. Only those with something to undo
/// are kept.
fn stored_tasks(
    session: &mut Session,
    caller: &Caller<'_>,
    refs: &[(String, String)],
) -> Result<Vec<(String, Value)>, ErrorObject> {
    if refs.is_empty() {
        return Err(invalid("invalid params: no tasks"));
    }
    for (account, _) in refs {
        session.server.accounts.check(account)?;
    }
    let reads = session.reads.store()?;
    let mut out = Vec::new();
    for (account, id) in refs {
        let task = reads
            .task(account, id)
            .map_err(internal)?
            .ok_or_else(|| ErrorObject::new(code::NOT_FOUND, format!("no task {id}")))?;
        if !undoable_class(json_str(&task, "__cls")) {
            return Err(invalid(format!(
                "task {id} is a {}, which the engine can't undo",
                json_str(&task, "__cls")
            )));
        }
        if json_str(&task, "status") == task_status::LOCAL {
            return Err(invalid(format!("task {id} hasn't run its local phase")));
        }
        if !task["error"].is_null() || json_str(&task, "status") == task_status::CANCELLED {
            return Err(invalid(format!("task {id} failed or was cancelled")));
        }
        if !caller.may_queue(&task) {
            return Err(crate::socket::forbidden_relayed(json_str(&task, "__cls")));
        }
        if has_undo(&task) {
            out.push((account.clone(), task));
        }
    }
    Ok(out)
}

/// `threads.snooze`'s answer, with the entry of its tasks as `undo`.
pub fn snoozed(
    session: &mut Session,
    caller: &Caller<'_>,
    account_id: &str,
    mut v: Value,
) -> Value {
    let refs: Vec<(String, String)> = strings(&v["taskIds"])
        .map(|id| (account_id.to_owned(), id))
        .collect();
    if refs.is_empty() {
        return v;
    }
    let Ok(tasks) = stored_tasks(session, caller, &refs) else {
        return v;
    };
    let count = tasks
        .iter()
        .filter(|(_, t)| json_str(t, "modelClassName") == "Thread")
        .count();
    let label = format!("Snoozed {}", items_of(count, false));
    if let Some(token) = record(session, caller, "snooze", label, count, tasks) {
        v["undo"] = json!(token);
    }
    v
}

/// `draft.send`'s answer, with an entry whose undo is `draft.cancelSend` when the send waits
/// out its undo window.
pub fn sent(
    session: &Session,
    caller: &Caller<'_>,
    account_id: &str,
    hmid: &str,
    mut v: Value,
) -> Value {
    let Some(until) = v["scheduledAt"].as_i64() else {
        return v;
    };
    let hub = &session.server.hub;
    let (token, changed) = hub.undo().record(NewEntry {
        client: caller.client.to_owned(),
        action: "send",
        label: "Sending 1 message".into(),
        count: 1,
        kind: Kind::Send {
            account_id: account_id.to_owned(),
            header_message_id: hmid.to_owned(),
            until,
        },
    });
    hub.undo_changed(changed);
    v["undo"] = json!(token);
    v
}

/// `undo.apply` (`redo` false) and `undo.redo`.
fn undo_apply(
    session: &mut Session,
    caller: &Caller<'_>,
    p: UndoApplyParams,
    redo: bool,
) -> Result<Outcome, ErrorObject> {
    let hub = session.server.hub.clone();
    let log = hub.undo();
    let (taken, changed) = log.take(caller.client, p.token.as_deref(), p.any, redo)?;
    hub.undo_changed(changed);
    let token = taken.token;
    let (queued, cancelled) = match taken.kind {
        Kind::Send {
            account_id,
            header_message_id,
            ..
        } => {
            let r = session.dispatch(
                &account_id,
                Request::DraftCancelSend(DraftCancelSendParams {
                    account_id: account_id.clone(),
                    header_message_id,
                }),
            );
            match r {
                Ok(v) => (Vec::new(), Some(v["cancelled"].as_bool().unwrap_or(false))),
                Err(e) => {
                    log.abort(&token);
                    return Err(e);
                }
            }
        }
        Kind::Tasks { tasks, .. } => {
            if let Some(t) = tasks.iter().find(|(_, t)| !caller.may_queue(t)) {
                log.abort(&token);
                return Err(crate::socket::forbidden_relayed(json_str(&t.1, "__cls")));
            }
            let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
            for (account, t) in tasks {
                match groups.last_mut() {
                    Some((a, list)) if *a == account => list.push(t),
                    _ => groups.push((account, vec![t])),
                }
            }
            if !redo {
                groups.reverse();
            }
            let mut queued = Vec::new();
            for (account, tasks) in groups {
                let r = session.dispatch(
                    &account,
                    Request::Undo {
                        tasks,
                        tz: p.tz.clone(),
                        redo,
                    },
                );
                match r {
                    Ok(v) => queued.extend(tasks_of(v).into_iter().map(|t| (account.clone(), t))),
                    Err(e) if queued.is_empty() => {
                        log.abort(&token);
                        return Err(e);
                    }
                    Err(e) => {
                        hub.undo_changed(log.finish(&token, queued));
                        return Err(e);
                    }
                }
            }
            (queued, None)
        }
    };
    let changed = log.finish(&token, queued.clone());
    let entry: UndoEntry = changed
        .first()
        .cloned()
        .or_else(|| log.entry(&token))
        .unwrap_or_default();
    hub.undo_changed(changed);
    let reads = session.reads.store()?;
    let models = Changed::of(reads, &queued)?.models(reads, false)?;
    Ok(Outcome {
        value: to_value(&UndoApplyResult {
            entry,
            task_ids: task_ids(&queued),
            models,
            cancelled,
        })?,
        tasks: queued,
        notify: p.notify,
    })
}
