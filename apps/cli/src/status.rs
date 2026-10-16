// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Engine-level commands: status, accounts, sync, calendar sync, raw rpc.

use ms_ipc::{method, CalendarSyncResult, EngineStatusResult};
use serde_json::{json, Value};

use crate::app::{call, Ctx};
use crate::discovery::{self, Engine};
use crate::error::{CliError, Result};
use crate::output::{self, Table};

pub fn status(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    if ctx.account.is_some() {
        discovery::targets(engines, ctx.account())?;
    }
    let mut docs = Vec::new();
    let mut table = Table::new(
        vec![
            "ACCOUNT", "ID", "STATE", "SERVER", "PID", "UPTIME", "ENGINE",
        ],
        1,
    );
    for e in engines.iter_mut() {
        let pid = e.entry.pid;
        let raw = e
            .client
            .call(method::ENGINE_STATUS, json!({}))
            .map_err(|err| {
                CliError::call(&format!("engine pid {pid}"), method::ENGINE_STATUS, err)
            })?;
        let s: EngineStatusResult = serde_json::from_value(raw.clone()).map_err(|err| {
            CliError::failed(format!("engine pid {pid}: odd engine.status: {err}"))
        })?;
        let accounts: Vec<_> = s
            .accounts
            .iter()
            .filter(|a| {
                ctx.account()
                    .is_none_or(|want| a.account_id == want || a.email.eq_ignore_ascii_case(want))
            })
            .collect();
        if accounts.is_empty() && ctx.account.is_some() {
            continue;
        }
        for a in &accounts {
            let server = if a.connection_error { "offline" } else { "ok" };
            table.row(
                vec![
                    a.email.clone(),
                    a.account_id.clone(),
                    a.state.clone(),
                    server.to_owned(),
                    s.pid.to_string(),
                    output::uptime(s.uptime_secs),
                    s.engine.clone(),
                ],
                false,
            );
            if a.connection_error {
                table.alert(3);
            }
        }
        let mut doc = raw;
        doc["socket"] = json!(e.entry.socket);
        docs.push(doc);
    }
    if ctx.json {
        output::json(&json!({ "engines": docs }));
    } else {
        table.print(&ctx.style);
    }
    Ok(())
}

pub fn accounts(ctx: &Ctx, engines: &[Engine]) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    if ctx.json {
        let list: Vec<Value> = targets
            .iter()
            .map(|t| {
                let e = &engines[t.engine];
                let state = e
                    .hello
                    .accounts
                    .iter()
                    .find(|a| a.account_id == t.account_id)
                    .map(|a| a.state.as_str());
                json!({
                    "accountId": t.account_id,
                    "email": t.email,
                    "state": state,
                    "pid": e.hello.pid,
                    "socket": e.entry.socket,
                })
            })
            .collect();
        output::json(&json!({ "accounts": list }));
        return Ok(());
    }
    let mut table = Table::new(vec!["EMAIL", "ID", "STATE", "PID"], 0);
    for t in &targets {
        let e = &engines[t.engine];
        let state = e
            .hello
            .accounts
            .iter()
            .find(|a| a.account_id == t.account_id)
            .map_or("", |a| a.state.as_str());
        table.row(
            vec![
                t.email.clone(),
                t.account_id.clone(),
                state.to_owned(),
                e.hello.pid.to_string(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

pub fn sync(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    for t in &targets {
        call::<Value>(
            engines,
            t,
            method::SYNC_WAKE,
            json!({ "accountId": t.account_id }),
        )?;
        if !ctx.json {
            output::line(&format!("Syncing {}", t.email));
        }
    }
    if ctx.json {
        let ids: Vec<&str> = targets.iter().map(|t| t.account_id.as_str()).collect();
        output::json(&json!({ "accounts": ids }));
    }
    Ok(())
}

pub fn calendar_sync(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    let t = discovery::one_target(engines, ctx.account())?;
    let r: CalendarSyncResult = call(
        engines,
        &t,
        method::CALENDAR_SYNC,
        json!({ "accountId": t.account_id }),
    )?;
    if ctx.json {
        output::json(&json!({ "accountId": t.account_id, "started": r.started }));
    } else if r.started {
        output::line(&format!("Calendar sync started for {}", t.email));
    } else {
        output::line(&format!(
            "A calendar sync is already running for {}",
            t.email
        ));
    }
    Ok(())
}

/// The engine a raw request goes to: the one holding `params.accountId` or `--account`, or
/// the only one. With `--account`, an object without `accountId` gets it.
fn rpc_engine(ctx: &Ctx, engines: &[Engine], params: &mut Value) -> Result<usize> {
    let in_params = params["accountId"].as_str().map(str::to_owned);
    if let Some(want) = in_params.as_deref().or(ctx.account()) {
        let t = discovery::one_target(engines, Some(want))?;
        if let Some(obj) = params.as_object_mut() {
            obj.entry("accountId")
                .or_insert_with(|| json!(t.account_id));
        }
        return Ok(t.engine);
    }
    if engines.len() == 1 {
        return Ok(0);
    }
    Err(CliError::usage(
        "several engines are running; pick one with --account or an accountId in the params",
    ))
}

pub fn rpc(ctx: &Ctx, engines: &mut [Engine], method: &str, params: Option<&str>) -> Result<()> {
    let mut params: Value = match params {
        None => json!({}),
        Some("-") => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
                .map_err(|e| CliError::failed(format!("could not read stdin: {e}")))?;
            parse_params(&s)?
        }
        Some(s) => parse_params(s)?,
    };
    let i = rpc_engine(ctx, engines, &mut params)?;
    let pid = engines[i].entry.pid;
    let result = engines[i]
        .client
        .call(method, params)
        .map_err(|e| CliError::call(&format!("engine pid {pid}"), method, e))?;
    if ctx.json {
        output::json(&result);
    } else {
        output::line(&serde_json::to_string_pretty(&result).unwrap_or_default());
    }
    Ok(())
}

fn parse_params(s: &str) -> Result<Value> {
    serde_json::from_str(s).map_err(|e| {
        CliError::usage(format!(
            "the params aren't JSON ({e}); quote them, e.g. '{{\"accountId\": \"...\"}}'"
        ))
    })
}
