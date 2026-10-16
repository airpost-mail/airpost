// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `--mode test` against scripted IMAP, SMTP and OAuth token servers on loopback (plan
//! section 3, "Auth test mode"). `apps/engine/tools/test_mode_ab.py` diffs the same mode
//! against the C++ binary on the harness's servers.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

type Lines = Arc<Mutex<Vec<String>>>;

struct Imap {
    port: u16,
    received: Lines,
}

/// One-connection-at-a-time IMAP server: password `pass`, any XOAUTH2 token, and a LIST
/// with `folders`.
fn imap_server(folders: &'static [&'static str]) -> Imap {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let received: Lines = Arc::default();
    let rec = received.clone();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { return };
            serve_imap(conn, folders, &rec);
        }
    });
    Imap { port, received }
}

fn serve_imap(conn: TcpStream, folders: &[&str], received: &Lines) {
    let mut out = conn.try_clone().unwrap();
    let mut reader = BufReader::new(conn);
    let caps = "IMAP4rev1 SASL-IR AUTH=PLAIN AUTH=XOAUTH2 NAMESPACE";
    let _ = write!(out, "* OK [CAPABILITY {caps}] ready\r\n");
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let text = line.trim_end().to_owned();
        received.lock().unwrap().push(text.clone());
        let (tag, rest) = text.split_once(' ').unwrap_or((&text, ""));
        let upper = rest.to_ascii_uppercase();
        let reply = if upper.starts_with("LOGIN ") {
            if rest.ends_with(" pass") {
                format!("{tag} OK [CAPABILITY {caps}] Logged in\r\n")
            } else {
                format!("{tag} NO [AUTHENTICATIONFAILED] Authentication failed.\r\n")
            }
        } else if upper.starts_with("AUTHENTICATE XOAUTH2 ") {
            format!("{tag} OK [CAPABILITY {caps}] Logged in\r\n")
        } else if upper == "CAPABILITY" {
            format!("* CAPABILITY {caps}\r\n{tag} OK done\r\n")
        } else if upper == "NAMESPACE" {
            format!("* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n{tag} OK done\r\n")
        } else if upper == "LIST \"\" \"\"" {
            format!("* LIST (\\Noselect) \"/\" \"\"\r\n{tag} OK done\r\n")
        } else if upper.starts_with("LIST ") {
            let mut r: String = folders
                .iter()
                .map(|f| format!("* LIST (\\HasNoChildren) \"/\" {f}\r\n"))
                .collect();
            r.push_str(&format!("{tag} OK done\r\n"));
            r
        } else if upper == "LOGOUT" {
            let _ = write!(out, "* BYE\r\n{tag} OK done\r\n");
            return;
        } else {
            format!("{tag} OK done\r\n")
        };
        if out.write_all(reply.as_bytes()).is_err() {
            return;
        }
    }
}

struct Smtp {
    port: u16,
    received: Lines,
}

/// SMTP server accepting AUTH only when `accept_auth`, and any envelope.
fn smtp_server(accept_auth: bool) -> Smtp {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let received: Lines = Arc::default();
    let rec = received.clone();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { return };
            serve_smtp(conn, accept_auth, &rec);
        }
    });
    Smtp { port, received }
}

fn serve_smtp(conn: TcpStream, accept_auth: bool, received: &Lines) {
    let mut out = conn.try_clone().unwrap();
    let mut reader = BufReader::new(conn);
    let _ = out.write_all(b"220 mail.example.test ESMTP\r\n");
    let mut line = String::new();
    let mut in_data = false;
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let text = line.trim_end().to_owned();
        received.lock().unwrap().push(text.clone());
        if in_data {
            if text == "." {
                in_data = false;
                let _ = out.write_all(b"250 2.0.0 queued\r\n");
            }
            continue;
        }
        let upper = text.to_ascii_uppercase();
        let reply: &[u8] = if upper.starts_with("EHLO") {
            b"250-mail.example.test\r\n250 AUTH PLAIN LOGIN XOAUTH2\r\n"
        } else if upper.starts_with("AUTH") {
            if accept_auth {
                b"235 2.7.0 ok\r\n"
            } else {
                b"535 5.7.8 Error: authentication failed\r\n"
            }
        } else if upper == "DATA" {
            in_data = true;
            b"354 go ahead\r\n"
        } else if upper == "QUIT" {
            let _ = out.write_all(b"221 bye\r\n");
            return;
        } else {
            b"250 ok\r\n"
        };
        if out.write_all(reply).is_err() {
            return;
        }
    }
}

/// An OAuth token endpoint answering every request with `body`; returns its URL.
fn token_server(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut length = 0usize;
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
            }
            let mut req_body = vec![0u8; length];
            let _ = reader.read_exact(&mut req_body);
            let _ = write!(
                conn,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    format!("http://127.0.0.1:{port}/token")
}

fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn account(imap_port: u16, smtp_port: u16, settings: Value) -> Value {
    let mut a = json!({
        "id": "c0ffee-harness",
        "__cls": "Account",
        "provider": "imap",
        "emailAddress": "me@example.test",
        "settings": {
            "imap_host": "127.0.0.1", "imap_port": imap_port, "imap_username": "me",
            "imap_password": "pass", "imap_security": "none", "imap_allow_insecure_ssl": false,
            "smtp_host": "127.0.0.1", "smtp_port": smtp_port, "smtp_username": "me",
            "smtp_password": "pass", "smtp_security": "none", "smtp_allow_insecure_ssl": false,
        },
    });
    for (k, v) in settings.as_object().unwrap() {
        if v.is_null() {
            a["settings"].as_object_mut().unwrap().remove(k);
        } else {
            a["settings"][k] = v.clone();
        }
    }
    a
}

struct Run {
    code: Option<i32>,
    result: Value,
    deltas: Vec<Value>,
    stdout: String,
}

fn run_test_mode(config: &Path, account: &Value, env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mailsync"));
    cmd.env("CONFIG_DIR_PATH", config)
        .env("IDENTITY_SERVER", "http://127.0.0.1:9/unreachable")
        .args(["--mode", "test", "--identity", "null", "--account"])
        .arg(account.to_string());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let mut deltas = vec![];
    let mut result = Value::Null;
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("modelClass").is_some() {
            deltas.push(v);
        } else {
            result = v;
        }
    }
    Run {
        code: out.status.code(),
        result,
        deltas,
        stdout,
    }
}

fn keys(v: &Value) -> Vec<&str> {
    v.as_object().unwrap().keys().map(String::as_str).collect()
}

#[test]
fn success_sends_the_test_email_and_echoes_the_account() {
    let dir = tempfile::tempdir().unwrap();
    let imap = imap_server(&["INBOX", "Sent"]);
    let smtp = smtp_server(true);
    let acct = account(imap.port, smtp.port, json!({}));
    let run = run_test_mode(dir.path(), &acct, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stdout);
    assert!(run.stdout.starts_with('{'), "{:?}", run.stdout);
    assert_eq!(
        keys(&run.result),
        ["account", "error", "error_service", "log"]
    );
    assert_eq!(run.result["error"], Value::Null);
    assert_eq!(run.result["error_service"], "smtp");
    assert_eq!(run.result["account"], acct);

    let log = run.result["log"].as_str().unwrap();
    assert!(log.starts_with("----------IMAP----------\n"), "{log}");
    assert!(log.contains("\n\n----------SMTP----------\n"), "{log}");
    assert!(log.contains("LIST"), "{log}");
    assert!(log.contains("Subject: Tern SMTP Test Email"), "{log}");
    // The AUTH commands reached the servers but not the transcript.
    assert!(imap.received.lock().unwrap()[0].contains("LOGIN me pass"));
    assert!(smtp
        .received
        .lock()
        .unwrap()
        .iter()
        .any(|l| l.starts_with("AUTH")));
    assert!(!log.contains("LOGIN me"), "{log}");
    assert!(!log.lines().any(|l| l.starts_with("AUTH")), "{log}");
    let rcpt = smtp.received.lock().unwrap().clone();
    assert!(
        rcpt.iter().any(|l| l == "RCPT TO:<me@example.test>"),
        "{rcpt:?}"
    );
}

#[test]
fn smtp_login_verification_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let imap = imap_server(&["INBOX"]);
    let smtp = smtp_server(true);
    let acct = account(imap.port, smtp.port, json!({"smtp_verification": "login"}));
    let run = run_test_mode(dir.path(), &acct, &[]);
    assert_eq!(run.code, Some(0), "{}", run.stdout);
    let received = smtp.received.lock().unwrap().clone();
    assert!(
        received.iter().any(|l| l.starts_with("AUTH")),
        "{received:?}"
    );
    assert!(
        !received.iter().any(|l| l.starts_with("MAIL")),
        "{received:?}"
    );
}

#[test]
fn imap_failures() {
    let dir = tempfile::tempdir().unwrap();
    let smtp = smtp_server(true);

    let imap = imap_server(&["INBOX"]);
    let acct = account(imap.port, smtp.port, json!({"imap_password": "wrong"}));
    let run = run_test_mode(dir.path(), &acct, &[]);
    assert_eq!(run.code, Some(1));
    assert_eq!(
        keys(&run.result),
        ["account", "error", "error_service", "log"]
    );
    assert_eq!(run.result["error"], "ErrorAuthentication");
    assert_eq!(run.result["error_service"], "imap");
    assert_eq!(run.result["account"], Value::Null);

    let acct = account(closed_port(), smtp.port, json!({}));
    let run = run_test_mode(dir.path(), &acct, &[]);
    assert_eq!(run.code, Some(1));
    assert_eq!(run.result["error"], "ErrorConnection");
    assert_eq!(run.result["error_service"], "imap");

    let imap = imap_server(&["Notes"]);
    let acct = account(imap.port, smtp.port, json!({}));
    let run = run_test_mode(dir.path(), &acct, &[]);
    assert_eq!(run.result["error"], "ErrorInvalidAccount");
    assert!(run.result["log"]
        .as_str()
        .unwrap()
        .contains("Required folder not found."));

    // A plaintext server answering a ClientHello is not outdated encryption.
    let imap = imap_server(&["INBOX"]);
    let acct = account(imap.port, smtp.port, json!({"imap_security": "SSL / TLS"}));
    let run = run_test_mode(dir.path(), &acct, &[]);
    assert_eq!(run.result["error"], "ErrorConnection");
    assert!(run.result.get("error_advice").is_none(), "{}", run.result);
}

#[test]
fn smtp_failure_reports_the_last_reply() {
    let dir = tempfile::tempdir().unwrap();
    let imap = imap_server(&["INBOX"]);
    let smtp = smtp_server(false);
    let acct = account(imap.port, smtp.port, json!({}));
    let run = run_test_mode(dir.path(), &acct, &[]);
    assert_eq!(run.code, Some(1));
    assert_eq!(run.result["error"], "ErrorAuthentication");
    assert_eq!(run.result["error_service"], "smtp");
    let log = run.result["log"].as_str().unwrap();
    assert!(
        log.ends_with("\n\nSMTP Last Response Code: 535\nSMTP Last Response: 5.7.8 Error: authentication failed\n"),
        "{log:?}"
    );
}

#[test]
fn oauth_refresh_rotates_the_refresh_token() {
    let dir = tempfile::tempdir().unwrap();
    let imap = imap_server(&["INBOX"]);
    let smtp = smtp_server(true);
    let url = token_server(
        200,
        r#"{"access_token":"at-1","expires_in":3600,"refresh_token":"rt-2"}"#,
    );
    let mut acct = account(
        imap.port,
        smtp.port,
        json!({"imap_password": null, "smtp_password": null,
               "refresh_token": "rt-1", "refresh_client_id": "cid"}),
    );
    acct["provider"] = json!("gmail");
    let run = run_test_mode(
        dir.path(),
        &acct,
        &[("MAILSYNC_GOOGLE_TOKEN_URL", url.as_str())],
    );

    assert_eq!(run.code, Some(0), "{}", run.stdout);
    assert_eq!(run.result["account"]["settings"]["refresh_token"], "rt-2");
    assert_eq!(run.deltas.len(), 1, "{:?}", run.deltas);
    assert_eq!(run.deltas[0]["modelClass"], "ProcessAccountSecretsUpdated");
    assert_eq!(
        run.deltas[0]["modelJSONs"][0]["settings"]["refresh_token"],
        "rt-2"
    );
    assert!(imap.received.lock().unwrap()[0].contains("AUTHENTICATE XOAUTH2 "));
    let log = run.result["log"].as_str().unwrap();
    assert!(!log.contains("AUTHENTICATE"), "{log}");
    assert!(!log.lines().any(|l| l.starts_with("AUTH")), "{log}");
}

#[test]
fn oauth_refusal_is_an_authentication_error() {
    let dir = tempfile::tempdir().unwrap();
    let url = token_server(400, r#"{"error":"invalid_grant"}"#);
    let mut acct = account(
        closed_port(),
        closed_port(),
        json!({"imap_password": null, "smtp_password": null,
               "refresh_token": "rt-1", "refresh_client_id": "cid"}),
    );
    acct["provider"] = json!("gmail");
    let run = run_test_mode(
        dir.path(),
        &acct,
        &[("MAILSYNC_GOOGLE_TOKEN_URL", url.as_str())],
    );
    assert_eq!(run.code, Some(1));
    assert_eq!(
        keys(&run.result),
        ["account", "error", "error_offline", "error_service", "log"]
    );
    assert_eq!(run.result["error"], "ErrorAuthentication");
    assert_eq!(run.result["error_offline"], false);
    assert_eq!(run.result["error_service"], "imap");

    // Nothing listening: retryable and offline.
    let url = format!("http://127.0.0.1:{}/token", closed_port());
    let run = run_test_mode(
        dir.path(),
        &acct,
        &[("MAILSYNC_GOOGLE_TOKEN_URL", url.as_str())],
    );
    assert_eq!(run.result["error"], "ErrorConnection");
    assert_eq!(run.result["error_offline"], true);
}
