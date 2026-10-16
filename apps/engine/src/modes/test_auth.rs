// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `--mode test`: checks the account's IMAP and SMTP settings and prints one JSON object
//! (docs/engine-port-plan.md section 3, "Auth test mode").
//!
//! Derived from Mailspring-Sync (`MailSync/main.cpp`: `runTestAuth`,
//! `runTestAuthReportingExceptions`, `AccumulatorLogger`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! Output: `{account, error, error_service, log}` plus `error_advice` after a TLS failure,
//! or `{account, error, error_offline, error_service, log}` when the OAuth refresh (or
//! anything outside the IMAP and SMTP sessions) fails. `error` is an `ErrorCodeToTypeMap`
//! name the client localises; `log` is the protocol transcript without AUTH payloads.
//! Success exits 0 and echoes the account, with any rotated refresh token applied;
//! failure exits 1 with `account: null`. The object is printed without a leading newline,
//! as the C++ did; the client takes the last JSON object on stdout.

use std::sync::Arc;

use ms_core::constants::{EXIT_FAILURE, EXIT_OK};
use ms_core::protolog::AccumulatingLogger;
use ms_core::{tls_failure_advice, Account, ErrorCode};
use ms_imap::{ImapAuth, ImapConfig, ImapSession, Session};
use ms_net::{AccessToken, HttpClient, TokenManager};
use ms_providers::roles::role_for_folder;
use ms_providers::MailboxModelKind;
use ms_smtp::{OAuthParts, SmtpClient, SmtpConfig};
use serde_json::{json, Value};

use crate::output::write_stdout;

const SERVICE_IMAP: &str = "imap";
const SERVICE_SMTP: &str = "smtp";

const REQUIRED_FOLDER_MISSING: &str = "\n\nRequired folder not found. Ensure your account has an `Inbox` folder, or if this is a Gmail account, verify that `All Mail` is enabled for IMAP in your Gmail settings.\n";

/// A session step that failed: the code for `error`, and the advice when TLS failed.
struct Failure {
    code: ErrorCode,
    advice: Option<String>,
}

pub fn run(account: &Account) -> i32 {
    let deltas = ms_store::DeltaStream::stdout();
    let deltas_for_tokens = deltas.clone();
    // A rotated refresh token must reach the client even though the process exits right
    // after, so the delta is written before the refresh returns.
    let tokens = TokenManager::new(Some(Box::new(move |a: &Account| {
        deltas_for_tokens.send_updated_secrets(a);
        deltas_for_tokens.flush();
    })));
    let (resp, ok) = evaluate(account, &tokens);
    deltas.flush();
    write_stdout(&resp.to_string());
    if ok {
        EXIT_OK
    } else {
        EXIT_FAILURE
    }
}

/// `account.test` in `serve` mode: the same object, returned. A rotated refresh token is in
/// the result's `account`, since `serve`'s stdout carries only responses.
pub fn result(account: &Account) -> Value {
    evaluate(account, &TokenManager::new(None)).0
}

/// The test's JSON object, and whether it passed.
fn evaluate(account: &Account, tokens: &TokenManager) -> (Value, bool) {
    let transcript = Arc::new(AccumulatingLogger::default());
    let mut service = SERVICE_IMAP;
    match test_auth(account, tokens, &transcript, &mut service) {
        Ok(None) => {
            let account = tokens.account_with_current_secrets(account).to_json();
            (response(Value::Null, service, &transcript, account), true)
        }
        Ok(Some(failure)) => {
            let mut resp = response(
                json!(failure.code.name()),
                service,
                &transcript,
                Value::Null,
            );
            if let Some(advice) = failure.advice {
                resp["error_advice"] = json!(advice);
            }
            (resp, false)
        }
        Err(e) => (raised(&e, service, &transcript), false),
    }
}

fn response(error: Value, service: &str, transcript: &AccumulatingLogger, account: Value) -> Value {
    json!({
        "error": error,
        "error_service": service,
        "log": transcript.contents(),
        "account": account,
    })
}

/// `runTestAuthReportingExceptions`: a failure outside the sessions, the OAuth refresh
/// above all (a revoked refresh token is a 400, an offline machine fails the request). A
/// `SyncError` is reported by retryability, the two codes the client has strings for.
fn raised(e: &ms_core::Error, service: &str, transcript: &AccumulatingLogger) -> Value {
    let (error, offline) = match e.as_sync() {
        Some(se) => {
            transcript.push(&format!("\n\n{}: {}\n", se.key, se.debuginfo));
            let error = if se.is_retryable() {
                ErrorCode::Connection.name()
            } else {
                ErrorCode::Authentication.name()
            };
            (error.to_owned(), se.is_offline())
        }
        None => (e.to_string(), false),
    };
    let error = if error.is_empty() {
        "Unknown error".to_owned()
    } else {
        error
    };
    json!({
        "error": error,
        "error_service": service,
        "error_offline": offline,
        "log": transcript.contents(),
        "account": null,
    })
}

/// `runTestAuth`. `Ok(None)` is success; `service` names the step that ran last.
fn test_auth(
    account: &Account,
    tokens: &TokenManager,
    transcript: &Arc<AccumulatingLogger>,
    service: &mut &'static str,
) -> ms_core::Result<Option<Failure>> {
    *service = SERVICE_IMAP;
    transcript.push("----------IMAP----------\n");
    let token = access_token(account, tokens)?;
    if let Some(failure) = test_imap(account, token.as_ref(), transcript) {
        return Ok(Some(failure));
    }

    transcript.push("\n\n----------SMTP----------\n");
    *service = SERVICE_SMTP;
    Ok(test_smtp(account, token.as_ref(), transcript))
}

/// The OAuth access token, fetched before any session exists: the refresh is blocking
/// HTTP and must not run on a session's runtime.
fn access_token(account: &Account, tokens: &TokenManager) -> ms_core::Result<Option<AccessToken>> {
    if account.refresh_token().is_empty() {
        return Ok(None);
    }
    let http = HttpClient::new()?;
    tokens.token_for(&http, account).map(Some)
}

/// Connect, list folders, and require the folder the account syncs from: `all` for
/// Gmail's label model (the C++ checked for `imap.gmail.com`), `inbox` otherwise.
fn test_imap(
    account: &Account,
    token: Option<&AccessToken>,
    transcript: &Arc<AccumulatingLogger>,
) -> Option<Failure> {
    let auth = match token {
        Some(t) => ImapAuth::XOAuth2(t.access_token.clone()),
        None => ImapAuth::Password(account.imap_password().to_owned()),
    };
    let mut config = ImapConfig::for_account(account, auth);
    config.logger = Some(transcript.clone());
    let mut session = Session::new(config);

    if let Err(e) = session.connect_if_needed() {
        let advice = tls_failure_advice(e.code, session.last_tls_error());
        if let Some(a) = &advice {
            transcript.push(&format!("\n\n{a}\n"));
        }
        return Some(Failure {
            code: e.code,
            advice,
        });
    }
    let folders = match session.list_folders() {
        Ok(f) => f,
        Err(e) => {
            return Some(Failure {
                code: e.code,
                advice: None,
            });
        }
    };

    let prefix = session
        .default_namespace()
        .map(|n| n.prefix.clone())
        .unwrap_or_default();
    let required = match session.profile().mailbox_model {
        MailboxModelKind::GmailLabels => "all",
        MailboxModelKind::Folders => "inbox",
    };
    let container = account.container_folder();
    let found = folders.iter().any(|f| {
        role_for_folder(container, &prefix, &f.path, f.delimiter, f.flags) == Some(required)
    });
    session.disconnect();
    if !found {
        transcript.push(REQUIRED_FOLDER_MISSING);
        return Some(Failure {
            code: ErrorCode::InvalidAccount,
            advice: None,
        });
    }
    None
}

/// Login only when `smtp_verification == "login"`, else the test email to the account's
/// own address (`SmtpClient::verify_account`).
fn test_smtp(
    account: &Account,
    token: Option<&AccessToken>,
    transcript: &Arc<AccumulatingLogger>,
) -> Option<Failure> {
    let oauth = token.map(|t| OAuthParts {
        username: &t.username,
        access_token: &t.access_token,
    });
    let mut config = SmtpConfig::for_account(account, oauth);
    config.logger = Some(transcript.clone());
    let mut smtp = SmtpClient::new(config);

    // Note: a failed session is dropped without QUIT, so the transcript ends with the
    // diagnostics and `failure_report` still describes the reply that failed.
    if let Err(e) = smtp.verify_account(account) {
        let advice = tls_failure_advice(e.code, smtp.last_tls_error());
        if let Some(a) = &advice {
            transcript.push(&format!("\n\n{a}\n"));
        }
        transcript.push(&smtp.failure_report());
        return Some(Failure {
            code: e.code,
            advice,
        });
    }
    smtp.disconnect();
    None
}
