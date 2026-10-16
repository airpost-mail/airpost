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
    /// Print deltas as the engines send them, until interrupted.
    Watch {
        /// Only these model classes (Thread, Message, Folder, Task, ...).
        #[arg(
            long = "class",
            short = 'c',
            value_delimiter = ',',
            value_name = "CLASS"
        )]
        classes: Vec<String>,
    },
    /// Start a sync pass now.
    Sync,
    /// Calendar commands.
    Calendar {
        #[command(subcommand)]
        command: CalendarCommand,
    },
    /// Queue, cancel and wait for tasks.
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
    /// Threads matching every word of the query.
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
    /// Send one raw JSON-RPC request and print its result.
    Rpc {
        /// e.g. engine.status, threads.list.
        method: String,
        /// The params object; `accountId` picks the engine when several run.
        params: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CalendarCommand {
    /// Start a calendar pass now.
    Sync,
}

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
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
    fn classes_split_on_commas_and_repeat() {
        let cli = parse(&["watch", "--class", "Thread,Message", "-c", "Task"]).unwrap();
        let Command::Watch { classes } = cli.command else {
            panic!("watch")
        };
        assert_eq!(classes, ["Thread", "Message", "Task"]);
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
    }
}
