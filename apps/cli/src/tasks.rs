// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern task queue|cancel|wait`. A task is done when a `Task` delta shows it `complete` or
//! `cancelled`, as the desktop's `TaskQueue` decides; its `error` says whether it failed.

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ms_ipc::{code, method, ClientError, DeltaParams, TasksGetResult};
use serde_json::{json, Value};

use crate::app::{call, is_not_found, Ctx};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::output;
use crate::watch::{deadline, lost, NextEvent, Subscriptions};

const TASK_CLASS: &str = "Task";

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

/// A task id no other client picks: the CLI's pid, the time in nanoseconds and a counter
/// (the clock may not tick between two calls).
fn new_task_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("tern-cli-{}-{nanos:x}-{seq}", std::process::id())
}

/// Fills in what the desktop's `Task` model always sends: `id`, `aid` and `status`.
pub fn complete_task(mut task: Value, account_id: &str) -> Value {
    if let Some(obj) = task.as_object_mut() {
        obj.entry("id").or_insert_with(|| json!(new_task_id()));
        obj.entry("aid").or_insert_with(|| json!(account_id));
        obj.entry("status").or_insert_with(|| json!("local"));
    }
    task
}

/// A finished task found in a `Task` delta.
pub fn finished(d: &DeltaParams, id: &str) -> Option<Value> {
    if d.model_class != TASK_CLASS {
        return None;
    }
    d.model_jsons
        .iter()
        .find(|t| t["id"].as_str() == Some(id))
        .filter(|t| {
            d.kind == "unpersist" || matches!(t["status"].as_str(), Some("complete" | "cancelled"))
        })
        .cloned()
}

fn describe_error(e: &Value) -> String {
    let key = e["key"].as_str().unwrap_or("");
    let info = e["debuginfo"].as_str().unwrap_or("");
    match (key.is_empty(), info.is_empty()) {
        (false, false) => format!("{key}: {info}"),
        (false, true) => key.to_owned(),
        _ => e.to_string(),
    }
}

/// Prints the outcome; an error is the command's failure.
fn report(ctx: &Ctx, task: &Value) -> Result<()> {
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

fn wait_on(ctx: &Ctx, mut subs: Subscriptions, id: &str, timeout: Option<u64>) -> Result<()> {
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
                    "task {id} didn't finish within {} s; it may still run (`tern task wait {id}`)",
                    timeout.unwrap_or_default()
                )))
            }
            NextEvent::Lost { pid, reason } => {
                return Err(lost(pid, &format!("{reason}, before task {id} finished")))
            }
            NextEvent::AllGone => {
                return Err(CliError::failed(format!(
                    "no engine to wait on task {id} with; `tern status` lists them"
                )))
            }
        }
    }
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
        let params = ms_ipc::SubscribeParams {
            accounts: Some(vec![t.account_id.clone()]),
            classes: Some(vec![TASK_CLASS.to_owned()]),
        };
        client
            .subscribe(&params)
            .map_err(|e| CliError::call(&t.email, method::SUBSCRIBE, e))?;
    }
    let params = json!({ "accountId": t.account_id, "task": task });
    let after_local = match client.call(method::TASK_QUEUE, params) {
        Ok(v) => v,
        Err(ClientError::Rpc(e)) if e.code == code::TASK_REJECTED => {
            let mut rejected = task.clone();
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

fn is_final(task: &Value) -> bool {
    matches!(task["status"].as_str(), Some("complete" | "cancelled"))
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
    let params = ms_ipc::SubscribeParams {
        accounts: Some(vec![t.account_id.clone()]),
        classes: Some(vec![TASK_CLASS.to_owned()]),
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
        assert!(t["id"].as_str().unwrap().starts_with("tern-cli-"));
        assert_eq!(
            (t["aid"].as_str(), t["status"].as_str()),
            (Some("acc"), Some("local"))
        );
        let t = complete_task(json!({"__cls": "X", "id": "mine", "aid": "other"}), "acc");
        assert_eq!(
            (t["id"].as_str(), t["aid"].as_str()),
            (Some("mine"), Some("other"))
        );
        assert_ne!(new_task_id(), new_task_id());
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
