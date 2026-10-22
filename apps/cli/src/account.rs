// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern account add`: an account on the serve engine.
//! IMAP accounts get the servers `account.discover` finds, the flags' changes and a
//! password; Gmail, Office 365 and Outlook sign in in the browser, whose redirect this
//! command receives on 127.0.0.1 and passes to `account.oauth.complete`. With `--paste` (a
//! server, where the browser runs on another machine) nothing listens: the redirect to
//! 127.0.0.1 fails to load in the browser and the user pastes its address instead. The
//! engine tests the account before it stores it, and keeps the secrets: none is printed.

use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tern_ipc::{code, feature, method, AccountListResult, AccountStatus, ClientError};

use crate::app::{confirm, Ctx};
use crate::cli::{AccountAddArgs, AccountUpdateArgs, SignInArgs};
use crate::discovery::Engine;
use crate::error::{CliError, Result};
use crate::output;

/// The providers that sign in with OAuth.
const OAUTH_PROVIDERS: &[&str] = &["gmail", "office365", "outlook"];
/// The most of a redirect request read.
const MAX_REQUEST_BYTES: usize = 16 * 1024;
/// How long a reset, or a removal that deletes the account's rows, may take.
const SLOW_CALL_TIMEOUT: Duration = Duration::from_secs(600);

fn setup_engine(engines: &mut [Engine]) -> Result<&mut Engine> {
    engines
        .iter_mut()
        .find(|e| e.has_feature(feature::ACCOUNT_SETUP))
        .ok_or_else(|| {
            CliError::failed(
                "the running engine doesn't add accounts; update Tern (account setup needs its \
                 serve engine)",
            )
        })
}

fn call(engine: &mut Engine, method: &str, params: Value) -> Result<Value> {
    let pid = engine.entry.pid;
    engine
        .client
        .call(method, params)
        .map_err(|e| failed(pid, method, e))
}

/// A failed test or sign-in in words; anything else as [`CliError::call`] puts it.
fn failed(pid: u32, method: &str, e: ClientError) -> CliError {
    if let ClientError::Rpc(err) = &e {
        if err.code == code::ACCOUNT_TEST_FAILED || err.code == code::OAUTH_FAILED {
            let mut msg = err.message.clone();
            let advice = err.data.as_ref().and_then(|d| d.get("error_advice"));
            if let Some(advice) = advice.and_then(Value::as_str) {
                msg.push_str(&format!("\n{advice}"));
            }
            return CliError::failed(msg);
        }
    }
    CliError::call(&format!("engine pid {pid}"), method, e)
}

pub fn add(ctx: &Ctx, engines: &mut [Engine], args: &AccountAddArgs) -> Result<()> {
    let email = args.email.trim();
    if !email.contains('@') {
        return Err(CliError::usage(format!(
            "{email:?} is not an email address"
        )));
    }
    let engine = setup_engine(engines)?;
    let provider = args.provider.as_str();
    let added = if OAUTH_PROVIDERS.contains(&provider) {
        if args.has_server_flags() || args.password_stdin {
            return Err(CliError::usage(format!(
                "{provider} accounts sign in in the browser: no servers or password to give"
            )));
        }
        let request = OAuthRequest {
            provider,
            shared_mailbox: args.shared_mailbox.as_deref(),
            account_id: None,
        };
        sign_in(engine, email, &request, &args.sign_in)?
    } else {
        if args.shared_mailbox.is_some() {
            return Err(CliError::usage(
                "--shared-mailbox is for office365 accounts",
            ));
        }
        if args.sign_in.paste {
            return Err(CliError::usage(
                "--paste is for accounts that sign in in the browser (gmail, office365, outlook)",
            ));
        }
        with_password(engine, email, args)?
    };
    let account = added.get("account").cloned().unwrap_or(Value::Null);
    if ctx.json {
        output::json(&added);
        return Ok(());
    }
    let id = added["accountId"].as_str().unwrap_or_default();
    let state = added["state"].as_str().unwrap_or_default();
    let address = account["emailAddress"].as_str().unwrap_or(email);
    output::line(&format!("Added {address} ({id}), {state}"));
    if added["stored"] == json!(false) {
        output::warn(
            "the engine could not store the account's credentials; it runs until the engine \
             stops",
        );
    }
    Ok(())
}

/// The accounts the engine has registered, running or not, in `account.list` order.
fn registered(engine: &mut Engine) -> Result<Vec<AccountStatus>> {
    let list: AccountListResult =
        serde_json::from_value(call(engine, method::ACCOUNT_LIST, json!({}))?)
            .map_err(|e| CliError::failed(format!("odd account.list result: {e}")))?;
    Ok(list.accounts)
}

fn find<'a>(list: &'a [AccountStatus], want: &str) -> Result<&'a AccountStatus> {
    list.iter()
        .find(|a| a.account_id == want || a.email.eq_ignore_ascii_case(want))
        .ok_or_else(|| {
            CliError::failed(format!(
                "the engine has no account {want}; `tern accounts` lists them"
            ))
        })
}

/// A long call (a reset, a removal that deletes rows) without the usual read timeout.
fn call_slow(engine: &mut Engine, method: &str, params: Value) -> Result<Value> {
    let set = |e: &Engine, t| {
        e.client
            .set_read_timeout(t)
            .map_err(|err| CliError::failed(err.to_string()))
    };
    set(engine, Some(SLOW_CALL_TIMEOUT))?;
    let r = call(engine, method, params);
    set(engine, Some(tern_client::CALL_TIMEOUT))?;
    r
}

// TODO(cli-parity): a running desktop ignores the `removed` account.state another client
// causes, so its window keeps listing the account until it restarts.
pub fn remove(
    ctx: &Ctx,
    engines: &mut [Engine],
    account: &str,
    delete_data: bool,
    yes: bool,
) -> Result<()> {
    let engine = setup_engine(engines)?;
    let list = registered(engine)?;
    let a = find(&list, account)?;
    let (id, email) = (a.account_id.clone(), a.email.clone());
    let question = if delete_data {
        format!("Remove {email} ({id}) from Tern and delete its downloaded mail?")
    } else {
        format!("Remove {email} ({id}) from Tern?")
    };
    confirm(ctx, yes, &question)?;
    let r = call_slow(
        engine,
        method::ACCOUNT_REMOVE,
        json!({ "accountId": id, "revoke": true, "deleteData": delete_data }),
    )?;
    if ctx.json {
        output::json(&json!({
            "accountId": id, "email": email, "removed": true, "dataDeleted": delete_data,
            "stillRunning": r["stillRunning"],
        }));
    } else if delete_data {
        output::line(&format!(
            "Removed {email} ({id}) and deleted its downloaded mail"
        ));
    } else {
        output::line(&format!(
            "Removed {email} ({id}); its downloaded mail stays on this computer"
        ));
    }
    Ok(())
}

/// `Account._makeAlias` (Preferences > Accounts): `Name <address>`, the account's name or
/// address standing in for the part not given.
fn make_alias(input: &str, name: &str, email: &str) -> String {
    let input = input.trim();
    let Some((at_start, at_end)) = address_in(input) else {
        let shown = if input.is_empty() { name } else { input };
        return format!("{shown} <{email}>");
    };
    let address = &input[at_start..at_end];
    let before = input[..at_start]
        .trim()
        .trim_end_matches('<')
        .trim()
        .trim_matches('"')
        .trim();
    let shown = if before.is_empty() { name } else { before };
    format!("{shown} <{address}>")
}

/// Where the first address in `s` starts and ends.
fn address_in(s: &str) -> Option<(usize, usize)> {
    let at = s.find('@')?;
    let is_part = |c: char| !c.is_whitespace() && !"<>\"(),;:".contains(c);
    let start = s[..at]
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_part(c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = s[at..]
        .char_indices()
        .find(|&(_, c)| !is_part(c))
        .map_or(s.len(), |(i, _)| at + i);
    (start < at && end > at + 1).then_some((start, end))
}

/// The alias of `aliases` that `want` names, by its text or its address.
fn find_alias<'a>(aliases: &'a [String], want: &str) -> Option<&'a String> {
    let address = |a: &str| address_in(a).map(|(s, e)| a[s..e].to_lowercase());
    let want_address = address(want);
    aliases.iter().find(|a| {
        a.eq_ignore_ascii_case(want.trim())
            || (want_address.is_some() && address(a) == want_address)
    })
}

/// The `account.update` patch for `args` over the account JSON.
fn update_patch(account: &Value, args: &AccountUpdateArgs) -> Result<Value> {
    let email = account["emailAddress"].as_str().unwrap_or_default();
    let mut name = account["name"].as_str().unwrap_or_default().to_owned();
    let mut patch = json!({});
    if let Some(n) = &args.name {
        name = n.trim().to_owned();
        patch["name"] = json!(name);
    }
    let mut aliases: Vec<String> = account["aliases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_str().map(str::to_owned))
        .collect();
    let mut default = account["defaultAlias"].as_str().map(str::to_owned);
    let changes_aliases = !args.add_aliases.is_empty() || !args.remove_aliases.is_empty();
    for gone in &args.remove_aliases {
        let Some(found) = find_alias(&aliases, gone).cloned() else {
            return Err(CliError::failed(format!(
                "{email} has no alias {gone:?}; its aliases: {}",
                if aliases.is_empty() {
                    "none".to_owned()
                } else {
                    aliases.join(", ")
                }
            )));
        };
        aliases.retain(|a| *a != found);
        if default.as_deref() == Some(found.as_str()) {
            default = None;
        }
    }
    for new in &args.add_aliases {
        let alias = make_alias(new, &name, email);
        if !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }
    if let Some(want) = &args.default_alias {
        default = if want.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(
                find_alias(&aliases, want)
                    .ok_or_else(|| {
                        CliError::failed(format!(
                            "{email} has no alias {want:?}; add it with --alias first"
                        ))
                    })?
                    .clone(),
            )
        };
    }
    if changes_aliases {
        patch["aliases"] = json!(aliases);
    }
    if changes_aliases || args.default_alias.is_some() {
        patch["defaultAlias"] = json!(default);
    }
    if patch.as_object().is_some_and(|p| p.is_empty()) {
        return Err(CliError::usage(
            "nothing to change: give --name, --alias, --remove-alias or --default-alias",
        ));
    }
    Ok(patch)
}

pub fn update(ctx: &Ctx, engines: &mut [Engine], args: &AccountUpdateArgs) -> Result<()> {
    let engine = setup_engine(engines)?;
    let list = registered(engine)?;
    let a = find(&list, &args.account)?;
    let account = a.account.clone().unwrap_or(Value::Null);
    let patch = update_patch(&account, args)?;
    let id = a.account_id.clone();
    let r = call(
        engine,
        method::ACCOUNT_UPDATE,
        json!({ "accountId": id, "patch": patch }),
    )?;
    if ctx.json {
        output::json(&r);
        return Ok(());
    }
    let updated = &r["account"];
    output::line(&format!(
        "Updated {} ({id})",
        updated["emailAddress"].as_str().unwrap_or(&a.email)
    ));
    output::line(&format!(
        "Name: {}",
        updated["name"].as_str().unwrap_or_default()
    ));
    let aliases: Vec<&str> = updated["aliases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !aliases.is_empty() {
        output::line(&format!("Aliases: {}", aliases.join(", ")));
    }
    if let Some(d) = updated["defaultAlias"].as_str() {
        output::line(&format!("Default alias: {d}"));
    }
    Ok(())
}

// TODO(cli-parity): the desktop keeps its own account order and doesn't follow
// `account.reorder` from another client.
pub fn reorder(ctx: &Ctx, engines: &mut [Engine], wanted: &[String]) -> Result<()> {
    let engine = setup_engine(engines)?;
    let list = registered(engine)?;
    let mut ids: Vec<String> = Vec::new();
    for w in wanted {
        let id = find(&list, w)?.account_id.clone();
        if ids.contains(&id) {
            return Err(CliError::usage(format!("{w} is named twice")));
        }
        ids.push(id);
    }
    call(engine, method::ACCOUNT_REORDER, json!({ "ids": ids }))?;
    let order: Vec<(String, String)> = registered(engine)?
        .into_iter()
        .map(|a| (a.account_id, a.email))
        .collect();
    if ctx.json {
        let accounts: Vec<Value> = order
            .iter()
            .map(|(id, email)| json!({ "accountId": id, "email": email }))
            .collect();
        output::json(&json!({ "accounts": accounts }));
    } else {
        for (i, (id, email)) in order.iter().enumerate() {
            output::line(&format!("{}. {email} ({id})", i + 1));
        }
    }
    Ok(())
}

pub fn resync(ctx: &Ctx, engines: &mut [Engine], account: &str, yes: bool) -> Result<()> {
    let engine = setup_engine(engines)?;
    let list = registered(engine)?;
    let a = find(&list, account)?;
    let (id, email) = (a.account_id.clone(), a.email.clone());
    confirm(
        ctx,
        yes,
        &format!("Delete the downloaded mail of {email} ({id}) and download it again?"),
    )?;
    let r = call_slow(
        engine,
        method::ACCOUNT_RESET,
        json!({ "accountId": id, "keepMetadata": true }),
    )?;
    if ctx.json {
        let mut doc = json!({ "accountId": id, "email": email, "reset": true });
        if let Some(state) = r.get("state") {
            doc["state"] = state.clone();
        }
        output::json(&doc);
    } else {
        output::line(&format!(
            "Rebuilding {email} ({id}): its mail downloads again (`tern status` shows the state)"
        ));
    }
    Ok(())
}

pub fn reauth(
    ctx: &Ctx,
    engines: &mut [Engine],
    account: &str,
    password_stdin: bool,
    args: &SignInArgs,
) -> Result<()> {
    let engine = setup_engine(engines)?;
    let list = registered(engine)?;
    let a = find(&list, account)?;
    let (id, email) = (a.account_id.clone(), a.email.clone());
    let json_of = a.account.clone().unwrap_or(Value::Null);
    let provider = json_of["provider"].as_str().unwrap_or("imap").to_owned();
    let done = if OAUTH_PROVIDERS.contains(&provider.as_str()) {
        if password_stdin {
            return Err(CliError::usage(format!(
                "{provider} accounts sign in in the browser: no password to give"
            )));
        }
        let request = OAuthRequest {
            provider: &provider,
            shared_mailbox: None,
            account_id: Some(&id),
        };
        sign_in(engine, &email, &request, args)?
    } else {
        if args.paste {
            return Err(CliError::usage(
                "--paste is for accounts that sign in in the browser (gmail, office365, outlook)",
            ));
        }
        let password = read_password(&email, password_stdin)?;
        call(
            engine,
            method::ACCOUNT_SET_SECRETS,
            json!({
                "accountId": id,
                "secrets": { "imap_password": password, "smtp_password": password },
                "test": true,
            }),
        )?
    };
    if ctx.json {
        output::json(&json!({
            "accountId": id, "email": email, "state": done["state"], "stored": done["stored"],
        }));
    } else {
        let state = done["state"].as_str().unwrap_or("running");
        output::line(&format!("Signed in to {email} ({id}) again, {state}"));
    }
    Ok(())
}

/// IMAP: discovery, the flags, a password, then `account.add {test}`.
fn with_password(engine: &mut Engine, email: &str, args: &AccountAddArgs) -> Result<Value> {
    let found = call(
        engine,
        method::ACCOUNT_DISCOVER,
        json!({ "email": email, "provider": args.provider }),
    )?;
    let mut account = found["account"].clone();
    apply_flags(&mut account, args);
    let host = account
        .pointer("/settings/imap_host")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if host.is_empty() {
        return Err(CliError::usage(format!(
            "no IMAP server found for {email}; give --imap-host and --smtp-host"
        )));
    }
    let password = read_password(email, args.password_stdin)?;
    let mut added = call(
        engine,
        method::ACCOUNT_ADD,
        json!({
            "account": account,
            "secrets": { "imap_password": password, "smtp_password": password },
            "test": true,
            "identity": null,
        }),
    )?;
    added["account"] = account;
    Ok(added)
}

/// The server flags over what discovery found.
fn apply_flags(account: &mut Value, args: &AccountAddArgs) {
    if let Some(name) = &args.name {
        account["name"] = json!(name);
    }
    let Some(settings) = account.get_mut("settings").and_then(Value::as_object_mut) else {
        return;
    };
    let mut set = |key: &str, v: Option<Value>| {
        if let Some(v) = v {
            settings.insert(key.to_owned(), v);
        }
    };
    set("imap_host", args.imap_host.as_ref().map(|v| json!(v)));
    set("imap_port", args.imap_port.map(|v| json!(v)));
    set("imap_security", args.imap_security.as_deref().map(security));
    set(
        "imap_username",
        args.imap_username.as_ref().map(|v| json!(v)),
    );
    set("smtp_host", args.smtp_host.as_ref().map(|v| json!(v)));
    set("smtp_port", args.smtp_port.map(|v| json!(v)));
    set("smtp_security", args.smtp_security.as_deref().map(security));
    set(
        "smtp_username",
        args.smtp_username.as_ref().map(|v| json!(v)),
    );
}

/// The engine's names for `tls` and `starttls`.
fn security(flag: &str) -> Value {
    json!(if flag == "tls" {
        "SSL / TLS"
    } else {
        "STARTTLS"
    })
}

/// One line from stdin with `--password-stdin`, else a prompt without echo on a terminal.
fn read_password(email: &str, from_stdin: bool) -> Result<String> {
    let stdin = std::io::stdin();
    if !from_stdin && !stdin.is_terminal() {
        return Err(CliError::usage(
            "no terminal to ask for the password on; pipe it with --password-stdin",
        ));
    }
    if !from_stdin {
        let _ = write!(std::io::stderr(), "Password for {email}: ");
        let _ = std::io::stderr().flush();
    }
    let echo = (!from_stdin).then(EchoOff::new).flatten();
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    drop(echo);
    if !from_stdin {
        let _ = writeln!(std::io::stderr());
    }
    read.map_err(|e| CliError::failed(format!("could not read the password: {e}")))?;
    let password = line.trim_end_matches(['\r', '\n']).to_owned();
    if password.is_empty() {
        return Err(CliError::usage("the password is empty"));
    }
    Ok(password)
}

/// Turns the terminal's echo off while it lives.
struct EchoOff {
    #[cfg(unix)]
    saved: libc::termios,
}

impl EchoOff {
    #[cfg(unix)]
    fn new() -> Option<Self> {
        // SAFETY: tcgetattr fills a termios, plain data, zeroed first.
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut t) } != 0 {
            return None;
        }
        let saved = t;
        t.c_lflag &= !libc::ECHO;
        // SAFETY: as above, with the struct tcgetattr filled.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t) } != 0 {
            return None;
        }
        Some(EchoOff { saved })
    }

    #[cfg(not(unix))]
    fn new() -> Option<Self> {
        None
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: restores the termios tcgetattr returned.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved);
        }
    }
}

// -- OAuth -------------------------------------------------------------------------------------

/// What `account.oauth.begin` is asked for.
struct OAuthRequest<'a> {
    provider: &'a str,
    shared_mailbox: Option<&'a str>,
    /// Reconnect: the registered account the sign-in renews.
    account_id: Option<&'a str>,
}

/// The browser sign-in: `begin`, the redirect on 127.0.0.1 (or pasted), `complete`.
fn sign_in(
    engine: &mut Engine,
    email: &str,
    request: &OAuthRequest,
    args: &SignInArgs,
) -> Result<Value> {
    // Bound first, so a port in use fails before the browser opens.
    let listener = if args.paste {
        None
    } else {
        Some(TcpListener::bind(("127.0.0.1", args.port)).map_err(|e| {
            CliError::failed(format!(
                "could not listen on 127.0.0.1:{} for the sign-in ({e}); try another --port",
                args.port
            ))
        })?)
    };
    let port = match &listener {
        Some(l) => l
            .local_addr()
            .map_err(|e| CliError::failed(e.to_string()))?
            .port(),
        None => args.port,
    };
    let mut params = json!({ "provider": request.provider, "redirectPort": port });
    if let Some(shared) = request.shared_mailbox {
        params["sharedMailbox"] = json!(shared);
    }
    if let Some(id) = request.account_id {
        params["accountId"] = json!(id);
    }
    let begun = call(engine, method::ACCOUNT_OAUTH_BEGIN, params)?;
    let (Some(url), Some(state), Some(session)) = (
        begun["authUrl"].as_str(),
        begun["state"].as_str(),
        begun["sessionId"].as_str(),
    ) else {
        return Err(CliError::failed(format!(
            "odd account.oauth.begin result: {begun}"
        )));
    };
    let code = match &listener {
        Some(listener) => {
            let _ = writeln!(
                std::io::stderr(),
                "Sign in to {email} in your browser; if it doesn't open, open this address:\n{url}"
            );
            if !args.no_browser {
                open_browser(url);
            }
            receive_redirect(listener, state, Duration::from_secs(args.timeout))?
        }
        None => {
            let _ = writeln!(
                std::io::stderr(),
                "Open this address in a browser on any computer and sign in to {email}:\n{url}\n\n\
                 The browser then goes to http://127.0.0.1:{port}/..., which won't load. Copy \
                 that address from the address bar and paste it here:"
            );
            pasted_redirect(&mut std::io::stdin().lock(), state)?
        }
    };
    let done = call(
        engine,
        method::ACCOUNT_OAUTH_COMPLETE,
        json!({ "sessionId": session, "code": code, "state": state }),
    )?;
    let signed_in = done
        .pointer("/account/emailAddress")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !signed_in.eq_ignore_ascii_case(email) {
        output::warn(&format!("signed in as {signed_in}, not {email}"));
    }
    Ok(done)
}

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let spawned = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if spawned.is_err() {
        output::warn(&format!(
            "could not run {opener}; open the address yourself"
        ));
    }
}

/// Waits for the provider's redirect carrying `state` and returns its code. Other requests
/// (a favicon, a stale tab of another sign-in) are answered and ignored.
fn receive_redirect(listener: &TcpListener, state: &str, timeout: Duration) -> Result<String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| CliError::failed(e.to_string()))?;
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(CliError::failed(format!(
                "no sign-in within {}s; run the command again",
                timeout.as_secs()
            )));
        }
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => {
                return Err(CliError::failed(format!(
                    "the sign-in receiver failed: {e}"
                )))
            }
        };
        match answer(stream, state) {
            Some(Ok(code)) => return Ok(code),
            Some(Err(e)) => return Err(e),
            None => {}
        }
    }
}

/// What a redirect's query says about the sign-in waiting for `state`.
#[derive(Debug, PartialEq, Eq)]
enum Redirect {
    Code(String),
    /// The provider's `error`: cancelled or refused.
    Refused(String),
    /// A redirect with another sign-in's state.
    OtherSignIn,
    Unrelated,
}

fn redirect_outcome(query: &str, state: &str) -> Redirect {
    let query = parse_query(query);
    let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    match (get("state"), get("code"), get("error")) {
        (Some(s), _, Some(error)) if s == state => Redirect::Refused(error.to_owned()),
        (Some(s), Some(code), None) if s == state && !code.is_empty() => {
            Redirect::Code(code.to_owned())
        }
        (Some(_), _, _) => Redirect::OtherSignIn,
        _ => Redirect::Unrelated,
    }
}

fn refused(error: &str) -> CliError {
    CliError::failed(format!("the sign-in failed: {error}"))
}

/// `--paste`: the address the browser was sent to, read from `input`.
fn pasted_redirect(input: &mut impl BufRead, state: &str) -> Result<String> {
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|e| CliError::failed(format!("could not read the address: {e}")))?;
    let address = line.trim();
    let query = address.split_once('?').map_or("", |(_, q)| q);
    let query = query.split_once('#').map_or(query, |(q, _)| q);
    match redirect_outcome(query, state) {
        Redirect::Code(code) => Ok(code),
        Redirect::Refused(error) => Err(refused(&error)),
        Redirect::OtherSignIn => Err(CliError::failed(
            "that address is from another sign-in; run the command again and use its address",
        )),
        Redirect::Unrelated => Err(CliError::failed(format!(
            "{address:?} is not the sign-in's redirect (it has no code and state)"
        ))),
    }
}

/// One request to the receiver: `Some` with the code, or with the provider's error, when it
/// is the redirect of this sign-in.
fn answer(mut stream: TcpStream, state: &str) -> Option<Result<String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    let mut reader = BufReader::new(stream.try_clone().ok()?).take(MAX_REQUEST_BYTES as u64);
    reader.read_line(&mut line).ok()?;
    // The headers are read so the browser isn't reset before it reads the page.
    let mut header = String::new();
    while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
        header.clear();
    }
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let query = target.split_once('?').map_or("", |(_, q)| q);
    let (status, page, outcome) = match redirect_outcome(query, state) {
        Redirect::Refused(error) => (
            "200 OK",
            "The sign-in was cancelled or refused. You can close this window.",
            Some(Err(refused(&error))),
        ),
        Redirect::Code(code) => (
            "200 OK",
            "Signed in. You can close this window and go back to the terminal.",
            Some(Ok(code)),
        ),
        Redirect::OtherSignIn => (
            "400 Bad Request",
            "This sign-in response is for another sign-in.",
            None,
        ),
        Redirect::Unrelated => ("404 Not Found", "Not found.", None),
    };
    let body = format!("<!doctype html><meta charset=utf-8><title>Tern</title><p>{page}</p>");
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    outcome
}

/// `application/x-www-form-urlencoded` pairs.
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_override_discovery() {
        use clap::Parser;
        let cli = crate::cli::Cli::try_parse_from([
            "tern",
            "account",
            "add",
            "a@b.test",
            "--imap-security",
            "tls",
            "--smtp-port",
            "587",
            "--smtp-security",
            "starttls",
            "--name",
            "A",
        ])
        .unwrap();
        let crate::cli::Command::Account {
            command: crate::cli::AccountCommand::Add(args),
        } = cli.command
        else {
            panic!("not account add");
        };
        let mut account = json!({"settings": {"imap_security": "STARTTLS", "smtp_port": 465}});
        apply_flags(&mut account, &args);
        assert_eq!(account["name"], "A");
        assert_eq!(account["settings"]["imap_security"], "SSL / TLS");
        assert_eq!(account["settings"]["smtp_security"], "STARTTLS");
        assert_eq!(account["settings"]["smtp_port"], 587);
        assert!(crate::cli::Cli::try_parse_from([
            "tern",
            "account",
            "add",
            "a@b.test",
            "--imap-security",
            "none"
        ])
        .is_err());
    }

    #[test]
    fn aliases_are_made_and_patched_as_preferences_does() {
        assert_eq!(make_alias("", "Ann", "a@x.test"), "Ann <a@x.test>");
        assert_eq!(make_alias("Sales", "Ann", "a@x.test"), "Sales <a@x.test>");
        assert_eq!(
            make_alias("sales@x.test", "Ann", "a@x.test"),
            "Ann <sales@x.test>"
        );
        assert_eq!(
            make_alias("\"Ann S\" <s@x.test>", "Ann", "a@x.test"),
            "Ann S <s@x.test>"
        );
        let args = |name: Option<&str>, add: &[&str], remove: &[&str], default: Option<&str>| {
            AccountUpdateArgs {
                account: "a@x.test".into(),
                name: name.map(str::to_owned),
                add_aliases: add.iter().map(|s| (*s).to_owned()).collect(),
                remove_aliases: remove.iter().map(|s| (*s).to_owned()).collect(),
                default_alias: default.map(str::to_owned),
            }
        };
        let account = json!({
            "emailAddress": "a@x.test", "name": "Ann",
            "aliases": ["Ann <s@x.test>"], "defaultAlias": "Ann <s@x.test>",
        });
        assert_eq!(
            update_patch(&account, &args(Some("Ann B"), &[], &[], None)).unwrap(),
            json!({"name": "Ann B"})
        );
        assert_eq!(
            update_patch(&account, &args(None, &["b@x.test"], &["s@x.test"], None)).unwrap(),
            json!({"aliases": ["Ann <b@x.test>"], "defaultAlias": null})
        );
        assert_eq!(
            update_patch(&account, &args(None, &["b@x.test"], &[], Some("b@x.test"))).unwrap()
                ["defaultAlias"],
            "Ann <b@x.test>"
        );
        for bad in [
            args(None, &[], &[], None),
            args(None, &[], &["nope@x.test"], None),
            args(None, &[], &[], Some("nope@x.test")),
        ] {
            assert!(update_patch(&account, &bad).is_err());
        }
    }

    #[test]
    fn queries_decode() {
        assert_eq!(
            parse_query("code=4%2F0Ab+c&state=xyz&scope="),
            vec![
                ("code".to_owned(), "4/0Ab c".to_owned()),
                ("state".to_owned(), "xyz".to_owned()),
                ("scope".to_owned(), String::new()),
            ]
        );
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
    }

    #[test]
    fn a_pasted_redirect_gives_the_code_of_its_sign_in_only() {
        let paste = |line: &str, state: &str| pasted_redirect(&mut line.as_bytes(), state);
        assert_eq!(
            paste(
                "  http://127.0.0.1:12141/?state=s1&code=4%2F0Ab&scope=mail\n",
                "s1"
            )
            .unwrap(),
            "4/0Ab"
        );
        assert_eq!(
            paste("http://127.0.0.1:12141/?code=c&state=s1#_", "s1").unwrap(),
            "c"
        );
        let err = paste("http://127.0.0.1:12141/?error=access_denied&state=s1", "s1").unwrap_err();
        assert!(err.to_string().contains("access_denied"), "{err}");
        let err = paste("http://127.0.0.1:12141/?code=c&state=s0", "s1").unwrap_err();
        assert!(err.to_string().contains("another sign-in"), "{err}");
        assert!(paste("4/0Ab", "s1").is_err());
        assert!(paste("", "s1").is_err());
    }

    fn redirect(listener: &TcpListener, path: &str) -> String {
        let port = listener.local_addr().unwrap().port();
        let path = path.to_owned();
        let t = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            let mut reply = String::new();
            s.read_to_string(&mut reply).unwrap();
            reply
        });
        t.join().unwrap()
    }

    #[test]
    fn the_receiver_takes_the_redirect_of_its_sign_in_only() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let waiting = std::thread::spawn({
            let listener = listener.try_clone().unwrap();
            move || receive_redirect(&listener, "s1", Duration::from_secs(20))
        });
        assert!(redirect(&listener, "/favicon.ico").starts_with("HTTP/1.1 404"));
        assert!(redirect(&listener, "/?code=c&state=other").starts_with("HTTP/1.1 400"));
        assert!(
            redirect(&listener, "/desktop?code=the%2Fcode&state=s1").starts_with("HTTP/1.1 200")
        );
        assert_eq!(waiting.join().unwrap().unwrap(), "the/code");

        let waiting = std::thread::spawn({
            let listener = listener.try_clone().unwrap();
            move || receive_redirect(&listener, "s2", Duration::from_secs(20))
        });
        redirect(&listener, "/?error=access_denied&state=s2");
        let err = waiting.join().unwrap().unwrap_err();
        assert!(err.to_string().contains("access_denied"), "{err}");
    }
}
