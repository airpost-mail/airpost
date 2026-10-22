// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The command line.

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
    /// Read, create, change, import and export contacts.
    Contact {
        #[command(subcommand)]
        command: ContactCommand,
    },
    /// Write drafts as the desktop's composer starts them (From, signature, quoted text,
    /// reply recipients), with attachments; list, edit, delete and send them. Needs the serve
    /// engine.
    Draft {
        #[command(subcommand)]
        command: DraftCommand,
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
    /// Run the engine as a user unit (`serve --detached`), started at login, for service
    /// plugins, agents and headless use.
    Engine {
        #[command(subcommand)]
        command: EngineCommand,
    },
    /// Run tern-gateway, the web client's server, as a user unit started on the first
    /// connection.
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
    /// Run `tern-mcp http` as a user unit started on the first connection.
    #[command(name = "mcp-http")]
    McpHttp {
        #[command(subcommand)]
        command: McpHttpCommand,
    },
    /// Remove every unit tern installed for the profile (service plugins, mcp-http, the
    /// gateway, the engine) and the `tern` link the desktop put on the PATH.
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
    /// in the browser (on a server: with --paste). The engine tests the account before it
    /// keeps it.
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
        /// The engine binary for the engine unit (default: tern-engine beside
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

/// What every `tern <engine|gateway|mcp-http> install` takes.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Take over a unit that runs another copy of Tern's binaries (the desktop's, or a
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
        /// The engine binary (default: tern-engine beside tern, or TERN_ENGINE_BIN).
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
        /// Keep the engine's secrets in a file sealed with this key instead of the OS
        /// keychain (TERN_ENGINE_KEY_FILE).
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
    /// Install a unit that listens on --bind and starts tern-gateway on the first connection.
    Install {
        /// The gateway binary (default: tern-gateway beside tern, or TERN_GATEWAY_BIN).
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
        #[arg(long, value_name = "ADDR:PORT", default_value = crate::builtin::DEFAULT_GATEWAY_BIND)]
        bind: std::net::SocketAddr,
        /// The built web client (default: one beside the gateway binary: tern-web, web or
        /// ../share/tern/web); empty means that default.
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
    /// Print a one-time code that pairs a browser with the gateway (`tern-gateway pair`).
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
    /// Install a unit that listens on 127.0.0.1 and starts `tern-mcp http` on the first
    /// connection. Needs http.enabled in mcp.json.
    Install {
        /// The tern-mcp binary (default: tern-mcp beside tern, or TERN_MCP_BIN).
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
    Create {
        /// The calendar's id.
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
    /// Fields as JSON, in the shape `tern contact get --json` prints under `data`.
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
}

/// What `tern draft new|reply|reply-all|forward|edit` set on the draft.
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
        /// The draft's id (`tern draft list` shows it) or its Message-ID.
        #[arg(value_name = "DRAFT_ID|MESSAGE_ID")]
        id: String,
        #[command(flatten)]
        fields: DraftFieldArgs,
        /// Remove an attachment, by file name or id; repeat for more.
        #[arg(long, value_name = "NAME|ID")]
        detach: Vec<String>,
    },
    /// Delete a draft. A send waiting out its undo window or a schedule is cancelled with
    /// it; one already going out can't be.
    Delete {
        /// The draft's id or its Message-ID.
        #[arg(value_name = "DRAFT_ID|MESSAGE_ID")]
        id: String,
    },
    /// Send a draft now. It can't be called back.
    Send {
        /// The draft's id (`tern draft list` shows it) or its Message-ID.
        #[arg(value_name = "DRAFT_ID|MESSAGE_ID")]
        id: String,
        /// Archive the thread once it is sent.
        #[arg(long)]
        archive: bool,
        /// Wait until the mail has gone; exit 1 if the send fails.
        #[arg(long)]
        wait: bool,
        /// With --wait, give up after this many seconds.
        #[arg(long, value_name = "SECS", requires = "wait")]
        timeout: Option<u64>,
    },
    /// The unsent drafts, newest first.
    List {
        /// How many drafts per account (default 50).
        #[arg(long, short = 'n')]
        limit: Option<usize>,
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
}
