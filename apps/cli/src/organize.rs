// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The commands that change mailboxes: `airpost archive|trash|move|mark-read|mark-unread|star|
//! unstar|spam|not-spam`, `airpost label add|remove`, `airpost label|folder create|rename|delete`
//! and `airpost folder empty`. They are the engine's intents (`threads.*`, `categories.*`): the
//! engine decides what each one means on each account (Gmail labels or folders, the archive
//! fallbacks), builds every account's tasks before it queues any, so a missing label or
//! folder changes nothing, says which threads were already as asked, and keeps an undo entry
//! (`airpost undo`). The CLI finds the threads, turns names into ids where the intent takes
//! ids, and words the outcome.

use std::cmp::Reverse;

use airpost_client::tasks::{self as builders, TASK_CLASS};
use airpost_ipc::{
    code, feature, method, CategoriesCreateParams, CategoriesRenameParams, CategoryIdParams,
    ClientError, IntentResult, MailTargets, QueryParams, QueryResult, QuerySearch, SubscribeParams,
    ThreadsActionParams, ThreadsLabelParams, ThreadsMarkReadParams, ThreadsMoveParams,
    ThreadsStarParams, READ_MAX_LIMIT,
};
use serde_json::{json, Map, Value};

use crate::app::{call_raw, confirm, is_not_found, Ctx};
use crate::cli::{CategoryWait, ThreadSelect};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::output;
use crate::read::{self, Kind};
use crate::tasks::{describe_error, failures, wait_all, ID_PREFIX};
use crate::watch::Subscriptions;

/// `source` on the tasks these commands queue.
const SOURCE: &str = "CLI";
/// How many threads `--query` without `--limit` may match.
pub const QUERY_CAP: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadAction<'a> {
    Archive,
    Trash,
    Move(&'a str),
    Unread(bool),
    Starred(bool),
    Label { name: &'a str, add: bool },
    Spam,
    NotSpam,
}

impl ThreadAction<'_> {
    fn name(self) -> &'static str {
        match self {
            ThreadAction::Archive => "archive",
            ThreadAction::Trash => "trash",
            ThreadAction::Move(_) => "move",
            ThreadAction::Unread(false) => "mark-read",
            ThreadAction::Unread(true) => "mark-unread",
            ThreadAction::Starred(true) => "star",
            ThreadAction::Starred(false) => "unstar",
            ThreadAction::Label { add: true, .. } => "label-add",
            ThreadAction::Label { add: false, .. } => "label-remove",
            ThreadAction::Spam => "spam",
            ThreadAction::NotSpam => "not-spam",
        }
    }

    /// The intent.
    fn method(self) -> &'static str {
        match self {
            ThreadAction::Archive => method::THREADS_ARCHIVE,
            ThreadAction::Trash => method::THREADS_TRASH,
            ThreadAction::Move(_) => method::THREADS_MOVE,
            ThreadAction::Unread(_) => method::THREADS_MARK_READ,
            ThreadAction::Starred(_) => method::THREADS_STAR,
            ThreadAction::Label { .. } => method::THREADS_LABEL,
            ThreadAction::Spam => method::THREADS_SPAM,
            ThreadAction::NotSpam => method::THREADS_NOT_SPAM,
        }
    }

    /// The intent's params for `thread_ids`. No perspective: a move takes every copy outside
    /// Sent and Drafts (and Not Spam only the Spam copies), as the CLI always did.
    fn params(self, thread_ids: Vec<String>, dry_run: bool) -> Value {
        let targets = MailTargets {
            thread_ids,
            message_ids: Vec::new(),
        };
        let v = match self {
            ThreadAction::Move(to) => serde_json::to_value(ThreadsMoveParams {
                targets,
                to: to.to_owned(),
                perspective: None,
                notify: false,
                dry_run,
            }),
            ThreadAction::Unread(unread) => serde_json::to_value(ThreadsMarkReadParams {
                targets,
                unread,
                notify: false,
                dry_run,
            }),
            ThreadAction::Starred(starred) => serde_json::to_value(ThreadsStarParams {
                targets,
                starred,
                notify: false,
                dry_run,
            }),
            ThreadAction::Label { name, add } => {
                let names = vec![name.to_owned()];
                let (add, remove) = if add {
                    (names, Vec::new())
                } else {
                    (Vec::new(), names)
                };
                serde_json::to_value(ThreadsLabelParams {
                    targets,
                    add,
                    remove,
                    notify: false,
                    dry_run,
                })
            }
            ThreadAction::Archive
            | ThreadAction::Trash
            | ThreadAction::Spam
            | ThreadAction::NotSpam => serde_json::to_value(ThreadsActionParams {
                targets,
                perspective: None,
                notify: false,
                dry_run,
            }),
        };
        v.unwrap_or_else(|_| json!({}))
    }

    /// What the output says: done, would do, and the threads left alone.
    fn sentence(self, n: usize, unchanged: usize, dry_run: bool) -> String {
        let threads = |n: usize| format!("{n} thread{}", if n == 1 { "" } else { "s" });
        let category = match self {
            ThreadAction::Move(c) | ThreadAction::Label { name: c, .. } => c,
            _ => "",
        };
        let (done, would, left) = match self {
            ThreadAction::Archive => ("Archived {}", "Would archive {}", "already archived"),
            ThreadAction::Trash => (
                "Moved {} to the trash",
                "Would move {} to the trash",
                "already there",
            ),
            ThreadAction::Move(_) => ("Moved {} to {c}", "Would move {} to {c}", "already there"),
            ThreadAction::Unread(false) => ("Marked {} read", "Would mark {} read", "already read"),
            ThreadAction::Unread(true) => {
                ("Marked {} unread", "Would mark {} unread", "already unread")
            }
            ThreadAction::Starred(true) => ("Starred {}", "Would star {}", "already starred"),
            ThreadAction::Starred(false) => ("Unstarred {}", "Would unstar {}", "not starred"),
            ThreadAction::Label { add: true, .. } => {
                ("Added {c} to {}", "Would add {c} to {}", "already have it")
            }
            ThreadAction::Label { add: false, .. } => (
                "Removed {c} from {}",
                "Would remove {c} from {}",
                "don't have it",
            ),
            ThreadAction::Spam => ("Moved {} to spam", "Would move {} to spam", "already there"),
            ThreadAction::NotSpam => (
                "Moved {} out of spam",
                "Would move {} out of spam",
                "not in spam",
            ),
        };
        let mut s = if n == 0 {
            "Nothing to change".to_owned()
        } else {
            (if dry_run { would } else { done })
                .replace("{c}", category)
                .replace("{}", &threads(n))
        };
        if unchanged > 0 {
            s.push_str(&format!(" ({} {left})", threads(unchanged)));
        }
        s
    }

    /// What to try next when the engine knows no folder or label by that name.
    fn hint(self, message: &str) -> String {
        let name = match self {
            ThreadAction::Move(c) | ThreadAction::Label { name: c, .. } => c,
            _ => return String::new(),
        };
        if message.starts_with("no folder or label") {
            format!("; `airpost labels` lists them, `airpost label create {name}` makes a label")
        } else if message.starts_with("no folder") {
            format!("; `airpost folders` lists them, `airpost folder create {name}` makes one")
        } else if message.starts_with("no label") {
            format!("; `airpost labels` lists them, `airpost label create {name}` makes one")
        } else if message.contains("folders, not labels") {
            "; `airpost move <folder>` files threads".to_owned()
        } else {
            String::new()
        }
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

/// The name a person writes for a stored folder or label path.
pub(crate) fn display_name(category: &Value) -> String {
    airpost_client::utf7::decode_or_raw(str_of(category, "path"))
}

/// A folder or label named by id, by path, or by the path as a person writes it, without
/// regard to case.
pub(crate) fn find_named<'a>(list: &'a [Value], name: &str) -> Option<&'a Value> {
    let lower = name.to_lowercase();
    list.iter()
        .find(|c| str_of(c, "id") == name)
        .or_else(|| {
            list.iter()
                .find(|c| str_of(c, "path").eq_ignore_ascii_case(name))
        })
        .or_else(|| {
            list.iter()
                .find(|c| display_name(c).to_lowercase() == lower)
        })
}

pub(crate) fn by_role<'a>(list: &'a [Value], role: &str) -> Option<&'a Value> {
    list.iter().find(|c| str_of(c, "role") == role)
}

/// An account's folders and labels, each with its `__cls`, and the counts of both.
pub(crate) struct Categories {
    pub(crate) folders: Vec<Value>,
    pub(crate) labels: Vec<Value>,
    pub(crate) counts: Map<String, Value>,
}

impl Categories {
    pub(crate) fn load(engines: &mut [Engine], t: &Target) -> Result<Categories> {
        let tagged = |list: Vec<Value>, cls: &str| -> Vec<Value> {
            list.into_iter()
                .map(|mut c| {
                    if c.get("__cls").is_none() {
                        c["__cls"] = json!(cls);
                    }
                    c
                })
                .collect()
        };
        let (folders, mut counts) = read::list_categories(engines, t, Kind::Folder)?;
        let (labels, label_counts) = read::list_categories(engines, t, Kind::Label)?;
        counts.extend(label_counts);
        Ok(Categories {
            folders: tagged(folders, "Folder"),
            labels: tagged(labels, "Label"),
            counts,
        })
    }

    /// `Account.usesLabels()`: Gmail, the only provider the engine keeps labels for.
    pub(crate) fn uses_labels(&self) -> bool {
        !self.labels.is_empty()
    }
}

/// A CLI task id, and the CLI as the source.
pub(crate) fn stamp(task: &mut Value) {
    task["id"] = json!(builders::new_task_id(ID_PREFIX));
    if task.get("source").is_some() {
        task["source"] = json!(SOURCE);
    }
}

/// Threads by account, accounts in the order first seen.
type ByAccount = Vec<(Target, Vec<Value>)>;
/// The threads of `ids`, by account in the order first seen, each found in whichever account
/// of `targets` has it.
fn by_ids(engines: &mut [Engine], targets: &[Target], ids: &[String]) -> Result<ByAccount> {
    let mut groups: ByAccount = Vec::new();
    for id in ids {
        if groups
            .iter()
            .any(|(_, list)| list.iter().any(|t| str_of(t, "id") == id))
        {
            continue;
        }
        let mut found = None;
        for t in targets {
            let params = json!({ "accountId": t.account_id, "id": id });
            match call_raw::<Value>(engines, t, method::THREADS_GET, params) {
                Ok(mut r) => {
                    found = Some((t, r["thread"].take()));
                    break;
                }
                Err(e) if is_not_found(&e) => {}
                Err(e) => return Err(CliError::call(&t.email, method::THREADS_GET, e)),
            }
        }
        let (t, thread) = found.ok_or_else(|| {
            CliError::failed(format!(
                "no thread {id} in {}",
                match targets {
                    [one] => one.email.as_str(),
                    _ => "any account",
                }
            ))
        })?;
        match groups
            .iter_mut()
            .find(|(g, _)| g.account_id == t.account_id)
        {
            Some((_, list)) => list.push(thread),
            None => groups.push((t.clone(), vec![thread])),
        }
    }
    Ok(groups)
}

/// The newest `want` threads of `targets` matching `query` (the search grammar), by account
/// in target order, and whether there were more. Paged with `query`'s cursor, so a query can
/// match more threads than one read returns.
fn by_query(
    engines: &mut [Engine],
    targets: &[Target],
    query: &str,
    want: usize,
) -> Result<(ByAccount, bool)> {
    let mut found: Vec<Value> = Vec::new();
    let mut more = false;
    let mut engines_done: Vec<usize> = Vec::new();
    for t in targets {
        if engines_done.contains(&t.engine) {
            continue;
        }
        engines_done.push(t.engine);
        let e = &engines[t.engine];
        if !e.has_feature(feature::QUERY) || !e.has_feature(feature::SEARCH_GRAMMAR) {
            return Err(CliError::failed(format!(
                "{}: this engine can't change threads by query (no {} with {}); update Airpost, \
                 or give thread ids",
                t.email,
                feature::QUERY,
                feature::SEARCH_GRAMMAR
            )));
        }
        let accounts: Vec<String> = targets
            .iter()
            .filter(|x| x.engine == t.engine)
            .map(|x| x.account_id.clone())
            .collect();
        let grammar = airpost_client::search::grammar_search(query, None);
        let mut params = QueryParams {
            class: "Thread".into(),
            accounts: Some(accounts),
            search: Some(QuerySearch {
                text: query.to_owned(),
                grammar: true,
                tz: grammar.tz,
                now: grammar.now,
            }),
            cursor: Some(String::new()),
            ..Default::default()
        };
        let mut got = 0;
        loop {
            params.limit = Some((want - got).clamp(1, READ_MAX_LIMIT) as i64);
            let p = serde_json::to_value(&params).expect("params");
            let r: QueryResult = match call_raw(engines, t, method::QUERY, p) {
                Ok(r) => r,
                Err(ClientError::Rpc(e)) if e.code == code::INVALID_PARAMS => {
                    let why = e.message.trim_start_matches("invalid params: ");
                    let why = why.strip_prefix("search: ").unwrap_or(why);
                    return Err(CliError::usage(format!("--query: {why}")));
                }
                Err(e) => return Err(CliError::call(&t.email, method::QUERY, e)),
            };
            got += r.rows.len();
            found.extend(r.rows);
            match r.next_cursor.flatten() {
                Some(c) if got < want => params.cursor = Some(c),
                Some(_) => {
                    more = true;
                    break;
                }
                None => break,
            }
        }
    }
    // Each engine's rows come in `Thread`'s natural order; merged the same way.
    found.sort_by_key(|t| Reverse((t["lmrt"].as_i64().unwrap_or(0), str_of(t, "id").to_owned())));
    if found.len() > want {
        found.truncate(want);
        more = true;
    }
    let mut groups: ByAccount = Vec::new();
    for t in targets {
        let mine: Vec<Value> = found
            .iter()
            .filter(|th| str_of(th, "aid") == t.account_id)
            .cloned()
            .collect();
        if !mine.is_empty() {
            groups.push((t.clone(), mine));
        }
    }
    Ok((groups, more))
}

/// Queues `task` for `t`, after its local phase; a local phase that fails is the error.
pub(crate) fn queue(engines: &mut [Engine], t: &Target, task: &Value) -> Result<Value> {
    let params = json!({ "accountId": t.account_id, "task": task });
    match engines[t.engine].client.call(method::TASK_QUEUE, params) {
        Ok(v) => Ok(v),
        Err(ClientError::Rpc(e)) if e.code == code::TASK_REJECTED => {
            let data = e.data.unwrap_or_else(|| json!({ "key": e.message }));
            Err(CliError::failed(format!(
                "{}: {} was refused: {}",
                t.email,
                str_of(task, "__cls"),
                describe_error(&data)
            )))
        }
        Err(e) => Err(CliError::call(&t.email, method::TASK_QUEUE, e)),
    }
}

/// Subscribes each engine to the `Task` deltas of `accounts` on it, so a wait that starts
/// after queueing misses none.
fn subscribe(engines: &mut [Engine], accounts: &[&Target]) -> Result<Vec<usize>> {
    let mut done: Vec<usize> = Vec::new();
    for t in accounts {
        if done.contains(&t.engine) {
            continue;
        }
        done.push(t.engine);
        let params = SubscribeParams {
            accounts: Some(
                accounts
                    .iter()
                    .filter(|x| x.engine == t.engine)
                    .map(|x| x.account_id.clone())
                    .collect(),
            ),
            classes: Some(vec![TASK_CLASS.to_owned()]),
            since: None,
        };
        engines[t.engine]
            .client
            .subscribe(&params)
            .map_err(|e| CliError::call(&t.email, method::SUBSCRIBE, e))?;
    }
    Ok(done)
}

/// The subscribed engines, for [`wait_all`].
fn take_engines(engines: Vec<Engine>, picked: &[usize]) -> Vec<(Engine, bool)> {
    engines
        .into_iter()
        .enumerate()
        .filter(|(i, _)| picked.contains(i))
        .map(|(_, e)| (e, true))
        .collect()
}
/// One account's threads in a thread command, and what the engine said of them.
struct Group {
    target: Target,
    threads: Vec<Value>,
    unchanged: Vec<String>,
}

impl Group {
    fn ids(&self) -> Vec<String> {
        self.threads
            .iter()
            .map(|t| str_of(t, "id").to_owned())
            .collect()
    }

    fn changed(&self) -> Vec<&Value> {
        self.threads
            .iter()
            .filter(|t| !self.unchanged.iter().any(|u| u == str_of(t, "id")))
            .collect()
    }

    fn json(&self) -> Value {
        json!({
            "accountId": self.target.account_id,
            "email": self.target.email,
            "threadIds": self.changed().iter().map(|t| str_of(t, "id")).collect::<Vec<_>>(),
            "unchangedThreadIds": self.unchanged,
        })
    }
}

/// The intent's error, with what to try next and the account when there is one.
fn intent_error(action: ThreadAction, groups: &[Group], on: &Target, e: ClientError) -> CliError {
    match e {
        ClientError::Rpc(err)
            if err.code == code::NOT_FOUND || err.code == code::INVALID_PARAMS =>
        {
            let why = err.message.trim_start_matches("invalid params: ");
            let msg = format!("{why}{}", action.hint(why));
            match groups {
                [one] => CliError::failed(format!("{}: {msg}", one.target.email)),
                _ => CliError::failed(msg),
            }
        }
        e => CliError::call(&on.email, action.method(), e),
    }
}

pub fn threads(
    ctx: &Ctx,
    mut engines: Vec<Engine>,
    action: ThreadAction,
    select: &ThreadSelect,
) -> Result<()> {
    // Not clap's `requires`: it doesn't fire while the conflicting thread ids are present.
    if select.limit.is_some() && select.query.is_none() {
        return Err(CliError::usage("--limit goes with --query"));
    }
    let targets = discovery::targets(&engines, ctx.account())?;
    let found = match &select.query {
        Some(q) => {
            let want = select.limit.unwrap_or(QUERY_CAP + 1).max(1);
            let (groups, more) = by_query(&mut engines, &targets, q, want)?;
            if more && select.limit.is_none() {
                return Err(CliError::usage(format!(
                    "--query matches more than {QUERY_CAP} threads; give --limit N to act on \
                     the newest N (with --dry-run to see them first)"
                )));
            }
            groups
        }
        None => by_ids(&mut engines, &targets, &select.threads)?,
    };
    let mut groups: Vec<Group> = found
        .into_iter()
        .map(|(target, threads)| Group {
            target,
            threads,
            unchanged: Vec::new(),
        })
        .collect();
    if select.query.is_some() && groups.is_empty() && !ctx.json {
        output::line("No threads match.");
        return Ok(());
    }
    // One call per engine, for all of its accounts' threads: one undo entry.
    let mut by_engine: Vec<usize> = Vec::new();
    for g in &groups {
        if !by_engine.contains(&g.target.engine) {
            by_engine.push(g.target.engine);
        }
    }
    for &e in &by_engine {
        if !engines[e].has_feature(feature::INTENTS) {
            let t = groups
                .iter()
                .find(|g| g.target.engine == e)
                .map(|g| &g.target);
            return Err(CliError::failed(format!(
                "{}: this engine can't {} threads (no {}); update Airpost",
                t.map_or("", |t| t.email.as_str()),
                action.name(),
                feature::INTENTS
            )));
        }
    }
    let subscribed = if select.wait {
        let acting: Vec<&Target> = groups.iter().map(|g| &g.target).collect();
        subscribe(&mut engines, &acting)?
    } else {
        Vec::new()
    };
    let mut task_ids: Vec<String> = Vec::new();
    let mut undo: Option<String> = None;
    for &e in &by_engine {
        let on = groups
            .iter()
            .find(|g| g.target.engine == e)
            .map(|g| g.target.clone())
            .expect("an engine of the groups");
        let ids: Vec<String> = groups
            .iter()
            .filter(|g| g.target.engine == e)
            .flat_map(Group::ids)
            .collect();
        let params = action.params(ids, select.dry_run);
        let r: IntentResult = match call_raw(&mut engines, &on, action.method(), params) {
            Ok(r) => r,
            Err(err) => {
                let err = intent_error(action, &groups, &on, err);
                return Err(match task_ids.as_slice() {
                    [] => err,
                    q => CliError::failed(format!("{err}\nAlready queued: {}", q.join(", "))),
                });
            }
        };
        for g in groups.iter_mut().filter(|g| g.target.engine == e) {
            g.unchanged = g
                .ids()
                .into_iter()
                .filter(|id| r.unchanged.contains(id))
                .collect();
        }
        task_ids.extend(r.task_ids);
        undo = undo.or(r.undo);
    }
    let multi = groups.len() > 1;
    let line = |g: &Group, dry_run: bool| {
        let s = action.sentence(g.changed().len(), g.unchanged.len(), dry_run);
        if multi {
            format!("{}: {s}", g.target.email)
        } else {
            s
        }
    };
    let mut doc = json!({
        "action": action.name(),
        "dryRun": select.dry_run,
        "accounts": groups.iter().map(Group::json).collect::<Vec<_>>(),
        "taskIds": task_ids,
        "undo": undo,
    });
    if select.dry_run {
        if ctx.json {
            let threads: Vec<&Value> = groups.iter().flat_map(|g| &g.threads).collect();
            doc["threads"] = json!(threads);
            output::json(&doc);
            return Ok(());
        }
        for g in &groups {
            output::line(&format!("{}:", line(g, true)));
            let rows: Vec<(Value, usize)> = g.threads.iter().map(|t| (t.clone(), 0)).collect();
            read::print_threads(ctx, std::slice::from_ref(&g.target), &rows);
        }
        return Ok(());
    }
    if !ctx.json {
        let changed = groups.iter().any(|g| !g.changed().is_empty());
        let tasks = match task_ids.as_slice() {
            _ if !changed => String::new(),
            [one] => format!("task {one}"),
            many => format!("tasks {}", many.join(", ")),
        };
        match groups.as_slice() {
            [one] if !tasks.is_empty() => output::line(&format!("{}; {tasks}", line(one, false))),
            _ => {
                for g in &groups {
                    output::line(&line(g, false));
                }
                if !tasks.is_empty() {
                    output::line(&format!("Queued {tasks}"));
                }
            }
        }
    }
    if !select.wait || task_ids.is_empty() {
        if ctx.json {
            output::json(&doc);
        }
        return Ok(());
    }
    let subs = Subscriptions::start(take_engines(engines, &subscribed), None, None)?;
    let finished = wait_all(subs, &task_ids, select.timeout)?;
    let failed = failures(&finished);
    if ctx.json {
        doc["tasks"] = json!(finished);
        output::json(&doc);
    } else if failed.is_empty() {
        output::line("Done: the server has the changes.");
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(CliError::failed(failed.join("\n")))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CategoryOp<'a> {
    Create(&'a str),
    Rename { from: &'a str, to: &'a str },
    Delete { name: &'a str, force: bool },
}

pub fn category(
    ctx: &Ctx,
    mut engines: Vec<Engine>,
    kind: Kind,
    op: CategoryOp,
    wait: &CategoryWait,
) -> Result<()> {
    let t = discovery::one_target(&engines, ctx.account())?;
    let cats = Categories::load(&mut engines, &t)?;
    let email = &t.email;
    let noun = kind.noun();
    match (kind, cats.uses_labels()) {
        (Kind::Label, false) => {
            return Err(CliError::failed(format!(
                "{email} has folders, not labels (only Gmail has labels); use `airpost folder`"
            )))
        }
        (Kind::Folder, true) => {
            return Err(CliError::failed(format!(
                "{email} is a Gmail account, with labels instead of folders; use `airpost label`"
            )))
        }
        _ => {}
    }
    let list = match kind {
        Kind::Folder => &cats.folders,
        Kind::Label => &cats.labels,
    };
    let existing = |name: &str| {
        find_named(list, name).ok_or_else(|| {
            CliError::failed(format!(
                "{email}: no {noun} {name}; `airpost {}` lists them",
                kind.key()
            ))
        })
    };
    let not_standard = |c: &Value, doing: &str| match str_of(c, "role") {
        "" => Ok(()),
        role => Err(CliError::failed(format!(
            "{email}: {} is the account's {role} {noun} and can't be {doing}",
            display_name(c)
        ))),
    };
    let named = |name: &str| -> Result<String> {
        let name = name.trim().trim_matches('/');
        if name.is_empty() {
            return Err(CliError::usage(format!("the {noun} needs a name")));
        }
        Ok(name.to_owned())
    };
    let id_of = |c: &Value| str_of(c, "id").to_owned();
    let (intent, params, done) = match op {
        CategoryOp::Create(name) => {
            let name = named(name)?;
            let params = serde_json::to_value(CategoriesCreateParams {
                account_id: t.account_id.clone(),
                name: name.clone(),
                notify: false,
            });
            (
                method::CATEGORIES_CREATE,
                params,
                format!("Created {noun} {name}"),
            )
        }
        CategoryOp::Rename { from, to } => {
            let c = existing(from)?;
            let to = named(to)?;
            let params = serde_json::to_value(CategoriesRenameParams {
                category_id: id_of(c),
                name: to.clone(),
                notify: false,
            });
            (
                method::CATEGORIES_RENAME,
                params,
                format!("Renamed {noun} {} to {to}", display_name(c)),
            )
        }
        CategoryOp::Delete { name, force } => {
            let c = existing(name)?;
            not_standard(c, "deleted")?;
            let total = cats
                .counts
                .get(str_of(c, "id"))
                .map_or(0, |n| n["total"].as_i64().unwrap_or(0));
            if kind == Kind::Folder && total > 0 && !force {
                return Err(CliError::failed(format!(
                    "{email}: {} holds {total} thread{}, and deleting the folder deletes them on \
                     the server; move them out first, or give --force",
                    display_name(c),
                    if total == 1 { "" } else { "s" }
                )));
            }
            let params = serde_json::to_value(CategoryIdParams {
                category_id: id_of(c),
                notify: false,
            });
            (
                method::CATEGORIES_DELETE,
                params,
                format!("Deleted {noun} {}", display_name(c)),
            )
        }
    };
    let subscribed = if wait.no_wait {
        Vec::new()
    } else {
        subscribe(&mut engines, &[&t])?
    };
    let id = category_intent(&mut engines, &t, intent, params.unwrap_or_default())?;
    if wait.no_wait {
        if ctx.json {
            output::json(&json!({"accountId": t.account_id, "taskId": id}));
        } else {
            output::line(&format!(
                "Queued: {}; task {id} (`airpost task wait {id}`)",
                done.to_lowercase()
            ));
        }
        return Ok(());
    }
    let subs = Subscriptions::start(take_engines(engines, &subscribed), None, None)?;
    let finished = wait_all(subs, std::slice::from_ref(&id), Some(wait.timeout))?
        .pop()
        .unwrap_or_default();
    if let Some(failure) = failures(std::slice::from_ref(&finished)).pop() {
        return Err(CliError::failed(format!("{email}: {failure}")));
    }
    let created = &finished["created"];
    if ctx.json {
        let mut doc = json!({"accountId": t.account_id, "taskId": id,
                             "status": finished["status"]});
        if created.is_object() {
            doc[noun] = created.clone();
        }
        output::json(&doc);
    } else if created.is_object() {
        output::line(&format!("{done} (id {})", str_of(created, "id")));
    } else {
        output::line(&done);
    }
    Ok(())
}

/// Calls a `categories.*` intent and returns its task's id.
fn category_intent(
    engines: &mut [Engine],
    t: &Target,
    intent: &str,
    params: Value,
) -> Result<String> {
    if !engines[t.engine].has_feature(feature::INTENTS) {
        return Err(CliError::failed(format!(
            "{}: this engine can't change folders or labels (no {}); update Airpost",
            t.email,
            feature::INTENTS
        )));
    }
    let r: IntentResult = match call_raw(engines, t, intent, params) {
        Ok(r) => r,
        Err(ClientError::Rpc(e)) if e.code == code::NOT_FOUND || e.code == code::INVALID_PARAMS => {
            let why = e.message.trim_start_matches("invalid params: ");
            return Err(CliError::failed(format!("{}: {why}", t.email)));
        }
        Err(e) => return Err(CliError::call(&t.email, intent, e)),
    };
    r.task_ids
        .into_iter()
        .next()
        .ok_or_else(|| CliError::failed(format!("{}: {intent} queued nothing", t.email)))
}

/// `airpost folder empty`'s arguments.
#[derive(Clone, Copy, Debug)]
pub struct EmptyArgs<'a> {
    pub folder: &'a str,
    pub dry_run: bool,
    pub yes: bool,
    pub wait: bool,
    pub timeout: Option<u64>,
}

/// The folder `name` names in `cats`, if it may be emptied: trash, spam, or one without a
/// role. `by_role` names trash or spam by role.
fn folder_to_empty<'a>(
    cats: &'a Categories,
    name: &str,
    by_role_name: bool,
    email: &str,
) -> Result<Option<&'a Value>> {
    let folder = if by_role_name {
        by_role(&cats.folders, &name.to_lowercase())
    } else {
        find_named(&cats.folders, name)
    };
    let Some(folder) = folder else {
        if by_role_name {
            return Ok(None);
        }
        let hint = if cats.uses_labels() {
            "; on Gmail only trash and spam are folders that can be emptied"
        } else {
            "; `airpost folders` lists them"
        };
        return Err(CliError::failed(format!("{email}: no folder {name}{hint}")));
    };
    match str_of(folder, "role") {
        "" | "trash" | "spam" => Ok(Some(folder)),
        role => Err(CliError::failed(format!(
            "{email}: {} is the account's {role} folder; only trash, spam and folders without \
             a role can be emptied",
            display_name(folder)
        ))),
    }
}

fn threads_text(n: i64) -> String {
    format!("{n} thread{}", if n == 1 { "" } else { "s" })
}

/// `airpost folder empty`: `ExpungeAllInFolderTask` per account, after a confirmation.
pub fn empty_folder(ctx: &Ctx, mut engines: Vec<Engine>, a: &EmptyArgs) -> Result<()> {
    let by_role_name = ["trash", "spam"].contains(&a.folder.to_lowercase().as_str());
    let targets = if by_role_name {
        discovery::targets(&engines, ctx.account())?
    } else {
        vec![discovery::one_target(&engines, ctx.account())?]
    };
    let mut plans: Vec<(Target, Value, i64)> = Vec::new();
    for t in &targets {
        let cats = Categories::load(&mut engines, t)?;
        match folder_to_empty(&cats, a.folder, by_role_name, &t.email)? {
            Some(f) => {
                let total = cats
                    .counts
                    .get(str_of(f, "id"))
                    .map_or(0, |n| n["total"].as_i64().unwrap_or(0));
                plans.push((t.clone(), f.clone(), total));
            }
            None if targets.len() == 1 => {
                return Err(CliError::failed(format!(
                    "{} has no {} folder",
                    t.email,
                    a.folder.to_lowercase()
                )))
            }
            None => {}
        }
    }
    let multi = plans.len() > 1;
    let line = |t: &Target, s: String| {
        if multi {
            format!("{}: {s}", t.email)
        } else {
            s
        }
    };
    let entry = |t: &Target, f: &Value, n: i64| {
        json!({"accountId": t.account_id, "email": t.email, "threads": n,
               "folder": {"id": f["id"], "path": f["path"], "role": f["role"]}})
    };
    if a.dry_run {
        if ctx.json {
            let accounts: Vec<Value> = plans.iter().map(|(t, f, n)| entry(t, f, *n)).collect();
            output::json(&json!({"dryRun": true, "accounts": accounts}));
        } else {
            for (t, f, n) in &plans {
                output::line(&line(
                    t,
                    format!(
                        "Would delete {} in {} for good",
                        threads_text(*n),
                        display_name(f)
                    ),
                ));
            }
        }
        return Ok(());
    }
    plans.retain(|(_, _, n)| *n > 0);
    if plans.is_empty() {
        if ctx.json {
            output::json(&json!({"dryRun": false, "accounts": []}));
        } else {
            output::line("Nothing to delete: the folder is empty");
        }
        return Ok(());
    }
    let question = match plans.as_slice() {
        [(t, f, n)] => format!(
            "Delete the {} in {} of {} for good?",
            threads_text(*n),
            display_name(f),
            t.email
        ),
        many => format!(
            "Delete {} in {} of {} accounts for good?",
            threads_text(many.iter().map(|(_, _, n)| n).sum()),
            a.folder.to_lowercase(),
            many.len()
        ),
    };
    confirm(ctx, a.yes, &question)?;
    let subscribed = if a.wait {
        let acting: Vec<&Target> = plans.iter().map(|(t, _, _)| t).collect();
        subscribe(&mut engines, &acting)?
    } else {
        Vec::new()
    };
    let mut queued: Vec<String> = Vec::new();
    for (t, f, n) in &plans {
        let params = serde_json::to_value(CategoryIdParams {
            category_id: str_of(f, "id").to_owned(),
            notify: false,
        });
        let id = category_intent(
            &mut engines,
            t,
            method::CATEGORIES_EMPTY,
            params.unwrap_or_default(),
        )?;
        if !ctx.json {
            output::line(&line(
                t,
                format!(
                    "Deleting {} in {}; task {id}",
                    threads_text(*n),
                    display_name(f)
                ),
            ));
        }
        queued.push(id);
    }
    let mut accounts: Vec<Value> = plans
        .iter()
        .zip(&queued)
        .map(|((t, f, n), id)| {
            let mut e = entry(t, f, *n);
            e["taskId"] = json!(id);
            e
        })
        .collect();
    if !a.wait {
        if ctx.json {
            output::json(&json!({"dryRun": false, "accounts": accounts}));
        }
        return Ok(());
    }
    let subs = Subscriptions::start(take_engines(engines, &subscribed), None, None)?;
    let finished = wait_all(subs, &queued, a.timeout)?;
    let failed = failures(&finished);
    if ctx.json {
        for (e, task) in accounts.iter_mut().zip(&finished) {
            e["task"] = task.clone();
        }
        output::json(&json!({"dryRun": false, "accounts": accounts}));
    } else if failed.is_empty() {
        output::line("Done: the server has deleted them.");
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(CliError::failed(failed.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cats() -> Categories {
        let folder = |id: &str, path: &str, role: &str| json!({"id": id, "aid": "a", "path": path, "role": role, "__cls": "Folder"});
        Categories {
            folders: vec![
                folder("f-in", "INBOX", "inbox"),
                folder("f-ar", "Archive", "archive"),
                folder("f-tr", "Trash", "trash"),
                folder("f-sent", "Sent", "sent"),
                folder("f-proj", "INBOX.Projects", ""),
                folder("f-spam", "Junk", "spam"),
            ],
            labels: vec![],
            counts: Map::new(),
        }
    }

    #[test]
    fn intents_and_their_params() {
        let ids = || vec!["t1".to_owned()];
        assert_eq!(ThreadAction::Archive.method(), method::THREADS_ARCHIVE);
        assert_eq!(
            ThreadAction::Archive.params(ids(), false),
            json!({"threadIds": ["t1"]})
        );
        assert_eq!(
            ThreadAction::Move("Receipts").params(ids(), true),
            json!({"threadIds": ["t1"], "to": "Receipts", "dryRun": true})
        );
        assert_eq!(
            ThreadAction::Unread(true).params(ids(), false),
            json!({"threadIds": ["t1"], "unread": true})
        );
        assert_eq!(
            ThreadAction::Starred(false).params(ids(), false),
            json!({"threadIds": ["t1"], "starred": false})
        );
        assert_eq!(
            ThreadAction::Label {
                name: "Work",
                add: false
            }
            .params(ids(), false),
            json!({"threadIds": ["t1"], "remove": ["Work"]})
        );
    }

    #[test]
    fn hints_for_unknown_names() {
        let mv = ThreadAction::Move("Nope");
        assert!(mv
            .hint("no folder Nope")
            .contains("airpost folder create Nope"));
        assert!(mv
            .hint("no folder or label Nope")
            .contains("airpost label create Nope"));
        let label = ThreadAction::Label {
            name: "Receipts",
            add: true,
        };
        assert!(label
            .hint("no label Receipts")
            .contains("airpost label create Receipts"));
        assert!(label
            .hint("the account has folders, not labels (only Gmail has labels)")
            .contains("airpost move"));
        assert_eq!(ThreadAction::Archive.hint("no folder x"), "");
    }

    #[test]
    fn only_trash_spam_and_plain_folders_are_emptied() {
        let c = cats();
        let id = |name, by_role| {
            folder_to_empty(&c, name, by_role, "me@x.test")
                .map(|f| f.map(|f| str_of(f, "id").to_owned()))
        };
        assert_eq!(id("trash", true).unwrap().as_deref(), Some("f-tr"));
        assert_eq!(id("spam", true).unwrap().as_deref(), Some("f-spam"));
        assert_eq!(
            id("INBOX.Projects", false).unwrap().as_deref(),
            Some("f-proj")
        );
        let mut no_spam = cats();
        no_spam.folders.retain(|f| f["role"] != "spam");
        assert_eq!(
            folder_to_empty(&no_spam, "spam", true, "me@x.test").unwrap(),
            None
        );
        for refused in ["INBOX", "Sent", "Archive"] {
            let e = id(refused, false).unwrap_err().to_string();
            assert!(e.contains("can be emptied"), "{refused}: {e}");
        }
        assert!(id("Nowhere", false).is_err());
    }

    #[test]
    fn sentences() {
        let s = |a: ThreadAction, n, u, dry| a.sentence(n, u, dry);
        assert_eq!(
            s(ThreadAction::Archive, 3, 1, false),
            "Archived 3 threads (1 thread already archived)"
        );
        assert_eq!(
            s(
                ThreadAction::Label {
                    name: "Work",
                    add: true
                },
                1,
                0,
                true
            ),
            "Would add Work to 1 thread"
        );
        assert_eq!(
            s(ThreadAction::Move("Work"), 0, 2, false),
            "Nothing to change (2 threads already there)"
        );
        assert_eq!(
            s(ThreadAction::Unread(false), 2, 0, false),
            "Marked 2 threads read"
        );
        assert_eq!(
            s(ThreadAction::NotSpam, 1, 0, false),
            "Moved 1 thread out of spam"
        );
    }
}
