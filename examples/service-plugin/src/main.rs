// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A sample service plugin: a separate program on
//! the engine socket that `tern service install tern-sample-service` runs as a user unit, so
//! it keeps working while the desktop is closed.
//!
//! It is a "remind me" plugin: a thread whose `tern-sample-service` metadata has an
//! `expiration` is starred and marked unread when that time comes, and the expiration is
//! cleared. The engine announces due rows of plugins it doesn't handle itself as
//! `metadata-expiration` deltas, once per scan until the row's expiration is cleared, so the
//! service acts on each (model, expiration) once.
//!
//! Without an engine it waits and tries again, and it reconnects when the engine goes away:
//! launchd has no start order, and the engine restarts on updates. Each event is one JSON
//! line on stdout (the unit's log); errors go to stderr.
//!
//!     tern-sample-service [--config-dir DIR] [--plugin-id ID]
//!
//! `TERN_CONFIG_DIR` and `TERN_SERVICE_PACKAGE`, which the unit sets, are the defaults.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tern_client::{tasks, Engine, Engines};
use tern_ipc::{method, DeltaParams, SubscribeParams};

const CLIENT_NAME: &str = concat!("tern-sample-service/", env!("CARGO_PKG_VERSION"));
const DEFAULT_PLUGIN_ID: &str = "tern-sample-service";
const MAX_RETRY: Duration = Duration::from_secs(30);

struct Options {
    config_dir: PathBuf,
    plugin_id: String,
}

fn options() -> Result<Options, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut config_dir = env("TERN_CONFIG_DIR").map(PathBuf::from);
    let mut plugin_id = env("TERN_SERVICE_PACKAGE");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config-dir" => config_dir = args.next().map(PathBuf::from),
            "--plugin-id" => plugin_id = args.next(),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let config_dir = config_dir
        .or_else(tern_client::discovery::default_config_dir)
        .ok_or("no profile directory: give --config-dir or set TERN_CONFIG_DIR")?;
    Ok(Options {
        config_dir,
        plugin_id: plugin_id.unwrap_or_else(|| DEFAULT_PLUGIN_ID.to_owned()),
    })
}

fn event(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}").and_then(|()| out.flush());
}

fn error(msg: &str) {
    let _ = writeln!(std::io::stderr(), "tern-sample-service: {msg}");
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn main() {
    let opts = match options() {
        Ok(o) => o,
        Err(e) => {
            error(&e);
            std::process::exit(2);
        }
    };
    let mut handled = HashSet::new();
    let mut retry = Duration::from_secs(1);
    let mut waiting_reported = false;
    loop {
        let mut engines = Engines::connect(&opts.config_dir, CLIENT_NAME, "tern-sample-service");
        // A serve engine holds every account, so one engine is the whole profile.
        let Some(engine) = engines.engines.drain(..).next() else {
            if !waiting_reported {
                event(json!({"event": "waiting", "configDir": opts.config_dir}));
                for problem in &engines.problems {
                    error(problem);
                }
                waiting_reported = true;
            }
            std::thread::sleep(retry);
            retry = (retry * 2).min(MAX_RETRY);
            continue;
        };
        retry = Duration::from_secs(1);
        waiting_reported = false;
        let pid = engine.entry.pid;
        match serve(engine, &opts.plugin_id, &mut handled) {
            Ok(()) => event(json!({"event": "disconnected", "pid": pid})),
            Err(e) => error(&format!("engine pid {pid}: {e}")),
        }
        std::thread::sleep(retry);
    }
}

/// Subscribes and acts on due rows until the connection ends.
fn serve(
    mut engine: Engine,
    plugin_id: &str,
    handled: &mut HashSet<(String, i64)>,
) -> Result<(), String> {
    let params = SubscribeParams {
        accounts: None,
        classes: Some(vec!["Thread".into(), "Message".into()]),
        since: None,
    };
    engine
        .client
        .subscribe(&params)
        .map_err(|e| e.to_string())?;
    engine
        .client
        .set_read_timeout(None)
        .map_err(|e| e.to_string())?;
    event(json!({"event": "connected", "pid": engine.entry.pid, "pluginId": plugin_id}));
    loop {
        let n = match engine.client.next_notification() {
            Ok(n) => n,
            Err(tern_ipc::ClientError::Closed) => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        if n.method == method::DISCONNECT {
            return Ok(());
        }
        if n.method != method::DELTA {
            continue;
        }
        let Ok(delta) = serde_json::from_value::<DeltaParams>(n.params) else {
            continue;
        };
        if delta.kind != "metadata-expiration" {
            continue;
        }
        for model in &delta.model_jsons {
            if let Some(due) = due(&delta.model_class, model, plugin_id, now()) {
                if handled.insert((due.model_id.clone(), due.expiration)) {
                    act(&mut engine, &delta.account_id, plugin_id, &due)?;
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Due {
    model_class: String,
    model_id: String,
    thread_id: String,
    expiration: i64,
    value: Value,
}

/// The model's row for `plugin_id` when its expiration has come; other plugins' rows ride in
/// the same delta and are left alone.
fn due(class: &str, model: &Value, plugin_id: &str, now: i64) -> Option<Due> {
    let value = model["metadata"]
        .as_array()?
        .iter()
        .find(|m| m["pluginId"] == plugin_id)
        .map(|m| &m["value"])?;
    let expiration = value["expiration"].as_i64()?;
    if expiration > now {
        return None;
    }
    let model_id = model["id"].as_str()?.to_owned();
    let thread_id = match class {
        "Thread" => model_id.clone(),
        "Message" => model["threadId"].as_str()?.to_owned(),
        _ => return None,
    };
    Some(Due {
        model_class: class.to_owned(),
        model_id,
        thread_id,
        expiration,
        value: value.clone(),
    })
}

fn act(engine: &mut Engine, account_id: &str, plugin_id: &str, due: &Due) -> Result<(), String> {
    let threads = [due.thread_id.clone()];
    let mut cleared = due.value.clone();
    if let Some(obj) = cleared.as_object_mut() {
        obj.remove("expiration");
        obj.insert("firedAt".into(), json!(due.expiration));
    }
    let clear = if due.model_class == "Thread" {
        tasks::syncback_thread_metadata(account_id, &due.model_id, plugin_id, cleared)
    } else {
        let message = json!({"id": due.model_id, "aid": account_id});
        tasks::syncback_message_metadata(&message, plugin_id, cleared, None)
    };
    for task in [
        tasks::change_starred(account_id, &threads, true),
        tasks::change_unread(account_id, &threads, true),
        clear,
    ] {
        engine
            .client
            .call(
                method::TASK_QUEUE,
                json!({"accountId": account_id, "task": task}),
            )
            .map_err(|e| format!("task.queue failed: {e}"))?;
    }
    event(json!({
        "event": "acted",
        "accountId": account_id,
        "modelClass": due.model_class,
        "modelId": due.model_id,
        "threadId": due.thread_id,
        "expiration": due.expiration,
    }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(metadata: Value) -> Value {
        json!({"id": "t1", "aid": "a1", "metadata": metadata})
    }

    #[test]
    fn only_its_own_due_rows() {
        let model = thread(json!([
            {"pluginId": "thread-snooze", "value": {"expiration": 10}, "v": 1},
            {"pluginId": "tern-sample-service", "value": {"expiration": 50, "note": "x"}, "v": 2},
        ]));
        assert_eq!(due("Thread", &model, "tern-sample-service", 49), None);
        let d = due("Thread", &model, "tern-sample-service", 50).unwrap();
        assert_eq!((d.thread_id.as_str(), d.expiration), ("t1", 50));
        assert_eq!(d.value["note"], "x");
        assert_eq!(due("Thread", &model, "other", 100), None);
        let cleared =
            thread(json!([{"pluginId": "tern-sample-service", "value": {"firedAt": 50}}]));
        assert_eq!(due("Thread", &cleared, "tern-sample-service", 100), None);
    }

    #[test]
    fn a_message_row_acts_on_its_thread() {
        let msg = json!({"id": "m1", "threadId": "t9", "metadata": [
            {"pluginId": "tern-sample-service", "value": {"expiration": 1}}
        ]});
        let d = due("Message", &msg, "tern-sample-service", 5).unwrap();
        assert_eq!((d.model_id.as_str(), d.thread_id.as_str()), ("m1", "t9"));
        assert_eq!(due("Folder", &msg, "tern-sample-service", 5), None);
    }
}
