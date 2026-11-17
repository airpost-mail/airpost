// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `airpost task list|queue|cancel|wait`. A task is done when a `Task` delta shows it
//! `complete` or `cancelled`, as the desktop's `TaskQueue` decides; its `error` says whether
//! it failed.

use std::io::Read;

use airpost_client::tasks::{finished, is_final, TASK_CLASS};
use airpost_ipc::{
    code, feature, method, ClientError, DeltaParams, TasksGetResult, TasksListResult,
};
use serde_json::{json, Value};

use crate::app::{call, is_not_found, Ctx};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::output::{self, Table};
use crate::watch::{deadline, lost, NextEvent, Subscriptions};

/// The prefix of the task ids the CLI picks.
pub(crate) const ID_PREFIX: &str = "airpost-cli";

/// Reads the task argument: JSON, or `-` for stdin.
fn read_task(arg: &str) -> Result<Value> {
    let text = if arg == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| CliError::failed(format!("could not read the task from stdin: {e}")))?;
        s
    } else {
        arg.to_owned()
    };
    let task: Value = serde_json::from_str(&text)
        .map_err(|e| CliError::usage(format!("the task isn't JSON ({e})")))?;
    if !task.is_object() {
        return Err(CliError::usage("the task must be a JSON object"));
    }
    if task["__cls"].as_str().is_none_or(str::is_empty) {
        return Err(CliError::usage(
            "the task needs its class in `__cls`, e.g. {\"__cls\": \"ChangeStarredTask\", ...}",
        ));
    }
    Ok(task)
}

/// Fills in what the desktop's `Task` model always sends: `id`, `aid` and `status`.
pub fn complete_task(task: Value, account_id: &str) -> Value {
    airpost_client::tasks::complete_task(task, account_id, ID_PREFIX)
}

pub(crate) fn describe_error(e: &Value) -> String {
    let key = e["key"].as_str().unwrap_or("");
    let info = e["debuginfo"].as_str().unwrap_or("");
    match (key.is_empty(), info.is_empty()) {
        (false, false) => format!("{key}: {info}"),
        (false, true) => key.to_owned(),
        _ => e.to_string(),
    }
}

/// Prints the outcome; an error is the command's failure.
pub(crate) fn report(ctx: &Ctx, task: &Value) -> Result<()> {
    let id = task["id"].as_str().unwrap_or("");
    let class = task["__cls"].as_str().unwrap_or("Task");
    let status = task["status"].as_str().unwrap_or("removed");
    if ctx.json {
        output::json(task);
    } else if task["error"].is_null() {
        output::line(&format!("{class} {id} {status}"));
    }
    match &task["error"] {
        Value::Null => Ok(()),
        e => Err(CliError::failed(format!(
            "{class} {id} failed: {}",
            describe_error(e)
        ))),
    }
}

pub(crate) fn wait_on(
    ctx: &Ctx,
    mut subs: Subscriptions,
    id: &str,
    timeout: Option<u64>,
) -> Result<()> {
    let until = deadline(timeout);
    loop {
        match subs.next(until) {
            NextEvent::Note(n) if n.method == method::DELTA => {
                let Ok(d) = serde_json::from_value::<DeltaParams>(n.params) else {
                    continue;
                };
                if let Some(task) = finished(&d, id) {
                    return report(ctx, &task);
                }
            }
            NextEvent::Note(_) => {}
            NextEvent::TimedOut => {
                return Err(CliError::failed(format!(
                "task {id} didn't finish within {} s; it may still run (`airpost task wait {id}`)",
                timeout.unwrap_or_default()
            )))
            }
            NextEvent::Lost { pid, reason } => {
                return Err(lost(pid, &format!("{reason}, before task {id} finished")))
            }
            NextEvent::AllGone => {
                return Err(CliError::failed(format!(
                    "no engine to wait on task {id} with; `airpost status` lists them"
                )))
            }
        }
    }
}

/// Waits until every task of `ids` has finished and returns them as they ended, in the order
/// of `ids`. Each engine in `subs` must have been subscribed to `Task` before its tasks were
/// queued.
pub(crate) fn wait_all(
    mut subs: Subscriptions,
    ids: &[String],
    timeout: Option<u64>,
) -> Result<Vec<Value>> {
    let until = deadline(timeout);
    let mut done: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    while done.len() < ids.len() {
        let pending = || {
            ids.iter()
                .filter(|id| !done.contains_key(*id))
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        };
        match subs.next(until) {
            NextEvent::Note(n) if n.method == method::DELTA => {
                let Ok(d) = serde_json::from_value::<DeltaParams>(n.params) else {
                    continue;
                };
                for id in ids {
                    if !done.contains_key(id) {
                        if let Some(task) = finished(&d, id) {
                            done.insert(id.clone(), task);
                        }
                    }
                }
            }
            NextEvent::Note(_) => {}
            NextEvent::TimedOut => {
                return Err(CliError::failed(format!(
                    "not finished within {} s: {}; they may still run (`airpost task wait <id>`)",
                    timeout.unwrap_or_default(),
                    pending()
                )))
            }
            NextEvent::Lost { pid, reason } => {
                return Err(lost(
                    pid,
                    &format!("{reason}, before {} finished", pending()),
                ))
            }
            NextEvent::AllGone => {
                return Err(CliError::failed(format!(
                    "no engine to wait on {} with; `airpost status` lists them",
                    pending()
                )))
            }
        }
    }
    Ok(ids.iter().filter_map(|id| done.remove(id)).collect())
}

/// The failures among finished tasks, one line each.
pub(crate) fn failures(tasks: &[Value]) -> Vec<String> {
    tasks
        .iter()
        .filter(|t| !t["error"].is_null())
        .map(|t| {
            format!(
                "{} {} failed: {}",
                t["__cls"].as_str().unwrap_or("Task"),
                t["id"].as_str().unwrap_or(""),
                describe_error(&t["error"])
            )
        })
        .collect()
}

/// What `airpost task list` filters on.
#[derive(Debug, Default)]
pub struct ListFilter {
    pub status: Vec<String>,
    pub class: Option<String>,
    pub header_message_id: Option<String>,
    pub limit: Option<usize>,
}

pub fn list(ctx: &Ctx, engines: &mut [Engine], filter: &ListFilter) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let mut rows: Vec<(String, Value)> = Vec::new();
    for t in &targets {
        if !engines[t.engine].has_feature(feature::TASKS_LIST) {
            return Err(CliError::failed(format!(
                "{}: this engine can't list tasks (no tasks.list); update Airpost",
                t.email
            )));
        }
        let params = airpost_ipc::TasksListParams {
            account_id: Some(t.account_id.clone()),
            status: (!filter.status.is_empty()).then(|| filter.status.clone()),
            class: filter.class.clone(),
            header_message_id: filter.header_message_id.clone(),
            limit: filter.limit,
        };
        let params = serde_json::to_value(&params).unwrap_or_else(|_| json!({}));
        let r: TasksListResult = call(engines, t, method::TASKS_LIST, params)?;
        rows.extend(r.tasks.into_iter().map(|task| (t.email.clone(), task)));
    }
    if ctx.json {
        output::json(&Value::Array(rows.into_iter().map(|(_, t)| t).collect()));
        return Ok(());
    }
    let multi = targets.len() > 1;
    let mut headers = vec!["CLASS", "STATUS", "ERROR", "ID"];
    if multi {
        headers.insert(0, "ACCOUNT");
    }
    let mut table = Table::new(headers, usize::from(multi) + 2);
    for (email, task) in rows {
        let error = match &task["error"] {
            Value::Null => String::new(),
            e => describe_error(e),
        };
        let failed = !error.is_empty();
        let mut cells = vec![
            task["__cls"].as_str().unwrap_or("").to_owned(),
            task["status"].as_str().unwrap_or("").to_owned(),
            error,
            task["id"].as_str().unwrap_or("").to_owned(),
        ];
        if multi {
            cells.insert(0, email);
        }
        table.row(cells, false);
        if failed {
            table.alert(usize::from(multi) + 2);
        }
    }
    if table.is_empty() {
        output::line("No tasks.");
    } else {
        table.print(&ctx.style);
    }
    Ok(())
}

pub fn queue(
    ctx: &Ctx,
    mut engines: Vec<Engine>,
    arg: &str,
    wait: bool,
    timeout: Option<u64>,
) -> Result<()> {
    let task = read_task(arg)?;
    let t = discovery::one_target(&engines, ctx.account())?;
    let task = complete_task(task, &t.account_id);
    let id = task["id"].as_str().unwrap_or_default().to_owned();
    let client = &mut engines[t.engine].client;
    if wait {
        // On the same connection, before queueing: the deltas that follow are buffered while
        // task.queue waits for its reply, so none is missed.
        let params = airpost_ipc::SubscribeParams {
            accounts: Some(vec![t.account_id.clone()]),
            classes: Some(vec![TASK_CLASS.to_owned()]),
            since: None,
        };
        client
            .subscribe(&params)
            .map_err(|e| CliError::call(&t.email, method::SUBSCRIBE, e))?;
    }
    let params = json!({ "accountId": t.account_id, "task": task });
    let after_local = match client.call(method::TASK_QUEUE, params) {
        Ok(v) => v,
        Err(ClientError::Rpc(e)) if e.code == code::TASK_REJECTED => {
            let mut rejected = task;
            rejected["status"] = json!("complete");
            rejected["error"] = e.data.unwrap_or_else(|| json!({ "key": e.message }));
            return report(ctx, &rejected);
        }
        Err(e) => return Err(CliError::call(&t.email, method::TASK_QUEUE, e)),
    };
    if !wait {
        if ctx.json {
            output::json(&after_local);
        } else {
            let class = after_local["__cls"].as_str().unwrap_or("Task");
            let status = after_local["status"].as_str().unwrap_or("");
            output::line(&format!("Queued {class} {id} ({status})"));
        }
        return Ok(());
    }
    let engine = engines.swap_remove(t.engine);
    let subs = Subscriptions::start(vec![(engine, true)], None, None)?;
    wait_on(ctx, subs, &id, timeout)
}

pub fn cancel(ctx: &Ctx, engines: &mut [Engine], id: &str) -> Result<()> {
    let t = discovery::one_target(engines, ctx.account())?;
    call::<Value>(
        engines,
        &t,
        method::TASK_CANCEL,
        json!({ "accountId": t.account_id, "taskId": id }),
    )?;
    if ctx.json {
        output::json(&json!({ "accountId": t.account_id, "taskId": id }));
    } else {
        output::line(&format!(
            "Asked {} to cancel {id}; a task that has started may still finish",
            t.email
        ));
    }
    Ok(())
}

/// The task as stored, None when the account has no such task.
fn get_task(engine: &mut Engine, t: &Target, id: &str) -> Result<Option<Value>> {
    let params = json!({ "accountId": t.account_id, "id": id });
    match engine
        .client
        .call_as::<TasksGetResult>(method::TASKS_GET, params)
    {
        Ok(r) => Ok(Some(r.task)),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(CliError::call(&t.email, method::TASKS_GET, e)),
    }
}

pub fn wait(ctx: &Ctx, mut engines: Vec<Engine>, id: &str, timeout: Option<u64>) -> Result<()> {
    let targets = discovery::targets(&engines, ctx.account())?;
    let mut holder = None;
    for t in targets {
        if let Some(task) = get_task(&mut engines[t.engine], &t, id)? {
            holder = Some((t, task));
            break;
        }
    }
    let Some((t, task)) = holder else {
        return Err(CliError::failed(format!(
            "no task {id}{}; finished tasks are kept only until a hundred newer ones finish",
            match ctx.account() {
                Some(_) => String::new(),
                None => " in any account".to_owned(),
            }
        )));
    };
    if is_final(&task) {
        return report(ctx, &task);
    }
    let mut engine = engines.swap_remove(t.engine);
    let params = airpost_ipc::SubscribeParams {
        accounts: Some(vec![t.account_id.clone()]),
        classes: Some(vec![TASK_CLASS.to_owned()]),
        since: None,
    };
    engine
        .client
        .subscribe(&params)
        .map_err(|e| CliError::call(&t.email, method::SUBSCRIBE, e))?;
    // It may have finished before the subscription took; a later finish arrives as a delta.
    match get_task(&mut engine, &t, id)? {
        Some(task) if is_final(&task) => return report(ctx, &task),
        Some(_) => {}
        None => {
            let mut removed = task;
            removed["status"] = Value::Null;
            return report(ctx, &removed);
        }
    }
    let subs = Subscriptions::start(vec![(engine, true)], None, None)?;
    wait_on(ctx, subs, id, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(kind: &str, class: &str, models: Vec<Value>) -> DeltaParams {
        DeltaParams {
            account_id: "a".into(),
            seq: None,
            model_class: class.into(),
            kind: kind.into(),
            model_jsons: models,
        }
    }

    #[test]
    fn completes_the_task_json() {
        let t = complete_task(
            json!({"__cls": "ChangeStarredTask", "starred": true}),
            "acc",
        );
        assert!(t["id"].as_str().unwrap().starts_with("airpost-cli-"));
        assert_eq!(
            (t["aid"].as_str(), t["status"].as_str()),
            (Some("acc"), Some("local"))
        );
        let t = complete_task(json!({"__cls": "X", "id": "mine", "aid": "other"}), "acc");
        assert_eq!(
            (t["id"].as_str(), t["aid"].as_str()),
            (Some("mine"), Some("other"))
        );
        assert_ne!(
            complete_task(json!({}), "a")["id"],
            complete_task(json!({}), "a")["id"]
        );
    }

    #[test]
    fn task_json_is_checked() {
        assert_eq!(read_task("nope").unwrap_err().exit_code(), 2);
        assert_eq!(read_task("[1]").unwrap_err().exit_code(), 2);
        assert_eq!(read_task(r#"{"id": "x"}"#).unwrap_err().exit_code(), 2);
        assert!(read_task(r#"{"__cls": "ChangeStarredTask"}"#).is_ok());
    }

    #[test]
    fn finished_only_for_the_task_in_a_final_state() {
        let local = json!({"id": "t1", "status": "remote"});
        let done = json!({"id": "t1", "status": "complete", "error": null});
        assert!(finished(&delta("persist", "Task", vec![local.clone()]), "t1").is_none());
        assert!(finished(&delta("persist", "Thread", vec![done.clone()]), "t1").is_none());
        assert!(finished(&delta("persist", "Task", vec![done.clone()]), "t2").is_none());
        assert_eq!(
            finished(
                &delta("persist", "Task", vec![json!({"id": "x"}), done.clone()]),
                "t1"
            ),
            Some(done)
        );
        assert!(finished(&delta("unpersist", "Task", vec![json!({"id": "t1"})]), "t1").is_some());
    }

    #[test]
    fn errors_read_as_key_and_debuginfo() {
        assert_eq!(
            describe_error(&json!({"key": "ErrorAuthentication", "debuginfo": "LOGIN failed"})),
            "ErrorAuthentication: LOGIN failed"
        );
        assert_eq!(describe_error(&json!({"key": "k"})), "k");
        assert_eq!(describe_error(&json!("boom")), "\"boom\"");
    }
}
