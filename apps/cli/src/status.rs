// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Engine-level commands: status, accounts, sync, calendar sync, raw rpc.

use serde_json::{json, Value};
use tern_ipc::{account_state, method, CalendarSyncResult, EngineStatusResult};

use crate::app::{call, Ctx};
use crate::builtin;
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
            let running = a.state == account_state::RUNNING;
            let server = match (running, a.connection_error) {
                (false, _) => "-",
                (true, true) => "offline",
                (true, false) => "ok",
            };
            table.row(
                vec![
                    a.email.clone(),
                    a.account_id.clone(),
                    state_text(&a.state, a.error.as_ref()),
                    server.to_owned(),
                    s.pid.to_string(),
                    output::uptime(s.uptime_secs),
                    s.engine.clone(),
                ],
                false,
            );
            if a.state == account_state::STOPPED {
                table.alert(2);
            }
            if running && a.connection_error {
                table.alert(3);
            }
        }
        let mut doc = raw;
        doc["socket"] = json!(e.entry.socket);
        docs.push(doc);
    }
    let place = builtin::status_place(&ctx.config_dir);
    let services = builtin::status_rows(&ctx.config_dir, place.as_ref(), engines);
    let skills = crate::skill::installed();
    if ctx.json {
        output::json(&json!({ "engines": docs, "services": services, "skills": skills }));
    } else {
        table.print(&ctx.style);
        output::line("");
        builtin::print_status_rows(ctx, &services);
        crate::skill::warn_stale(&skills);
    }
    Ok(())
}

/// `tern status` with no engine running: the services alone.
pub fn services_only(ctx: &Ctx) {
    let place = builtin::status_place(&ctx.config_dir);
    let services = builtin::status_rows(&ctx.config_dir, place.as_ref(), &[]);
    let skills = crate::skill::installed();
    if ctx.json {
        output::json(&json!({ "engines": [], "services": services, "skills": skills }));
    } else {
        builtin::print_status_rows(ctx, &services);
        crate::skill::warn_stale(&skills);
    }
}

/// An account's `state`, with the error key that stopped it (`stopped: ErrorAuthentication`).
fn state_text(state: &str, error: Option<&Value>) -> String {
    let key = error
        .and_then(|e| e["key"].as_str())
        .filter(|k| !k.is_empty());
    match key {
        Some(k) if state == account_state::STOPPED => format!("{state}: {k}"),
        _ => state.to_owned(),
    }
}

pub fn accounts(ctx: &Ctx, engines: &[Engine]) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    if ctx.json {
        let list: Vec<Value> = targets
            .iter()
            .map(|t| {
                let e = &engines[t.engine];
                let info = e
                    .hello
                    .accounts
                    .iter()
                    .find(|a| a.account_id == t.account_id);
                json!({
                    "accountId": t.account_id,
                    "email": t.email,
                    "state": info.map(|a| a.state.as_str()),
                    "error": info.and_then(|a| a.error.clone()),
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
            .map_or(String::new(), |a| state_text(&a.state, a.error.as_ref()));
        table.row(
            vec![
                t.email.clone(),
                t.account_id.clone(),
                state,
                e.hello.pid.to_string(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

pub fn sync(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    let mut targets = discovery::targets(engines, ctx.account())?;
    // Without --account, a stopped account (`serve` mode) is skipped rather than failing
    // the whole command; it has to be added again to sync.
    if ctx.account.is_none() {
        targets.retain(|t| {
            let state = engines[t.engine]
                .hello
                .accounts
                .iter()
                .find(|a| a.account_id == t.account_id)
                .map(|a| a.state.as_str());
            let skip = state == Some(account_state::STOPPED);
            if skip && !ctx.json {
                output::line(&format!("Skipping {}: stopped", t.email));
            }
            !skip
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_states_name_their_error() {
        let e = json!({"key": "ErrorAuthentication", "retryable": false});
        assert_eq!(state_text("running", None), "running");
        assert_eq!(
            state_text("stopped", Some(&e)),
            "stopped: ErrorAuthentication"
        );
        assert_eq!(state_text("stopped", Some(&json!({}))), "stopped");
        assert_eq!(state_text("starting", Some(&e)), "starting");
    }
}
