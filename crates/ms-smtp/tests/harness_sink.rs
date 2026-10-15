//! `SmtpClient` against the harness SMTP sink (`apps/engine/test/fakeimap/smtp.py`), the
//! server the send scenarios run against. Needs `python3` on PATH.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use ms_core::protolog::AccumulatingLogger;
use ms_core::{Account, ErrorCode, Security};
use ms_smtp::{SmtpAuth, SmtpClient, SmtpConfig, TEST_EMAIL_SUBJECT};
use serde_json::{json, Value};

struct Sink {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    port: u16,
}

impl Sink {
    fn start(options: Value) -> Sink {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let harness = manifest.join("../../apps/engine/test");
        let driver = manifest.join("tests/sink_driver.py");
        let mut child = Command::new("python3")
            .arg(&driver)
            .arg(&harness)
            .arg(options.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3 is needed to run the harness SMTP sink");
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let hello: Value = serde_json::from_str(&line).expect("sink start line");
        let port = hello["port"].as_u64().unwrap() as u16;
        Sink {
            child,
            stdin,
            stdout,
            port,
        }
    }

    fn dump(&mut self) -> Value {
        writeln!(self.stdin, "dump").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    /// The commands the server received, in order.
    fn client_lines(&mut self) -> Vec<String> {
        self.dump()["transcript"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] == "C")
            .map(|e| e[1].as_str().unwrap().to_owned())
            .collect()
    }

    fn messages(&mut self) -> Vec<Value> {
        self.dump()["messages"].as_array().unwrap().clone()
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

fn config(port: u16, user: &str, password: &str) -> SmtpConfig {
    SmtpConfig {
        host: "127.0.0.1".into(),
        port,
        security: Security::Clear,
        username: user.into(),
        auth: SmtpAuth::Password(password.into()),
        allow_obsolete_tls: false,
        outlook: false,
        timeout: Duration::from_secs(10),
        logger: None,
        tls_config: None,
    }
}

fn with_transcript(mut cfg: SmtpConfig) -> (SmtpConfig, Arc<AccumulatingLogger>) {
    let log = Arc::new(AccumulatingLogger::default());
    cfg.logger = Some(log.clone());
    (cfg, log)
}

const RAW: &[u8] = b"From: Me <me@example.test>\r\n\
To: Ann <ann@example.test>\r\n\
Cc: bob@example.test\r\n\
Bcc: carl@example.test\r\n\
Subject: Hello\r\n\
Message-ID: <m1@example.test>\r\n\
\r\n\
First line\r\n\
.hidden dot line\r\n";

#[test]
fn login_uses_plain_and_keeps_credentials_out_of_the_transcript() {
    let mut sink = Sink::start(json!({"credentials": ["test", "s3cret-pass"]}));
    let (cfg, log) = with_transcript(config(sink.port, "test", "s3cret-pass"));
    let mut smtp = SmtpClient::new(cfg);
    smtp.login().expect("login");

    let lines = sink.client_lines();
    assert!(lines[0].starts_with("EHLO "), "{lines:?}");
    assert!(lines[1].starts_with("AUTH PLAIN "), "{lines:?}");

    let transcript = log.contents();
    assert!(transcript.contains("220 mail.example.test ESMTP ready"));
    assert!(transcript.contains("EHLO "));
    assert!(transcript.contains("235 2.7.0 Authentication successful"));
    assert!(
        !transcript.lines().any(|l| l.starts_with("AUTH")),
        "{transcript}"
    );
    let payload = &lines[1]["AUTH PLAIN ".len()..];
    assert!(!transcript.contains(payload));
    assert!(!transcript.contains("s3cret-pass"));
}

#[test]
fn wrong_password_is_an_authentication_error() {
    let sink = Sink::start(json!({"credentials": ["test", "right"]}));
    let mut smtp = SmtpClient::new(config(sink.port, "test", "wrong"));
    let err = smtp.login().unwrap_err();
    assert_eq!(err.code, ErrorCode::Authentication);
    assert_eq!(err.code.name(), "ErrorAuthentication");
    assert_eq!(smtp.last_response_code(), 535);
    assert!(smtp
        .failure_report()
        .contains("SMTP Last Response Code: 535"));
}

#[test]
fn check_account_sends_the_test_email_to_self() {
    let mut sink = Sink::start(json!({}));
    let (cfg, log) = with_transcript(config(sink.port, "test", "pass"));
    let mut smtp = SmtpClient::new(cfg);
    smtp.check_account("me@example.test")
        .expect("check_account");

    let msgs = sink.messages();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["sender"], "me@example.test");
    assert_eq!(msgs[0]["recipients"], json!(["me@example.test"]));
    let raw = msgs[0]["raw"].as_str().unwrap();
    assert!(raw.contains(&format!("Subject: {TEST_EMAIL_SUBJECT}\r\n")));
    assert!(raw.contains("To: me@example.test\r\n"));
    let transcript = log.contents();
    assert!(transcript.contains("MAIL FROM:<me@example.test>\r\n"));
    assert!(transcript.contains("RCPT TO:<me@example.test>\r\n"));
    assert!(transcript.contains("354 End data"));
}

#[test]
fn verify_account_in_login_mode_sends_nothing() {
    let mut sink = Sink::start(json!({}));
    let account = Account::from_json(json!({
        "id": "c0ffee-harness",
        "emailAddress": "me@example.test",
        "settings": {
            "smtp_host": "127.0.0.1",
            "smtp_port": sink.port,
            "smtp_username": "test",
            "smtp_password": "pass",
            "smtp_security": "none",
            "smtp_verification": "login",
        }
    }))
    .unwrap();
    let mut smtp = SmtpClient::new(SmtpConfig::for_account(&account, None));
    smtp.verify_account(&account)
        .expect("login-only verification");
    let lines = sink.client_lines();
    assert!(!lines.iter().any(|l| l.starts_with("MAIL")), "{lines:?}");
    assert!(sink.messages().is_empty());

    let mut account_json = account.to_json();
    account_json["settings"]["smtp_verification"] = json!("send");
    let account = Account::from_json(account_json).unwrap();
    let mut smtp = SmtpClient::new(SmtpConfig::for_account(&account, None));
    smtp.verify_account(&account).expect("full verification");
    assert_eq!(sink.messages().len(), 1);
}

#[test]
fn send_takes_the_envelope_from_the_headers_and_strips_bcc() {
    let mut sink = Sink::start(json!({}));
    let mut smtp = SmtpClient::new(config(sink.port, "test", "pass"));
    smtp.send(RAW).expect("send");

    let msgs = sink.messages();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["sender"], "me@example.test");
    assert_eq!(
        msgs[0]["recipients"],
        json!(["ann@example.test", "bob@example.test", "carl@example.test"])
    );
    let raw = msgs[0]["raw"].as_str().unwrap();
    assert!(!raw.contains("Bcc:"), "{raw}");
    assert!(raw.contains("To: Ann <ann@example.test>\r\n"));
    assert!(raw.contains("\r\n.hidden dot line\r\n"), "{raw}");

    // Each send ends its connection, so the next one logs in again.
    assert!(smtp.is_disconnected());
    smtp.send(RAW).expect("second send");
    // QUIT is sent without waiting for the 221, so the sink may log it a moment later.
    let mut lines = sink.client_lines();
    for _ in 0..50 {
        if lines.iter().filter(|l| *l == "QUIT").count() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        lines = sink.client_lines();
    }
    assert_eq!(lines.iter().filter(|l| l.starts_with("AUTH")).count(), 2);
    assert_eq!(lines.iter().filter(|l| *l == "QUIT").count(), 2);
    let mail = lines.iter().find(|l| l.starts_with("MAIL")).unwrap();
    assert!(
        mail.starts_with("MAIL FROM:<me@example.test> SIZE="),
        "{mail}"
    );
}

#[test]
fn multisend_sets_one_recipient_and_keeps_the_headers() {
    let mut sink = Sink::start(json!({}));
    let mut smtp = SmtpClient::new(config(sink.port, "test", "pass"));
    for rcpt in ["ann@example.test", "bob@example.test"] {
        smtp.send_to("me@example.test", &[rcpt.to_owned()], RAW)
            .expect("send_to");
    }
    let msgs = sink.messages();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["recipients"], json!(["ann@example.test"]));
    assert_eq!(msgs[1]["recipients"], json!(["bob@example.test"]));
    for m in &msgs {
        let raw = m["raw"].as_str().unwrap();
        assert!(raw.contains("To: Ann <ann@example.test>\r\nCc: bob@example.test\r\n"));
        assert!(!raw.contains("carl@example.test"));
    }
}

#[test]
fn ehlo_name_passes_postfix_helo_checks() {
    let mut sink = Sink::start(json!({"reject_non_fqdn_helo": true}));
    let mut smtp = SmtpClient::new(config(sink.port, "test", "pass"));
    smtp.send(RAW).expect("send with HELO validation");
    let helo = sink.messages()[0]["helo"].as_str().unwrap().to_owned();
    assert!(
        helo == "[127.0.0.1]" || (helo.contains('.') && !helo.ends_with('.')),
        "{helo}"
    );
}

#[test]
fn no_credentials_sends_without_auth() {
    let mut sink = Sink::start(json!({"require_auth": false}));
    let mut cfg = config(sink.port, "", "");
    cfg.auth = SmtpAuth::None;
    let mut smtp = SmtpClient::new(cfg);
    smtp.send(RAW).expect("send without AUTH");
    assert!(!sink.client_lines().iter().any(|l| l.starts_with("AUTH")));
    assert_eq!(sink.messages().len(), 1);
}

#[test]
fn unauthenticated_mail_is_refused() {
    let sink = Sink::start(json!({}));
    let mut cfg = config(sink.port, "", "");
    cfg.auth = SmtpAuth::None;
    let mut smtp = SmtpClient::new(cfg);
    // 530 to MAIL is neither 550 nor 552 for libetpan, so it is a plain send failure.
    assert_eq!(smtp.send(RAW).unwrap_err().code, ErrorCode::SendMessage);
    assert_eq!(smtp.last_response_code(), 530);
    let err = smtp.check_account("me@example.test").unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidAccount);
}

#[test]
fn envelope_errors() {
    let sink = Sink::start(json!({}));
    let mut smtp = SmtpClient::new(config(sink.port, "test", "pass"));
    let no_from = b"To: a@example.test\r\n\r\nx";
    assert_eq!(smtp.send(no_from).unwrap_err().code, ErrorCode::NoSender);
    let no_rcpt = b"From: a@example.test\r\n\r\nx";
    assert_eq!(smtp.send(no_rcpt).unwrap_err().code, ErrorCode::NoRecipient);
}

#[test]
fn connection_refused_is_a_connection_error() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut smtp = SmtpClient::new(config(port, "test", "pass"));
    let err = smtp.login().unwrap_err();
    assert_eq!(err.code, ErrorCode::Connection);
    assert!(ms_core::Error::from(err).is_offline());
}
