// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `--mode install-check`: can this machine reach the network over TLS.
//!
//! Derived from Mailspring-Sync (`MailSync/main.cpp`: `runInstallCheck`), Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Prints `{error, http_check, imap_check, smtp_check, tidy_check, legacy_tls_check, log}`
//! as the C++ did; each check is `{"success": true}` or `{"error": "…"}`, `http_check` is
//! `{"status_code": n}`, and `error` is `"One or more checks failed"` or null. Exit 0 when
//! every check passed, else 1.
//!
//! Targets:
//! - HTTP: a GET of the Google OAuth token endpoint (`AIRPOST_ENGINE_GOOGLE_TOKEN_URL` overrides
//!   it), a host the engine itself talks to. Any HTTP status means DNS, TCP and TLS work.
//!   The C++ pinged `$IDENTITY_SERVER/ping`, an identity server this engine never calls.
//! - IMAP: a TLS connect to imap.gmail.com:993, reading the greeting and CAPABILITY.
//! - SMTP: a TLS connect to smtp.gmail.com:465 with EHLO and QUIT. The C++ also tried AUTH
//!   with fake credentials to prove cyrus-sasl's plugins loaded; SASL is built in here.
//! - `tidy_check` and `legacy_tls_check` diagnosed libtidy and OpenSSL's legacy handshake,
//!   neither of which this engine has, so both report success.

use std::sync::Arc;

use airpost_core::constants::{EXIT_FAILURE, EXIT_OK};
use airpost_core::protolog::AccumulatingLogger;
use airpost_core::{ErrorCode, Security};
use airpost_imap::{ImapAuth, ImapConfig, ImapSession, Session};
use airpost_net::http::transport_error;
use airpost_net::{Endpoints, HttpClient};
use airpost_providers::OAuthProvider;
use airpost_smtp::{SmtpAuth, SmtpClient, SmtpConfig};
use serde_json::{json, Value};

use crate::output::write_stdout;

const IMAP_HOST: &str = "imap.gmail.com";
const IMAP_PORT: u16 = 993;
const SMTP_HOST: &str = "smtp.gmail.com";
const SMTP_PORT: u16 = 465;

pub fn run() -> i32 {
    let transcript = Arc::new(AccumulatingLogger::default());

    let http_check = http_check();
    transcript.push("----------IMAP----------\n");
    let imap_check = outcome(imap_check(&transcript));
    transcript.push("\n\n----------SMTP----------\n");
    let smtp_check = outcome(smtp_check(&transcript));
    transcript.push("\n\n----------LEGACY TLS----------\n");
    transcript.push("\nTLS below 1.2 is not supported; nothing to check.\n");
    // `compatibility_level` keeps the C++ shape; rustls has a single level.
    let legacy_tls_check = json!({ "success": true, "compatibility_level": 0 });
    let tidy_check = json!({ "success": true });

    let success = [&http_check, &imap_check, &smtp_check]
        .iter()
        .all(|c| c.get("error").is_none());
    let resp = json!({
        "error": if success { Value::Null } else { json!("One or more checks failed") },
        "http_check": http_check,
        "imap_check": imap_check,
        "smtp_check": smtp_check,
        "tidy_check": tidy_check,
        "legacy_tls_check": legacy_tls_check,
        "log": transcript.contents(),
    });
    write_stdout(&resp.to_string());
    if success {
        EXIT_OK
    } else {
        EXIT_FAILURE
    }
}

fn outcome(result: Result<(), String>) -> Value {
    match result {
        Ok(()) => json!({ "success": true }),
        Err(e) => json!({ "error": e }),
    }
}

fn http_check() -> Value {
    let url = Endpoints::from_env()
        .token_url(OAuthProvider::Google)
        .to_owned();
    let probe = HttpClient::new()
        .map_err(|e| e.to_string())
        .and_then(|http| {
            http.probe(&url)
                .map_err(|e| format!("{}: {e}", transport_error(&e, &url).key))
        });
    match probe {
        Ok(status) => json!({ "status_code": status }),
        Err(e) => json!({ "error": e }),
    }
}

/// Connected, or refused for want of credentials: either way TLS worked.
fn imap_check(transcript: &Arc<AccumulatingLogger>) -> Result<(), String> {
    let mut config = ImapConfig::new(
        IMAP_HOST,
        IMAP_PORT,
        Security::Tls,
        "",
        ImapAuth::Password(String::new()),
    );
    config.logger = Some(transcript.clone());
    let mut session = Session::new(config);
    let result = match session.connect_if_needed() {
        Ok(()) => Ok(()),
        Err(e)
            if matches!(
                e.code,
                ErrorCode::Authentication | ErrorCode::AuthenticationRequired
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(e.code.name().to_owned()),
    };
    session.disconnect();
    result
}

/// Connect and EHLO without credentials (`SmtpAuth::None` skips AUTH), then QUIT.
fn smtp_check(transcript: &Arc<AccumulatingLogger>) -> Result<(), String> {
    let config = SmtpConfig {
        host: SMTP_HOST.to_owned(),
        port: SMTP_PORT,
        security: Security::Tls,
        username: String::new(),
        auth: SmtpAuth::None,
        allow_obsolete_tls: false,
        outlook: false,
        timeout: airpost_smtp::DEFAULT_TIMEOUT,
        logger: Some(transcript.clone()),
        tls_config: None,
    };
    let mut smtp = SmtpClient::new(config);
    let result = smtp.login().map_err(|e| e.code.name().to_owned());
    smtp.disconnect();
    result
}
