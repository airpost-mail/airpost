// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The command line.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "airpost",
    version,
    about = "Talk to the running Airpost mail engines",
    long_about = "Talk to the running Airpost mail engines: read, search and draft mail, send \
                  drafts, archive, move, label and mark threads, manage folders and labels, and \
                  run Airpost's services. airpost acts with the user's full rights; the \
                  access level and hidden accounts and folders in Airpost's MCP settings apply to \
                  airpost-mcp only.",
    after_help = "Ids: `threads` and `search` print thread ids, `show <thread-id>` message \
                  ids, `show <message-id>` file ids, `draft list` draft ids; --json prints \
                  every field. `airpost <command> \
                  --help` explains a command.\n\n\
                  Exit codes: 0 ok, 1 command failed, 2 usage, 3 no engine running."
)]
pub struct Cli {
    /// The Airpost profile directory (default: the desktop app's profile for this OS).
    #[arg(long, global = true, env = "AIRPOST_CONFIG_DIR", value_name = "DIR")]
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

// TODO(cli-parity): what the desktop does that airpost doesn't yet, all served by the engine:
// - Contacts: delete, and contact groups (`contacts.*`).
// - Integrations: translate, detect, grammar, enrich (`integrations.*`), and setting up
//   integration and AI provider keys (`secrets.set`, `integrations.test`, `ai.test`,
//   `ai.models`).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// The running engines and the state of their accounts.
    Status,
    /// The accounts the running engines hold.
    Accounts,
    /// Add, change, reorder, resync, sign in again to and remove accounts of the serve engine.
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
    /// The calendars: id (for `calendar create --calendar` and `calendar list --calendar`),
    /// account, name and whether it is read-only.
    Calendars,
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
    /// A message (or, given a thread id, the thread and its messages without bodies).
    Show {
        /// Message id, or a thread id.
        id: String,
        /// With a message id: include the body, downloading it first if needed.
        #[arg(long)]
        body: bool,
    },
    /// Threads matching a query in the search bar's grammar, newest first.
    ///
    /// The grammar: words (each a word prefix), "phrases", from:, to:, subject:, in: (inbox,
    /// sent, archive, trash, spam, drafts, or a folder or label name),
    /// is:unread|read|starred|unstarred, has:attachment, after: or since: (includes the day)
    /// and before: (excludes it) with dates (2026-03-01, today, yesterday, "3 days ago", 2w, "last month"), AND, OR,
    /// NOT and parentheses. An engine without the grammar matches every word instead.
    #[command(
        after_help = "Quote the whole query so the shell keeps its quotes and parentheses: \
                      airpost search 'from:alice \"project alpha\" after:2w'. There is no label:, \
                      cc:, -term or larger:; use in:<label> and NOT."
    )]
    Search {
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// How many threads, newest first (default 50).
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// Contacts whose name or address starts with the query's words.
    Contacts {
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// How many contacts (default 50).
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// Read, create, change, import and export contacts.
    Contact {
        #[command(subcommand)]
        command: ContactCommand,
    },
    /// Write, change, check, send and list drafts, attachments included.
    ///
    /// Drafts start as the desktop's composer starts them (From, signature, quoted text,
    /// reply recipients). Needs the serve engine (Airpost's own, or `airpost engine install`).
    #[command(
        after_help = "A draft is saved to the account and shows in Airpost; nothing goes out until \
                      `draft send`. Write it (new, reply, reply-all, forward), change it in place \
                      (edit, with --attach and --detach), check it (validate), then send it \
                      (send --wait), later (send --at) or with an undo window (send --undo). \
                      --remind-in, --track-opens and --track-links work on every command that \
                      writes or sends. Drafts take a draft id from `draft list` or the id these \
                      commands print, or a Message-ID."
    )]
    Draft {
        #[command(subcommand)]
        command: DraftCommand,
    },
    /// Archive threads, as Airpost's Archive does: Gmail takes them out of the inbox, other
    /// accounts move them to Archive (or All Mail).
    #[command(after_help = THREAD_ACTION_HELP)]
    Archive(ThreadSelect),
    /// Move threads to the trash.
    #[command(after_help = THREAD_ACTION_HELP)]
    Trash(ThreadSelect),
    /// Move threads to a folder. On Gmail the target can also be a label: the threads' labels
    /// are replaced by it, as Airpost's Move to does.
    #[command(after_help = THREAD_ACTION_HELP)]
    Move {
        /// The folder, by path, id or role (inbox, archive, trash, spam, ...).
        #[arg(value_name = "FOLDER")]
        folder: String,
        #[command(flatten)]
        select: ThreadSelect,
    },
    /// Mark threads read.
    #[command(after_help = THREAD_ACTION_HELP)]
    MarkRead(ThreadSelect),
    /// Mark threads unread.
    #[command(after_help = THREAD_ACTION_HELP)]
    MarkUnread(ThreadSelect),
    /// Star threads.
    #[command(after_help = THREAD_ACTION_HELP)]
    Star(ThreadSelect),
    /// Unstar threads.
    #[command(after_help = THREAD_ACTION_HELP)]
    Unstar(ThreadSelect),
    /// Move threads to the Spam folder, as Airpost's Mark as Spam does.
    #[command(after_help = THREAD_ACTION_HELP)]
    Spam(ThreadSelect),
    /// Move threads out of Spam: to the inbox, or to All Mail on Gmail, as Airpost's Not Spam
    /// does. Only their copies in Spam move.
    #[command(after_help = THREAD_ACTION_HELP)]
    NotSpam(ThreadSelect),
    /// Add labels to threads and take them off (Gmail), and create, rename and delete labels.
    Label {
        #[command(subcommand)]
        command: LabelCommand,
    },
    /// Create, rename and delete folders (accounts other than Gmail).
    Folder {
        #[command(subcommand)]
        command: FolderCommand,
    },
    /// List, show, save and delete the composer's templates (Airpost's Preferences > Templates).
    ///
    /// `airpost draft new|reply|reply-all|forward --template NAME` writes a draft from one.
    #[command(visible_alias = "templates")]
    Template {
        #[command(subcommand)]
        command: TemplateCommand,
    },
    /// Mail rules (Airpost's Preferences > Mail Rules): list, add, change, turn on and off,
    /// remove, and run them over the inbox. The engine applies them to arriving mail, with or
    /// without Airpost open.
    #[command(visible_alias = "rule", after_help = RULES_HELP)]
    Rules {
        #[command(subcommand)]
        command: RulesCommand,
    },
    /// The composer's signatures (Airpost's Preferences > Signatures) and which one each
    /// address starts with.
    #[command(visible_alias = "signatures")]
    Signature {
        #[command(subcommand)]
        command: SignatureCommand,
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
    /// Undo airpost's last change to threads (archive, trash, move, labels, read, star, spam,
    /// snooze), from the engine's undo log.
    ///
    /// Only airpost's own changes unless --any: Cmd-Z in Airpost and an agent's undo keep to theirs
    /// too. The log is in memory: it holds the last 100 changes of the last 24 hours and is
    /// gone when the engine restarts.
    Undo {
        #[command(flatten)]
        args: UndoArgs,
        /// Show the log instead, newest first: every client's changes, with their tokens.
        #[arg(long, conflicts_with_all = ["token", "any", "wait"])]
        list: bool,
        /// With --list, airpost's own changes only.
        #[arg(long, requires = "list")]
        mine: bool,
        /// With --list, at most this many.
        #[arg(long, short = 'n', requires = "list")]
        limit: Option<usize>,
    },
    /// Make the change `airpost undo` took back again.
    Redo(UndoArgs),
    /// Unsubscribe from the mailing list a message came from.
    ///
    /// By its List-Unsubscribe header: one-click when it offers it, else by sending the
    /// unsubscribe mail, else print the web link to open.
    Unsubscribe {
        #[arg(value_name = "MESSAGE_ID")]
        message: String,
        /// Use this way only.
        #[arg(long, value_parser = ["one-click", "mailto", "web"])]
        method: Option<String>,
    },
    /// Print an account's engine log, or a service's: engine, gateway or mcp-http (no engine
    /// needed).
    Logs {
        /// Account id or email address, or engine, gateway or mcp-http. Defaults to
        /// `--account`, else the only account with a log.
        #[arg(value_name = "ACCOUNT|SERVICE")]
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
    /// AI summaries, triage and token usage, through the provider set up in Airpost's
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
    /// Run the engine as a user unit (`serve --detached`), started at login, for service
    /// plugins, agents and headless use.
    Engine {
        #[command(subcommand)]
        command: EngineCommand,
    },
    /// Run airpost-gateway, the web client's server, as a user unit started on the first
    /// connection.
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
    /// Install the agent skill for airpost into Claude Code and Codex, or print it.
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    /// Run `airpost-mcp http` as a user unit started on the first connection.
    #[command(name = "mcp-http")]
    McpHttp {
        #[command(subcommand)]
        command: McpHttpCommand,
    },
    /// Remove every unit airpost installed for the profile (service plugins, mcp-http, the
    /// gateway, the engine) and the `airpost` link the desktop put on the PATH.
    Uninstall {
        /// Also delete the profile: the mail cache, accounts and settings.
        #[arg(long)]
        purge_profile: bool,
        /// Don't ask before deleting the profile.
        #[arg(long, short = 'y', requires = "purge_profile")]
        yes: bool,
        /// Look for the desktop's link here instead (tests).
        #[arg(long = "link-path", hide = true, value_name = "PATH")]
        link_paths: Vec<PathBuf>,
        #[command(flatten)]
        units: UnitArgs,
    },
    /// Remind the user about a thread when no one has replied by a time, as the desktop's
    /// send reminders do: the thread comes back to the top of the inbox then. Given a draft,
    /// the reminder goes with it when it is sent.
    Remind {
        /// A thread id, a sent message's id (its thread), or a draft's id.
        #[arg(value_name = "THREAD_ID|MESSAGE_ID|DRAFT_ID")]
        id: String,
        /// When: 30m, 2h, 3d, 1w, tomorrow, tomorrow 14:30, a date or a date and time.
        #[arg(long = "in", value_name = "WHEN", required_unless_present = "clear")]
        when: Option<String>,
        /// Remove the reminder.
        #[arg(long, conflicts_with = "when")]
        clear: bool,
    },
    /// Opens and link clicks of sent mail that had tracking on (`draft ... --track-opens`).
    #[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
    Tracking {
        #[command(subcommand)]
        command: Option<TrackingCommand>,
        /// A sent message's id: its opens and clicks.
        #[arg(value_name = "MESSAGE_ID", required = true)]
        message: Option<String>,
    },
    /// Save attachments to files.
    Attachment {
        #[command(subcommand)]
        command: AttachmentCommand,
    },
    /// Save a message as an .eml file, or a folder as an mbox file, from the server's copy.
    #[command(after_help = "airpost export <message-id> -o msg.eml\n\
                      airpost export --mbox --folder INBOX -a me@example.com -o inbox.mbox\n\n\
                      The mbox is mboxrd, as Airpost's Export folder as mbox writes it.")]
    Export {
        /// The message (`airpost show <thread-id>` lists them).
        #[arg(
            value_name = "MESSAGE_ID",
            required_unless_present = "mbox",
            conflicts_with = "mbox"
        )]
        message: Option<String>,
        /// Export the folder --folder of the account as one mbox file.
        #[arg(long, requires = "folder")]
        mbox: bool,
        /// With --mbox: the folder, by path, id or role (inbox, sent, ...).
        #[arg(long, value_name = "FOLDER", requires = "mbox")]
        folder: Option<String>,
        /// The file to write.
        #[arg(long, short = 'o', value_name = "FILE")]
        output: PathBuf,
        /// Replace the file if it exists.
        #[arg(long)]
        force: bool,
        /// Give up after this many seconds (default: 120 for a message, none for a folder).
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
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
    /// Add an account. IMAP accounts (the default) get the servers Airpost finds for the
    /// address, changed by the flags below, and a password: asked for on a terminal, or one
    /// line on stdin with --password-stdin. Gmail, Office 365 and Outlook accounts sign in
    /// in the browser (on a server: with --paste). The engine tests the account before it
    /// keeps it.
    Add(AccountAddArgs),
    /// Stop an account and remove it from the engine with its stored credentials (and a
    /// Google grant). Airpost's window keeps listing it until it is removed there too.
    Remove {
        /// The account, by id or email address.
        #[arg(value_name = "ID|EMAIL")]
        account: String,
        /// Also delete its mail cache (the downloaded mail, its plugin data). Without it the
        /// cache stays for an account added again.
        #[arg(long)]
        delete_data: bool,
        /// Don't ask first. Needed without a terminal, and with --json.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Change how an account shows: its name, aliases and the alias new mail starts from.
    Update(AccountUpdateArgs),
    /// Put accounts in this order (the engine's account list, `airpost accounts`); accounts not
    /// named keep their order after these. Airpost's window keeps its own order.
    Reorder {
        #[arg(required = true, num_args = 1.., value_name = "ID|EMAIL")]
        accounts: Vec<String>,
    },
    /// Delete an account's mail cache and download its mail again from the server, as Airpost's
    /// Preferences > Accounts > Rebuild this account does.
    Resync {
        #[arg(value_name = "ID|EMAIL")]
        account: String,
        /// Don't ask first. Needed without a terminal, and with --json.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Sign in to an account again: the browser for Gmail, Office 365 and Outlook, a new
    /// password for the others. The engine tests the new credentials before it keeps them.
    Reauth {
        #[arg(value_name = "ID|EMAIL")]
        account: String,
        /// Read the new password (IMAP and SMTP) from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
        #[command(flatten)]
        sign_in: SignInArgs,
    },
}

#[derive(Debug, Args)]
pub struct AccountUpdateArgs {
    #[arg(value_name = "ID|EMAIL")]
    pub account: String,
    /// Your name, as recipients see it.
    #[arg(long)]
    pub name: Option<String>,
    /// Add an alias to send from: `Name <address>`, or an address (your name), or a name
    /// (your address). Repeat for more.
    #[arg(long = "alias", value_name = "ALIAS")]
    pub add_aliases: Vec<String>,
    /// Remove an alias, by its text or its address. Repeat for more.
    #[arg(long = "remove-alias", value_name = "ALIAS")]
    pub remove_aliases: Vec<String>,
    /// The alias new mail starts from, by its text or address; `none` for the account's own
    /// address.
    #[arg(long, value_name = "ALIAS|none")]
    pub default_alias: Option<String>,
}

/// How a browser sign-in reaches the engine.
#[derive(Debug, Args)]
pub struct SignInArgs {
    /// Print the sign-in address instead of opening the browser.
    #[arg(long)]
    pub no_browser: bool,
    /// For a server without a browser: print the sign-in address, then read the address the
    /// browser ends on (http://127.0.0.1:<port>/?code=...) from stdin instead of listening
    /// for it.
    #[arg(long)]
    pub paste: bool,
    /// The port on 127.0.0.1 the browser's sign-in comes back to.
    #[arg(long, default_value_t = 12141)]
    pub port: u16,
    /// How many seconds to wait for the sign-in.
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
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
    /// tls or starttls; Airpost never sends a password unencrypted.
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
    #[command(flatten)]
    pub sign_in: SignInArgs,
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
    #[arg(long, value_name = "DIR", env = "AIRPOST_UNIT_DIR")]
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
    /// Copy the plugin's service binary (airpost.service in its package.json) into the profile
    /// and install a unit that runs it. The engine unit is installed first when it is
    /// missing.
    Install {
        /// The plugin's name, as installed in Airpost.
        package: String,
        /// The plugin's directory, instead of looking it up in the profile.
        #[arg(long, value_name = "DIR")]
        package_dir: Option<PathBuf>,
        /// The engine binary for the engine unit (default: airpost-engine beside
        /// airpost).
        #[arg(long, value_name = "PATH")]
        engine_bin: Option<PathBuf>,
        /// For the engine unit: keep the engine's secrets in a file sealed with this key
        /// (AIRPOST_ENGINE_KEY_FILE), as the desktop's own engine does in a dev run.
        #[arg(long, value_name = "PATH")]
        engine_key_file: Option<PathBuf>,
        /// Refuse unless the service binary has this SHA-256 (the one a dry run showed).
        #[arg(long, value_name = "HEX")]
        expect_sha256: Option<String>,
        /// Refuse unless the install would write exactly what a dry run showed (its
        /// planSha256: the binary's hash and both units).
        #[arg(long, value_name = "HEX")]
        expect_plan: Option<String>,
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
pub enum SkillCommand {
    /// Write the skill into ~/.claude/skills/airpost and ~/.agents/skills/airpost (Codex), replacing
    /// a copy an earlier airpost installed. The copy carries this airpost's version.
    Install {
        #[command(flatten)]
        place: SkillPlace,
        /// Replace a skill directory that airpost didn't install.
        #[arg(long)]
        force: bool,
    },
    /// Remove the skill airpost installed.
    Uninstall {
        #[command(flatten)]
        place: SkillPlace,
        /// Remove the directory even if airpost didn't install it.
        #[arg(long)]
        force: bool,
    },
    /// Print the skill's SKILL.md.
    Print,
}

#[derive(Debug, Args)]
pub struct SkillPlace {
    /// Into ./.claude/skills and ./.agents/skills instead of the home directory.
    #[arg(long)]
    pub project: bool,
    /// Which agents: claude (Claude Code), codex, or all.
    #[arg(long, default_value = "all", value_parser = ["claude", "codex", "all"])]
    pub agent: String,
}

/// What every `airpost <engine|gateway|mcp-http> install` takes.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Take over a unit that runs another copy of Airpost's binaries (the desktop's, or a
    /// package manager's).
    #[arg(long)]
    pub replace: bool,
    /// Enable the unit for login but don't start it now.
    #[arg(long)]
    pub no_start: bool,
    /// Print the units and write nothing.
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub units: UnitArgs,
}

/// Start, stop, restart or remove an installed unit.
#[derive(Debug, Subcommand)]
pub enum UnitCommand {
    /// Stop the unit and remove it.
    Uninstall {
        #[command(flatten)]
        units: UnitArgs,
    },
    /// Start the installed unit's program if it isn't running.
    Start {
        #[command(flatten)]
        units: UnitArgs,
    },
    /// Stop the unit's program until the next login or `start`.
    Stop {
        #[command(flatten)]
        units: UnitArgs,
    },
    /// Restart the unit's program, so it runs the binary now at its path.
    Restart {
        #[command(flatten)]
        units: UnitArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum EngineCommand {
    /// Install a unit that runs the engine detached, started at login. With the desktop's own
    /// engine running it is only enabled, as with --no-start.
    Install {
        /// The engine binary (default: airpost-engine beside airpost, or AIRPOST_ENGINE_BIN).
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
        /// Keep the engine's secrets in a file sealed with this key instead of the OS
        /// keychain (AIRPOST_ENGINE_KEY_FILE).
        #[arg(long, value_name = "PATH")]
        key_file: Option<PathBuf>,
        #[command(flatten)]
        install: InstallArgs,
    },
    #[command(flatten)]
    Unit(UnitCommand),
}

#[derive(Debug, Subcommand)]
pub enum GatewayCommand {
    /// Install a unit that listens on --bind and starts airpost-gateway on the first connection.
    Install {
        /// The gateway binary (default: airpost-gateway beside airpost, or AIRPOST_GATEWAY_BIN).
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
        #[arg(long, value_name = "ADDR:PORT", default_value = crate::builtin::DEFAULT_GATEWAY_BIND)]
        bind: std::net::SocketAddr,
        /// The built web client (default: AIRPOST_GATEWAY_WEB_ROOT, else
        /// ../share/airpost-gateway/web or airpost-web beside the gateway binary); empty means
        /// that default.
        #[arg(long, value_name = "DIR")]
        web_root: Option<String>,
        /// Where browsers reach the gateway when it isn't the bind address, e.g. behind a
        /// TLS proxy.
        #[arg(long, value_name = "URL")]
        public_url: Option<String>,
        #[command(flatten)]
        install: InstallArgs,
    },
    #[command(flatten)]
    Unit(UnitCommand),
    /// Print a one-time code that pairs a browser with the gateway (`airpost-gateway pair`).
    Pair {
        /// Grant the admin scope.
        #[arg(long)]
        admin: bool,
        /// The device's name; otherwise the browser's.
        #[arg(long)]
        name: Option<String>,
        /// How long the code stays valid: 90s, 10m, 1h (at most 1h).
        #[arg(long)]
        ttl: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum McpHttpCommand {
    /// Install a unit that listens on 127.0.0.1 and starts `airpost-mcp http` on the first
    /// connection. Needs http.enabled in mcp.json.
    Install {
        /// The airpost-mcp binary (default: airpost-mcp beside airpost, or AIRPOST_MCP_BIN).
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
        /// The port (default: http.port in mcp.json).
        #[arg(long)]
        port: Option<u16>,
        #[command(flatten)]
        install: InstallArgs,
    },
    #[command(flatten)]
    Unit(UnitCommand),
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
    /// Events between two times, every occurrence of a repeating one.
    List {
        /// From (30m, 2h, 3d, tomorrow, a date or a date and time); now by default.
        #[arg(long, value_name = "WHEN")]
        from: Option<String>,
        /// To; a week after --from by default.
        #[arg(long, value_name = "WHEN")]
        to: Option<String>,
        /// Only these calendars (ids).
        #[arg(long = "calendar", value_delimiter = ',', value_name = "ID")]
        calendars: Vec<String>,
    },
    /// Create an event; with guests it is organised by the account and they are invited
    /// when the server sends invitations.
    #[command(
        after_help = "`airpost calendars` lists the calendars with their ids and accounts; give \
                      the account with --account when there are several. A read-only calendar \
                      takes no new events."
    )]
    Create {
        /// The calendar's id (`airpost calendars` lists them).
        #[arg(long, value_name = "ID")]
        calendar: String,
        #[arg(long)]
        title: String,
        #[arg(long, value_name = "WHEN")]
        start: String,
        /// An hour after --start by default (a day for --all-day).
        #[arg(long, value_name = "WHEN")]
        end: Option<String>,
        #[arg(long)]
        all_day: bool,
        #[arg(long)]
        location: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Guests' addresses.
        #[arg(long = "guest", value_delimiter = ',', value_name = "EMAIL")]
        guests: Vec<String>,
        /// An RRULE, e.g. FREQ=WEEKLY;BYDAY=MO.
        #[arg(long, value_name = "RULE")]
        repeat: Option<String>,
    },
    /// Change or delete an event, one occurrence of it, or it from one occurrence on.
    Update {
        #[arg(value_name = "EVENT_ID")]
        event: String,
        /// Which occurrences of a repeating event.
        #[arg(long, default_value = "all", value_parser = ["this", "following", "all"])]
        scope: String,
        /// The occurrence --scope this or following starts from, by its start.
        #[arg(long, value_name = "WHEN")]
        occurrence: Option<String>,
        #[arg(long, value_name = "WHEN")]
        start: Option<String>,
        #[arg(long, value_name = "WHEN")]
        end: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        location: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        delete: bool,
    },
    /// Answer an invitation, and tell the organizer by mail; or propose another time.
    Rsvp {
        #[arg(value_name = "EVENT_ID")]
        event: String,
        #[arg(value_parser = ["accepted", "tentative", "declined"])]
        status: String,
        #[arg(long)]
        comment: Option<String>,
        /// Propose this start instead (with --propose-end): a counter-proposal.
        #[arg(long, value_name = "WHEN", requires = "propose_end")]
        propose_start: Option<String>,
        #[arg(long, value_name = "WHEN", requires = "propose_start")]
        propose_end: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct ContactFieldArgs {
    /// Fields as JSON, in the shape `airpost contact get --json` prints under `data`.
    #[arg(long, value_name = "JSON")]
    pub patch: Option<String>,
    /// "Given Family".
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub email: Option<String>,
    #[arg(long)]
    pub phone: Option<String>,
    #[arg(long)]
    pub company: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub notes: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ContactCommand {
    /// A contact's details.
    Get {
        #[arg(value_name = "CONTACT_ID")]
        id: String,
    },
    /// A new contact in the account's address book.
    Create {
        #[command(flatten)]
        fields: ContactFieldArgs,
    },
    /// Change a contact; the fields given replace the stored ones.
    Update {
        #[arg(value_name = "CONTACT_ID")]
        id: String,
        #[command(flatten)]
        fields: ContactFieldArgs,
    },
    /// Add the vCards of a .vcf file (or - for stdin) as new contacts.
    Import {
        #[arg(value_name = "FILE")]
        file: String,
    },
    /// Print contacts as a .vcf file.
    Export {
        #[arg(required = true, num_args = 1.., value_name = "CONTACT_ID")]
        ids: Vec<String>,
    },
}

/// What a draft command writes into the draft.
#[derive(Debug, Args)]
pub struct DraftContentArgs {
    #[command(flatten)]
    pub fields: DraftFieldArgs,
    /// A plain-text draft. Without --html or --plain the draft follows the composer's
    /// setting (core.composing.html).
    #[arg(long, conflicts_with = "html")]
    pub plain: bool,
    /// Write the text from a template (`airpost template list`) as the composer's template
    /// picker does: its text above the signature and quote, and its subject on a new draft
    /// without --subject. Fill its variables with `draft edit` after.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["body", "body_file"])]
    pub template: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// The templates: name, subject, variables and the start of the text.
    List,
    /// A template's subject, variables and text (HTML in --json).
    Show {
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Save a template from text, or from a draft as the composer's "Save as template"
    /// does (the text above the signature and quote, named after the subject).
    Save(TemplateSaveArgs),
    /// Delete a template.
    Delete {
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Debug, Args)]
pub struct TemplateSaveArgs {
    /// Letters, digits, spaces, - and _. A name in use gets a number unless --replace.
    #[arg(value_name = "NAME", required_unless_present = "from_draft")]
    pub name: Option<String>,
    /// The subject a new draft from the template gets.
    #[arg(long)]
    pub subject: Option<String>,
    /// The text, plain unless --html; `{{name}}` becomes a variable.
    #[arg(long, allow_hyphen_values = true, conflicts_with_all = ["body_file", "from_draft"])]
    pub body: Option<String>,
    /// Read the text from a file, or from stdin with -.
    #[arg(long, value_name = "FILE|-", conflicts_with = "from_draft")]
    pub body_file: Option<PathBuf>,
    /// The text is HTML.
    #[arg(long)]
    pub html: bool,
    /// Save this draft (`airpost draft list`) as the template.
    #[arg(long, value_name = "DRAFT_ID")]
    pub from_draft: Option<String>,
    /// Write over the template of that name.
    #[arg(long)]
    pub replace: bool,
}

/// What `airpost draft new|reply|reply-all|forward|edit` set on the draft.
#[derive(Debug, Args)]
pub struct DraftFieldArgs {
    /// Recipients, replacing the draft's (the ones a reply or forward starts with):
    /// addresses or `Name <address>`, comma-separated or repeated.
    #[arg(long, value_name = "ADDRESSES")]
    pub to: Vec<String>,
    /// Cc recipients, as --to.
    #[arg(long, value_name = "ADDRESSES")]
    pub cc: Vec<String>,
    /// Bcc recipients, as --to.
    #[arg(long, value_name = "ADDRESSES")]
    pub bcc: Vec<String>,
    /// The subject, replacing the draft's ("Re:" or "Fwd:" for a reply or forward).
    #[arg(long)]
    pub subject: Option<String>,
    /// The text, written above the signature and the quoted text (replacing the text there
    /// in a draft that has some). Plain text unless --html.
    #[arg(long, allow_hyphen_values = true, conflicts_with = "body_file")]
    pub body: Option<String>,
    /// Read the text from a file, or from stdin with -.
    #[arg(long, value_name = "FILE|-")]
    pub body_file: Option<PathBuf>,
    /// The text is HTML, and a new draft an HTML one.
    #[arg(long)]
    pub html: bool,
    /// Attach a file; repeat for more. A file the draft already has (same name and
    /// contents) isn't attached again.
    #[arg(long, value_name = "PATH")]
    pub attach: Vec<PathBuf>,
    #[command(flatten)]
    pub follow_up: FollowUpArgs,
}

/// The reminder and tracking a draft command turns on.
#[derive(Debug, Args, Default)]
pub struct FollowUpArgs {
    /// Remind the user if no one replies by then (30m, 2h, 3d, 1w, tomorrow, a date): the
    /// thread comes back to the top of the inbox. Set on the draft, it goes with the send.
    #[arg(long, value_name = "WHEN")]
    pub remind_in: Option<String>,
    /// Track opens. Needs an Airpost server that offers tracking, and an HTML draft.
    #[arg(long)]
    pub track_opens: bool,
    /// Track link clicks. Needs an Airpost server that offers tracking, and an HTML draft.
    #[arg(long)]
    pub track_links: bool,
}

/// The reply commands' arguments.
#[derive(Debug, Args)]
pub struct DraftReplyArgs {
    /// The message to answer, or a thread id for its newest message that isn't a draft.
    #[arg(value_name = "MESSAGE_ID|THREAD_ID")]
    pub id: String,
    /// Edit the draft already answering the message, if there is one (the default), so
    /// running the command again revises the reply instead of starting another.
    #[arg(long, overrides_with = "no_prefer_existing")]
    pub prefer_existing: bool,
    /// Always start a new draft.
    #[arg(long, overrides_with = "prefer_existing")]
    pub no_prefer_existing: bool,
    #[command(flatten)]
    pub content: DraftContentArgs,
}

#[derive(Debug, Subcommand)]
pub enum DraftCommand {
    /// A new message, from --account (default: the default sending account).
    New {
        #[command(flatten)]
        content: DraftContentArgs,
    },
    /// A reply to the sender.
    Reply(DraftReplyArgs),
    /// A reply to the sender and every other recipient.
    ReplyAll(DraftReplyArgs),
    /// A forward of a message, with its attachments.
    Forward {
        /// The message, or a thread id for its newest message that isn't a draft.
        #[arg(value_name = "MESSAGE_ID|THREAD_ID")]
        id: String,
        #[command(flatten)]
        content: DraftContentArgs,
    },
    /// Change a draft in place: recipients, subject, the text above the signature and
    /// quote, attachments. Refused while it is being sent or scheduled to send.
    Edit {
        /// The draft's id (`airpost draft list` shows it) or its Message-ID.
        #[arg(value_name = "DRAFT_ID|MESSAGE_ID")]
        id: String,
        #[command(flatten)]
        fields: DraftFieldArgs,
        /// Remove an attachment, by file name or id; repeat for more.
        #[arg(long, value_name = "NAME|ID")]
        detach: Vec<String>,
    },
    /// Run the checks `send` runs, without sending: errors (no recipients, a bad address)
    /// exit 1; warnings (no subject, a forgotten attachment) don't stop a send.
    Validate {
        /// The draft's id or its Message-ID.
        #[arg(value_name = "DRAFT_ID|MESSAGE_ID")]
        id: String,
    },
    /// Delete a draft. A send waiting out its undo window or a schedule is cancelled with
    /// it; one already going out can't be.
    Delete {
        /// The draft's id or its Message-ID.
        #[arg(value_name = "DRAFT_ID|MESSAGE_ID")]
        id: String,
    },
    /// Send a draft now, later (--at), or after an undo window (--undo). Once it goes it
    /// can't be called back. The checks of `validate` run first: errors stop it, warnings
    /// are printed and don't.
    #[command(
        after_help = "--at takes 30m, 2h, 3d, 1w, tomorrow (9:00), tomorrow 14:30, a date (9:00 \
                      that day) or an ISO 8601 date and time (2026-11-02T14:30, local unless it \
                      has an offset). The engine sends it then, with or without Airpost open. \
                      `draft scheduled` lists pending sends; `draft cancel-send` stops one."
    )]
    Send {
        /// The draft's id (`airpost draft list` shows it) or its Message-ID.
        #[arg(value_name = "DRAFT_ID|MESSAGE_ID")]
        id: String,
        /// Archive the thread once it is sent.
        #[arg(long)]
        archive: bool,
        /// Wait until the mail has gone; exit 1 if the send fails.
        #[arg(long, conflicts_with_all = ["at", "undo"])]
        wait: bool,
        /// With --wait, give up after this many seconds.
        #[arg(long, value_name = "SECS", requires = "wait")]
        timeout: Option<u64>,
        /// Send later, at this time, as the desktop's Send Later does.
        #[arg(long, value_name = "WHEN", conflicts_with = "undo")]
        at: Option<String>,
        /// Hold the send this many seconds (at most 3600); `draft cancel-send` calls it back
        /// until then.
        #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u64).range(1..=3600))]
        undo: Option<u64>,
        #[command(flatten)]
        follow_up: FollowUpArgs,
    },
    /// Stop a draft's pending send, one scheduled with --at or waiting out --undo. The draft
    /// stays. A send that has started can't be stopped.
    CancelSend {
        /// The draft's id or its Message-ID.
        #[arg(value_name = "DRAFT_ID|MESSAGE_ID")]
        id: String,
    },
    /// The drafts waiting to send (scheduled, or in an undo window), soonest first, and
    /// scheduled sends that failed.
    Scheduled,
    /// The unsent drafts, newest first.
    List {
        /// How many drafts per account (default 50).
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TrackingCommand {
    /// Sent messages with tracking on, newest first, with their opens and clicks.
    List {
        /// Only messages tracking this.
        #[arg(long = "type", default_value = "either", value_parser = ["opens", "links", "either"])]
        kind: String,
        /// Only mail sent since then: 3h, 7d, 2w, or a date.
        #[arg(long, value_name = "WHEN")]
        since: Option<String>,
        /// How many messages (default 50).
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// The most clicked tracked links.
    Links {
        /// Only mail sent since then: 3h, 7d, 2w, or a date.
        #[arg(long, value_name = "WHEN")]
        since: Option<String>,
        /// How many links (default 10).
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AttachmentCommand {
    /// Save one attachment, downloading it first if needed, and print its path.
    Get {
        /// The file's id (`airpost show <message-id>` lists them).
        #[arg(value_name = "FILE_ID")]
        file: String,
        /// Where to write it (default: its name in the current directory); - for stdout.
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<PathBuf>,
        /// Replace a file that exists.
        #[arg(long)]
        force: bool,
    },
    /// Save every attachment of a message into a directory and print their paths.
    Save {
        #[arg(value_name = "MESSAGE_ID")]
        message: String,
        /// The directory (default: the current one); made if missing.
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Replace files that exist.
        #[arg(long)]
        force: bool,
    },
}

pub const THREAD_ACTION_HELP: &str =
    "Give thread ids (from `search` or `threads`), or --query to act on every thread a search \
     finds (newest first, at most --limit; without --limit a query matching more than 500 \
     threads is refused). Threads already as asked are left alone. --dry-run lists the threads \
     it takes and checks the folder or label, changing nothing. The change shows in Airpost at \
     once and reaches the server in the background; --wait waits for that. `airpost undo` takes \
     it back.";

/// The threads a thread command acts on, and how it waits.
#[derive(Debug, Args)]
pub struct ThreadSelect {
    /// Thread ids, from `search` or `threads`, in any of the accounts.
    #[arg(
        value_name = "THREAD_ID",
        required_unless_present = "query",
        conflicts_with = "query"
    )]
    pub threads: Vec<String>,
    /// Every thread matching this search, in `airpost search`'s grammar, instead of ids.
    #[arg(long, short = 'q', value_name = "QUERY")]
    pub query: Option<String>,
    /// With --query: at most this many threads, newest first.
    #[arg(long, short = 'n')]
    pub limit: Option<usize>,
    /// List the threads it takes and check the folder or label; change nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Wait until the change has reached the server; exit 1 if it fails.
    #[arg(long, conflicts_with = "dry_run")]
    pub wait: bool,
    /// With --wait, give up after this many seconds.
    #[arg(long, value_name = "SECS", requires = "wait")]
    pub timeout: Option<u64>,
}

/// Which change `airpost undo|redo` takes, and how it waits.
#[derive(Debug, Args)]
pub struct UndoArgs {
    /// The change, by the token `airpost undo --list` shows (or a command's `undo` in --json).
    #[arg(value_name = "TOKEN", conflicts_with = "any")]
    pub token: Option<String>,
    /// The latest change of any client (Airpost, an agent, a script), not only airpost's.
    #[arg(long)]
    pub any: bool,
    /// Wait until the change has reached the server; exit 1 if it fails.
    #[arg(long)]
    pub wait: bool,
    /// With --wait, give up after this many seconds.
    #[arg(long, value_name = "SECS", requires = "wait")]
    pub timeout: Option<u64>,
}

/// How a label or folder command waits for its task.
#[derive(Debug, Args)]
pub struct CategoryWait {
    /// Queue the change and return without waiting for the server.
    #[arg(long)]
    pub no_wait: bool,
    /// Give up waiting after this many seconds.
    #[arg(
        long,
        value_name = "SECS",
        default_value_t = 60,
        conflicts_with = "no_wait"
    )]
    pub timeout: u64,
}

#[derive(Debug, Subcommand)]
pub enum LabelCommand {
    /// Add a label to threads. The label must exist: `airpost labels` lists them, `label create`
    /// makes one.
    #[command(after_help = THREAD_ACTION_HELP)]
    Add {
        /// The label, by name (its path) or id.
        #[arg(value_name = "LABEL")]
        label: String,
        #[command(flatten)]
        select: ThreadSelect,
    },
    /// Take a label off threads.
    #[command(after_help = THREAD_ACTION_HELP)]
    Remove {
        /// The label, by name (its path) or id.
        #[arg(value_name = "LABEL")]
        label: String,
        #[command(flatten)]
        select: ThreadSelect,
    },
    /// Create a label on the account (--account, or the only one) and wait until the server
    /// has it. `/` nests it: Work/Clients.
    Create {
        #[arg(value_name = "NAME")]
        name: String,
        #[command(flatten)]
        wait: CategoryWait,
    },
    /// Rename a label. The new name is the whole name, `/` between levels.
    Rename {
        /// The label, by name or id.
        #[arg(value_name = "LABEL")]
        label: String,
        #[arg(value_name = "NEW_NAME")]
        new_name: String,
        #[command(flatten)]
        wait: CategoryWait,
    },
    /// Delete a label. Its mail stays, without the label.
    Delete {
        /// The label, by name or id.
        #[arg(value_name = "LABEL")]
        label: String,
        #[command(flatten)]
        wait: CategoryWait,
    },
}

#[derive(Debug, Subcommand)]
pub enum FolderCommand {
    /// Create a folder on the account (--account, or the only one) and wait until the server
    /// has it. `/` nests it: Projects/2026.
    Create {
        #[arg(value_name = "NAME")]
        name: String,
        #[command(flatten)]
        wait: CategoryWait,
    },
    /// Rename a folder. The new name is the whole name, `/` between levels.
    Rename {
        /// The folder, by path or id.
        #[arg(value_name = "FOLDER")]
        folder: String,
        #[arg(value_name = "NEW_NAME")]
        new_name: String,
        #[command(flatten)]
        wait: CategoryWait,
    },
    /// Delete a folder. The server deletes the mail in it too, so a folder that holds threads
    /// is refused without --force.
    Delete {
        /// The folder, by path or id.
        #[arg(value_name = "FOLDER")]
        folder: String,
        /// Delete it even though it holds threads.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        wait: CategoryWait,
    },
    /// Delete every message in a folder for good, as Airpost's "Empty trash now" does. Asks
    /// first; trash and spam by role empty them in every account without --account.
    Empty {
        /// trash, spam, or a folder without a role, by path or id.
        #[arg(value_name = "trash|spam|FOLDER")]
        folder: String,
        /// Show what would be deleted and delete nothing.
        #[arg(long)]
        dry_run: bool,
        /// Don't ask first. Needed without a terminal, and with --json.
        #[arg(long, short = 'y', conflicts_with = "dry_run")]
        yes: bool,
        /// Wait until the server has deleted them; exit 1 if it fails.
        #[arg(long, conflicts_with = "dry_run")]
        wait: bool,
        /// With --wait, give up after this many seconds.
        #[arg(long, value_name = "SECS", requires = "wait")]
        timeout: Option<u64>,
    },
}

pub const RULES_HELP: &str =
    "A rule has conditions (--if, all of them unless --match any) and actions (--then), each \
     repeatable:\n  \
     airpost rules add --name Receipts --if 'from contains shop.example' --then 'move Receipts'\n\n\
     --if FIELD COMPARATOR VALUE: fields from, to, cc, bcc, recipient, reply-to, \
     attachment-name, subject, body; comparators contains, not-contains, begins-with, \
     ends-with, equals, matches (a regular expression). Also has-attachment and starred, \
     with true (the default) or false.\n\
     --then ACTION: mark-read, star, trash, forward ADDRESS, move FOLDER; on Gmail also label \
     LABEL, move LABEL (replacing the other labels), archive, important.\n\
     --rule JSON (or - for stdin) takes the rule in the shape `rules show --json` prints.\n\n\
     A rule is checked before it is saved: an unknown field, a bad expression or a folder \
     that doesn't exist is refused. Rules act on mail as it arrives; `rules reprocess` runs \
     them over the inbox. A RULE is its id or its name.";

/// What `airpost rules add|update` set on a rule.
#[derive(Debug, Args)]
pub struct RuleArgs {
    /// The rule's name.
    #[arg(long)]
    pub name: Option<String>,
    /// A condition: FIELD COMPARATOR VALUE, e.g. 'subject contains invoice'. Repeat for
    /// more; on `update`, they replace the rule's conditions.
    #[arg(long = "if", value_name = "CONDITION")]
    pub conditions: Vec<String>,
    /// An action: mark-read, star, trash, forward ADDRESS, move FOLDER, label LABEL, archive,
    /// important. Repeat for more; on `update`, they replace the rule's actions.
    #[arg(long = "then", value_name = "ACTION")]
    pub actions: Vec<String>,
    /// Whether every condition must match (all) or one is enough (any).
    #[arg(long = "match", value_parser = ["all", "any"])]
    pub mode: Option<String>,
    /// The rule (on `update`, the fields to change) as JSON, or - to read it from stdin.
    #[arg(long, value_name = "JSON|-", conflicts_with_all = ["conditions", "actions", "mode"])]
    pub rule: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum RulesCommand {
    /// The rules in the order they run: id, name, account, on or off, conditions, actions.
    List,
    /// A rule: its conditions and actions, and why the engine turned it off if it did.
    Show {
        #[arg(value_name = "RULE")]
        rule: String,
    },
    /// Add a rule to the account (--account, or the only one), after the others.
    Add {
        #[command(flatten)]
        args: RuleArgs,
        /// Save it turned off.
        #[arg(long)]
        disabled: bool,
    },
    /// Change a rule's name, conditions, actions or match.
    Update {
        // Its own id: `--rule` in RuleArgs is "rule".
        #[arg(id = "target", value_name = "RULE")]
        rule: String,
        #[command(flatten)]
        args: RuleArgs,
    },
    /// Delete a rule.
    #[command(visible_alias = "delete")]
    Remove {
        #[arg(value_name = "RULE")]
        rule: String,
    },
    /// Turn a rule on again, also one the engine turned off after an error.
    Enable {
        #[arg(value_name = "RULE")]
        rule: String,
    },
    /// Turn a rule off.
    Disable {
        #[arg(value_name = "RULE")]
        rule: String,
    },
    /// Run the rules that are on over the inbox, newest threads first, as Airpost's "Process
    /// entire inbox" does, showing the progress. The engine stops the run if this command
    /// is interrupted.
    Reprocess {
        /// Also wait until the changes the rules made have reached the server; exit 1 if
        /// one fails.
        #[arg(long)]
        wait: bool,
        /// Give up, stopping the run, after this many seconds.
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SignatureCommand {
    /// The signatures: id, name, the addresses that start with each, and the start of the
    /// text.
    List,
    /// A signature's text (HTML in --json) and the addresses that use it.
    Show {
        #[arg(value_name = "NAME|ID")]
        signature: String,
    },
    /// Save a signature: the one with that name or id, else a new one named NAME.
    Set {
        #[arg(value_name = "NAME|ID")]
        signature: String,
        /// Rename it.
        #[arg(long)]
        title: Option<String>,
        /// The text, plain unless --html.
        #[arg(long, allow_hyphen_values = true, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read the text from a file, or from stdin with -.
        #[arg(long, value_name = "FILE|-")]
        body_file: Option<PathBuf>,
        /// The text is HTML.
        #[arg(long)]
        html: bool,
    },
    /// Delete a signature; the addresses that used it start without one.
    Delete {
        #[arg(value_name = "NAME|ID")]
        signature: String,
    },
    /// The signature an address starts new mail and replies with: the account's address
    /// (--account, or the only account), or the addresses --for names (an alias's too).
    Default {
        /// The signature, or `none`.
        #[arg(value_name = "NAME|ID|none")]
        signature: String,
        /// The address: an account's own or one of its aliases'. Repeat for more.
        #[arg(long = "for", value_name = "ADDRESS")]
        addresses: Vec<String>,
    },
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
        Cli::try_parse_from(std::iter::once("airpost").chain(args.iter().copied()))
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
    fn batch_b_shapes() {
        for ok in [
            &["spam", "t1", "t2"][..],
            &["not-spam", "--query", "from:x"],
            &["folder", "empty", "trash", "--dry-run"],
            &["folder", "empty", "spam", "-y", "--wait", "--timeout", "30"],
            &["signature", "set", "Work", "--body", "Ann"],
            &["signatures", "list"],
            &["signature", "default", "Work", "--for", "a@x.test"],
            &["account", "remove", "a@x.test", "--delete-data", "--yes"],
            &["account", "update", "a@x.test", "--alias", "s@x.test"],
            &["account", "reorder", "b@x.test", "a@x.test"],
            &["account", "resync", "a@x.test", "-y"],
            &["account", "reauth", "a@x.test", "--password-stdin"],
            &["rules", "add", "--if", "from contains x", "--then", "star"],
            &["rule", "update", "R", "--match", "any", "--if", "starred"],
            &["rules", "add", "--rule", "-", "--disabled"],
            &["rules", "reprocess", "--wait", "--timeout", "60"],
            &["rules", "delete", "R"],
            &["calendars", "--json"],
        ] {
            assert!(parse(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            &["folder", "empty", "trash", "--dry-run", "--yes"][..],
            &["folder", "empty", "trash", "--timeout", "5"],
            &["rules", "add", "--rule", "{}", "--if", "starred"],
            &["rules", "update", "R", "--match", "some"],
        ] {
            assert_eq!(parse(bad).unwrap_err().exit_code(), 2, "{bad:?}");
        }
        let cli = parse(&["rules", "add", "--if", "a", "--if", "b", "--then", "c"]).unwrap();
        let Command::Rules {
            command: RulesCommand::Add { args, disabled },
        } = cli.command
        else {
            panic!("rules add")
        };
        assert_eq!(
            (args.conditions.len(), args.actions.len(), disabled),
            (2, 1, false)
        );
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
    fn draft_shapes() {
        let cli = parse(&[
            "draft", "reply", "m1", "--body", "-- Ann", "--to", "a@x.test", "--to", "b@x.test",
        ])
        .unwrap();
        let Command::Draft {
            command: DraftCommand::Reply(r),
        } = cli.command
        else {
            panic!("draft reply")
        };
        assert_eq!((r.id.as_str(), r.no_prefer_existing), ("m1", false));
        assert_eq!(r.content.fields.body.as_deref(), Some("-- Ann"));
        assert_eq!(r.content.fields.to, ["a@x.test", "b@x.test"]);
        let cli = parse(&[
            "draft",
            "reply-all",
            "t1",
            "--prefer-existing",
            "--no-prefer-existing",
        ])
        .unwrap();
        let Command::Draft {
            command: DraftCommand::ReplyAll(r),
        } = cli.command
        else {
            panic!("draft reply-all")
        };
        assert!(r.no_prefer_existing && !r.prefer_existing);
        assert!(parse(&["draft", "new", "--subject", "Hi", "--body-file", "-"]).is_ok());
        assert!(parse(&["draft", "new", "--body", "x", "--body-file", "f"]).is_err());
        assert!(parse(&["draft", "new", "--html", "--plain"]).is_err());
        assert!(parse(&["draft", "forward", "m1", "--to", "a@x.test", "--plain"]).is_ok());
        assert!(parse(&["draft", "forward"]).is_err());
        assert!(parse(&[
            "draft",
            "send",
            "d1",
            "--archive",
            "--wait",
            "--timeout",
            "30"
        ])
        .is_ok());
        assert!(parse(&["draft", "send", "d1", "--timeout", "30"]).is_err());
        assert!(parse(&["draft", "list", "-n", "5"]).is_ok());
        let cli = parse(&[
            "draft",
            "edit",
            "d1",
            "--attach",
            "a.pdf",
            "--attach",
            "b.pdf",
            "--detach",
            "old.pdf",
            "--subject",
            "S",
        ])
        .unwrap();
        let Command::Draft {
            command: DraftCommand::Edit { id, fields, detach },
        } = cli.command
        else {
            panic!("draft edit")
        };
        assert_eq!(id, "d1");
        assert_eq!(
            fields.attach,
            [PathBuf::from("a.pdf"), PathBuf::from("b.pdf")]
        );
        assert_eq!(detach, ["old.pdf"]);
        assert!(parse(&["draft", "edit", "d1", "--plain"]).is_err());
        assert!(parse(&["draft", "new", "--attach", "a.pdf", "--html"]).is_ok());
        assert!(parse(&["draft", "delete", "d1"]).is_ok());
        assert!(parse(&["draft", "delete"]).is_err());
        assert!(parse(&["draft", "validate", "d1"]).is_ok());
    }

    #[test]
    fn mailbox_shapes() {
        let cli = parse(&["archive", "t1", "t2", "--wait", "--timeout", "30"]).unwrap();
        let Command::Archive(s) = cli.command else {
            panic!("archive")
        };
        assert_eq!(s.threads, ["t1", "t2"]);
        assert!(s.wait && s.timeout == Some(30) && s.query.is_none());
        let cli = parse(&[
            "label",
            "add",
            "Receipts",
            "--query",
            "from:shop.example after:2026-01-01",
            "-n",
            "2000",
            "--dry-run",
        ])
        .unwrap();
        let Command::Label {
            command: LabelCommand::Add { label, select },
        } = cli.command
        else {
            panic!("label add")
        };
        assert_eq!(label, "Receipts");
        assert_eq!(
            (select.query.as_deref(), select.limit, select.dry_run),
            (Some("from:shop.example after:2026-01-01"), Some(2000), true)
        );
        let cli = parse(&["move", "Projects", "t1"]).unwrap();
        let Command::Move { folder, select } = cli.command else {
            panic!("move")
        };
        assert_eq!((folder.as_str(), select.threads.len()), ("Projects", 1));
        for ok in [
            &["trash", "-q", "in:spam"][..],
            &["mark-read", "t1"],
            &["mark-unread", "t1"],
            &["star", "t1"],
            &["unstar", "--query", "is:starred", "--limit", "5"],
            &["label", "remove", "Work", "t1"],
            &["label", "create", "Work/Clients"],
            &["label", "rename", "Work", "Clients", "--no-wait"],
            &["label", "delete", "Work", "--timeout", "10"],
            &["folder", "create", "Projects/2026"],
            &["folder", "delete", "Projects", "--force"],
            &["folder", "rename", "Projects", "Clients"],
        ] {
            assert!(parse(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            &["archive"][..],
            &["archive", "t1", "--query", "x"],
            &["archive", "t1", "--dry-run", "--wait"],
            &["archive", "t1", "--timeout", "5"],
            &["move", "t1"],
            &["label", "add", "Work"],
            &["label", "create", "X", "--no-wait", "--timeout", "5"],
            &["folder", "delete"],
        ] {
            assert_eq!(parse(bad).unwrap_err().exit_code(), 2, "{bad:?}");
        }
        assert!(parse(&["labels"]).is_ok() && parse(&["folders"]).is_ok());
    }

    #[test]
    fn skill_shapes() {
        let cli = parse(&["skill", "install", "--project", "--agent", "codex"]).unwrap();
        let Command::Skill {
            command: SkillCommand::Install { place, force },
        } = cli.command
        else {
            panic!("skill install")
        };
        assert!(place.project && !force);
        assert_eq!(place.agent, "codex");
        assert!(parse(&["skill", "install", "--agent", "cursor"]).is_err());
        assert!(parse(&["skill", "uninstall", "--force"]).is_ok());
        assert!(parse(&["skill", "print"]).is_ok());
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
    fn undo_shapes() {
        let cli = parse(&["undo"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Undo {
                list: false,
                args: UndoArgs {
                    token: None,
                    any: false,
                    ..
                },
                ..
            }
        ));
        let cli = parse(&["undo", "--any", "--wait", "--timeout", "9"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Undo {
                args: UndoArgs {
                    any: true,
                    wait: true,
                    timeout: Some(9),
                    ..
                },
                ..
            }
        ));
        let cli = parse(&["undo", "--list", "--mine", "-n", "5"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Undo {
                list: true,
                mine: true,
                limit: Some(5),
                ..
            }
        ));
        for bad in [
            &["undo", "tok", "--any"][..],
            &["undo", "--list", "--any"],
            &["undo", "--mine"],
            &["redo", "--timeout", "3"],
        ] {
            assert_eq!(parse(bad).unwrap_err().exit_code(), 2, "{bad:?}");
        }
        let cli = parse(&["redo", "tok"]).unwrap();
        let Command::Redo(a) = cli.command else {
            panic!("redo")
        };
        assert_eq!(a.token.as_deref(), Some("tok"));
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
        assert!(parse(&[
            "calendar",
            "list",
            "--from",
            "tomorrow",
            "--calendar",
            "c1,c2"
        ])
        .is_ok());
        assert!(parse(&[
            "calendar",
            "create",
            "--calendar",
            "c",
            "--title",
            "x",
            "--start",
            "2h"
        ])
        .is_ok());
        assert!(parse(&[
            "calendar",
            "update",
            "e1",
            "--scope",
            "this",
            "--occurrence",
            "2026-11-02T09:00",
            "--delete"
        ])
        .is_ok());
        assert!(parse(&["calendar", "update", "e1", "--scope", "some"]).is_err());
        assert!(parse(&["calendar", "rsvp", "e1", "accepted"]).is_ok());
        assert!(parse(&[
            "calendar",
            "rsvp",
            "e1",
            "tentative",
            "--propose-start",
            "2h"
        ])
        .is_err());
        assert!(parse(&["contact", "update", "c1", "--title", "Lead"]).is_ok());
        assert!(parse(&["contact", "import", "-"]).is_ok());
        assert!(parse(&["contact", "export"]).is_err());
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
        assert!(parse(&["engine", "restart"]).is_ok());
        assert!(parse(&["engine", "start", "--no-load"]).is_ok());
        assert!(parse(&["service", "install", "x", "--expect-plan", "ab"]).is_ok());
        // What the desktop runs (src/features/preferences/tabs/local-services.ts).
        for args in [
            &[
                "engine",
                "install",
                "--bin",
                "/e",
                "--no-start",
                "--key-file",
                "/k",
            ][..],
            &["engine", "install", "--bin", "/e", "--no-load"],
            &[
                "gateway",
                "install",
                "--bind",
                "127.0.0.1:5180",
                "--web-root",
                "",
            ],
            &["mcp-http", "install", "--port", "2587"],
            &["gateway", "pair"],
            &["engine", "stop"],
            &["mcp-http", "uninstall"],
            &["gateway", "start", "--unit-dir", "/tmp/u"],
        ] {
            assert!(parse(args).is_ok(), "{args:?}");
        }
        let cli = parse(&["gateway", "install", "--replace", "--dry-run"]).unwrap();
        let Command::Gateway {
            command: GatewayCommand::Install { bind, install, .. },
        } = cli.command
        else {
            panic!("gateway install")
        };
        assert_eq!(bind.to_string(), "127.0.0.1:5180");
        assert!(install.replace && install.dry_run && !install.no_start);
        assert!(parse(&["gateway", "install", "--bind", "nowhere"]).is_err());
        assert!(parse(&["gateway", "pair", "--admin", "--ttl", "5m"]).is_ok());
        assert!(parse(&["uninstall", "--purge-profile", "-y"]).is_ok());
        assert!(parse(&["uninstall", "--yes"]).is_err());
        assert!(parse(&["logs", "gateway", "-f"]).is_ok());
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

    #[test]
    fn send_later_follow_up_and_file_shapes() {
        let cli = parse(&[
            "draft",
            "send",
            "d1",
            "--at",
            "tomorrow 9:00",
            "--remind-in",
            "3d",
            "--track-opens",
        ])
        .unwrap();
        let Command::Draft {
            command:
                DraftCommand::Send {
                    at,
                    undo,
                    follow_up,
                    ..
                },
        } = cli.command
        else {
            panic!("draft send")
        };
        assert_eq!((at.as_deref(), undo), (Some("tomorrow 9:00"), None));
        assert_eq!(follow_up.remind_in.as_deref(), Some("3d"));
        assert!(follow_up.track_opens && !follow_up.track_links);
        for bad in [
            &["draft", "send", "d1", "--at", "2h", "--undo", "10"][..],
            &["draft", "send", "d1", "--at", "2h", "--wait"],
            &["draft", "send", "d1", "--undo", "10", "--wait"],
            &["draft", "send", "d1", "--undo", "0"],
            &["draft", "cancel-send"],
            &["remind", "t1"],
            &["remind", "t1", "--in", "2d", "--clear"],
            &["tracking"],
            &["tracking", "list", "--type", "clicks"],
            &["attachment", "get"],
            &["export", "m1"],
            &["export", "m1", "--mbox", "--folder", "INBOX", "-o", "x"],
            &["export", "--mbox", "-o", "x.mbox"],
        ] {
            assert_eq!(parse(bad).unwrap_err().exit_code(), 2, "{bad:?}");
        }
        for ok in [
            &["draft", "send", "d1", "--undo", "10", "--archive"][..],
            &["draft", "cancel-send", "d1"],
            &["draft", "scheduled"],
            &[
                "draft",
                "new",
                "--to",
                "a@x.test",
                "--track-links",
                "--remind-in",
                "1w",
            ],
            &["draft", "reply", "m1", "--track-opens"],
            &["draft", "edit", "d1", "--remind-in", "2d"],
            &["remind", "t1", "--in", "tomorrow"],
            &["remind", "t1", "--clear"],
            &["tracking", "m1"],
            &[
                "tracking", "list", "--type", "opens", "--since", "7d", "-n", "5",
            ],
            &["tracking", "links", "-n", "3"],
            &["attachment", "get", "f1", "-o", "-", "--force"],
            &["attachment", "save", "m1", "--dir", "/tmp/x"],
            &["export", "m1", "-o", "m.eml", "--force"],
            &["export", "--mbox", "--folder", "INBOX", "-o", "x.mbox"],
        ] {
            assert!(parse(ok).is_ok(), "{ok:?}");
        }
        let cli = parse(&["tracking", "list"]).unwrap();
        let Command::Tracking {
            command: Some(TrackingCommand::List { kind, .. }),
            message: None,
        } = cli.command
        else {
            panic!("tracking list")
        };
        assert_eq!(kind, "either");
    }
}
