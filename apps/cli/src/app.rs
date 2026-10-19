// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Running a parsed command line: the shared context, and the calls every command makes.

use std::path::PathBuf;

use ms_ipc::{code, ClientError};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::cli::{
    AccountCommand, AiCommand, CalendarCommand, Cli, Command, ContactCommand, EngineCommand,
    ServiceCommand, SettingsCommand, TaskCommand, UnitArgs,
};
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
    match &cli.command {
        Command::Service { command } => return service(&ctx, command),
        Command::Engine { command } => return engine_unit(&ctx, command),
        _ => {}
    }
    let mut engines = discovery::connect_all(&ctx.config_dir)?;
    match cli.command {
        Command::Status => crate::status::status(&ctx, &mut engines),
        Command::Accounts => crate::status::accounts(&ctx, &engines),
        Command::Account {
            command: AccountCommand::Add(args),
        } => crate::account::add(&ctx, &mut engines, &args),
        Command::Sync => crate::status::sync(&ctx, &mut engines),
        Command::Calendar { command } => calendar(&ctx, &mut engines, command),
        Command::Contact { command } => contact(&ctx, &mut engines, command),
        Command::Settings { command } => match command {
            SettingsCommand::Get { keys } => crate::settings::get(&ctx, &mut engines, keys),
            SettingsCommand::Set { key, value } => {
                crate::settings::set(&ctx, &mut engines, &key, &value)
            }
        },
        Command::Ai { command } => match command {
            AiCommand::Summarize { thread, refresh } => {
                crate::ai::summarize(&ctx, &mut engines, &thread, refresh)
            }
            AiCommand::Triage { since, threads } => {
                crate::ai::triage(&ctx, &mut engines, since.as_deref(), &threads)
            }
            AiCommand::Usage { days } => crate::ai::usage(&ctx, &mut engines, days),
        },
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
        Command::Snooze { threads, until } => {
            crate::mail_actions::snooze(&ctx, &mut engines, &threads, &until)
        }
        Command::Unsnooze { threads } => {
            crate::mail_actions::unsnooze(&ctx, &mut engines, &threads)
        }
        Command::Unsubscribe { message, method } => {
            crate::mail_actions::unsubscribe(&ctx, &mut engines, &message, method.as_deref())
        }
        Command::Watch { classes, new_mail } => {
            crate::watch::watch(&ctx, engines, classes, new_mail)
        }
        Command::Task { command } => match command {
            TaskCommand::List {
                status,
                class,
                header_message_id,
                limit,
            } => crate::tasks::list(
                &ctx,
                &mut engines,
                &crate::tasks::ListFilter {
                    status,
                    class,
                    header_message_id,
                    limit,
                },
            ),
            TaskCommand::Queue {
                task,
                wait,
                timeout,
            } => crate::tasks::queue(&ctx, engines, &task, wait, timeout),
            TaskCommand::Cancel { id } => crate::tasks::cancel(&ctx, &mut engines, &id),
            TaskCommand::Wait { id, timeout } => crate::tasks::wait(&ctx, engines, &id, timeout),
        },
        Command::Logs { .. } | Command::Service { .. } | Command::Engine { .. } => {
            unreachable!("handled before connecting")
        }
    }
}

fn place_args(u: &UnitArgs) -> crate::service::PlaceArgs {
    use crate::units::Manager;
    crate::service::PlaceArgs {
        unit_dir: u.unit_dir.clone(),
        manager: u.manager.as_deref().map(|m| match m {
            "launchd" => Manager::Launchd,
            _ => Manager::Systemd,
        }),
        no_load: u.no_load,
    }
}

/// Unit commands need no engine: they write files and talk to the service manager.
fn service(ctx: &Ctx, command: &ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Install {
            package,
            package_dir,
            engine_bin,
            engine_key_file,
            expect_sha256,
            expect_plan,
            dry_run,
            units,
        } => crate::service::install_command(
            ctx,
            &place_args(units),
            &crate::service::ServiceArgs {
                package: package.clone(),
                package_dir: package_dir.clone(),
                engine_bin: engine_bin.clone(),
                engine_key_file: engine_key_file.clone(),
                expect_sha256: expect_sha256.clone(),
                expect_plan: expect_plan.clone(),
                dry_run: *dry_run,
            },
        ),
        ServiceCommand::Uninstall { package, units } => {
            crate::service::uninstall_command(ctx, &place_args(units), package)
        }
        ServiceCommand::List => crate::service::list_command(ctx),
    }
}

fn engine_unit(ctx: &Ctx, command: &EngineCommand) -> Result<()> {
    match command {
        EngineCommand::Install {
            bin,
            key_file,
            dry_run,
            units,
        } => crate::service::engine_install_command(
            ctx,
            &place_args(units),
            bin.as_deref(),
            key_file.as_deref(),
            *dry_run,
        ),
        EngineCommand::Uninstall { units } => {
            crate::service::engine_uninstall_command(ctx, &place_args(units))
        }
        EngineCommand::Start { units } => {
            crate::service::engine_start_command(ctx, &place_args(units), false)
        }
        EngineCommand::Restart { units } => {
            crate::service::engine_start_command(ctx, &place_args(units), true)
        }
    }
}

fn calendar(ctx: &Ctx, engines: &mut [Engine], command: CalendarCommand) -> Result<()> {
    use crate::pim;
    match command {
        CalendarCommand::Sync => crate::status::calendar_sync(ctx, engines),
        CalendarCommand::List {
            from,
            to,
            calendars,
        } => pim::list(ctx, engines, from.as_deref(), to.as_deref(), &calendars),
        CalendarCommand::Create {
            calendar,
            title,
            start,
            end,
            all_day,
            location,
            description,
            guests,
            repeat,
        } => pim::create(
            ctx,
            engines,
            pim::NewEvent {
                calendar: &calendar,
                title: &title,
                start: &start,
                end: end.as_deref(),
                all_day,
                location: location.as_deref(),
                description: description.as_deref(),
                guests: &guests,
                rrule: repeat.as_deref(),
            },
        ),
        CalendarCommand::Update {
            event,
            scope,
            occurrence,
            start,
            end,
            title,
            location,
            description,
            delete,
        } => pim::update(
            ctx,
            engines,
            pim::EventEdit {
                event: &event,
                scope: &scope,
                occurrence: occurrence.as_deref(),
                start: start.as_deref(),
                end: end.as_deref(),
                title: title.as_deref(),
                location: location.as_deref(),
                description: description.as_deref(),
                delete,
            },
        ),
        CalendarCommand::Rsvp {
            event,
            status,
            comment,
            propose_start,
            propose_end,
        } => {
            let propose = propose_start.as_deref().zip(propose_end.as_deref());
            pim::rsvp(ctx, engines, &event, &status, comment.as_deref(), propose)
        }
    }
}

fn contact(ctx: &Ctx, engines: &mut [Engine], command: ContactCommand) -> Result<()> {
    use crate::pim;
    let fields = |f: &crate::cli::ContactFieldArgs| {
        pim::contact_fields(
            f.patch.as_deref(),
            f.name.as_deref(),
            f.email.as_deref(),
            f.phone.as_deref(),
            f.company.as_deref(),
            f.title.as_deref(),
            f.notes.as_deref(),
        )
    };
    match command {
        ContactCommand::Get { id } => pim::contact_get(ctx, engines, &id),
        ContactCommand::Create { fields: f } => pim::contact_create(ctx, engines, fields(&f)?),
        ContactCommand::Update { id, fields: f } => {
            pim::contact_update(ctx, engines, &id, fields(&f)?)
        }
        ContactCommand::Import { file } => pim::contact_import(ctx, engines, &file),
        ContactCommand::Export { ids } => pim::contact_export(ctx, engines, &ids),
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
