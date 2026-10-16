// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The SMTP side of SendDraftTask and EventRSVPTask.
//!
//! Derived from Mailspring-Sync `MailUtils::configureSessionForAccount(SMTPSession &)`,
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use std::sync::Arc;
use std::time::Duration;

use ms_core::{Account, MailError, Result};
use ms_net::{HttpClient, TokenManager};
use ms_smtp::{OAuthParts, SmtpClient, SmtpConfig};

/// One SMTP session's worth of sending. [`SmtpClient`] in the process; tests substitute
/// their own.
pub trait Mailer {
    /// `sendMessage(Data *)`: the envelope comes from the message's headers, Bcc included,
    /// and Bcc is stripped from the wire copy.
    fn send(&mut self, raw: &[u8]) -> std::result::Result<(), MailError>;
    /// `sendMessage(from, recipients, data)`: an explicit envelope over the full headers.
    fn send_to(
        &mut self,
        from: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), MailError>;
    /// The last reply code, for the failure log line (0 before any).
    fn last_response_code(&self) -> u16 {
        0
    }
}

impl Mailer for SmtpClient {
    fn send(&mut self, raw: &[u8]) -> std::result::Result<(), MailError> {
        SmtpClient::send(self, raw)
    }

    fn send_to(
        &mut self,
        from: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), MailError> {
        SmtpClient::send_to(self, from, recipients, raw)
    }

    fn last_response_code(&self) -> u16 {
        SmtpClient::last_response_code(self)
    }
}

/// Opens a [`Mailer`] for the account. Called on the foreground thread, outside any
/// runtime, so it may refresh an OAuth token (blocking HTTP).
pub type MailerFactory = Arc<dyn Fn(&Account) -> Result<Box<dyn Mailer>> + Send + Sync>;

/// How the send path waits between polls of the Sent folder.
pub type Sleeper = Arc<dyn Fn(Duration) + Send + Sync>;

/// The process's mailer: `SmtpConfig::for_account`, with a current access token for an
/// OAuth account, and SMTP traffic logged with `--verbose`.
pub fn smtp_mailer(tokens: Arc<TokenManager>, verbose: bool) -> MailerFactory {
    Arc::new(move |account: &Account| {
        let token = if account.refresh_token().is_empty() {
            None
        } else {
            let http = HttpClient::new()?;
            Some(tokens.token_for(&http, account)?)
        };
        let oauth = token.as_ref().map(|t| OAuthParts {
            username: &t.username,
            access_token: &t.access_token,
        });
        let mut config = SmtpConfig::for_account(account, oauth);
        if verbose {
            config.logger = Some(Arc::new(ms_core::protolog::TracingLogger));
        }
        Ok(Box::new(SmtpClient::new(config)) as Box<dyn Mailer>)
    })
}

pub fn thread_sleeper() -> Sleeper {
    Arc::new(std::thread::sleep)
}
