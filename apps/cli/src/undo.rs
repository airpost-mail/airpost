// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern undo` and `tern redo`: the engine's undo log (`undo.apply|redo|list`). Without a
//! token they take tern's own latest change, so `tern undo` never takes back what Tern or an
//! agent did; `--any` takes the latest change of any client, `--list` shows the log.

use chrono::Local;
use serde_json::{json, Value};
use tern_ipc::{
    code, feature, method, undo_state, ClientError, SubscribeParams, UndoApplyParams,
    UndoApplyResult, UndoEntry, UndoListParams, UndoListResult,
};

use crate::app::Ctx;
use crate::discovery::{self, Engine};
use crate::error::{CliError, Result};
use crate::output::{self, Table};
use crate::tasks::{failures, wait_all};
use crate::watch::Subscriptions;

/// What `tern undo|redo` was given.
#[derive(Clone, Debug, Default)]
pub struct UndoArgs<'a> {
    pub token: Option<&'a str>,
    pub any: bool,
    pub redo: bool,
    pub wait: bool,
    pub timeout: Option<u64>,
}

/// The engine whose log to use: the one holding `--account`, else the first with the log.
fn pick(ctx: &Ctx, engines: &[Engine]) -> Result<usize> {
    let mut candidates: Vec<usize> = Vec::new();
    for t in discovery::targets(engines, ctx.account())? {
        if !candidates.contains(&t.engine) {
            candidates.push(t.engine);
        }
    }
    candidates
        .into_iter()
        .find(|&i| engines[i].has_feature(feature::UNDO))
        .ok_or_else(|| {
            CliError::failed(format!(
                "this engine keeps no undo log (no {}); update Tern",
                feature::UNDO
            ))
        })
}

fn refused(what: &str, e: ClientError) -> CliError {
    match e {
        ClientError::Rpc(err)
            if err.code == code::NOT_FOUND || err.code == code::INVALID_PARAMS =>
        {
            CliError::failed(
                err.message
                    .trim_start_matches("invalid params: ")
                    .to_owned(),
            )
        }
        e => CliError::failed(format!("{what} failed: {e}")),
    }
}

pub fn undo(ctx: &Ctx, mut engines: Vec<Engine>, a: &UndoArgs) -> Result<()> {
    let i = pick(ctx, &engines)?;
    let what = if a.redo {
        method::UNDO_REDO
    } else {
        method::UNDO_APPLY
    };
    if a.wait {
        let params = SubscribeParams {
            accounts: None,
            classes: Some(vec![tern_client::tasks::TASK_CLASS.to_owned()]),
            since: None,
        };
        engines[i]
            .client
            .subscribe(&params)
            .map_err(|e| CliError::failed(format!("{} failed: {e}", method::SUBSCRIBE)))?;
    }
    let params = UndoApplyParams {
        token: a.token.map(str::to_owned),
        any: a.any,
        tz: tern_client::search::local_time_zone(),
        notify: false,
    };
    let params = serde_json::to_value(&params).unwrap_or_else(|_| json!({}));
    let r: UndoApplyResult = engines[i]
        .client
        .call_as(what, params)
        .map_err(|e| refused(what, e))?;
    let too_late = r.cancelled == Some(false);
    let mut doc = serde_json::to_value(&r).unwrap_or(Value::Null);
    if !ctx.json {
        output::line(&if too_late {
            "Too late: the message was already sent".to_owned()
        } else {
            format!(
                "{}: {} ({})",
                if a.redo { "Redid" } else { "Undid" },
                r.entry.label,
                r.entry.client
            )
        });
    }
    if too_late {
        if ctx.json {
            output::json(&doc);
        }
        return Err(CliError::failed("the message was already sent"));
    }
    if !a.wait || r.task_ids.is_empty() {
        if ctx.json {
            output::json(&doc);
        }
        return Ok(());
    }
    let engine = engines.swap_remove(i);
    let subs = Subscriptions::start(vec![(engine, true)], None, None)?;
    let finished = wait_all(subs, &r.task_ids, a.timeout)?;
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

/// What an entry's state column says.
fn state(e: &UndoEntry) -> String {
    match (e.state.as_str(), e.can_undo, e.can_redo) {
        (undo_state::DONE, true, _) => "done".into(),
        (undo_state::UNDONE, _, true) => "undone (redo)".into(),
        (undo_state::UNDONE, _, false) => "undone".into(),
        (s, _, _) => s.to_owned(),
    }
}

pub fn list(ctx: &Ctx, mut engines: Vec<Engine>, mine: bool, limit: Option<usize>) -> Result<()> {
    let i = pick(ctx, &engines)?;
    let params = serde_json::to_value(UndoListParams { limit, mine }).unwrap_or_default();
    let r: UndoListResult = engines[i]
        .client
        .call_as(method::UNDO_LIST, params)
        .map_err(|e| refused(method::UNDO_LIST, e))?;
    if ctx.json {
        output::json(&serde_json::to_value(&r).unwrap_or(Value::Null));
        return Ok(());
    }
    if r.entries.is_empty() {
        output::line("Nothing to undo.");
        return Ok(());
    }
    let now = Local::now();
    let mut table = Table::new(vec!["WHEN", "CLIENT", "CHANGE", "STATE", "TOKEN"], 2);
    for e in &r.entries {
        table.row(
            vec![
                output::short_date(e.at, &now),
                e.client.clone(),
                e.label.clone(),
                state(e),
                e.token.clone(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}
