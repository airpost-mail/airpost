// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The [`ImapSession`] implementation.
//!
//! Derived from mailcore2's `IMAPSession` (`Vendor/mailcore2/src/core/imap/MCIMAPSession.cpp`:
//! `connect`, `login`, `enableFeatures`, `select`, `folderStatus`, `fetchAllFolders`,
//! `fetchMessages`, `storeFlagsAndCustomFlags`, `storeLabels`, `copyMessages`,
//! `moveMessages`, `appendMessage…`, `findUIDsOfRecentHeaderMessageID`, `idle`) and the
//! libetpan commands it drives (`mailimap.c`, `mailimap_sender.c`, `qresync.c`,
//! `condstore.c`, `uidplus.c`, `xgmlabels.c`, `mailimap_oauth2.c`). mailcore2 is Copyright ©
//! 2001-2013 MailCore team and libetpan Copyright © 2001-2005 DINH Viet Hoa, both
//! BSD-3-Clause; Mailspring-Sync's changes to them are Copyright © 2017 Foundry 376, GPL-3.0.
//! See NOTICE.md ("mailcore2", "libEtPan!").
//!
//! Errors follow mailcore: a stream failure is `Connection`, an unparseable tagged response
//! is `Parse` (both drop the connection, so the next call reconnects), and a NO or BAD is
//! the command's own code (`Fetch`, `Store`, `NonExistantFolder`, …).

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use airpost_core::protolog::Direction;
use airpost_core::{ErrorCode, FolderFlags, MailError, MessageFlags, Security, UidRange, UidSet};
use airpost_providers::{ConnectionInfo, ServerProfile};
use base64::Engine;

use crate::envelope;
use crate::parser::{self, Code, Response, Status, StatusText, Token, Untagged};
use crate::transport::{self, TlsFailure, Wire};
use crate::{
    Capabilities, FetchKind, FetchResult, FolderStatus, IdleInterrupt, ImapAuth, ImapConfig,
    ImapMessage, ImapSession, ListedFolder, Namespace, Result, SelectInfo, StoreOp, SyncResult,
    UidMapping,
};

/// Ranges per UID STORE of flags (`splitSet(set, 50)`).
const STORE_FLAGS_CHUNK: usize = 50;
/// Ranges per UID STORE of labels, UID COPY and UID MOVE (`splitSet(set, 10)`).
const COMMAND_CHUNK: usize = 10;
/// How many of the newest messages `findUIDsOfRecentHeaderMessageID` looks at, minus one.
const RECENT_HEADER_WINDOW: u32 = 15;
/// The greeting that turns on libetpan's 163 workaround (`C`-prefixed tags).
const COREMAIL_GREETING: &str = "Coremail System IMap Server Ready";
/// The header fields of the FullHeaders request kind (mailcore's mbox-mail workaround).
const FULL_HEADER_FIELDS: &str =
    "Date Subject From Sender Reply-To To Cc Message-ID References In-Reply-To";

pub struct Session {
    // Note: `inner` is declared first so its sockets are dropped while the runtime that
    // registered them still exists.
    inner: Inner,
    rt: tokio::runtime::Runtime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum State {
    Disconnected,
    Connected,
    LoggedIn,
    Selected,
}

struct Inner {
    config: ImapConfig,
    wire: Option<Wire>,
    state: State,
    /// libetpan's `mImap != NULL`: set by a connect attempt, cleared only by
    /// [`ImapSession::disconnect`]. NOOP on a session that was never set up does nothing.
    set_up: bool,
    tag_prefix: &'static str,
    tag_counter: u32,
    greeting: String,
    /// What the server advertised.
    server_caps: Capabilities,
    /// With the profile's overrides (`capabilities()`).
    caps: Capabilities,
    /// A CAPABILITY response or response code seen during the current command.
    pending_caps: Option<Vec<String>>,
    profile: ServerProfile,
    namespace: Option<Namespace>,
    delimiter: Option<char>,
    current_folder: Option<String>,
    /// The folder a SELECT in flight is selecting, for VANISHED that arrive with it.
    select_target: Option<String>,
    qresync: bool,
    condstore: bool,
    xlist: bool,
    /// mailcore's `mNeedsMboxMailWorkaround`: header fetches ask for the header fields
    /// instead of ENVELOPE, after the server returned FETCH responses without one.
    full_headers: bool,
    vanished: HashMap<String, UidSet>,
    /// Per folder: the last SELECT allowed new keywords (`SelectInfo::allows_new_keywords`).
    new_keywords: HashMap<String, bool>,
    last_tls_error: Option<String>,
}

/// A tagged response and the untagged ones that came before it.
struct Reply {
    st: StatusText,
    untagged: Vec<Untagged>,
}

impl Reply {
    fn ok(&self) -> bool {
        self.st.status == Status::Ok
    }

    /// Response codes from the tagged response and every untagged status response.
    fn codes(&self) -> impl Iterator<Item = &Code> {
        self.untagged
            .iter()
            .filter_map(|u| match u {
                Untagged::State(st) => st.code.as_ref(),
                _ => None,
            })
            .chain(self.st.code.as_ref())
    }

    fn fail(&self, code: ErrorCode) -> MailError {
        let detail = match &self.st.code {
            Some(c) => format!("[{}] {}", c.name, self.st.text),
            None => self.st.text.clone(),
        };
        MailError::new(code, detail)
    }
}

/// What a message fetch asks for (`IMAPMessagesRequestKind`).
#[derive(Clone, Copy)]
struct FetchSpec<'a> {
    by_uid: bool,
    flags: bool,
    headers: bool,
    internal_date: bool,
    gmail_labels: bool,
    gmail_ids: bool,
    modseq: u64,
    /// Only UIDs in this set are returned (`uidsFilter`).
    uid_filter: Option<&'a UidSet>,
}

struct FetchOutcome {
    messages: Vec<ImapMessage>,
    vanished: UidSet,
    hit_message_limit: bool,
}

fn astring(s: &str) -> String {
    // libetpan's `is_atom` workaround for Courier, which rejects quoted field names.
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        s.to_owned()
    } else {
        quoted(s)
    }
}

fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn format_ranges(ranges: &[UidRange]) -> String {
    ranges
        .iter()
        .map(|r| match r.end {
            Some(e) if e == r.start => r.start.to_string(),
            Some(e) => format!("{}:{}", r.start, e),
            None => format!("{}:*", r.start),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// A COPYUID set expanded in the order it was written, as libetpan's `arrayFromSet` does.
fn expand_ordered(set: &str) -> Vec<u32> {
    let mut out = vec![];
    for part in set.split(',') {
        let (a, b) = part.split_once(':').unwrap_or((part, part));
        let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) else {
            continue;
        };
        out.extend(a.min(b)..=a.max(b));
    }
    out
}

/// mailcore's login error text mapping (`IMAPSession::login`).
fn login_error_code(response: &str) -> ErrorCode {
    let table: [(&str, ErrorCode); 10] = [
        ("not enabled for IMAP use", ErrorCode::GmailIMAPNotEnabled),
        ("IMAP access is disabled", ErrorCode::GmailIMAPNotEnabled),
        ("bandwidth limits", ErrorCode::GmailExceededBandwidthLimit),
        (
            "Too many simultaneous connections",
            ErrorCode::GmailTooManySimultaneousConnections,
        ),
        (
            "Maximum number of connections",
            ErrorCode::GmailTooManySimultaneousConnections,
        ),
        (
            "Application-specific password required",
            ErrorCode::GmailApplicationSpecificPasswordRequired,
        ),
        ("http://me.com/move", ErrorCode::MobileMeMoved),
        ("OCF12", ErrorCode::YahooUnavailable),
        (
            "Login to your account via a web browser",
            ErrorCode::OutlookLoginViaWebBrowser,
        ),
        ("", ErrorCode::Authentication),
    ];
    table
        .iter()
        .find(|(needle, _)| response.contains(needle))
        .map_or(ErrorCode::Authentication, |(_, code)| *code)
}

fn flag_list(flags: MessageFlags) -> String {
    format!("({})", flags.to_imap_atoms().join(" "))
}

fn store_sign(op: StoreOp) -> &'static str {
    match op {
        StoreOp::Add => "+",
        StoreOp::Remove => "-",
        StoreOp::Set => "",
    }
}

impl Session {
    /// A disconnected session; nothing happens on the network until a call needs it.
    pub fn new(config: ImapConfig) -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread tokio runtime");
        Session {
            inner: Inner::new(config),
            rt,
        }
    }

    pub fn config(&self) -> &ImapConfig {
        &self.inner.config
    }

    /// The greeting of the current connection, without `* OK`.
    pub fn greeting(&self) -> &str {
        &self.inner.greeting
    }

    /// COMPRESS=DEFLATE is active on the current connection.
    pub fn is_compressed(&self) -> bool {
        self.inner.wire.as_ref().is_some_and(Wire::is_compressed)
    }
}

impl Inner {
    fn new(config: ImapConfig) -> Self {
        Inner {
            config,
            wire: None,
            state: State::Disconnected,
            set_up: false,
            tag_prefix: "",
            tag_counter: 0,
            greeting: String::new(),
            server_caps: Capabilities::default(),
            caps: Capabilities::default(),
            pending_caps: None,
            profile: ServerProfile::default(),
            namespace: None,
            delimiter: None,
            current_folder: None,
            select_target: None,
            qresync: false,
            condstore: false,
            xlist: false,
            full_headers: false,
            vanished: HashMap::new(),
            new_keywords: HashMap::new(),
            last_tls_error: None,
        }
    }

    // -- Plumbing --------------------------------------------------------------------------

    /// Drops the connection after a stream or parse failure. Like mailcore's
    /// `mShouldDisconnect`, the next call reconnects.
    fn teardown(&mut self) {
        self.wire = None;
        self.state = State::Disconnected;
        self.current_folder = None;
        self.select_target = None;
        self.vanished.clear();
    }

    fn stream_error(&mut self, e: io::Error) -> MailError {
        if let Some(w) = &self.wire {
            w.note(&format!("connection error: {e}"));
        }
        self.teardown();
        MailError::new(ErrorCode::Connection, e.to_string())
    }

    fn parse_error(&mut self, detail: String) -> MailError {
        self.teardown();
        MailError::new(ErrorCode::Parse, detail)
    }

    fn wire(&mut self) -> Result<&mut Wire> {
        match self.wire.as_mut() {
            Some(w) => Ok(w),
            None => Err(MailError::new(ErrorCode::Connection, "not connected")),
        }
    }

    fn next_tag(&mut self) -> String {
        self.tag_counter += 1;
        format!("{}{}", self.tag_prefix, self.tag_counter)
    }

    async fn write(&mut self, bytes: &[u8], private: bool) -> Result<()> {
        let r = self.wire()?.write(bytes, private).await;
        r.map_err(|e| self.stream_error(e))
    }

    async fn read_raw(&mut self) -> Result<Vec<u8>> {
        let r = self.wire()?.read_response().await;
        r.map_err(|e| self.stream_error(e))
    }

    fn log_unparsed(&self, raw: &[u8]) {
        if let Some(l) = &self.config.logger {
            l.data(Direction::ErrorParse, raw);
        }
    }

    /// Bookkeeping every untagged response gets, whichever command it came with.
    fn dispatch(&mut self, u: &Untagged) {
        match u {
            Untagged::Vanished {
                earlier: false,
                uids,
            } => {
                let folder = self
                    .select_target
                    .clone()
                    .or_else(|| self.current_folder.clone());
                if let Some(folder) = folder {
                    self.vanished.entry(folder).or_default().extend(uids);
                }
            }
            Untagged::Capability(caps) => self.pending_caps = Some(caps.clone()),
            Untagged::State(st) => self.note_code(st),
            _ => {}
        }
    }

    fn note_code(&mut self, st: &StatusText) {
        if let Some(code) = &st.code {
            if code.name == "CAPABILITY" {
                self.pending_caps = Some(code.arg_words().map(str::to_owned).collect());
            }
        }
    }

    /// Reads until the tagged response for `tag`. An untagged response that cannot be
    /// parsed is logged and skipped; an unparseable tagged one is a parse error.
    async fn read_until_tagged(&mut self, tag: &str) -> Result<Reply> {
        let mut untagged = vec![];
        loop {
            let raw = self.read_raw().await?;
            match parser::parse_response(&raw) {
                Ok(Response::Tagged { tag: t, st }) if t == tag => {
                    self.note_code(&st);
                    return Ok(Reply { st, untagged });
                }
                Ok(Response::Untagged(u)) => {
                    self.dispatch(&u);
                    untagged.push(u);
                }
                // A continuation nobody asked for, or another command's tag.
                Ok(_) => self.log_unparsed(&raw),
                Err(e) => {
                    let ours = raw.starts_with(tag.as_bytes())
                        && raw.get(tag.len()).is_some_and(|b| *b == b' ');
                    if ours {
                        self.log_unparsed(&raw);
                        return Err(self.parse_error(e.0));
                    }
                    self.log_unparsed(&raw);
                }
            }
        }
    }

    /// Sends `<tag> <command>` and reads its response. A NO or BAD is a `Reply`, not an
    /// error; the caller maps it to its own code.
    async fn run(&mut self, command: &str) -> Result<Reply> {
        self.run_with(command, false).await
    }

    async fn run_with(&mut self, command: &str, private: bool) -> Result<Reply> {
        let tag = self.next_tag();
        let line = format!("{tag} {command}\r\n");
        self.write(line.as_bytes(), private).await?;
        self.read_until_tagged(&tag).await
    }

    async fn run_expect(&mut self, command: &str, code: ErrorCode) -> Result<Reply> {
        let reply = self.run(command).await?;
        if reply.ok() {
            Ok(reply)
        } else {
            Err(reply.fail(code))
        }
    }

    // -- Connection ------------------------------------------------------------------------

    fn tls_config(&self) -> Result<Arc<rustls::ClientConfig>> {
        match &self.config.tls_config {
            Some(c) => Ok(c.clone()),
            None => transport::platform_tls_config()
                .map_err(|e| MailError::new(ErrorCode::Connection, e)),
        }
    }

    async fn handshake(&mut self, failure: ErrorCode) -> Result<()> {
        let config = self.tls_config()?;
        let host = self.config.host.clone();
        let wire = self.wire()?;
        match wire.upgrade_tls(config, &host).await {
            Ok(()) => Ok(()),
            Err(TlsFailure::Certificate(e)) => {
                wire.note(&format!("certificate of {host} rejected: {e}"));
                self.teardown();
                Err(MailError::new(ErrorCode::Certificate, e))
            }
            Err(TlsFailure::Handshake(e)) => {
                wire.note(&format!("TLS handshake with {host} rejected: {e}"));
                self.teardown();
                self.last_tls_error = Some(e.clone());
                Err(MailError::new(failure, e))
            }
        }
    }

    fn connection_info(&self) -> ConnectionInfo {
        ConnectionInfo {
            host: self.config.host.to_ascii_lowercase(),
            greeting: self.greeting.clone(),
            id_response: vec![],
            capabilities: self.server_caps.iter().map(str::to_owned).collect(),
        }
    }

    /// `connectWithCurrentCompatibilityLevel`. rustls has no weaker protocol versions to
    /// fall back to, so there is one attempt.
    async fn connect(&mut self) -> Result<()> {
        self.teardown();
        self.set_up = true;
        self.last_tls_error = None;
        self.tag_counter = 0;
        self.tag_prefix = "";
        self.greeting.clear();
        self.server_caps = Capabilities::default();
        self.caps = Capabilities::default();
        self.pending_caps = None;
        self.namespace = None;
        self.delimiter = None;
        self.qresync = false;
        self.condstore = false;
        self.xlist = false;

        let (host, port) = (self.config.host.clone(), self.config.port);
        if let Some(l) = &self.config.logger {
            l.note(&format!("connect {host} {port}"));
        }
        let wire = Wire::connect(&host, port, self.config.timeout, self.config.logger.clone())
            .await
            .map_err(|e| MailError::new(ErrorCode::Connection, e.to_string()))?;
        self.wire = Some(wire);

        if self.config.security == Security::Tls {
            self.handshake(ErrorCode::Connection).await?;
        }

        let raw = self.read_raw().await?;
        match parser::parse_response(&raw) {
            Ok(Response::Untagged(Untagged::State(st)))
                if matches!(st.status, Status::Ok | Status::PreAuth) =>
            {
                self.note_code(&st);
                self.greeting = st.text;
            }
            _ => {
                self.teardown();
                return Err(MailError::new(
                    ErrorCode::Connection,
                    format!(
                        "unexpected greeting: {}",
                        String::from_utf8_lossy(&raw).trim_end()
                    ),
                ));
            }
        }
        if self.greeting.contains(COREMAIL_GREETING) {
            self.tag_prefix = "C";
        }

        if self.config.security == Security::StartTls {
            // libetpan's mailimap_socket_starttls: any failure but a rejected certificate
            // is ErrorTLSNotAvailable.
            self.pending_caps = None;
            let reply = self
                .run("STARTTLS")
                .await
                .map_err(|e| MailError::new(ErrorCode::TLSNotAvailable, e.detail))?;
            if !reply.ok() {
                self.teardown();
                return Err(reply.fail(ErrorCode::TLSNotAvailable));
            }
            if self.wire()?.has_buffered_input() {
                self.teardown();
                return Err(MailError::new(
                    ErrorCode::TLSNotAvailable,
                    "data received after STARTTLS reply",
                ));
            }
            self.handshake(ErrorCode::TLSNotAvailable).await?;
            // Capabilities from before TLS cannot be trusted.
            self.pending_caps = None;
        }

        self.state = State::Connected;
        match self.pending_caps.take() {
            Some(caps) => self.server_caps = Capabilities::new(caps),
            None => self.capability().await?,
        }
        self.apply_capabilities();
        Ok(())
    }

    async fn capability(&mut self) -> Result<()> {
        self.pending_caps = None;
        self.run_expect("CAPABILITY", ErrorCode::Capability).await?;
        let caps = self.pending_caps.take().unwrap_or_default();
        self.server_caps = Capabilities::new(caps);
        Ok(())
    }

    /// `applyCapabilities` plus the profile's overrides.
    fn apply_capabilities(&mut self) {
        self.profile = airpost_providers::resolve(&self.connection_info());
        let mut caps = self.server_caps.clone();
        if self.profile.xlist_disabled {
            caps.remove("XLIST");
        }
        if self.profile.force_idle {
            caps.insert("IDLE");
        }
        if self.profile.force_namespace {
            caps.insert("NAMESPACE");
        }
        if self.profile.ignore_namespace_capability {
            caps.remove("NAMESPACE");
        }
        if self.config.disable_qresync || self.profile.disable_qresync {
            caps.remove("QRESYNC");
        }
        self.xlist = caps.has("XLIST");
        self.condstore = caps.has("CONDSTORE");
        self.qresync = caps.has("QRESYNC");
        self.caps = caps;
    }

    async fn connect_if_needed(&mut self) -> Result<()> {
        if self.wire.is_none() {
            self.connect().await?;
        }
        Ok(())
    }

    async fn login_if_needed(&mut self) -> Result<()> {
        self.connect_if_needed().await?;
        if self.state == State::Connected {
            self.login().await?;
        }
        Ok(())
    }

    /// `IMAPSession::login`: authenticate, CAPABILITY, `enableFeatures`, ID, NAMESPACE.
    async fn login(&mut self) -> Result<()> {
        self.pending_caps = None;
        let user = self.config.username.clone();
        let reply = match self.config.auth.clone() {
            ImapAuth::Password(password) => {
                let cmd = format!("LOGIN {} {}", astring(&user), astring(&password));
                self.run_with(&cmd, true).await?
            }
            ImapAuth::XOAuth2(token) => self.authenticate_xoauth2(&user, &token).await?,
        };
        if !reply.ok() {
            let response = match &reply.st.code {
                Some(c) => format!("[{} {}] {}", c.name, c.args, reply.st.text),
                None => reply.st.text.clone(),
            };
            return Err(reply.fail(login_error_code(&response)));
        }
        self.state = State::LoggedIn;

        match self.pending_caps.take() {
            Some(caps) => self.server_caps = Capabilities::new(caps),
            None => self.capability().await?,
        }
        self.apply_capabilities();

        // enableFeatures
        if self.caps.has("COMPRESS=DEFLATE") {
            let reply = self.run("COMPRESS DEFLATE").await?;
            if reply.ok() {
                self.wire()?.start_compression();
            }
        }
        if self.qresync {
            self.run("ENABLE QRESYNC").await?;
        } else if self.condstore {
            self.run("ENABLE CONDSTORE").await?;
        }

        // shouldSendClientIdentity: Coremail refuses SELECT until it has seen ID.
        if self.profile.id_before_select && self.caps.has("ID") {
            if let Some((name, version)) = self.config.client_identity.clone() {
                let cmd = format!(
                    "ID ({} {} {} {})",
                    quoted("name"),
                    quoted(&name),
                    quoted("version"),
                    quoted(&version)
                );
                self.run_expect(&cmd, ErrorCode::Identity).await?;
            }
        }

        let mut namespace = None;
        if self.caps.has("NAMESPACE") {
            let reply = self.run_expect("NAMESPACE", ErrorCode::Namespace).await?;
            namespace = reply.untagged.iter().find_map(|u| match u {
                Untagged::Namespace { personal } => personal.first().map(|(p, d)| Namespace {
                    prefix: String::from_utf8_lossy(p).into_owned(),
                    delimiter: *d,
                }),
                _ => None,
            });
        }
        if namespace.is_none() {
            let reply = self
                .run_expect("LIST \"\" \"\"", ErrorCode::NonExistantFolder)
                .await?;
            let delimiter = reply.untagged.iter().find_map(|u| match u {
                Untagged::List { delimiter, .. } => Some(*delimiter),
                _ => None,
            });
            let Some(delimiter) = delimiter else {
                return Err(MailError::new(
                    ErrorCode::NonExistantFolder,
                    "LIST \"\" \"\" returned nothing",
                ));
            };
            namespace = Some(Namespace {
                prefix: String::new(),
                delimiter,
            });
        }
        self.delimiter = namespace.as_ref().and_then(|n| n.delimiter);
        self.namespace = namespace;
        Ok(())
    }

    /// `mailimap_oauth2_authenticate`: the initial response inline; a continuation means
    /// the server is reporting an error, acknowledged with an empty line.
    async fn authenticate_xoauth2(&mut self, user: &str, token: &str) -> Result<Reply> {
        let payload = format!("user={user}\x01auth=Bearer {token}\x01\x01");
        let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let tag = self.next_tag();
        let line = format!("{tag} AUTHENTICATE XOAUTH2 {b64}\r\n");
        self.write(line.as_bytes(), true).await?;
        let raw = self.read_raw().await?;
        match parser::parse_response(&raw) {
            Ok(Response::Continue(_)) => {
                self.write(b"\r\n", true).await?;
                self.read_until_tagged(&tag).await
            }
            Ok(Response::Tagged { tag: t, st }) if t == tag => {
                self.note_code(&st);
                Ok(Reply {
                    st,
                    untagged: vec![],
                })
            }
            Ok(Response::Untagged(u)) => {
                self.dispatch(&u);
                let mut reply = self.read_until_tagged(&tag).await?;
                reply.untagged.insert(0, u);
                Ok(reply)
            }
            _ => {
                self.log_unparsed(&raw);
                Err(self.parse_error("unexpected response to AUTHENTICATE".into()))
            }
        }
    }

    // -- Folders ---------------------------------------------------------------------------

    async fn select(&mut self, path: &str) -> Result<SelectInfo> {
        self.login_if_needed().await?;
        self.select_target = Some(path.to_owned());
        let result = self.run(&format!("SELECT {}", astring(path))).await;
        self.select_target = None;
        let reply = result?;
        if !reply.ok() {
            self.state = State::LoggedIn;
            self.current_folder = None;
            return Err(reply.fail(ErrorCode::NonExistantFolder));
        }
        let mut info = SelectInfo::default();
        for u in &reply.untagged {
            match u {
                Untagged::Exists(n) => info.exists = *n,
                Untagged::Flags(flags) => info.flags = flags.clone(),
                _ => {}
            }
        }
        for code in reply.codes() {
            match code.name.as_str() {
                "UIDVALIDITY" => info.uid_validity = code.number().unwrap_or(0) as u32,
                "UIDNEXT" => info.uid_next = code.number().unwrap_or(0) as u32,
                "HIGHESTMODSEQ" => info.highest_modseq = code.number().unwrap_or(0),
                "NOMODSEQ" => info.highest_modseq = 0,
                "PERMANENTFLAGS" => {
                    let list = code
                        .args
                        .trim()
                        .trim_start_matches('(')
                        .trim_end_matches(')');
                    info.permanent_flags =
                        Some(list.split_whitespace().map(str::to_owned).collect());
                }
                _ => {}
            }
        }
        self.new_keywords
            .insert(path.to_owned(), info.allows_new_keywords());
        self.current_folder = Some(path.to_owned());
        self.state = State::Selected;
        Ok(info)
    }

    async fn select_if_needed(&mut self, path: &str) -> Result<()> {
        self.login_if_needed().await?;
        let same = self.state == State::Selected
            && self
                .current_folder
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case(path));
        if !same {
            self.select(path).await?;
        }
        Ok(())
    }

    async fn noop(&mut self) -> Result<()> {
        if !self.set_up {
            return Ok(());
        }
        self.login_if_needed().await?;
        self.run_expect("NOOP", ErrorCode::Noop).await?;
        Ok(())
    }

    async fn status(&mut self, path: &str) -> Result<FolderStatus> {
        self.login_if_needed().await?;
        let mut items = String::from("UNSEEN MESSAGES RECENT UIDNEXT UIDVALIDITY");
        if self.condstore || self.caps.has("XYMHIGHESTMODSEQ") {
            items.push_str(" HIGHESTMODSEQ");
        }
        let reply = self
            .run_expect(
                &format!("STATUS {} ({items})", astring(path)),
                ErrorCode::NonExistantFolder,
            )
            .await?;
        let mut st = FolderStatus::default();
        for u in &reply.untagged {
            let Untagged::MailboxStatus { items, .. } = u else {
                continue;
            };
            for (name, value) in items {
                match name.as_str() {
                    "UNSEEN" => st.unseen_count = *value as u32,
                    "MESSAGES" => st.message_count = *value as u32,
                    "RECENT" => st.recent_count = *value as u32,
                    "UIDNEXT" => st.uid_next = *value as u32,
                    "UIDVALIDITY" => st.uid_validity = *value as u32,
                    "HIGHESTMODSEQ" => st.highest_modseq = *value,
                    _ => {}
                }
            }
        }
        Ok(st)
    }

    async fn list(&mut self, command: &str) -> Result<Vec<ListedFolder>> {
        let reply = self
            .run_expect(command, ErrorCode::NonExistantFolder)
            .await?;
        Ok(reply
            .untagged
            .into_iter()
            .filter_map(|u| match u {
                Untagged::List {
                    flags,
                    delimiter,
                    name,
                } => {
                    let name = String::from_utf8_lossy(&name).into_owned();
                    let path = if name.eq_ignore_ascii_case("INBOX") {
                        "INBOX".to_owned()
                    } else {
                        name
                    };
                    let flags = flags
                        .iter()
                        .filter_map(|f| FolderFlags::from_attribute(f))
                        .fold(FolderFlags::empty(), |a, f| a | f);
                    Some(ListedFolder {
                        path,
                        delimiter,
                        flags,
                    })
                }
                _ => None,
            })
            .collect())
    }

    /// `fetchAllFolders`.
    async fn list_folders(&mut self) -> Result<Vec<ListedFolder>> {
        self.login_if_needed().await?;
        if self.delimiter.is_none() {
            let probe = self.list("LIST \"\" \"\"").await?;
            self.delimiter = probe.first().and_then(|f| f.delimiter);
        }
        let mut prefix = self
            .namespace
            .as_ref()
            .map(|n| n.prefix.clone())
            .unwrap_or_default();
        if let Some(d) = self.delimiter {
            if !prefix.is_empty() && !prefix.ends_with(d) {
                prefix.push(d);
            }
        }
        let verb = if self.xlist { "XLIST" } else { "LIST" };
        let mut folders = self
            .list(&format!("{verb} {} \"*\"", astring_or_empty(&prefix)))
            .await?;
        if !folders.iter().any(|f| f.path == "INBOX") {
            // Some providers give the inbox another name and flag it \Inbox.
            if let Some(f) = folders
                .iter_mut()
                .find(|f| f.flags.contains(FolderFlags::INBOX))
            {
                f.path = "INBOX".to_owned();
            } else {
                let inbox = self.list("LIST \"\" \"INBOX\"").await?;
                folders.extend(inbox);
            }
        }
        Ok(folders)
    }

    async fn create_folder(&mut self, path: &str) -> Result<()> {
        self.select_if_needed("INBOX").await?;
        self.run_expect(&format!("CREATE {}", astring(path)), ErrorCode::Create)
            .await?;
        self.run_expect(
            &format!("SUBSCRIBE {}", astring(path)),
            ErrorCode::Subscribe,
        )
        .await?;
        Ok(())
    }

    async fn rename_folder(&mut self, path: &str, new_path: &str) -> Result<()> {
        self.select_if_needed("INBOX").await?;
        self.run_expect(
            &format!("RENAME {} {}", astring(path), astring(new_path)),
            ErrorCode::Rename,
        )
        .await?;
        Ok(())
    }

    async fn delete_folder(&mut self, path: &str) -> Result<()> {
        self.select_if_needed("INBOX").await?;
        self.run_expect(&format!("DELETE {}", astring(path)), ErrorCode::Delete)
            .await?;
        Ok(())
    }

    // -- Messages --------------------------------------------------------------------------

    fn is_gmail(&self) -> bool {
        self.server_caps.has("X-GM-EXT-1")
    }

    fn spec_for<'a>(&self, kind: FetchKind, uids: &'a UidSet, modseq: u64) -> FetchSpec<'a> {
        let gmail = self.is_gmail();
        let heavy = kind == FetchKind::Heavy;
        FetchSpec {
            by_uid: true,
            flags: true,
            headers: heavy,
            internal_date: heavy,
            gmail_labels: gmail,
            gmail_ids: gmail && heavy,
            modseq,
            uid_filter: Some(uids),
        }
    }

    /// `fetchMessages`, including the FullHeaders retry: a header fetch where no message
    /// survived and some requested UID came back without an ENVELOPE is asked again for the
    /// header fields, and if that works the session asks that way from then on.
    ///
    /// mailcore gates the retry on `clist_count(fetch_result) > 0`, which is always 0 with a
    /// msg_att_handler installed, so the C++ never retries. Counting only in-filter FETCHes
    /// that lack ENVELOPE keeps the fallback for servers without ENVELOPE while leaving the
    /// transcript identical to the C++ for every server that has it.
    async fn fetch_messages(&mut self, set: &str, spec: FetchSpec<'_>) -> Result<FetchOutcome> {
        let full_headers = spec.headers && self.full_headers;
        let (first, missing_envelope) = self.fetch_once(set, spec, full_headers).await?;
        if spec.headers && !full_headers && first.messages.is_empty() && missing_envelope > 0 {
            let (mut retry, _) = self.fetch_once(set, spec, true).await?;
            if !retry.messages.is_empty() {
                self.full_headers = true;
            }
            retry.vanished.extend(&first.vanished);
            retry.hit_message_limit |= first.hit_message_limit;
            return Ok(retry);
        }
        Ok(first)
    }

    async fn fetch_once(
        &mut self,
        set: &str,
        spec: FetchSpec<'_>,
        full_headers: bool,
    ) -> Result<(FetchOutcome, usize)> {
        let section = if full_headers {
            format!("BODY.PEEK[HEADER.FIELDS ({FULL_HEADER_FIELDS})]")
        } else {
            "BODY.PEEK[HEADER.FIELDS (References)]".to_owned()
        };
        let mut atts = vec!["UID"];
        if spec.flags {
            atts.push("FLAGS");
        }
        if spec.gmail_labels {
            atts.push("X-GM-LABELS");
        }
        if spec.gmail_ids {
            atts.push("X-GM-THRID");
            atts.push("X-GM-MSGID");
        }
        if spec.headers && !full_headers {
            atts.push("ENVELOPE");
        }
        if spec.internal_date {
            atts.push("INTERNALDATE");
        }
        if spec.headers {
            atts.push(&section);
        }
        let mut command = format!(
            "{}FETCH {set} ({})",
            if spec.by_uid { "UID " } else { "" },
            atts.join(" ")
        );
        if spec.modseq != 0 && (self.condstore || self.qresync) {
            if self.qresync {
                command.push_str(&format!(" (CHANGEDSINCE {} VANISHED)", spec.modseq));
            } else {
                command.push_str(&format!(" (CHANGEDSINCE {})", spec.modseq));
            }
        }
        let reply = self.run_expect(&command, ErrorCode::Fetch).await?;

        let hit_message_limit = reply.codes().any(|c| c.name == "MESSAGELIMIT");
        let mut vanished = UidSet::new();
        let mut messages: Vec<ImapMessage> = vec![];
        let mut index: HashMap<u32, usize> = HashMap::new();
        let mut missing_envelope = 0;
        for u in reply.untagged {
            match u {
                Untagged::Vanished {
                    earlier: true,
                    uids,
                } => vanished.extend(&uids),
                Untagged::Fetch { items, .. } => {
                    if spec.headers && !full_headers && lacks_envelope(&items, spec) {
                        missing_envelope += 1;
                    }
                    let Some(m) = message_from_items(items, spec, full_headers) else {
                        continue;
                    };
                    // A flag change another session made can arrive as an unsolicited
                    // FETCH before the requested one; the later response is the current one.
                    match index.get(&m.uid) {
                        Some(&i) => messages[i] = m,
                        None => {
                            index.insert(m.uid, messages.len());
                            messages.push(m);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok((
            FetchOutcome {
                messages,
                vanished,
                hit_message_limit,
            },
            missing_envelope,
        ))
    }

    async fn fetch_by_uid(
        &mut self,
        path: &str,
        kind: FetchKind,
        uids: &UidSet,
        modseq: u64,
    ) -> Result<FetchOutcome> {
        if uids.is_empty() {
            return Ok(FetchOutcome {
                messages: vec![],
                vanished: UidSet::new(),
                hit_message_limit: false,
            });
        }
        self.select_if_needed(path).await?;
        let spec = self.spec_for(kind, uids, modseq);
        self.fetch_messages(&uids.to_string(), spec).await
    }

    async fn fetch_message_bytes(&mut self, path: &str, uid: u32) -> Result<Option<Vec<u8>>> {
        self.select_if_needed(path).await?;
        let reply = self
            .run_expect(&format!("UID FETCH {uid} BODY.PEEK[]"), ErrorCode::Fetch)
            .await?;
        for u in reply.untagged {
            let Untagged::Fetch { items, .. } = u else {
                continue;
            };
            let item_uid = items
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("UID"))
                .and_then(|(_, v)| v.number());
            if item_uid.is_some_and(|u| u != uid as u64) {
                continue;
            }
            for (name, value) in items {
                if name.to_ascii_uppercase().starts_with("BODY[]") {
                    return Ok(Some(value.bytes().map(<[u8]>::to_vec).unwrap_or_default()));
                }
            }
        }
        Ok(None)
    }

    /// Selects `path` and runs one command there.
    async fn in_folder(&mut self, path: &str, command: &str, code: ErrorCode) -> Result<()> {
        self.select_if_needed(path).await?;
        self.run_expect(command, code).await?;
        Ok(())
    }

    async fn store(&mut self, path: &str, uids: &UidSet, chunk: usize, item: &str) -> Result<()> {
        self.select_if_needed(path).await?;
        for ranges in uids.ranges().chunks(chunk) {
            let cmd = format!("UID STORE {} {item}", format_ranges(ranges));
            self.run_expect(&cmd, ErrorCode::Store).await?;
        }
        Ok(())
    }

    async fn copy_or_move(
        &mut self,
        verb: &str,
        path: &str,
        uids: &UidSet,
        dest: &str,
    ) -> Result<UidMapping> {
        self.select_if_needed(path).await?;
        let mut mapping = vec![];
        for ranges in uids.ranges().chunks(COMMAND_CHUNK) {
            let cmd = format!("UID {verb} {} {}", format_ranges(ranges), astring(dest));
            let reply = self.run_expect(&cmd, ErrorCode::Copy).await?;
            // Dovecot answers MOVE with an untagged OK carrying COPYUID; COPY carries it on
            // the tagged OK.
            let copyuid = reply.codes().find(|c| c.name == "COPYUID").cloned();
            if let Some(code) = copyuid {
                let words: Vec<&str> = code.arg_words().collect();
                if words.len() >= 3 {
                    let src = expand_ordered(words[1]);
                    let dst = expand_ordered(words[2]);
                    mapping.extend(src.into_iter().zip(dst));
                }
            }
        }
        Ok(UidMapping(mapping))
    }

    async fn append(
        &mut self,
        path: &str,
        data: &[u8],
        flags: MessageFlags,
    ) -> Result<Option<u32>> {
        self.select_if_needed(path).await?;
        let data = crlf_normalise(data);
        let tag = self.next_tag();
        let line = format!(
            "{tag} APPEND {} {} {{{}}}\r\n",
            astring(path),
            flag_list(flags),
            data.len()
        );
        self.write(line.as_bytes(), false).await?;
        // Wait for the continuation; a tagged answer instead is a refusal.
        loop {
            let raw = self.read_raw().await?;
            match parser::parse_response(&raw) {
                Ok(Response::Continue(_)) => break,
                Ok(Response::Tagged { tag: t, st }) if t == tag => {
                    let reply = Reply {
                        st,
                        untagged: vec![],
                    };
                    return Err(reply.fail(ErrorCode::Append));
                }
                Ok(Response::Untagged(u)) => self.dispatch(&u),
                Ok(_) => self.log_unparsed(&raw),
                Err(e) => {
                    self.log_unparsed(&raw);
                    if raw.starts_with(tag.as_bytes()) {
                        return Err(self.parse_error(e.0));
                    }
                }
            }
        }
        let mut body = Vec::with_capacity(data.len() + 2);
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\r\n");
        self.write(&body, false).await?;
        let reply = self.read_until_tagged(&tag).await?;
        if !reply.ok() {
            return Err(reply.fail(ErrorCode::Append));
        }
        let uid = reply
            .codes()
            .find(|c| c.name == "APPENDUID")
            .and_then(|c| c.arg_words().nth(1)?.parse().ok());
        Ok(uid)
    }

    async fn find_uids_of_recent_header_message_id(
        &mut self,
        path: &str,
        header_message_id: &str,
    ) -> Result<Vec<u32>> {
        // Always re-SELECT: only SELECT refreshes the message count, so on a folder already
        // selected the range would stop short of a copy filed after that SELECT.
        self.login_if_needed().await?;
        let info = self.select(path).await?;
        let count = info.exists;
        if count == 0 {
            return Ok(vec![]);
        }
        let min = count.saturating_sub(RECENT_HEADER_WINDOW).max(1);
        let spec = FetchSpec {
            by_uid: false,
            flags: false,
            headers: true,
            internal_date: false,
            gmail_labels: false,
            gmail_ids: false,
            modseq: 0,
            uid_filter: None,
        };
        let outcome = self.fetch_messages(&format!("{min}:{count}"), spec).await?;
        Ok(outcome
            .messages
            .into_iter()
            .filter(|m| {
                m.header
                    .as_ref()
                    .and_then(|h| h.message_id.as_deref())
                    .is_some_and(|id| id == header_message_id)
            })
            .map(|m| m.uid)
            .collect())
    }

    // -- IDLE ------------------------------------------------------------------------------

    async fn idle(&mut self, path: &str, interrupt: &IdleInterrupt) -> Result<()> {
        self.select_if_needed(path).await?;
        let tag = self.next_tag();
        self.write(format!("{tag} IDLE\r\n").as_bytes(), false)
            .await?;
        let mut has_data = false;
        loop {
            let raw = self.read_raw().await?;
            match parser::parse_response(&raw) {
                Ok(Response::Continue(_)) => break,
                Ok(Response::Tagged { tag: t, st }) if t == tag => {
                    let reply = Reply {
                        st,
                        untagged: vec![],
                    };
                    return Err(reply.fail(ErrorCode::Idle));
                }
                Ok(Response::Untagged(u)) => {
                    // libetpan skips the wait when EXISTS or RECENT came before the `+`.
                    has_data |= matches!(u, Untagged::Exists(_) | Untagged::Recent(_));
                    self.dispatch(&u);
                }
                Ok(_) => self.log_unparsed(&raw),
                Err(e) => {
                    self.log_unparsed(&raw);
                    if raw.starts_with(tag.as_bytes()) {
                        return Err(self.parse_error(e.0));
                    }
                }
            }
        }
        if !has_data {
            let max = Duration::from_secs(airpost_core::constants::IDLE_TIMEOUT as u64);
            let wire = self.wire()?;
            let waited = tokio::select! {
                r = wire.wait_readable() => r,
                _ = interrupt.wait() => Ok(()),
                _ = tokio::time::sleep(max) => Ok(()),
            };
            if let Err(e) = waited {
                return Err(self.stream_error(e));
            }
        }
        self.write(b"DONE\r\n", false).await?;
        let reply = self.read_until_tagged(&tag).await?;
        if !reply.ok() {
            return Err(reply.fail(ErrorCode::Idle));
        }
        Ok(())
    }
}

/// `astring`, except that the empty string is `""` (a LIST reference).
fn astring_or_empty(s: &str) -> String {
    if s.is_empty() {
        "\"\"".to_owned()
    } else {
        astring(s)
    }
}

/// libetpan's `mailstream_send_data_crlf`: CRLF stays, a bare LF or a bare CR becomes CRLF.
/// The APPEND literal is sized after this, as `mailstream_get_data_crlf_size` does.
fn crlf_normalise(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let clean = data.iter().enumerate().all(|(i, &b)| match b {
        b'\r' => data.get(i + 1) == Some(&b'\n'),
        b'\n' => i > 0 && data[i - 1] == b'\r',
        _ => true,
    });
    if clean {
        return std::borrow::Cow::Borrowed(data);
    }
    let mut out = Vec::with_capacity(data.len() + data.len() / 32);
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            b'\r' => {
                out.extend_from_slice(b"\r\n");
                if data.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            b'\n' => out.extend_from_slice(b"\r\n"),
            b => out.push(b),
        }
        i += 1;
    }
    std::borrow::Cow::Owned(out)
}

/// A FETCH response for a requested UID that carries no ENVELOPE item at all.
fn lacks_envelope(items: &[(String, Token)], spec: FetchSpec<'_>) -> bool {
    let uid = items
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("UID"))
        .and_then(|(_, v)| v.number())
        .unwrap_or(0) as u32;
    if uid == 0 || spec.uid_filter.is_some_and(|f| !f.contains(uid)) {
        return false;
    }
    !items
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case("ENVELOPE"))
}

/// `msg_att_handler`: one FETCH response to an [`ImapMessage`], or `None` when it lacks a
/// requested attribute, has no UID, or is outside the requested UIDs.
fn message_from_items(
    items: Vec<(String, Token)>,
    spec: FetchSpec<'_>,
    full_headers: bool,
) -> Option<ImapMessage> {
    let mut m = ImapMessage::default();
    let (mut has_flags, mut has_header, mut has_labels, mut has_thrid, mut has_msgid) =
        (false, false, false, false, false);
    let mut references = None;
    for (name, value) in items {
        let upper = name.to_ascii_uppercase();
        match upper.as_str() {
            "UID" => m.uid = value.number().unwrap_or(0) as u32,
            "FLAGS" => {
                has_flags = true;
                for f in value.list().unwrap_or_default() {
                    let Some(atom) = f.text() else { continue };
                    match MessageFlags::from_imap_atom(&atom) {
                        Some(flag) => m.flags |= flag,
                        None if !atom.starts_with('\\') => m.keywords.push(atom),
                        None => {}
                    }
                }
            }
            "MODSEQ" => {
                m.modseq = value
                    .list()
                    .and_then(|l| l.first())
                    .and_then(Token::number)
                    .unwrap_or(0);
            }
            "INTERNALDATE" => {
                m.internal_date = value
                    .text()
                    .and_then(|d| airpost_core::date::parse_internaldate(&d));
            }
            "X-GM-LABELS" => {
                has_labels = true;
                m.gmail_labels = value
                    .list()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(Token::text)
                    .collect();
            }
            "X-GM-THRID" => {
                has_thrid = true;
                m.gmail_thread_id = value.number();
            }
            "X-GM-MSGID" => {
                has_msgid = true;
                m.gmail_message_id = value.number();
            }
            "ENVELOPE" => {
                if let Some(env) = value.list() {
                    let mut header = envelope::header_from_envelope(env);
                    if let Some(refs) = references.take() {
                        header.references = refs;
                    }
                    m.header = Some(header);
                    has_header = true;
                }
            }
            _ if upper.starts_with("BODY[") => {
                let bytes = value.bytes().unwrap_or_default();
                if full_headers {
                    m.header = Some(envelope::header_from_section(bytes));
                    has_header = true;
                } else {
                    let refs = envelope::references_from_section(bytes);
                    match m.header.as_mut() {
                        Some(h) => h.references = refs,
                        None => references = Some(refs),
                    }
                }
            }
            _ => {}
        }
    }
    let missing = (spec.headers && !has_header)
        || (spec.flags && !has_flags)
        || (spec.gmail_ids && (!has_thrid || !has_msgid))
        || (spec.gmail_labels && !has_labels);
    if missing || m.uid == 0 {
        return None;
    }
    if let Some(filter) = spec.uid_filter {
        if !filter.contains(m.uid) {
            return None;
        }
    }
    Some(m)
}

impl ImapSession for Session {
    fn connect_if_needed(&mut self) -> Result<()> {
        self.rt.block_on(self.inner.connect_if_needed())
    }

    fn login_if_needed(&mut self) -> Result<()> {
        self.rt.block_on(self.inner.login_if_needed())
    }

    fn is_disconnected(&self) -> bool {
        self.inner.wire.is_none()
    }

    fn disconnect(&mut self) {
        if let Some(wire) = self.inner.wire.take() {
            self.rt.block_on(wire.shutdown());
        }
        self.inner.teardown();
        self.inner.set_up = false;
    }

    fn set_auth(&mut self, auth: ImapAuth) {
        self.inner.config.auth = auth;
    }

    fn capabilities(&self) -> &Capabilities {
        &self.inner.caps
    }

    fn profile(&self) -> &ServerProfile {
        &self.inner.profile
    }

    fn default_namespace(&self) -> Option<&Namespace> {
        self.inner.namespace.as_ref()
    }

    fn is_qresync_enabled(&self) -> bool {
        self.inner.qresync
    }

    fn message_limit(&self) -> Option<u32> {
        self.inner
            .caps
            .value_of("MESSAGELIMIT")
            .and_then(|v| v.parse().ok())
    }

    fn current_folder(&self) -> Option<&str> {
        if self.inner.state == State::Selected {
            self.inner.current_folder.as_deref()
        } else {
            None
        }
    }

    fn last_tls_error(&self) -> Option<&str> {
        self.inner.last_tls_error.as_deref()
    }

    fn list_folders(&mut self) -> Result<Vec<ListedFolder>> {
        self.rt.block_on(self.inner.list_folders())
    }

    fn select(&mut self, path: &str) -> Result<SelectInfo> {
        self.rt.block_on(self.inner.select(path))
    }

    fn allows_new_keywords(&self, path: &str) -> Option<bool> {
        self.inner.new_keywords.get(path).copied()
    }

    fn select_if_needed(&mut self, path: &str) -> Result<()> {
        self.rt.block_on(self.inner.select_if_needed(path))
    }

    fn noop(&mut self) -> Result<()> {
        self.rt.block_on(self.inner.noop())
    }

    fn status(&mut self, path: &str) -> Result<FolderStatus> {
        self.rt.block_on(self.inner.status(path))
    }

    fn create_folder(&mut self, path: &str) -> Result<()> {
        self.rt.block_on(self.inner.create_folder(path))
    }

    fn rename_folder(&mut self, path: &str, new_path: &str) -> Result<()> {
        self.rt.block_on(self.inner.rename_folder(path, new_path))
    }

    fn delete_folder(&mut self, path: &str) -> Result<()> {
        self.rt.block_on(self.inner.delete_folder(path))
    }

    fn fetch_by_uid(&mut self, path: &str, kind: FetchKind, uids: &UidSet) -> Result<FetchResult> {
        let o = self
            .rt
            .block_on(self.inner.fetch_by_uid(path, kind, uids, 0))?;
        Ok(FetchResult {
            messages: o.messages,
            hit_message_limit: o.hit_message_limit,
        })
    }

    fn sync_by_uid(
        &mut self,
        path: &str,
        kind: FetchKind,
        uids: &UidSet,
        modseq: u64,
    ) -> Result<SyncResult> {
        let o = self
            .rt
            .block_on(self.inner.fetch_by_uid(path, kind, uids, modseq))?;
        Ok(SyncResult {
            modified_or_added: o.messages,
            vanished: o.vanished,
            hit_message_limit: o.hit_message_limit,
        })
    }

    fn take_vanished(&mut self, path: &str) -> UidSet {
        let key = self
            .inner
            .vanished
            .keys()
            .find(|k| k.eq_ignore_ascii_case(path))
            .cloned();
        key.and_then(|k| self.inner.vanished.remove(&k))
            .unwrap_or_default()
    }

    fn fetch_message_bytes(&mut self, path: &str, uid: u32) -> Result<Option<Vec<u8>>> {
        self.rt.block_on(self.inner.fetch_message_bytes(path, uid))
    }

    fn store_flags(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        flags: MessageFlags,
    ) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        let item = format!("{}FLAGS.SILENT {}", store_sign(op), flag_list(flags));
        self.rt
            .block_on(self.inner.store(path, uids, STORE_FLAGS_CHUNK, &item))
    }

    fn store_keywords(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        keywords: &[String],
    ) -> Result<()> {
        if op == StoreOp::Set {
            return Err(MailError::new(
                ErrorCode::Store,
                "a keyword STORE adds or removes; FLAGS would replace the system flags",
            ));
        }
        if let Some(bad) = keywords
            .iter()
            .find(|k| !airpost_core::keywords::is_keyword_atom(k))
        {
            return Err(MailError::new(
                ErrorCode::Store,
                format!("not a keyword: {bad:?}"),
            ));
        }
        if uids.is_empty() || keywords.is_empty() {
            return Ok(());
        }
        let item = format!("{}FLAGS.SILENT ({})", store_sign(op), keywords.join(" "));
        self.rt
            .block_on(self.inner.store(path, uids, STORE_FLAGS_CHUNK, &item))
    }

    fn store_labels(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        labels: &[String],
    ) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        let labels: Vec<String> = labels.iter().map(|l| astring(l)).collect();
        let item = format!(
            "{}X-GM-LABELS.SILENT ({})",
            store_sign(op),
            labels.join(" ")
        );
        self.rt
            .block_on(self.inner.store(path, uids, COMMAND_CHUNK, &item))
    }

    fn move_uids(&mut self, path: &str, uids: &UidSet, dest_path: &str) -> Result<UidMapping> {
        if uids.is_empty() {
            return Ok(UidMapping::default());
        }
        self.rt
            .block_on(self.inner.copy_or_move("MOVE", path, uids, dest_path))
    }

    fn copy_uids(&mut self, path: &str, uids: &UidSet, dest_path: &str) -> Result<UidMapping> {
        if uids.is_empty() {
            return Ok(UidMapping::default());
        }
        self.rt
            .block_on(self.inner.copy_or_move("COPY", path, uids, dest_path))
    }

    fn expunge(&mut self, path: &str) -> Result<()> {
        self.rt
            .block_on(self.inner.in_folder(path, "EXPUNGE", ErrorCode::Expunge))
    }

    fn expunge_uids(&mut self, path: &str, uids: &UidSet) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        let cmd = format!("UID EXPUNGE {uids}");
        self.rt
            .block_on(self.inner.in_folder(path, &cmd, ErrorCode::Expunge))
    }

    fn append(&mut self, path: &str, data: &[u8], flags: MessageFlags) -> Result<Option<u32>> {
        self.rt.block_on(self.inner.append(path, data, flags))
    }

    fn find_uids_of_recent_header_message_id(
        &mut self,
        path: &str,
        header_message_id: &str,
    ) -> Result<Vec<u32>> {
        self.rt.block_on(
            self.inner
                .find_uids_of_recent_header_message_id(path, header_message_id),
        )
    }

    fn supports_idle(&self) -> bool {
        self.inner.caps.has("IDLE") || self.inner.profile.force_idle
    }

    fn idle(&mut self, path: &str, interrupt: &IdleInterrupt) -> Result<()> {
        self.rt.block_on(self.inner.idle(path, interrupt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn astrings() {
        assert_eq!(astring("INBOX"), "INBOX");
        assert_eq!(astring("Sent-Items"), "Sent-Items");
        assert_eq!(astring("Temp/Sub"), "\"Temp/Sub\"");
        assert_eq!(astring("[Gmail]/All Mail"), "\"[Gmail]/All Mail\"");
        assert_eq!(astring("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(astring(""), "\"\"");
        assert_eq!(astring("\\Inbox"), "\"\\\\Inbox\"");
        assert_eq!(astring_or_empty(""), "\"\"");
    }

    #[test]
    fn crlf() {
        let n = |d: &[u8]| crlf_normalise(d).into_owned();
        assert!(matches!(
            crlf_normalise(b"a\r\nb\r\n"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(n(b"a\nb\n"), b"a\r\nb\r\n");
        assert_eq!(n(b"\na\r\n\n"), b"\r\na\r\n\r\n");
        assert_eq!(n(b"a\rb\r"), b"a\r\nb\r\n");
        assert_eq!(n(b"a\r\r\nb"), b"a\r\n\r\nb");
        assert_eq!(n(b""), b"");
    }

    #[test]
    fn ranges() {
        let set = UidSet::parse("1:3,5,9:*").unwrap();
        assert_eq!(format_ranges(set.ranges()), "1:3,5,9:*");
        assert_eq!(expand_ordered("304,319:320"), vec![304, 319, 320]);
        assert_eq!(expand_ordered("9,3:2"), vec![9, 2, 3]);
    }

    #[test]
    fn login_errors() {
        assert_eq!(
            login_error_code("[ALERT] Application-specific password required: x"),
            ErrorCode::GmailApplicationSpecificPasswordRequired
        );
        assert_eq!(
            login_error_code("Invalid credentials (Failure)"),
            ErrorCode::Authentication
        );
        assert_eq!(login_error_code("Error OCF12"), ErrorCode::YahooUnavailable);
        assert_eq!(
            login_error_code("Login to your account via a web browser"),
            ErrorCode::OutlookLoginViaWebBrowser
        );
    }

    #[test]
    fn flags() {
        assert_eq!(flag_list(MessageFlags::empty()), "()");
        assert_eq!(
            flag_list(MessageFlags::SEEN | MessageFlags::FORWARDED),
            "(\\Seen $Forwarded)"
        );
    }

    fn items(raw: &str) -> Vec<(String, Token)> {
        match parser::parse_response(raw.as_bytes()).unwrap() {
            Response::Untagged(Untagged::Fetch { items, .. }) => items,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn messages_from_fetch() {
        let all = UidSet::range(1, None);
        let heavy = FetchSpec {
            by_uid: true,
            flags: true,
            headers: true,
            internal_date: true,
            gmail_labels: false,
            gmail_ids: false,
            modseq: 0,
            uid_filter: Some(&all),
        };
        let m = message_from_items(
            items("* 1 FETCH (UID 7 FLAGS (\\Seen $Forwarded $Junk \\Recent) MODSEQ (42) INTERNALDATE \"05-Jan-2026 09:00:00 +0000\" BODY[HEADER.FIELDS (References)] {21}\r\nReferences: <a@b>\r\n\r\n ENVELOPE (NIL \"s\" NIL NIL NIL NIL NIL NIL NIL \"<m@b>\"))\r\n"),
            heavy,
            false,
        )
        .unwrap();
        assert_eq!(m.uid, 7);
        assert_eq!(m.flags, MessageFlags::SEEN | MessageFlags::FORWARDED);
        assert_eq!(m.keywords, vec!["$Junk"]);
        assert_eq!(m.modseq, 42);
        assert_eq!(m.internal_date, Some(1767603600));
        let h = m.header.unwrap();
        assert_eq!(h.references, vec!["a@b"]);
        assert_eq!(h.message_id.as_deref(), Some("m@b"));

        // Missing ENVELOPE, missing FLAGS, UID 0, outside the filter.
        assert!(
            message_from_items(items("* 1 FETCH (UID 7 FLAGS ())\r\n"), heavy, false).is_none()
        );
        let light = FetchSpec {
            headers: false,
            internal_date: false,
            ..heavy
        };
        assert!(message_from_items(items("* 1 FETCH (UID 7)\r\n"), light, false).is_none());
        assert!(
            message_from_items(items("* 1 FETCH (UID 0 FLAGS ())\r\n"), light, false).is_none()
        );
        let some = UidSet::single(3);
        let filtered = FetchSpec {
            uid_filter: Some(&some),
            ..light
        };
        assert!(
            message_from_items(items("* 1 FETCH (UID 7 FLAGS ())\r\n"), filtered, false).is_none()
        );
        assert!(
            message_from_items(items("* 1 FETCH (UID 3 FLAGS ())\r\n"), filtered, false).is_some()
        );

        // Gmail attributes are required when asked for.
        let gmail = FetchSpec {
            gmail_labels: true,
            gmail_ids: true,
            ..heavy
        };
        let m = message_from_items(
            items("* 1 FETCH (X-GM-THRID 11 X-GM-MSGID 12 X-GM-LABELS (\\Inbox \"My Label\") UID 5 FLAGS () ENVELOPE (NIL NIL NIL NIL NIL NIL NIL NIL NIL NIL) BODY[HEADER.FIELDS (REFERENCES)] NIL)\r\n"),
            gmail,
            false,
        )
        .unwrap();
        assert_eq!(m.gmail_labels, vec!["\\Inbox", "My Label"]);
        assert_eq!(m.gmail_thread_id, Some(11));
        assert_eq!(m.gmail_message_id, Some(12));
        assert!(message_from_items(
            items("* 1 FETCH (UID 5 FLAGS () X-GM-LABELS ())\r\n"),
            gmail,
            false
        )
        .is_none());

        // FullHeaders mode reads the header section.
        let m = message_from_items(
            items("* 1 FETCH (UID 9 FLAGS () BODY[HEADER.FIELDS (DATE SUBJECT FROM)] {31}\r\nSubject: hi\r\nFrom: a@b.test\r\n\r\n)\r\n"),
            heavy,
            true,
        )
        .unwrap();
        let h = m.header.unwrap();
        assert_eq!(h.subject.as_deref(), Some("hi"));
        assert_eq!(h.from[0].email, "a@b.test");
    }
}
