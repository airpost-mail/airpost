// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Running a parsed command line: the shared context, and the calls every command makes.

use std::path::PathBuf;

use ms_ipc::{code, ClientError};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::cli::{CalendarCommand, Cli, Command, TaskCommand};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::output::Style;

pub struct Ctx {
    pub config_dir: PathBuf,
    pub account: Option<String>,
    pub json: bool,
    pub style: Style,
}

impl Ctx {
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }
}

pub fn run(cli: Cli) -> Result<()> {
    let config_dir = cli
        .config_dir
        .or_else(discovery::default_config_dir)
        .ok_or_else(|| {
            CliError::usage(
                "could not work out the Tern profile directory; give --config-dir or set \
                 TERN_CONFIG_DIR",
            )
        })?;
    let ctx = Ctx {
        config_dir,
        account: cli.account,
        json: cli.json,
        style: Style::detect(),
    };
    if let Command::Logs {
        account,
        follow,
        lines,
    } = &cli.command
    {
        let account = match account.as_ref().or(ctx.account.as_ref()) {
            Some(a) => a.clone(),
            None => match crate::logs::logged_accounts(&ctx.config_dir).as_slice() {
                [only] => only.clone(),
                [] => {
                    return Err(CliError::failed(format!(
                        "no engine logs in {}",
                        ctx.config_dir.display()
                    )))
                }
                several => {
                    return Err(CliError::usage(format!(
                        "logs exist for several accounts ({}); pick one",
                        several.join(", ")
                    )))
                }
            },
        };
        return crate::logs::run(&ctx.config_dir, &account, *follow, *lines, ctx.json);
    }
    let mut engines = discovery::connect_all(&ctx.config_dir)?;
    match cli.command {
        Command::Status => crate::status::status(&ctx, &mut engines),
        Command::Accounts => crate::status::accounts(&ctx, &engines),
        Command::Sync => crate::status::sync(&ctx, &mut engines),
        Command::Calendar {
            command: CalendarCommand::Sync,
        } => crate::status::calendar_sync(&ctx, &mut engines),
        Command::Rpc { method, params } => {
            crate::status::rpc(&ctx, &mut engines, &method, params.as_deref())
        }
        Command::Folders => crate::read::categories(&ctx, &mut engines, crate::read::Kind::Folder),
        Command::Labels => crate::read::categories(&ctx, &mut engines, crate::read::Kind::Label),
        Command::Threads(args) => crate::read::threads(&ctx, &mut engines, &args),
        Command::Show { id, body } => crate::read::show(&ctx, &mut engines, &id, body),
        Command::Search { query, limit } => {
            crate::read::search(&ctx, &mut engines, &query.join(" "), limit)
        }
        Command::Contacts { query, limit } => {
            crate::read::contacts(&ctx, &mut engines, &query.join(" "), limit)
        }
        Command::Watch { classes } => crate::watch::watch(&ctx, engines, classes),
        Command::Task { command } => match command {
            TaskCommand::Queue {
                task,
                wait,
                timeout,
            } => crate::tasks::queue(&ctx, engines, &task, wait, timeout),
            TaskCommand::Cancel { id } => crate::tasks::cancel(&ctx, &mut engines, &id),
            TaskCommand::Wait { id, timeout } => crate::tasks::wait(&ctx, engines, &id, timeout),
        },
        Command::Logs { .. } => unreachable!("handled before connecting"),
    }
}

/// Calls `method` on the engine of `t`, reading the result as `T`.
pub fn call<T: DeserializeOwned>(
    engines: &mut [Engine],
    t: &Target,
    method: &str,
    params: Value,
) -> Result<T> {
    call_raw(engines, t, method, params).map_err(|e| CliError::call(&t.email, method, e))
}

/// [`call`] keeping the client's error, for callers that handle some codes themselves.
pub fn call_raw<T: DeserializeOwned>(
    engines: &mut [Engine],
    t: &Target,
    method: &str,
    params: Value,
) -> std::result::Result<T, ClientError> {
    engines[t.engine].client.call_as(method, params)
}

pub fn is_not_found(e: &ClientError) -> bool {
    e.code() == Some(code::NOT_FOUND)
}
