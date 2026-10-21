// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! SMTP (mailcore2's `SMTPSession` as used by `TaskProcessor` and `test` mode).
//!
//! Derived from Mailspring-Sync (`MailUtils::configureSessionForAccount`, `runTestAuth` in
//! `main.cpp`), Copyright © 2017 Foundry 376, GPL-3.0, and mailcore2 (`MCSMTPSession.cpp`)
//! with the libetpan SMTP code it drives: Copyright © 2001-2013 MailCore team and © 2001-2005
//! DINH Viet Hoa, BSD-3-Clause. See NOTICE.md ("mailcore2", "libEtPan!").
//!
//! [`SmtpClient`] is synchronous: it owns a current-thread tokio runtime and blocks on it,
//! so it must not be called from inside another runtime. Errors are [`MailError`]s carrying
//! the `ErrorCodeToTypeMap` code the client keys on (`ErrorSendMessage`,
//! `ErrorAuthentication`, …) and the server's last reply as detail. The send task reports
//! them as `send-failed` / `send-partially-failed` with `code.name()`.
//!
//! Every send opens its own connection and ends it with QUIT, as libetpan did, so a
//! multisend logs in once per recipient. With a [`ProtocolLogger`] attached, every line is
//! recorded as sent or received; AUTH commands and SASL payloads are
//! [`Direction::SentPrivate`](ms_core::protolog::Direction) and never recorded.

mod conn;
mod ehlo;
pub mod message;
pub mod sasl;
mod session;

use std::sync::Arc;
use std::time::Duration;

use ms_core::protolog::ProtocolLogger;
use ms_core::{Account, ErrorCode, MailError, Security};

pub use message::TEST_EMAIL_SUBJECT;

pub type Result<T> = std::result::Result<T, MailError>;

/// mailcore's default SMTP timeout (`SMTPSession::init`), per read or write.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub enum SmtpAuth {
    None,
    Password(String),
    /// An OAuth access token fetched beforehand.
    XOAuth2(String),
}

/// The OAuth identity for SMTP: `XOAuth2TokenManager::partsForAccount`'s username and token.
#[derive(Clone, Copy, Debug)]
pub struct OAuthParts<'a> {
    pub username: &'a str,
    pub access_token: &'a str,
}

#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub auth: SmtpAuth,
    /// `smtp_allow_insecure_ssl`, carried for the account settings but without effect: TLS
    /// below 1.2 is gone, and the TLS advice never suggests the setting.
    pub allow_obsolete_tls: bool,
    /// smtp-mail.outlook.com takes XOAUTH2 in two steps and keeps Bcc itself
    /// (`MCSMTPSession.cpp:91`), decided by [`ms_providers::smtp_uses_outlook_xoauth2`].
    pub outlook: bool,
    pub timeout: Duration,
    pub logger: Option<Arc<dyn ProtocolLogger>>,
    /// Replaces the platform-verifier TLS configuration (tests, private deployments).
    pub tls_config: Option<Arc<rustls::ClientConfig>>,
}

impl SmtpConfig {
    /// `MailUtils::configureSessionForAccount(SMTPSession &)`. With OAuth, `smtp_username`
    /// wins over the token's username: O365 shared mailboxes authenticate IMAP as the shared
    /// mailbox, but SMTP AUTH accepts only the signed-in user.
    pub fn for_account(account: &Account, oauth: Option<OAuthParts<'_>>) -> SmtpConfig {
        let (username, auth) = match oauth {
            Some(parts) => {
                let username = if account.smtp_username().is_empty() {
                    parts.username
                } else {
                    account.smtp_username()
                };
                (
                    username.to_owned(),
                    SmtpAuth::XOAuth2(parts.access_token.to_owned()),
                )
            }
            None => (
                account.smtp_username().to_owned(),
                SmtpAuth::Password(account.smtp_password().to_owned()),
            ),
        };
        let host = account.smtp_host().to_owned();
        SmtpConfig {
            outlook: ms_providers::smtp_uses_outlook_xoauth2(&host),
            host,
            port: account.smtp_port(),
            security: account.smtp_security(),
            username,
            auth,
            allow_obsolete_tls: account.smtp_allow_insecure_ssl(),
            timeout: DEFAULT_TIMEOUT,
            logger: None,
            tls_config: None,
        }
    }
}

pub struct SmtpClient {
    // Declared first so an open socket is dropped while its runtime still exists.
    session: session::Session,
    runtime: Option<tokio::runtime::Runtime>,
}

impl SmtpClient {
    pub fn new(config: SmtpConfig) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .ok();
        SmtpClient {
            session: session::Session::new(config),
            runtime,
        }
    }

    pub fn config(&self) -> &SmtpConfig {
        &self.session.config
    }

    fn block_on<T>(
        &mut self,
        f: impl AsyncFnOnce(&mut session::Session) -> Result<T>,
    ) -> Result<T> {
        let Some(rt) = &self.runtime else {
            return Err(MailError::new(
                ErrorCode::Connection,
                "could not start the SMTP runtime",
            ));
        };
        rt.block_on(f(&mut self.session))
    }

    /// Connects and authenticates (`loginIfNeeded`), the `smtp_verification == "login"`
    /// check. The connection stays open for a following call.
    pub fn login(&mut self) -> Result<()> {
        self.block_on(async |s| s.login_if_needed().await)
    }

    /// `checkAccount`: MAIL FROM / RCPT TO `from`, then the "Tern SMTP Test Email" to
    /// `from` itself. Refusals of MAIL map to `ErrorInvalidAccount`, of RCPT and DATA to
    /// `ErrorInvalidRelaySMTP`.
    pub fn check_account(&mut self, from: &str) -> Result<()> {
        self.block_on(async |s| s.check_account(from).await)
    }

    /// The `test` mode SMTP step (`runTestAuth`): login only when
    /// `settings.smtp_verification == "login"` (O365 shared mailboxes may lack send rights,
    /// and every member would see the test email), otherwise [`SmtpClient::check_account`].
    pub fn verify_account(&mut self, account: &Account) -> Result<()> {
        if account.smtp_verification() == "login" {
            self.login()
        } else {
            self.check_account(account.email_address())
        }
    }

    /// `sendMessage(Data *)`: the envelope is the first From address and every To, Cc and
    /// Bcc address. Bcc is stripped from the wire copy.
    pub fn send(&mut self, raw: &[u8]) -> Result<()> {
        let env = message::envelope_from_headers(raw);
        let from = env.from.unwrap_or_default();
        self.send_to(&from, &env.recipients, raw)
    }

    /// `sendMessage(from, recipients, data)`: an explicit envelope over the message's own
    /// headers, which is how multisend delivers each recipient's body with RCPT set to that
    /// one address while To and Cc still name everyone. Bcc is stripped here too.
    pub fn send_to(&mut self, from: &str, recipients: &[String], raw: &[u8]) -> Result<()> {
        self.block_on(async |s| s.send_message(from, recipients, raw).await)
    }

    /// Ends the connection with QUIT, if one is open.
    pub fn disconnect(&mut self) {
        let _ = self.block_on(async |s| {
            s.disconnect().await;
            Ok(())
        });
    }

    pub fn is_disconnected(&self) -> bool {
        self.session.is_disconnected()
    }

    /// The text of the last server reply, lines joined with `\n` (`lastSMTPResponse`).
    pub fn last_response(&self) -> Option<&str> {
        self.session.last_reply.as_ref().map(|r| r.text.as_str())
    }

    /// The code of the last server reply, 0 before any (`lastSMTPResponseCode`).
    pub fn last_response_code(&self) -> u16 {
        self.session.last_reply.as_ref().map_or(0, |r| r.code)
    }

    /// What the TLS stack said when the handshake was rejected, for
    /// `MailUtils::tlsFailureAdvice`. Not set for certificate failures.
    pub fn last_tls_error(&self) -> Option<&str> {
        self.session.last_tls_error.as_deref()
    }

    /// The diagnostics `runTestAuth` appends to the transcript after a failed SMTP step.
    pub fn failure_report(&self) -> String {
        match &self.session.last_reply {
            Some(r) => format!(
                "\n\nSMTP Last Response Code: {}\nSMTP Last Response: {}",
                r.code, r.text
            ),
            None => String::new(),
        }
    }
}
