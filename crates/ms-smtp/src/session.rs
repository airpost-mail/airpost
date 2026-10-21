// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! mailcore2's `SMTPSession` state machine: connect, log in, send, check the account, and
//! map every failure to the `ErrorCode` the client and the send task key on.
//!
//! Derived from mailcore2's `MCSMTPSession.cpp` and libetpan's `mailsmtp.c`,
//! `mailsmtp_helper.c` and `mailsmtp_oauth2.c`. mailcore2 is Copyright © 2001-2013 MailCore
//! team and libetpan Copyright © 2001-2005 DINH Viet Hoa, both BSD-3-Clause;
//! Mailspring-Sync's changes are Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md
//! ("mailcore2", "libEtPan!").

use std::sync::Arc;

use base64::Engine;
use ms_core::{ErrorCode, MailError, Security};

use crate::conn::{Connection, Reply, StreamError, TlsError};
use crate::message;
use crate::sasl::{self, Mechanism, SaslClient};
use crate::{Result, SmtpAuth, SmtpConfig};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// SASL mechanisms servers advertise that need libraries we do not ship.
const UNSUPPORTED_MECHANISMS: [&str; 4] = ["SRP", "NTLM", "KERBEROS_V4", "GSSAPI"];

/// Most 334 rounds any mechanism we speak needs, plus slack for servers that re-prompt.
const MAX_AUTH_ROUNDS: usize = 8;

/// How a MAIL / RCPT / DATA exchange failed, in libetpan's terms
/// (`mailesmtp_mail_size`, `mailesmtp_rcpt`, `mailsmtp_data_message`).
enum TransactionFailure {
    Stream(StreamError),
    /// `MAILSMTP_ERROR_EXCEED_STORAGE_ALLOCATION`: a 552, or larger than the advertised SIZE.
    ExceedStorage,
    /// `MAILSMTP_ERROR_MAILBOX_UNAVAILABLE`: 550 to MAIL, 550 or 450 to RCPT.
    MailboxUnavailable,
    Other,
}

enum AuthFailure {
    Stream(StreamError),
    NotImplemented(String),
    Refused,
}

pub(crate) struct Session {
    pub config: SmtpConfig,
    conn: Option<Connection>,
    logged_in: bool,
    should_disconnect: bool,
    pub last_reply: Option<Reply>,
    pub last_tls_error: Option<String>,
}

impl Session {
    pub fn new(config: SmtpConfig) -> Self {
        Session {
            config,
            conn: None,
            logged_in: false,
            should_disconnect: false,
            last_reply: None,
            last_tls_error: None,
        }
    }

    pub fn is_disconnected(&self) -> bool {
        self.conn.is_none()
    }

    fn record(&mut self, reply: &Reply) {
        self.last_reply = Some(reply.clone());
    }

    /// `"<code> <text>"` of the last reply, for error detail.
    fn reply_detail(&self) -> String {
        match &self.last_reply {
            Some(r) => format!("{} {}", r.code, r.text.trim_end().replace('\n', " / ")),
            None => String::new(),
        }
    }

    fn err(&self, code: ErrorCode) -> MailError {
        MailError::new(code, self.reply_detail())
    }

    fn stream_err(&mut self, code: ErrorCode, e: StreamError) -> MailError {
        self.should_disconnect = true;
        MailError::new(code, e.0)
    }

    pub async fn disconnect(&mut self) {
        if let Some(conn) = self.conn.take() {
            conn.quit().await;
        }
        self.logged_in = false;
    }

    /// `connectIfNeeded`
    async fn connect_if_needed(&mut self) -> Result<()> {
        self.last_reply = None;
        if self.should_disconnect {
            self.disconnect().await;
            self.should_disconnect = false;
        }
        if self.conn.is_none() {
            self.logged_in = false;
            self.connect().await?;
        }
        Ok(())
    }

    /// `connect` / `connectWithCurrentCompatibilityLevel`. There is no TLS compatibility
    /// ladder: rustls speaks only TLS 1.2 and 1.3, so one attempt is final.
    async fn connect(&mut self) -> Result<()> {
        self.last_tls_error = None;
        let host = self.config.host.clone();
        let port = self.config.port;
        let logger = self.config.logger.clone();
        if let Some(l) = &logger {
            l.note(&format!("connect {host} {port}"));
        }
        let mut conn = Connection::open(&host, port, self.config.timeout, logger)
            .await
            .map_err(|e| MailError::new(ErrorCode::Connection, e.0))?;

        match self.config.security {
            Security::Tls => {
                self.handshake(&mut conn, &host, ErrorCode::Connection)
                    .await?;
                self.greet_and_init(&mut conn).await?;
            }
            Security::StartTls => {
                self.greet_and_init(&mut conn).await?;
                conn.note("start TLS");
                self.starttls(&mut conn, &host).await?;
                conn.note("init after starttls");
                self.init(&mut conn).await?;
            }
            Security::Clear => self.greet_and_init(&mut conn).await?,
        }
        self.conn = Some(conn);
        Ok(())
    }

    async fn greet_and_init(&mut self, conn: &mut Connection) -> Result<()> {
        // mailsmtp_connect: anything but 220 (554 included) is a connection failure.
        let greeting = conn
            .read_reply()
            .await
            .map_err(|e| MailError::new(ErrorCode::Connection, e.0))?;
        self.record(&greeting);
        if greeting.code != 220 {
            return Err(self.err(ErrorCode::Connection));
        }
        self.init(conn).await
    }

    async fn init(&mut self, conn: &mut Connection) -> Result<()> {
        match conn.init().await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(refusal)) => {
                self.record(&refusal);
                Err(self.err(ErrorCode::Connection))
            }
            Err(e) => Err(MailError::new(ErrorCode::Connection, e.0)),
        }
    }

    fn tls_config(&self) -> Result<Arc<rustls::ClientConfig>> {
        match &self.config.tls_config {
            Some(c) => Ok(c.clone()),
            None => crate::conn::platform_tls_config()
                .map_err(|e| MailError::new(ErrorCode::Connection, e)),
        }
    }

    async fn handshake(
        &mut self,
        conn: &mut Connection,
        host: &str,
        failure: ErrorCode,
    ) -> Result<()> {
        let config = self.tls_config()?;
        match conn.upgrade_tls(config, host).await {
            Ok(()) => Ok(()),
            Err(TlsError::Certificate(e)) => {
                conn.note(&format!("certificate of {host} rejected: {e}"));
                Err(MailError::new(ErrorCode::Certificate, e))
            }
            Err(TlsError::Handshake(e)) => {
                conn.note(&format!("TLS handshake with {host} rejected: {e}"));
                self.last_tls_error = Some(e.clone());
                Err(MailError::new(failure, e))
            }
        }
    }

    /// `mailsmtp_socket_starttls`: every failure but a rejected certificate is
    /// `ErrorStartTLSNotAvailable`.
    async fn starttls(&mut self, conn: &mut Connection, host: &str) -> Result<()> {
        let not_available =
            |detail: String| MailError::new(ErrorCode::StartTLSNotAvailable, detail);
        if !conn.ext.as_ref().is_some_and(|e| e.starttls) {
            return Err(not_available("STARTTLS is not advertised".into()));
        }
        let reply = conn
            .command("STARTTLS\r\n", false)
            .await
            .map_err(|e| not_available(e.0))?;
        self.record(&reply);
        // Bytes after the 220 and before the handshake may be a response injection attack
        // (CVE-2011-0411 class); libetpan refuses the connection.
        if conn.has_buffered_input() {
            return Err(not_available("data received after STARTTLS reply".into()));
        }
        if reply.code != 220 {
            return Err(self.err(ErrorCode::StartTLSNotAvailable));
        }
        self.handshake(conn, host, ErrorCode::StartTLSNotAvailable)
            .await
    }

    /// `loginIfNeeded`
    pub async fn login_if_needed(&mut self) -> Result<()> {
        self.connect_if_needed().await?;
        if !self.logged_in {
            self.login().await?;
        }
        Ok(())
    }

    /// `SMTPSession::login`
    async fn login(&mut self) -> Result<()> {
        let username = self.config.username.clone();
        let outcome = match self.config.auth.clone() {
            SmtpAuth::XOAuth2(token) => self.auth_xoauth2(&username, &token).await,
            SmtpAuth::Password(password) if !username.is_empty() && !password.is_empty() => {
                self.auth_password(&username, &password).await
            }
            // No credentials: the server is expected to relay without AUTH.
            SmtpAuth::None | SmtpAuth::Password(_) => Ok(()),
        };
        match outcome {
            Ok(()) => {
                self.logged_in = true;
                Ok(())
            }
            Err(AuthFailure::Stream(e)) => Err(self.stream_err(ErrorCode::Connection, e)),
            Err(AuthFailure::NotImplemented(detail)) => {
                self.should_disconnect = true;
                Err(MailError::new(ErrorCode::NoImplementedAuthMethods, detail))
            }
            Err(AuthFailure::Refused) => {
                let tiscali = self.last_reply.as_ref().is_some_and(|r| {
                    r.code == 535 && r.text.contains("your password is too simple")
                });
                Err(self.err(if tiscali {
                    ErrorCode::TiscaliSimplePassword
                } else {
                    ErrorCode::Authentication
                }))
            }
        }
    }

    /// The mechanism `login` picks: PLAIN, LOGIN, DIGEST-MD5, CRAM-MD5 in that order, and
    /// PLAIN when the server lists none (or did not answer EHLO).
    fn choose_mechanism(&self) -> std::result::Result<Mechanism, AuthFailure> {
        let ext = self
            .conn
            .as_ref()
            .and_then(|c| c.ext.clone())
            .unwrap_or_default();
        for m in [
            Mechanism::Plain,
            Mechanism::Login,
            Mechanism::DigestMd5,
            Mechanism::CramMd5,
        ] {
            if ext.has_auth(m.name()) {
                return Ok(m);
            }
        }
        if let Some(m) = UNSUPPORTED_MECHANISMS.iter().find(|m| ext.has_auth(m)) {
            return Err(AuthFailure::NotImplemented(format!(
                "the server offers only {m}, which is not supported"
            )));
        }
        Ok(Mechanism::Plain)
    }

    /// `mailesmtp_auth_sasl`
    async fn auth_password(
        &mut self,
        username: &str,
        password: &str,
    ) -> std::result::Result<(), AuthFailure> {
        let mechanism = self.choose_mechanism()?;
        let mut client = SaslClient::new(mechanism, username, password, &self.config.host);
        let command = match client.initial_response() {
            Some(ir) => format!("AUTH {} {}\r\n", mechanism.name(), B64.encode(ir)),
            None => format!("AUTH {}\r\n", mechanism.name()),
        };
        let conn = self.conn.as_mut().expect("connected before login");
        let mut reply = conn
            .command(&command, true)
            .await
            .map_err(AuthFailure::Stream)?;
        for _ in 0..MAX_AUTH_ROUNDS {
            self.last_reply = Some(reply.clone());
            match reply.code {
                220 | 235 => return Ok(()),
                334 => {
                    let encoded = reply.first_line().trim();
                    let challenge = match B64.decode(encoded) {
                        Ok(c) => c,
                        // cyrus-sasl's LOGIN client never reads the prompt text.
                        Err(_) if mechanism == Mechanism::Login => Vec::new(),
                        Err(_) => return Err(self.abort_auth().await),
                    };
                    let response = match client.step(&challenge) {
                        Ok(r) => r,
                        Err(e) => {
                            if let Some(c) = &self.conn {
                                c.note(&format!("{} failed: {}", mechanism.name(), e.0));
                            }
                            return Err(self.abort_auth().await);
                        }
                    };
                    let line = format!("{}\r\n", B64.encode(response));
                    let conn = self.conn.as_mut().expect("connected before login");
                    reply = conn
                        .command(&line, true)
                        .await
                        .map_err(AuthFailure::Stream)?;
                }
                _ => return Err(AuthFailure::Refused),
            }
        }
        Err(self.abort_auth().await)
    }

    /// Cancels an exchange we cannot continue (RFC 4954 §4 `*`), so the session is left in
    /// a known state for a retry.
    async fn abort_auth(&mut self) -> AuthFailure {
        if let Some(conn) = self.conn.as_mut() {
            match conn.command("*\r\n", true).await {
                Ok(reply) => self.last_reply = Some(reply),
                Err(e) => return AuthFailure::Stream(e),
            }
        }
        AuthFailure::Refused
    }

    /// `mailsmtp_oauth2_authenticate` / `mailsmtp_oauth2_outlook_authenticate`. Gmail takes
    /// the token as the initial response and on failure sends a 334 with error JSON, which
    /// is acknowledged with an empty line; smtp-mail.outlook.com wants a bare
    /// `AUTH XOAUTH2` and the token after the 334 (`MCSMTPSession.cpp:638-642`).
    async fn auth_xoauth2(
        &mut self,
        username: &str,
        token: &str,
    ) -> std::result::Result<(), AuthFailure> {
        let payload = B64.encode(sasl::xoauth2_response(username, token));
        let outlook = self.config.outlook;
        let conn = self.conn.as_mut().expect("connected before login");
        let first = if outlook {
            "AUTH XOAUTH2\r\n".to_owned()
        } else {
            format!("AUTH XOAUTH2 {payload}\r\n")
        };
        let reply = conn
            .command(&first, true)
            .await
            .map_err(AuthFailure::Stream)?;
        self.last_reply = Some(reply.clone());
        match reply.code {
            220 | 235 => return Ok(()),
            334 => {}
            _ => return Err(AuthFailure::Refused),
        }
        let second = if outlook {
            format!("{payload}\r\n")
        } else {
            "\r\n".to_owned()
        };
        let conn = self.conn.as_mut().expect("connected before login");
        let reply = conn
            .command(&second, true)
            .await
            .map_err(AuthFailure::Stream)?;
        self.last_reply = Some(reply.clone());
        match reply.code {
            220 | 235 => Ok(()),
            _ => Err(AuthFailure::Refused),
        }
    }

    /// `checkAccount`: MAIL FROM and RCPT TO the account's own address, then the test email.
    pub async fn check_account(&mut self, from: &str) -> Result<()> {
        self.login_if_needed().await?;
        let Some(email) = message::build_test_email(from) else {
            return Err(MailError::new(
                ErrorCode::InvalidAccount,
                format!("{from} is not a valid address"),
            ));
        };
        let steps: [(String, &[u16], ErrorCode); 3] = [
            (
                format!("MAIL FROM:<{from}>\r\n"),
                &[250],
                ErrorCode::InvalidAccount,
            ),
            (
                format!("RCPT TO:<{from}>\r\n"),
                &[250, 251],
                ErrorCode::InvalidRelaySMTP,
            ),
            ("DATA\r\n".to_owned(), &[354], ErrorCode::InvalidRelaySMTP),
        ];
        for (command, ok, failure) in steps {
            let conn = self.conn.as_mut().expect("logged in");
            let reply = match conn.command(&command, false).await {
                Ok(r) => r,
                Err(e) => return Err(self.stream_err(ErrorCode::Connection, e)),
            };
            self.record(&reply);
            if !ok.contains(&reply.code) {
                return Err(self.err(failure));
            }
        }
        let conn = self.conn.as_mut().expect("logged in");
        let reply = match conn.send_data(&message::dot_stuff(&email)).await {
            Ok(r) => r,
            Err(e) => return Err(self.stream_err(ErrorCode::Connection, e)),
        };
        self.record(&reply);
        if reply.code != 250 {
            return Err(self.err(ErrorCode::InvalidRelaySMTP));
        }
        Ok(())
    }

    /// `internalSendMessage`. Each send ends the connection (libetpan QUITs after
    /// `mailesmtp_send`, or closes after the pipelined variant), so the next one logs in
    /// again.
    pub async fn send_message(
        &mut self,
        from: &str,
        recipients: &[String],
        data: &[u8],
    ) -> Result<()> {
        if from.is_empty() {
            return Err(MailError::new(ErrorCode::NoSender, ""));
        }
        if recipients.is_empty() {
            return Err(MailError::new(ErrorCode::NoRecipient, ""));
        }
        // Outlook.com strips Bcc itself and keeps it in the Sent copy it files.
        let data = if self.config.outlook {
            data.to_vec()
        } else {
            message::filter_bcc(data)
        };

        self.login_if_needed().await?;
        let outcome = self.transaction(from, recipients, &data).await;
        self.disconnect().await;
        match outcome {
            Ok(()) => Ok(()),
            Err(failure) => Err(self.map_send_failure(failure)),
        }
    }

    async fn transaction(
        &mut self,
        from: &str,
        recipients: &[String],
        data: &[u8],
    ) -> std::result::Result<(), TransactionFailure> {
        let conn = self.conn.as_mut().expect("logged in");
        let size_limit = conn.ext.as_ref().and_then(|e| e.size);
        let mail = match size_limit {
            Some(limit) if limit != 0 && data.len() as u64 > limit => {
                return Err(TransactionFailure::ExceedStorage)
            }
            Some(_) => format!("MAIL FROM:<{from}> SIZE={}\r\n", data.len()),
            None => format!("MAIL FROM:<{from}>\r\n"),
        };
        let reply = self.step(&mail).await?;
        match reply.code {
            250 => {}
            552 => return Err(TransactionFailure::ExceedStorage),
            550 => return Err(TransactionFailure::MailboxUnavailable),
            _ => return Err(TransactionFailure::Other),
        }
        for rcpt in recipients {
            let reply = self.step(&format!("RCPT TO:<{rcpt}>\r\n")).await?;
            match reply.code {
                250 | 251 => {}
                550 | 450 => return Err(TransactionFailure::MailboxUnavailable),
                552 => return Err(TransactionFailure::ExceedStorage),
                _ => return Err(TransactionFailure::Other),
            }
        }
        if self.step("DATA\r\n").await?.code != 354 {
            return Err(TransactionFailure::Other);
        }
        let conn = self.conn.as_mut().expect("logged in");
        let reply = conn
            .send_data(&message::dot_stuff(data))
            .await
            .map_err(TransactionFailure::Stream)?;
        self.record(&reply);
        match reply.code {
            250 => Ok(()),
            552 => Err(TransactionFailure::ExceedStorage),
            _ => Err(TransactionFailure::Other),
        }
    }

    async fn step(&mut self, command: &str) -> std::result::Result<Reply, TransactionFailure> {
        let conn = self.conn.as_mut().expect("logged in");
        let reply = conn
            .command(command, false)
            .await
            .map_err(TransactionFailure::Stream)?;
        self.record(&reply);
        Ok(reply)
    }

    /// The error mapping at the end of `internalSendMessage`, including the provider texts
    /// it recognises (Outlook.com's 5.3.4 / 5.7.1, Yahoo's 521 and 554).
    fn map_send_failure(&mut self, failure: TransactionFailure) -> MailError {
        let (code, text) = self
            .last_reply
            .as_ref()
            .map(|r| (r.code, r.text.clone()))
            .unwrap_or_default();
        let error = match failure {
            TransactionFailure::Stream(e) => return self.stream_err(ErrorCode::Connection, e),
            TransactionFailure::ExceedStorage if text.contains("5.7.0") => {
                ErrorCode::SendMessageIllegalAttachment
            }
            TransactionFailure::ExceedStorage => ErrorCode::StorageLimit,
            TransactionFailure::MailboxUnavailable => ErrorCode::SendMessageNotAllowed,
            TransactionFailure::Other => match code {
                550 if text.starts_with("5.3.4 ") => ErrorCode::NeedsConnectToWebmail,
                550 if text.starts_with("5.7.1 ") => ErrorCode::SendMessageNotAllowed,
                521 if text.contains("over the limit") => {
                    ErrorCode::YahooSendMessageDailyLimitExceeded
                }
                554 if text.contains("spam") => ErrorCode::YahooSendMessageSpamSuspected,
                _ => ErrorCode::SendMessage,
            },
        };
        self.err(error)
    }
}
