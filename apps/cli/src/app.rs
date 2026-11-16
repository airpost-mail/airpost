// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Running a parsed command line: the shared context, and the calls every command makes.

use std::path::PathBuf;

use airpost_ipc::{code, ClientError};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::builtin::{self, Action, Kind};
use crate::cli::{
    AccountCommand, AiCommand, CalendarCommand, Cli, Command, ContactCommand, EngineCommand,
    FolderCommand, GatewayCommand, InstallArgs, LabelCommand, McpHttpCommand, ServiceCommand,
    SettingsCommand, TaskCommand, UnitArgs, UnitCommand,
};
use crate::cli::{AttachmentCommand, TrackingCommand, UndoArgs};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::organize::{self, CategoryOp, ThreadAction};
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
    // The skill doesn't depend on a profile.
    if let Command::Skill { command } = &cli.command {
        let ctx = Ctx {
            config_dir: PathBuf::new(),
            account: None,
            json: cli.json,
            style: Style::detect(),
        };
        return crate::skill::run(&ctx, command);
    }
    let config_dir = cli
        .config_dir
        .or_else(discovery::default_config_dir)
        .ok_or_else(|| {
            CliError::usage(
                "could not work out the Airpost profile directory; give --config-dir or set \
                 AIRPOST_CONFIG_DIR",
            )
        })?;
    // Absolute, because units and services.json record it and both copies of airpost compare it.
    let config_dir = std::path::absolute(&config_dir).unwrap_or(config_dir);
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
        if let Some(kind) = account.as_deref().and_then(Kind::parse) {
            return builtin::logs(&ctx, kind, *follow, *lines);
        }
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
        Command::Gateway { command } => return gateway_unit(&ctx, command),
        Command::McpHttp { command } => return mcp_http_unit(&ctx, command),
        Command::Uninstall {
            purge_profile,
            yes,
            link_paths,
            units,
        } => {
            let args = builtin::UninstallAll {
                purge_profile: *purge_profile,
                yes: *yes,
                link_paths: link_paths.clone(),
            };
            return builtin::uninstall_all_command(&ctx, &place_args(units), &args);
        }
        // The services are shown with no engine running too; the exit code still says there
        // is none.
        Command::Status => {
            return match discovery::connect_all(&ctx.config_dir) {
                Ok(mut engines) => crate::status::status(&ctx, &mut engines),
                Err(e @ CliError::NoEngine(_)) => {
                    crate::status::services_only(&ctx);
                    Err(e)
                }
                Err(e) => Err(e),
            }
        }
        _ => {}
    }
    let mut engines = discovery::connect_all(&ctx.config_dir)?;
    match cli.command {
        Command::Accounts => crate::status::accounts(&ctx, &engines),
        Command::Account { command } => account(&ctx, &mut engines, command),
        Command::Calendars => crate::pim::calendars(&ctx, &mut engines),
        Command::Rules { command } => crate::rules::run(&ctx, &mut engines, command),
        Command::Signature { command } => crate::signature::run(&ctx, &mut engines, command),
        Command::Spam(select) => organize::threads(&ctx, engines, ThreadAction::Spam, &select),
        Command::NotSpam(select) => {
            organize::threads(&ctx, engines, ThreadAction::NotSpam, &select)
        }
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
        Command::Draft { command } => crate::draft::run(&ctx, engines, command),
        Command::Archive(select) => {
            organize::threads(&ctx, engines, ThreadAction::Archive, &select)
        }
        Command::Trash(select) => organize::threads(&ctx, engines, ThreadAction::Trash, &select),
        Command::Move { folder, select } => {
            organize::threads(&ctx, engines, ThreadAction::Move(&folder), &select)
        }
        Command::MarkRead(select) => {
            organize::threads(&ctx, engines, ThreadAction::Unread(false), &select)
        }
        Command::MarkUnread(select) => {
            organize::threads(&ctx, engines, ThreadAction::Unread(true), &select)
        }
        Command::Star(select) => {
            organize::threads(&ctx, engines, ThreadAction::Starred(true), &select)
        }
        Command::Unstar(select) => {
            organize::threads(&ctx, engines, ThreadAction::Starred(false), &select)
        }
        Command::Label { command } => label(&ctx, engines, command),
        Command::Folder { command } => folder(&ctx, engines, command),
        Command::Template { command } => crate::template::run(&ctx, &mut engines, command),
        Command::Unsnooze { threads } => {
            crate::mail_actions::unsnooze(&ctx, &mut engines, &threads)
        }
        Command::Undo {
            list: true,
            mine,
            limit,
            ..
        } => crate::undo::list(&ctx, engines, mine, limit),
        Command::Undo { args, .. } => crate::undo::undo(&ctx, engines, &undo_args(&args, false)),
        Command::Redo(args) => crate::undo::undo(&ctx, engines, &undo_args(&args, true)),
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
        Command::Remind { id, when, clear } => {
            crate::followup::remind(&ctx, &mut engines, &id, when.as_deref(), clear)
        }
        Command::Tracking { command, message } => match (command, message) {
            (Some(TrackingCommand::List { kind, since, limit }), _) => {
                crate::followup::list(&ctx, &mut engines, &kind, since.as_deref(), limit)
            }
            (Some(TrackingCommand::Links { since, limit }), _) => {
                crate::followup::links(&ctx, &mut engines, since.as_deref(), limit)
            }
            (None, Some(id)) => crate::followup::show(&ctx, &mut engines, &id),
            (None, None) => Err(CliError::usage(
                "give a message id, or `airpost tracking list` or `airpost tracking links`",
            )),
        },
        Command::Attachment { command } => match command {
            AttachmentCommand::Get {
                file,
                output,
                force,
            } => crate::files::attachment_get(&ctx, &mut engines, &file, output.as_deref(), force),
            AttachmentCommand::Save {
                message,
                dir,
                force,
            } => crate::files::attachment_save(&ctx, &mut engines, &message, dir.as_deref(), force),
        },
        Command::Export {
            message,
            mbox: _,
            folder,
            output,
            force,
            timeout,
        } => match (message, folder) {
            (Some(m), _) => {
                crate::files::export_message(&ctx, engines, &m, &output, force, timeout)
            }
            (None, Some(f)) => {
                crate::files::export_mbox(&ctx, engines, &f, &output, force, timeout)
            }
            (None, None) => Err(CliError::usage("give a message id, or --mbox --folder")),
        },
        Command::Status
        | Command::Logs { .. }
        | Command::Service { .. }
        | Command::Engine { .. }
        | Command::Gateway { .. }
        | Command::McpHttp { .. }
        | Command::Uninstall { .. }
        | Command::Skill { .. } => {
            unreachable!("handled before connecting")
        }
    }
}

fn account(ctx: &Ctx, engines: &mut [Engine], command: AccountCommand) -> Result<()> {
    use crate::account;
    match command {
        AccountCommand::Add(args) => account::add(ctx, engines, &args),
        AccountCommand::Remove {
            account,
            delete_data,
            yes,
        } => account::remove(ctx, engines, &account, delete_data, yes),
        AccountCommand::Update(args) => account::update(ctx, engines, &args),
        AccountCommand::Reorder { accounts } => account::reorder(ctx, engines, &accounts),
        AccountCommand::Resync { account, yes } => account::resync(ctx, engines, &account, yes),
        AccountCommand::Reauth {
            account,
            password_stdin,
            sign_in,
        } => account::reauth(ctx, engines, &account, password_stdin, &sign_in),
    }
}

fn undo_args(a: &UndoArgs, redo: bool) -> crate::undo::UndoArgs<'_> {
    crate::undo::UndoArgs {
        token: a.token.as_deref(),
        any: a.any,
        redo,
        wait: a.wait,
        timeout: a.timeout,
    }
}

fn label(ctx: &Ctx, engines: Vec<Engine>, command: LabelCommand) -> Result<()> {
    use crate::read::Kind;
    match command {
        LabelCommand::Add { label, select } => organize::threads(
            ctx,
            engines,
            ThreadAction::Label {
                name: &label,
                add: true,
            },
            &select,
        ),
        LabelCommand::Remove { label, select } => organize::threads(
            ctx,
            engines,
            ThreadAction::Label {
                name: &label,
                add: false,
            },
            &select,
        ),
        LabelCommand::Create { name, wait } => {
            organize::category(ctx, engines, Kind::Label, CategoryOp::Create(&name), &wait)
        }
        LabelCommand::Rename {
            label,
            new_name,
            wait,
        } => organize::category(
            ctx,
            engines,
            Kind::Label,
            CategoryOp::Rename {
                from: &label,
                to: &new_name,
            },
            &wait,
        ),
        LabelCommand::Delete { label, wait } => organize::category(
            ctx,
            engines,
            Kind::Label,
            CategoryOp::Delete {
                name: &label,
                force: true,
            },
            &wait,
        ),
    }
}

fn folder(ctx: &Ctx, engines: Vec<Engine>, command: FolderCommand) -> Result<()> {
    use crate::read::Kind;
    match command {
        FolderCommand::Create { name, wait } => {
            organize::category(ctx, engines, Kind::Folder, CategoryOp::Create(&name), &wait)
        }
        FolderCommand::Rename {
            folder,
            new_name,
            wait,
        } => organize::category(
            ctx,
            engines,
            Kind::Folder,
            CategoryOp::Rename {
                from: &folder,
                to: &new_name,
            },
            &wait,
        ),
        FolderCommand::Delete {
            folder,
            force,
            wait,
        } => organize::category(
            ctx,
            engines,
            Kind::Folder,
            CategoryOp::Delete {
                name: &folder,
                force,
            },
            &wait,
        ),
        FolderCommand::Empty {
            folder,
            dry_run,
            yes,
            wait,
            timeout,
        } => organize::empty_folder(
            ctx,
            engines,
            &organize::EmptyArgs {
                folder: &folder,
                dry_run,
                yes,
                wait,
                timeout,
            },
        ),
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
        no_start: false,
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

fn install_place(i: &InstallArgs) -> crate::service::PlaceArgs {
    crate::service::PlaceArgs {
        no_start: i.no_start,
        ..place_args(&i.units)
    }
}

fn unit_action(ctx: &Ctx, kind: Kind, command: &UnitCommand) -> Result<()> {
    let (units, action) = match command {
        UnitCommand::Uninstall { units } => (units, Action::Uninstall),
        UnitCommand::Start { units } => (units, Action::Start),
        UnitCommand::Stop { units } => (units, Action::Stop),
        UnitCommand::Restart { units } => (units, Action::Restart),
    };
    builtin::command(ctx, kind, &place_args(units), action)
}

fn install_options(i: &InstallArgs, bin: &Option<PathBuf>) -> builtin::Options {
    builtin::Options {
        bin: bin.clone(),
        replace: i.replace,
        dry_run: i.dry_run,
        ..builtin::Options::default()
    }
}

fn engine_unit(ctx: &Ctx, command: &EngineCommand) -> Result<()> {
    match command {
        EngineCommand::Install {
            bin,
            key_file,
            install,
        } => {
            let opts = builtin::Options {
                key_file: key_file.clone(),
                ..install_options(install, bin)
            };
            builtin::command(
                ctx,
                Kind::Engine,
                &install_place(install),
                Action::Install(opts),
            )
        }
        EngineCommand::Unit(u) => unit_action(ctx, Kind::Engine, u),
    }
}

fn gateway_unit(ctx: &Ctx, command: &GatewayCommand) -> Result<()> {
    match command {
        GatewayCommand::Install {
            bin,
            bind,
            web_root,
            public_url,
            install,
        } => {
            let opts = builtin::Options {
                bind: Some(*bind),
                web_root: web_root
                    .as_deref()
                    .filter(|d| !d.is_empty())
                    .map(PathBuf::from),
                public_url: public_url.clone(),
                ..install_options(install, bin)
            };
            builtin::command(
                ctx,
                Kind::Gateway,
                &install_place(install),
                Action::Install(opts),
            )
        }
        GatewayCommand::Unit(u) => unit_action(ctx, Kind::Gateway, u),
        GatewayCommand::Pair { admin, name, ttl } => builtin::pair(
            ctx,
            &builtin::PairArgs {
                admin: *admin,
                name: name.as_deref(),
                ttl: ttl.as_deref(),
            },
        ),
    }
}

fn mcp_http_unit(ctx: &Ctx, command: &McpHttpCommand) -> Result<()> {
    match command {
        McpHttpCommand::Install { bin, port, install } => {
            let opts = builtin::Options {
                port: *port,
                ..install_options(install, bin)
            };
            builtin::command(
                ctx,
                Kind::McpHttp,
                &install_place(install),
                Action::Install(opts),
            )
        }
        McpHttpCommand::Unit(u) => unit_action(ctx, Kind::McpHttp, u),
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

/// Asks `question` (a yes/no question) on the terminal unless `yes`. Without a terminal, or
/// with --json, the command needs --yes instead.
pub fn confirm(ctx: &Ctx, yes: bool, question: &str) -> Result<()> {
    use std::io::{IsTerminal, Write};
    if yes {
        return Ok(());
    }
    if ctx.json || !std::io::stdin().is_terminal() {
        return Err(CliError::usage(format!(
            "{question} Give --yes to go ahead (nothing is asked with --json or without a \
             terminal)"
        )));
    }
    let mut err = std::io::stderr();
    let _ = write!(err, "{question} [y/N] ");
    let _ = err.flush();
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::failed("nothing changed"))
    }
}
