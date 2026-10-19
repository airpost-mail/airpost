// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The command line (docs/engine-protocol-plan.md section 3, "CLI").

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "tern",
    version,
    about = "Talk to the running Tern mail engines",
    after_help = "Exit codes: 0 ok, 1 command failed, 2 usage, 3 no engine running."
)]
pub struct Cli {
    /// The Tern profile directory (default: the desktop app's profile for this OS).
    #[arg(long, global = true, env = "TERN_CONFIG_DIR", value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// The account to act on, by id or email address. Read commands without it cover every
    /// account.
    #[arg(long, short = 'a', global = true, value_name = "ID|EMAIL")]
    pub account: Option<String>,

    /// Machine-readable output: one JSON document, or NDJSON for streams.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// The running engines and the state of their accounts.
    Status,
    /// The accounts the running engines hold.
    Accounts,
    /// Add an account to the serve engine.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Print deltas and new mail as the engines send them, until interrupted.
    Watch {
        /// Only these model classes (Thread, Message, Folder, Task, ...). New mail is
        /// printed either way.
        #[arg(
            long = "class",
            short = 'c',
            value_delimiter = ',',
            value_name = "CLASS"
        )]
        classes: Vec<String>,
        /// Only new mail: one line per message that lands unread in the inbox.
        #[arg(long, conflicts_with = "classes")]
        new_mail: bool,
    },
    /// Start a sync pass now.
    Sync,
    /// Calendar commands.
    Calendar {
        #[command(subcommand)]
        command: CalendarCommand,
    },
    /// List, queue, cancel and wait for tasks.
    #[command(visible_alias = "tasks")]
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Folders with unread and total thread counts.
    Folders,
    /// Labels with unread and total thread counts.
    Labels,
    /// Threads, newest first.
    Threads(ThreadsArgs),
    /// A message (or, given a thread id, the thread and its messages).
    Show {
        /// Message id, or a thread id.
        id: String,
        /// Include the body, downloading it first if needed.
        #[arg(long)]
        body: bool,
    },
    /// Threads matching the query, in the search bar's grammar: words, "phrases",
    /// from:, to:, subject:, in:, is:unread|read|starred|unstarred, has:attachment,
    /// before:/after: dates (2024-03-01, yesterday, "3 days ago", 2w), AND, OR, NOT and
    /// parentheses. An engine without the grammar matches every word instead.
    Search {
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// Contacts whose name or address starts with the query's words.
    Contacts {
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// Move threads to Snoozed until a time, as the desktop's snooze menu does.
    Snooze {
        #[arg(required = true, num_args = 1.., value_name = "THREAD_ID")]
        threads: Vec<String>,
        /// 30m, 2h, 3d, 1w, tomorrow (9:00), a date (9:00 that day) or a date and time
        /// (2026-11-02T14:30, local unless it has an offset).
        #[arg(long, value_name = "WHEN")]
        until: String,
    },
    /// End a snooze now: the threads go back to the inbox.
    Unsnooze {
        #[arg(required = true, num_args = 1.., value_name = "THREAD_ID")]
        threads: Vec<String>,
    },
    /// Unsubscribe from the list a message came from, by its List-Unsubscribe header:
    /// one-click when it offers it, else by sending the unsubscribe mail, else print the web
    /// link to open.
    Unsubscribe {
        #[arg(value_name = "MESSAGE_ID")]
        message: String,
        /// Use this way only.
        #[arg(long, value_parser = ["one-click", "mailto", "web"])]
        method: Option<String>,
    },
    /// Print an account's engine log (no engine needed).
    Logs {
        /// Account id or email address. Defaults to `--account`, else the only account with
        /// a log.
        account: Option<String>,
        /// Keep printing lines as they are written, across log rotation.
        #[arg(long, short = 'f')]
        follow: bool,
        /// How many of the last lines to print first.
        #[arg(long, short = 'n', default_value_t = 20)]
        lines: usize,
    },
    /// Read and change the settings the engine keeps for every client (undo send, the default
    /// sending account, signatures, ...).
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// AI summaries, triage and token usage, through the provider set up in Tern's
    /// Preferences > AI.
    Ai {
        #[command(subcommand)]
        command: AiCommand,
    },
    /// Install and remove service plugins: a plugin's background program, run by launchd
    /// (macOS) or systemd --user (Linux) after the engine, also while the desktop is closed.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Run the engine as a user unit (`serve --detached`), for service plugins and headless
    /// use.
    Engine {
        #[command(subcommand)]
        command: EngineCommand,
    },
    /// Send one raw JSON-RPC request and print its result.
    Rpc {
        /// e.g. engine.status, threads.list.
        method: String,
        /// The params object; `accountId` picks the engine when several run.
        params: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AccountCommand {
    /// Add an account. IMAP accounts (the default) get the servers Tern finds for the
    /// address, changed by the flags below, and a password: asked for on a terminal, or one
    /// line on stdin with --password-stdin. Gmail, Office 365 and Outlook accounts sign in
    /// in the browser. The engine tests the account before it keeps it.
    Add(AccountAddArgs),
}

#[derive(Debug, Args)]
pub struct AccountAddArgs {
    /// The account's email address.
    pub email: String,
    /// imap, gmail, office365, outlook, or a preset the servers come from (yahoo, icloud,
    /// fastmail, ...).
    #[arg(long, default_value = "imap")]
    pub provider: String,
    /// Your name, as recipients see it.
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long, value_name = "HOST")]
    pub imap_host: Option<String>,
    #[arg(long, value_name = "PORT")]
    pub imap_port: Option<u16>,
    /// tls or starttls; Tern never sends a password unencrypted.
    #[arg(long, value_parser = ["tls", "starttls"])]
    pub imap_security: Option<String>,
    #[arg(long, value_name = "USER")]
    pub imap_username: Option<String>,
    #[arg(long, value_name = "HOST")]
    pub smtp_host: Option<String>,
    #[arg(long, value_name = "PORT")]
    pub smtp_port: Option<u16>,
    /// tls or starttls.
    #[arg(long, value_parser = ["tls", "starttls"])]
    pub smtp_security: Option<String>,
    #[arg(long, value_name = "USER")]
    pub smtp_username: Option<String>,
    /// Read the password (IMAP and SMTP) from the first line of stdin.
    #[arg(long)]
    pub password_stdin: bool,
    /// Office 365: open this shared mailbox with your own sign-in.
    #[arg(long, value_name = "EMAIL")]
    pub shared_mailbox: Option<String>,
    /// Print the sign-in address instead of opening the browser.
    #[arg(long)]
    pub no_browser: bool,
    /// The port on 127.0.0.1 the browser's sign-in comes back to.
    #[arg(long, default_value_t = 12141)]
    pub port: u16,
    /// How many seconds to wait for the sign-in.
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
}

impl AccountAddArgs {
    pub fn has_server_flags(&self) -> bool {
        self.imap_host.is_some()
            || self.imap_port.is_some()
            || self.imap_security.is_some()
            || self.imap_username.is_some()
            || self.smtp_host.is_some()
            || self.smtp_port.is_some()
            || self.smtp_security.is_some()
            || self.smtp_username.is_some()
    }
}

/// Where units go. Units are loaded only in the service manager's own directory.
#[derive(Debug, Args)]
pub struct UnitArgs {
    /// Write the units here instead of ~/Library/LaunchAgents or ~/.config/systemd/user, and
    /// load nothing.
    #[arg(long, value_name = "DIR", env = "TERN_UNIT_DIR")]
    pub unit_dir: Option<PathBuf>,
    /// Write the units but don't load or start them.
    #[arg(long)]
    pub no_load: bool,
    /// The unit format; with --unit-dir only (tests).
    #[arg(long, hide = true, requires = "unit_dir", value_parser = ["launchd", "systemd"])]
    pub manager: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Copy the plugin's service binary (tern.service in its package.json) into the profile
    /// and install a unit that runs it. The engine unit is installed first when it is
    /// missing.
    Install {
        /// The plugin's name, as installed in Tern.
        package: String,
        /// The plugin's directory, instead of looking it up in the profile.
        #[arg(long, value_name = "DIR")]
        package_dir: Option<PathBuf>,
        /// The engine binary for the engine unit (default: mailsync-rs or mailsync beside
        /// tern).
        #[arg(long, value_name = "PATH")]
        engine_bin: Option<PathBuf>,
        /// For the engine unit: keep the engine's secrets in a file sealed with this key
        /// (TERN_ENGINE_KEY_FILE), as the desktop's own engine does in a dev run.
        #[arg(long, value_name = "PATH")]
        engine_key_file: Option<PathBuf>,
        /// Refuse unless the service binary has this SHA-256 (the one a dry run showed).
        #[arg(long, value_name = "HEX")]
        expect_sha256: Option<String>,
        /// Print the binary, its hash and the units, and write nothing.
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        units: UnitArgs,
    },
    /// Stop the plugin's service and remove its unit and binary copy.
    Uninstall {
        package: String,
        #[command(flatten)]
        units: UnitArgs,
    },
    /// The installed services: source, binary, hash and unit.
    List,
}

#[derive(Debug, Subcommand)]
pub enum EngineCommand {
    /// Install a unit that runs the engine detached, started at login.
    Install {
        /// The engine binary (default: mailsync-rs or mailsync beside tern).
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
        /// Keep the engine's secrets in a file sealed with this key instead of the OS
        /// keychain (TERN_ENGINE_KEY_FILE).
        #[arg(long, value_name = "PATH")]
        key_file: Option<PathBuf>,
        /// Print the unit and write nothing.
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        units: UnitArgs,
    },
    /// Stop the engine unit and remove it. Refused while services are installed.
    Uninstall {
        #[command(flatten)]
        units: UnitArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum SettingsCommand {
    /// The stored values, of these keys or of every key. A key without a value is left out:
    /// each client uses its own default.
    Get {
        #[arg(value_name = "KEY")]
        keys: Vec<String>,
    },
    /// Store a value: JSON (`15000`, `true`, `{...}`), else the text as a string. `null`
    /// clears the key.
    Set {
        key: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AiCommand {
    /// Summarize a thread, printing the text as the model writes it.
    Summarize {
        #[arg(value_name = "THREAD_ID")]
        thread: String,
        /// Ask the model again instead of printing the cached summary.
        #[arg(long)]
        refresh: bool,
    },
    /// Triage inbox threads (needs-reply, important, ...) and wait until they are done. The
    /// engine finishes the job if this is interrupted.
    Triage {
        /// Inbox threads from this far back: 7d, 2w, or days (default 7d).
        #[arg(long, value_name = "AGE", conflicts_with = "threads")]
        since: Option<String>,
        /// These threads instead of the inbox.
        #[arg(value_name = "THREAD_ID")]
        threads: Vec<String>,
    },
    /// Tokens used per day, provider and feature.
    Usage {
        /// How many days back (default 30).
        #[arg(long)]
        days: Option<u32>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CalendarCommand {
    /// Start a calendar pass now.
    Sync,
}

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// The stored tasks: pending ones first, oldest queued first, then finished ones, newest
    /// first.
    List {
        /// Only these statuses: local, remote, complete, cancelled.
        #[arg(long, value_delimiter = ',', value_name = "STATUS")]
        status: Vec<String>,
        /// Only this class, e.g. SendDraftTask.
        #[arg(long, value_name = "CLASS")]
        class: Option<String>,
        /// Only the tasks naming this draft.
        #[arg(long, value_name = "ID")]
        header_message_id: Option<String>,
        /// How many tasks per account (default 50).
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// Queue a task: its JSON, or `-` to read it from stdin. `id`, `aid` and `status` are
    /// filled in when missing.
    Queue {
        #[arg(value_name = "JSON|-")]
        task: String,
        /// Wait until the task completes; exit 1 if it fails.
        #[arg(long)]
        wait: bool,
        /// With --wait, give up after this many seconds.
        #[arg(long, value_name = "SECS", requires = "wait")]
        timeout: Option<u64>,
    },
    /// Ask the engine to cancel a task.
    Cancel { id: String },
    /// Wait until a queued task completes; exit 1 if it fails or there is no such task. A
    /// task that has already finished is reported at once.
    Wait {
        id: String,
        /// Give up after this many seconds.
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
    },
}

#[derive(Debug, Args)]
pub struct ThreadsArgs {
    /// A folder, by path or id.
    #[arg(long, conflicts_with_all = ["label", "role"])]
    pub folder: Option<String>,
    /// A label, by path or id.
    #[arg(long, conflicts_with = "role")]
    pub label: Option<String>,
    /// A role: inbox, sent, drafts, trash, spam, archive, all, important, ...
    #[arg(long)]
    pub role: Option<String>,
    /// How many threads (default 50; with --all, the page size).
    #[arg(long, short = 'n')]
    pub limit: Option<usize>,
    /// Page through every thread.
    #[arg(long, conflicts_with = "before")]
    pub all: bool,
    /// Continue after a page: the `next` of `--json threads` (one account only).
    #[arg(long, value_name = "CURSOR")]
    pub before: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("tern").chain(args.iter().copied()))
    }

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_options_go_anywhere() {
        let cli = parse(&[
            "threads",
            "--json",
            "--account",
            "me@x.test",
            "--role",
            "inbox",
        ])
        .unwrap();
        assert!(cli.json);
        assert_eq!(cli.account.as_deref(), Some("me@x.test"));
        let Command::Threads(t) = cli.command else {
            panic!("threads")
        };
        assert_eq!(t.role.as_deref(), Some("inbox"));
        let cli = parse(&["--config-dir", "/tmp/x", "status"]).unwrap();
        assert_eq!(cli.config_dir, Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn one_category_at_most() {
        let e = parse(&["threads", "--folder", "INBOX", "--label", "x"]).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert_eq!(
            parse(&["threads", "--label", "a", "--role", "inbox"])
                .unwrap_err()
                .exit_code(),
            2
        );
        assert!(parse(&["threads", "--all", "-n", "10", "--role", "inbox"]).is_ok());
    }

    #[test]
    fn settings_shapes() {
        let cli = parse(&["settings", "set", "core.sending.undoSend", "-1"]).unwrap();
        let Command::Settings {
            command: SettingsCommand::Set { key, value },
        } = cli.command
        else {
            panic!("settings set")
        };
        assert_eq!(
            (key.as_str(), value.as_str()),
            ("core.sending.undoSend", "-1")
        );
        assert!(parse(&["settings", "get"]).is_ok());
        assert!(parse(&["settings", "set", "signatures"]).is_err());
    }

    #[test]
    fn ai_shapes() {
        let cli = parse(&["ai", "summarize", "t1", "--refresh"]).unwrap();
        let Command::Ai {
            command: AiCommand::Summarize { thread, refresh },
        } = cli.command
        else {
            panic!("ai summarize")
        };
        assert_eq!((thread.as_str(), refresh), ("t1", true));
        let cli = parse(&["ai", "triage", "--since", "2w"]).unwrap();
        let Command::Ai {
            command: AiCommand::Triage { since, threads },
        } = cli.command
        else {
            panic!("ai triage")
        };
        assert_eq!(since.as_deref(), Some("2w"));
        assert!(threads.is_empty());
        assert!(parse(&["ai", "triage", "t1", "t2"]).is_ok());
        assert!(parse(&["ai", "triage", "--since", "3d", "t1"]).is_err());
        assert!(parse(&["ai", "summarize"]).is_err());
        assert!(parse(&["ai", "usage", "--days", "7"]).is_ok());
    }

    #[test]
    fn classes_split_on_commas_and_repeat() {
        let cli = parse(&["watch", "--class", "Thread,Message", "-c", "Task"]).unwrap();
        let Command::Watch { classes, .. } = cli.command else {
            panic!("watch")
        };
        assert_eq!(classes, ["Thread", "Message", "Task"]);
        assert!(parse(&["watch", "--new-mail"]).is_ok());
        assert!(parse(&["watch", "--new-mail", "-c", "Task"]).is_err());
    }

    #[test]
    fn task_and_search_shapes() {
        let cli = parse(&["task", "queue", "-", "--wait", "--timeout", "30"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Task {
                command: TaskCommand::Queue {
                    wait: true,
                    timeout: Some(30),
                    ..
                }
            }
        ));
        assert_eq!(
            parse(&["task", "queue", "{}", "--timeout", "3"])
                .unwrap_err()
                .exit_code(),
            2
        );
        let cli = parse(&["search", "quarterly", "report", "-n", "5"]).unwrap();
        let Command::Search { query, limit } = cli.command else {
            panic!("search")
        };
        assert_eq!(
            (query.join(" "), limit),
            ("quarterly report".into(), Some(5))
        );
        assert_eq!(parse(&["search"]).unwrap_err().exit_code(), 2);
        assert_eq!(parse(&["calendar"]).unwrap_err().exit_code(), 2);
        assert!(parse(&["logs", "acc", "-f"]).is_ok());
        assert!(parse(&["-a", "acc", "logs", "-n", "3"]).is_ok());
        assert!(parse(&["rpc", "engine.status"]).is_ok());
        let cli = parse(&[
            "service",
            "install",
            "sample",
            "--dry-run",
            "--unit-dir",
            "/tmp/u",
            "--manager",
            "systemd",
        ])
        .unwrap();
        let Command::Service {
            command: ServiceCommand::Install { dry_run, units, .. },
        } = cli.command
        else {
            panic!("service install")
        };
        assert!(dry_run && units.manager.as_deref() == Some("systemd"));
        assert!(parse(&["service", "install", "x", "--manager", "launchd"]).is_err());
        assert!(parse(&["service", "uninstall", "x", "--no-load"]).is_ok());
        assert!(parse(&["engine", "install", "--bin", "/x", "--dry-run"]).is_ok());
        let cli = parse(&["snooze", "t1", "t2", "--until", "2h"]).unwrap();
        let Command::Snooze { threads, until } = cli.command else {
            panic!("snooze")
        };
        assert_eq!((threads.len(), until.as_str()), (2, "2h"));
        assert_eq!(parse(&["snooze", "t1"]).unwrap_err().exit_code(), 2);
        assert!(parse(&["unsnooze", "t1"]).is_ok());
        assert!(parse(&["unsubscribe", "m1", "--method", "mailto"]).is_ok());
        assert_eq!(
            parse(&["unsubscribe", "m1", "--method", "body-link"])
                .unwrap_err()
                .exit_code(),
            2
        );
        let cli = parse(&["tasks", "list", "--status", "local,remote"]).unwrap();
        let Command::Task {
            command: TaskCommand::List { status, .. },
        } = cli.command
        else {
            panic!("tasks list")
        };
        assert_eq!(status, ["local", "remote"]);
    }
}
