// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern account add`: an account on the serve engine.
//! IMAP accounts get the servers `account.discover` finds, the flags' changes and a
//! password; Gmail, Office 365 and Outlook sign in in the browser, whose redirect this
//! command receives on 127.0.0.1 and passes to `account.oauth.complete`. The engine tests
//! the account before it stores it, and keeps the secrets: none is printed.

use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tern_ipc::{code, feature, method, ClientError};

use crate::app::Ctx;
use crate::cli::AccountAddArgs;
use crate::discovery::Engine;
use crate::error::{CliError, Result};
use crate::output;

/// The providers that sign in with OAuth.
const OAUTH_PROVIDERS: &[&str] = &["gmail", "office365", "outlook"];
/// The most of a redirect request read.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

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
        sign_in(engine, email, args)?
    } else {
        if args.shared_mailbox.is_some() {
            return Err(CliError::usage(
                "--shared-mailbox is for office365 accounts",
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

/// The browser sign-in: `begin`, the redirect on 127.0.0.1, `complete`.
fn sign_in(engine: &mut Engine, email: &str, args: &AccountAddArgs) -> Result<Value> {
    // Bound first, so a port in use fails before the browser opens.
    let listener = TcpListener::bind(("127.0.0.1", args.port)).map_err(|e| {
        CliError::failed(format!(
            "could not listen on 127.0.0.1:{} for the sign-in ({e}); try another --port",
            args.port
        ))
    })?;
    let port = listener
        .local_addr()
        .map_err(|e| CliError::failed(e.to_string()))?
        .port();
    let mut params = json!({ "provider": args.provider, "redirectPort": port });
    if let Some(shared) = &args.shared_mailbox {
        params["sharedMailbox"] = json!(shared);
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
    let _ = writeln!(
        std::io::stderr(),
        "Sign in to {email} in your browser; if it doesn't open, open this address:\n{url}"
    );
    if !args.no_browser {
        open_browser(url);
    }
    let code = receive_redirect(&listener, state, Duration::from_secs(args.timeout))?;
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
    let query = parse_query(target.split_once('?').map_or("", |(_, q)| q));
    let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let (status, page, outcome) = match (get("state"), get("code"), get("error")) {
        (Some(s), _, Some(error)) if s == state => (
            "200 OK",
            "The sign-in was cancelled or refused. You can close this window.",
            Some(Err(CliError::failed(format!(
                "the sign-in failed: {error}"
            )))),
        ),
        (Some(s), Some(code), None) if s == state && !code.is_empty() => (
            "200 OK",
            "Signed in. You can close this window and go back to the terminal.",
            Some(Ok(code.to_owned())),
        ),
        (Some(_), _, _) => (
            "400 Bad Request",
            "This sign-in response is for another sign-in.",
            None,
        ),
        _ => ("404 Not Found", "Not found.", None),
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
