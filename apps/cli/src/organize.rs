// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The commands that change mailboxes: `tern archive|trash|move|mark-read|mark-unread|star|
//! unstar`, `tern label add|remove`, and `tern label|folder create|rename|delete`. Thread
//! changes queue the task JSON the desktop queues for the same action
//! (`tern_client::tasks`): per account, one task for every [`THREADS_PER_TASK`] threads,
//! leaving out the threads already as asked. Everything is looked up before the first task
//! is queued, so a missing label or folder changes nothing.

use std::cmp::Reverse;

use serde_json::{json, Map, Value};
use tern_client::tasks::{self as builders, TASK_CLASS};
use tern_ipc::{
    code, feature, method, ClientError, QueryParams, QueryResult, QuerySearch, SubscribeParams,
    READ_MAX_LIMIT,
};

use crate::app::{call_raw, is_not_found, Ctx};
use crate::cli::{CategoryWait, ThreadSelect};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::output;
use crate::read::{self, Kind};
use crate::tasks::{describe_error, failures, wait_all, ID_PREFIX};
use crate::watch::Subscriptions;

/// `source` on the tasks these commands queue.
const SOURCE: &str = "CLI";
/// Threads per task: a bulk change is a few tasks, each one local transaction and a bounded
/// remote phase.
const THREADS_PER_TASK: usize = 500;
/// How many threads `--query` without `--limit` may match.
pub const QUERY_CAP: usize = 500;
/// Folders a move leaves a thread's copies in (`ChangeFolderTask` with no
/// `sourceFolderIds`).
const KEPT_ROLES: [&str; 2] = ["sent", "drafts"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadAction<'a> {
    Archive,
    Trash,
    Move(&'a str),
    Unread(bool),
    Starred(bool),
    Label { name: &'a str, add: bool },
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
        }
    }

    /// What the output says: done, would do, and the threads left alone.
    fn sentence(self, category: &str, n: usize, unchanged: usize, dry_run: bool) -> String {
        let threads = |n: usize| format!("{n} thread{}", if n == 1 { "" } else { "s" });
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
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

/// `unread` and `starred` are counts on a thread, booleans on a message.
fn flag_of(v: &Value, key: &str) -> bool {
    v[key].as_i64().map(|n| n > 0).or(v[key].as_bool()) == Some(true)
}

fn ids_in<'a>(thread: &'a Value, key: &str) -> impl Iterator<Item = &'a str> {
    thread[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| str_of(c, "id"))
}

/// The name a person writes for a stored folder or label path.
pub(crate) fn display_name(category: &Value) -> String {
    tern_client::utf7::decode_or_raw(str_of(category, "path"))
}

/// A folder or label named by id, by path, or by the path as a person writes it, without
/// regard to case.
fn find_named<'a>(list: &'a [Value], name: &str) -> Option<&'a Value> {
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

fn by_role<'a>(list: &'a [Value], role: &str) -> Option<&'a Value> {
    list.iter().find(|c| str_of(c, "role") == role)
}

/// An account's folders and labels, each with its `__cls`, and the counts of both.
struct Categories {
    folders: Vec<Value>,
    labels: Vec<Value>,
    counts: Map<String, Value>,
}

impl Categories {
    fn load(engines: &mut [Engine], t: &Target) -> Result<Categories> {
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
    fn uses_labels(&self) -> bool {
        !self.labels.is_empty()
    }

    /// The stored Label JSON for a label on a thread, else the thread's copy of it.
    fn label(&self, on_thread: &Value) -> Value {
        self.labels
            .iter()
            .find(|l| str_of(l, "id") == str_of(on_thread, "id"))
            .cloned()
            .unwrap_or_else(|| on_thread.clone())
    }
}

/// What an action does on one account, its folder or label resolved.
#[derive(Clone, Debug, PartialEq)]
enum Change {
    Labels {
        add: Vec<Value>,
        remove: Vec<Value>,
    },
    /// Gmail's Move to on a label: the threads' labels are replaced by it.
    LabelMove(Value),
    Folder(Value),
    Unread(bool),
    Starred(bool),
}

impl Change {
    fn resolve(action: ThreadAction, cats: &Categories, email: &str) -> Result<Change> {
        let (folders, labels) = (&cats.folders, &cats.labels);
        Ok(match action {
            // TaskFactory.tasksForArchiving: Gmail drops the Inbox label; folders move to
            // Archive, else All Mail.
            ThreadAction::Archive => match by_role(labels, "inbox") {
                Some(inbox) => Change::Labels {
                    add: vec![],
                    remove: vec![inbox.clone()],
                },
                None => Change::Folder(
                    by_role(folders, "archive")
                        .or_else(|| by_role(folders, "all"))
                        .ok_or_else(|| CliError::failed(format!("{email} has no Archive folder")))?
                        .clone(),
                ),
            },
            ThreadAction::Trash => Change::Folder(
                by_role(folders, "trash")
                    .ok_or_else(|| CliError::failed(format!("{email} has no Trash folder")))?
                    .clone(),
            ),
            ThreadAction::Move(name) => {
                let role = name.to_lowercase();
                let folder = find_named(folders, name).or_else(|| by_role(folders, &role));
                let label = || find_named(labels, name).or_else(|| by_role(labels, &role));
                match (folder, cats.uses_labels()) {
                    (Some(f), _) => Change::Folder(f.clone()),
                    (None, true) => match label() {
                        Some(l) => Change::LabelMove(l.clone()),
                        None => {
                            return Err(CliError::failed(format!(
                                "{email}: no folder or label {name}; `tern labels` lists them, \
                                 `tern label create {name}` makes a label"
                            )))
                        }
                    },
                    (None, false) => {
                        return Err(CliError::failed(format!(
                            "{email}: no folder {name}; `tern folders` lists them, `tern folder \
                             create {name}` makes one"
                        )))
                    }
                }
            }
            ThreadAction::Unread(u) => Change::Unread(u),
            ThreadAction::Starred(s) => Change::Starred(s),
            ThreadAction::Label { name, add } => {
                if !cats.uses_labels() {
                    return Err(CliError::failed(format!(
                        "{email} has folders, not labels (only Gmail has labels); `tern move \
                         <folder>` files threads"
                    )));
                }
                let label = find_named(labels, name)
                    .or_else(|| by_role(labels, &name.to_lowercase()))
                    .ok_or_else(|| {
                        CliError::failed(format!(
                            "{email}: no label {name}; `tern labels` lists them, `tern label \
                             create {name}` makes one"
                        ))
                    })?
                    .clone();
                if add {
                    Change::Labels {
                        add: vec![label],
                        remove: vec![],
                    }
                } else {
                    Change::Labels {
                        add: vec![],
                        remove: vec![label],
                    }
                }
            }
        })
    }

    /// The folder or label the change names, for the output.
    fn category(&self) -> Option<&Value> {
        match self {
            Change::Labels { add, remove } => add.first().or(remove.first()),
            Change::LabelMove(c) | Change::Folder(c) => Some(c),
            Change::Unread(_) | Change::Starred(_) => None,
        }
    }

    /// Whether the change would do anything to `thread`.
    fn applies(&self, thread: &Value) -> bool {
        let has_label = |id: &str| ids_in(thread, "labels").any(|l| l == id);
        match self {
            Change::Labels { add, remove } => {
                add.iter().any(|l| !has_label(str_of(l, "id")))
                    || remove.iter().any(|l| has_label(str_of(l, "id")))
            }
            Change::LabelMove(target) => {
                let id = str_of(target, "id");
                !has_label(id) || ids_in(thread, "labels").any(|l| l != id)
            }
            Change::Folder(target) => thread["folders"].as_array().into_iter().flatten().any(|f| {
                str_of(f, "id") != str_of(target, "id") && !KEPT_ROLES.contains(&str_of(f, "role"))
            }),
            Change::Unread(u) => flag_of(thread, "unread") != *u,
            Change::Starred(s) => flag_of(thread, "starred") != *s,
        }
    }

    fn task(&self, account_id: &str, threads: &[Value], cats: &Categories) -> Value {
        let ids: Vec<String> = threads.iter().map(|t| str_of(t, "id").to_owned()).collect();
        let mut task = match self {
            Change::Labels { add, remove } => {
                builders::change_labels(account_id, &ids, add.clone(), remove.clone())
            }
            Change::LabelMove(target) => {
                let mut remove: Vec<Value> = Vec::new();
                for l in threads
                    .iter()
                    .flat_map(|t| t["labels"].as_array().into_iter().flatten())
                {
                    let id = str_of(l, "id");
                    if id != str_of(target, "id") && !remove.iter().any(|r| str_of(r, "id") == id) {
                        remove.push(cats.label(l));
                    }
                }
                builders::change_labels(account_id, &ids, vec![target.clone()], remove)
            }
            Change::Folder(f) => builders::change_folder(account_id, &ids, f.clone()),
            Change::Unread(u) => builders::change_unread(account_id, &ids, *u),
            Change::Starred(s) => builders::change_starred(account_id, &ids, *s),
        };
        stamp(&mut task);
        task
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

/// One account's part of a thread command.
struct Plan {
    target: Target,
    change: Change,
    changed: Vec<Value>,
    unchanged: Vec<String>,
    tasks: Vec<Value>,
}

impl Plan {
    fn new(target: Target, change: Change, threads: Vec<Value>, cats: &Categories) -> Plan {
        let (changed, rest): (Vec<Value>, Vec<Value>) =
            threads.into_iter().partition(|t| change.applies(t));
        let tasks = changed
            .chunks(THREADS_PER_TASK)
            .map(|chunk| change.task(&target.account_id, chunk, cats))
            .collect();
        Plan {
            target,
            change,
            changed,
            unchanged: rest.iter().map(|t| str_of(t, "id").to_owned()).collect(),
            tasks,
        }
    }

    fn category_name(&self) -> String {
        self.change.category().map(display_name).unwrap_or_default()
    }

    fn task_ids(&self) -> Vec<String> {
        self.tasks
            .iter()
            .map(|t| str_of(t, "id").to_owned())
            .collect()
    }

    fn json(&self, dry_run: bool) -> Value {
        let mut v = json!({
            "accountId": self.target.account_id,
            "email": self.target.email,
            "threadIds": self.changed.iter().map(|t| str_of(t, "id")).collect::<Vec<_>>(),
            "unchangedThreadIds": self.unchanged,
            "taskIds": if dry_run { Vec::new() } else { self.task_ids() },
        });
        if let Some(c) = self.change.category() {
            v["category"] = json!({"id": c["id"], "path": c["path"], "role": c["role"],
                                   "__cls": c["__cls"]});
        }
        v
    }
}

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
                "{}: this engine can't change threads by query (no {} with {}); update Tern, \
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
        let grammar = tern_client::search::grammar_search(query, None);
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
    let groups = match &select.query {
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
    let mut plans = Vec::new();
    for (t, threads) in groups {
        let cats = Categories::load(&mut engines, &t)?;
        let change = Change::resolve(action, &cats, &t.email)?;
        plans.push(Plan::new(t, change, threads, &cats));
    }
    let multi = plans.len() > 1;
    let line = |p: &Plan, dry_run: bool| {
        let s = action.sentence(
            &p.category_name(),
            p.changed.len(),
            p.unchanged.len(),
            dry_run,
        );
        if multi {
            format!("{}: {s}", p.target.email)
        } else {
            s
        }
    };
    let mut doc = json!({
        "action": action.name(),
        "dryRun": select.dry_run,
        "accounts": plans.iter().map(|p| p.json(select.dry_run)).collect::<Vec<_>>(),
    });
    if select.query.is_some() && plans.is_empty() && !ctx.json {
        output::line("No threads match.");
        return Ok(());
    }
    if select.dry_run {
        if ctx.json {
            let threads: Vec<&Value> = plans.iter().flat_map(|p| &p.changed).collect();
            doc["threads"] = json!(threads);
            output::json(&doc);
            return Ok(());
        }
        for p in &plans {
            let mut s = line(p, true);
            if !p.changed.is_empty() {
                s.push(':');
            }
            output::line(&s);
            if !p.changed.is_empty() {
                let rows: Vec<(Value, usize)> = p.changed.iter().map(|t| (t.clone(), 0)).collect();
                read::print_threads(ctx, std::slice::from_ref(&p.target), &rows);
            }
        }
        return Ok(());
    }
    let acting: Vec<&Target> = plans
        .iter()
        .filter(|p| !p.tasks.is_empty())
        .map(|p| &p.target)
        .collect();
    let subscribed = if select.wait {
        subscribe(&mut engines, &acting)?
    } else {
        Vec::new()
    };
    let mut queued: Vec<String> = Vec::new();
    for p in &plans {
        for task in &p.tasks {
            if let Err(e) = queue(&mut engines, &p.target, task) {
                return Err(match queued.as_slice() {
                    [] => e,
                    q => CliError::failed(format!("{e}\nAlready queued: {}", q.join(", "))),
                });
            }
            queued.push(str_of(task, "id").to_owned());
        }
    }
    if !ctx.json {
        for p in &plans {
            let mut s = line(p, false);
            match p.task_ids().as_slice() {
                [] => {}
                [one] => s.push_str(&format!("; task {one}")),
                many => s.push_str(&format!("; tasks {}", many.join(", "))),
            }
            output::line(&s);
        }
    }
    if !select.wait || queued.is_empty() {
        if ctx.json {
            output::json(&doc);
        }
        return Ok(());
    }
    let subs = Subscriptions::start(take_engines(engines, &subscribed), None, None)?;
    let finished = wait_all(subs, &queued, select.timeout)?;
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
                "{email} has folders, not labels (only Gmail has labels); use `tern folder`"
            )))
        }
        (Kind::Folder, true) => {
            return Err(CliError::failed(format!(
                "{email} is a Gmail account, with labels instead of folders; use `tern label`"
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
                "{email}: no {noun} {name}; `tern {}` lists them",
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
    let free = |name: &str| match find_named(list, name) {
        Some(c) => Err(CliError::failed(format!(
            "{email} already has a {noun} {} ({})",
            display_name(c),
            str_of(c, "id")
        ))),
        None => Ok(()),
    };
    let named = |name: &str| -> Result<String> {
        let name = name.trim().trim_matches('/');
        if name.is_empty() {
            return Err(CliError::usage(format!("the {noun} needs a name")));
        }
        Ok(name.to_owned())
    };
    let (mut task, done) = match op {
        CategoryOp::Create(name) => {
            let name = named(name)?;
            free(&name)?;
            (
                builders::create_category(&t.account_id, &name),
                format!("Created {noun} {name}"),
            )
        }
        CategoryOp::Rename { from, to } => {
            let c = existing(from)?;
            not_standard(c, "renamed")?;
            let to = named(to)?;
            if find_named(list, &to).is_some_and(|other| other["id"] != c["id"]) {
                free(&to)?;
            }
            (
                builders::rename_category(&t.account_id, str_of(c, "path"), &to),
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
            (
                builders::destroy_category(&t.account_id, str_of(c, "path")),
                format!("Deleted {noun} {}", display_name(c)),
            )
        }
    };
    stamp(&mut task);
    let id = str_of(&task, "id").to_owned();
    let subscribed = if wait.no_wait {
        Vec::new()
    } else {
        subscribe(&mut engines, &[&t])?
    };
    let after_local = queue(&mut engines, &t, &task)?;
    if wait.no_wait {
        if ctx.json {
            output::json(&json!({"accountId": t.account_id, "taskId": id,
                                 "status": after_local["status"]}));
        } else {
            output::line(&format!(
                "Queued: {}; task {id} (`tern task wait {id}`)",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cats(gmail: bool) -> Categories {
        let folder = |id: &str, path: &str, role: &str| json!({"id": id, "aid": "a", "path": path, "role": role, "__cls": "Folder"});
        let label = |id: &str, path: &str, role: &str| json!({"id": id, "aid": "a", "path": path, "role": role, "__cls": "Label"});
        if gmail {
            Categories {
                folders: vec![
                    folder("f-all", "[Gmail]/All Mail", "all"),
                    folder("f-trash", "[Gmail]/Trash", "trash"),
                ],
                labels: vec![
                    label("l-inbox", "INBOX", "inbox"),
                    label("l-sent", "[Gmail]/Sent Mail", "sent"),
                    label("l-work", "Work", ""),
                    label("l-recus", "Re&AOc-us", ""),
                ],
                counts: Map::new(),
            }
        } else {
            Categories {
                folders: vec![
                    folder("f-in", "INBOX", "inbox"),
                    folder("f-ar", "Archive", "archive"),
                    folder("f-tr", "Trash", "trash"),
                    folder("f-sent", "Sent", "sent"),
                    folder("f-proj", "INBOX.Projects", ""),
                ],
                labels: vec![],
                counts: Map::new(),
            }
        }
    }

    fn thread(id: &str, folders: &[(&str, &str)], labels: &[&str], unread: i64) -> Value {
        json!({
            "id": id, "aid": "a", "unread": unread, "starred": 0,
            "folders": folders.iter().map(|(id, role)| json!({"id": id, "role": role})).collect::<Vec<_>>(),
            "labels": labels.iter().map(|id| json!({"id": id, "path": id, "role": ""})).collect::<Vec<_>>(),
        })
    }

    fn resolve(action: ThreadAction, gmail: bool) -> Result<Change> {
        Change::resolve(action, &cats(gmail), "me@x.test")
    }

    #[test]
    fn archive_follows_the_task_factory() {
        let Change::Labels { add, remove } = resolve(ThreadAction::Archive, true).unwrap() else {
            panic!("gmail archive drops the inbox label")
        };
        assert!(add.is_empty() && remove[0]["id"] == "l-inbox");
        let Change::Folder(f) = resolve(ThreadAction::Archive, false).unwrap() else {
            panic!("folder archive")
        };
        assert_eq!(f["id"], "f-ar");
        let Change::Folder(f) = resolve(ThreadAction::Trash, true).unwrap() else {
            panic!("trash")
        };
        assert_eq!(f["id"], "f-trash");
    }

    #[test]
    fn names_resolve_by_id_path_display_name_and_role() {
        for (name, id) in [
            ("INBOX.Projects", "f-proj"),
            ("inbox.projects", "f-proj"),
            ("f-proj", "f-proj"),
            ("archive", "f-ar"),
            ("Inbox", "f-in"),
        ] {
            let Change::Folder(f) = resolve(ThreadAction::Move(name), false).unwrap() else {
                panic!("{name}")
            };
            assert_eq!(f["id"], id, "{name}");
        }
        let add = |name| resolve(ThreadAction::Label { name, add: true }, true);
        let Change::Labels { add: l, .. } = add("reçus").unwrap() else {
            panic!("decoded name")
        };
        assert_eq!(l[0]["id"], "l-recus");
        let e = add("Receipts").unwrap_err().to_string();
        assert!(e.contains("no label Receipts") && e.contains("tern label create Receipts"));
        let e = resolve(
            ThreadAction::Label {
                name: "x",
                add: true,
            },
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("folders, not labels"), "{e}");
        let e = resolve(ThreadAction::Move("Nope"), false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("tern folder create Nope"), "{e}");
        assert!(matches!(
            resolve(ThreadAction::Move("work"), true).unwrap(),
            Change::LabelMove(_)
        ));
    }

    #[test]
    fn threads_already_as_asked_are_left_out() {
        let c = cats(false);
        let archive = resolve(ThreadAction::Archive, false).unwrap();
        let in_inbox = thread("t1", &[("f-in", "inbox"), ("f-sent", "sent")], &[], 1);
        let archived = thread("t2", &[("f-ar", "archive"), ("f-sent", "sent")], &[], 0);
        let only_sent = thread("t3", &[("f-sent", "sent")], &[], 0);
        let p = Plan::new(
            Target {
                engine: 0,
                account_id: "a".into(),
                email: "me@x.test".into(),
            },
            archive,
            vec![in_inbox.clone(), archived.clone(), only_sent],
            &c,
        );
        assert_eq!(p.changed, std::slice::from_ref(&in_inbox));
        assert_eq!(p.unchanged, ["t2", "t3"]);
        assert_eq!(p.tasks.len(), 1);
        let task = &p.tasks[0];
        assert_eq!(task["__cls"], "ChangeFolderTask");
        assert_eq!(task["threadIds"], json!(["t1"]));
        assert_eq!(task["source"], SOURCE);
        assert!(task["id"].as_str().unwrap().starts_with("tern-cli-"));

        let read = Change::Unread(false);
        assert!(read.applies(&in_inbox) && !read.applies(&archived));
        let work = resolve(
            ThreadAction::Label {
                name: "Work",
                add: true,
            },
            true,
        )
        .unwrap();
        assert!(work.applies(&thread("t", &[], &["l-inbox"], 0)));
        assert!(!work.applies(&thread("t", &[], &["l-inbox", "l-work"], 0)));
    }

    #[test]
    fn bulk_changes_are_batched() {
        let c = cats(true);
        let threads: Vec<Value> = (0..1201)
            .map(|i| thread(&format!("t{i}"), &[], &["l-inbox"], 1))
            .collect();
        let p = Plan::new(
            Target {
                engine: 0,
                account_id: "a".into(),
                email: "me@x.test".into(),
            },
            Change::Unread(false),
            threads,
            &c,
        );
        let sizes: Vec<usize> = p
            .tasks
            .iter()
            .map(|t| t["threadIds"].as_array().unwrap().len())
            .collect();
        assert_eq!(sizes, [500, 500, 201]);
        let ids = p.task_ids();
        assert!(ids[0] != ids[1] && ids[1] != ids[2]);
    }

    #[test]
    fn a_label_move_replaces_the_labels() {
        let c = cats(true);
        let target = c.labels[2].clone();
        let change = Change::LabelMove(target.clone());
        let threads = [
            thread("t1", &[], &["l-inbox", "l-work"], 0),
            thread("t2", &[], &["l-inbox", "l-sent"], 0),
        ];
        let task = change.task("a", &threads, &c);
        assert_eq!(task["labelsToAdd"], json!([target]));
        let removed: Vec<&str> = task["labelsToRemove"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["id"].as_str().unwrap())
            .collect();
        assert_eq!(removed, ["l-inbox", "l-sent"]);
        assert_eq!(task["labelsToRemove"][0]["__cls"], "Label");
        assert!(!change.applies(&thread("t3", &[], &["l-work"], 0)));
    }

    #[test]
    fn sentences() {
        let s = |a: ThreadAction, n, u, dry| a.sentence("Work", n, u, dry);
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
    }
}
