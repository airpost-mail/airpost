// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `SmtpClient` against a small in-test SMTP server, for what the harness sink does not
//! cover: TLS and STARTTLS, CRAM-MD5 and DIGEST-MD5 with the RFC 2195 / RFC 2831 vectors,
//! both XOAUTH2 variants, and the reply-to-error mapping of `internalSendMessage`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use md5::{Digest, Md5};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tern_core::protolog::AccumulatingLogger;
use tern_core::{ErrorCode, Security};
use tern_smtp::{SmtpAuth, SmtpClient, SmtpConfig};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const CA_PEM: &[u8] = include_bytes!("fixtures/ca.pem");
const SERVER_PEM: &[u8] = include_bytes!("fixtures/server.pem");
const SERVER_KEY: &[u8] = include_bytes!("fixtures/server.key");

const CRAM_CHALLENGE: &str = "<1896.697170952@postoffice.reston.mci.net>";
const DIGEST_CHALLENGE: &str = r#"realm="elwood.innosoft.com",nonce="OA6MG9tEQGm2hh",qop="auth",algorithm=md5-sess,charset=utf-8"#;

const RAW: &[u8] = b"From: me@example.test\r\nTo: you@example.test\r\nSubject: s\r\n\r\nbody\r\n";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ServerTls {
    None,
    Implicit,
    StartTls,
}

#[derive(Clone)]
struct Script {
    tls: ServerTls,
    advertise_starttls: bool,
    greeting: &'static str,
    ehlo_ok: bool,
    auth: &'static str,
    size: Option<u64>,
    user: &'static str,
    password: &'static str,
    /// Replaces the success reply to AUTH (after any challenge exchange).
    auth_reply: Option<&'static str>,
    /// DIGEST-MD5: send this rspauth instead of the correct one.
    bad_rspauth: bool,
    /// XOAUTH2: answer with the Gmail error challenge and then this reply.
    xoauth2_error: Option<&'static str>,
    starttls_inject: bool,
    mail_reply: &'static str,
    rcpt_reply: &'static str,
    data_end_reply: &'static str,
    /// Read the DATA payload at about 1.3 MB/s for its first two seconds.
    slow_data: bool,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            tls: ServerTls::None,
            advertise_starttls: false,
            greeting: "220 scripted.example.test ESMTP",
            ehlo_ok: true,
            auth: "PLAIN LOGIN",
            size: None,
            user: "user",
            password: "pass",
            auth_reply: None,
            bad_rspauth: false,
            xoauth2_error: None,
            starttls_inject: false,
            mail_reply: "250 2.1.0 Ok",
            rcpt_reply: "250 2.1.5 Ok",
            data_end_reply: "250 2.0.0 Ok: queued",
            slow_data: false,
        }
    }
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ServerConnection, TcpStream>>),
}

impl Stream {
    fn io(&mut self) -> &mut dyn ReadWrite {
        match self {
            Stream::Plain(s) => s,
            Stream::Tls(s) => s.as_mut(),
        }
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

struct Server {
    port: u16,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start(script: Script) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let record = lines.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { return };
                let mut session = ServerSession {
                    script: script.clone(),
                    stream: Some(Stream::Plain(conn)),
                    lines: record.clone(),
                };
                std::thread::spawn(move || session.run());
            }
        });
        Server { port, lines }
    }

    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

fn server_tls() -> Arc<rustls::ServerConfig> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(SERVER_PEM)
        .map(|c| c.unwrap())
        .collect();
    let key = PrivateKeyDer::from_pem_slice(SERVER_KEY).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap(),
    )
}

fn client_tls() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(CA_PEM) {
        roots.add(c.unwrap()).unwrap();
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

struct ServerSession {
    script: Script,
    stream: Option<Stream>,
    lines: Arc<Mutex<Vec<String>>>,
}

impl ServerSession {
    fn send(&mut self, line: &str) -> std::io::Result<()> {
        let io = self.stream.as_mut().unwrap().io();
        io.write_all(format!("{line}\r\n").as_bytes())?;
        io.flush()
    }

    fn read_line(&mut self) -> std::io::Result<String> {
        let io = self.stream.as_mut().unwrap().io();
        let mut out = Vec::new();
        let mut b = [0u8];
        loop {
            if io.read(&mut b)? == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            if b[0] == b'\n' {
                break;
            }
            out.push(b[0]);
        }
        let line = String::from_utf8_lossy(&out)
            .trim_end_matches('\r')
            .to_owned();
        self.lines.lock().unwrap().push(line.clone());
        Ok(line)
    }

    fn start_tls(&mut self) {
        let Some(Stream::Plain(tcp)) = self.stream.take() else {
            panic!("already TLS")
        };
        let conn = rustls::ServerConnection::new(server_tls()).unwrap();
        self.stream = Some(Stream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))));
    }

    fn run(&mut self) -> std::io::Result<()> {
        let s = self.script.clone();
        if s.tls == ServerTls::Implicit {
            self.start_tls();
        }
        self.send(s.greeting)?;
        if !s.greeting.starts_with("220") {
            return Ok(());
        }
        loop {
            let line = self.read_line()?;
            let (verb, arg) = line.split_once(' ').unwrap_or((&line, ""));
            match verb.to_ascii_uppercase().as_str() {
                "EHLO" if !s.ehlo_ok => self.send("502 5.5.2 EHLO not supported")?,
                "EHLO" => {
                    self.send("250-scripted.example.test")?;
                    if s.advertise_starttls && s.tls == ServerTls::StartTls {
                        self.send("250-STARTTLS")?;
                    }
                    if let Some(size) = s.size {
                        self.send(&format!("250-SIZE {size}"))?;
                    }
                    self.send(&format!("250-AUTH {}", s.auth))?;
                    self.send("250 8BITMIME")?;
                }
                "HELO" => self.send("250 scripted.example.test")?,
                "STARTTLS" => {
                    if s.starttls_inject {
                        let io = self.stream.as_mut().unwrap().io();
                        io.write_all(b"220 2.0.0 Ready\r\n250 injected\r\n")?;
                        io.flush()?;
                        return Ok(());
                    }
                    self.send("220 2.0.0 Ready to start TLS")?;
                    self.start_tls();
                }
                "AUTH" => self.auth(arg)?,
                "MAIL" => self.send(s.mail_reply)?,
                "RCPT" => self.send(s.rcpt_reply)?,
                "DATA" => {
                    self.send("354 go ahead")?;
                    if s.slow_data {
                        self.read_data_slowly()?;
                    } else {
                        while self.read_line()? != "." {}
                    }
                    self.send(s.data_end_reply)?;
                }
                "QUIT" => {
                    self.send("221 bye")?;
                    return Ok(());
                }
                _ => self.send("502 5.5.2 unknown command")?,
            }
        }
    }

    fn read_data_slowly(&mut self) -> std::io::Result<()> {
        let io = self.stream.as_mut().unwrap().io();
        let mut tail = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        let started = std::time::Instant::now();
        loop {
            // Only the start is slow: once the client has written its last byte it waits
            // for the reply under the same timeout, and the socket buffers (several MB on
            // loopback) must drain well before that.
            if started.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(50));
            }
            let n = io.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            tail.extend_from_slice(&chunk[..n]);
            if tail.ends_with(b"\r\n.\r\n") {
                return Ok(());
            }
            let keep = tail.len().saturating_sub(5);
            tail.drain(..keep);
        }
    }

    fn auth_ok(&mut self, ok: bool) -> std::io::Result<()> {
        let reply = if ok {
            self.script
                .auth_reply
                .unwrap_or("235 2.7.0 Authentication successful")
        } else {
            "535 5.7.8 Authentication credentials invalid"
        };
        self.send(reply)
    }

    fn auth(&mut self, arg: &str) -> std::io::Result<()> {
        let s = self.script.clone();
        let (mech, initial) = arg.split_once(' ').unwrap_or((arg, ""));
        match mech.to_ascii_uppercase().as_str() {
            "PLAIN" => {
                let decoded = B64.decode(initial).unwrap_or_default();
                let expected = format!("\0{}\0{}", s.user, s.password);
                self.auth_ok(decoded == expected.as_bytes())
            }
            "LOGIN" => {
                self.send("334 VXNlcm5hbWU6")?;
                let user = B64.decode(self.read_line()?).unwrap_or_default();
                self.send("334 UGFzc3dvcmQ6")?;
                let pass = B64.decode(self.read_line()?).unwrap_or_default();
                self.auth_ok(user == s.user.as_bytes() && pass == s.password.as_bytes())
            }
            "CRAM-MD5" => {
                self.send(&format!("334 {}", B64.encode(CRAM_CHALLENGE)))?;
                let resp = B64.decode(self.read_line()?).unwrap_or_default();
                // RFC 2195 §2: the known answer for tim / tanstaaftanstaaf.
                let ok =
                    s.user != "tim" || resp == b"tim b913a602c7eda7a495b4e6e7334d3890".as_slice();
                self.auth_ok(ok && resp.starts_with(format!("{} ", s.user).as_bytes()))
            }
            "DIGEST-MD5" => {
                self.send(&format!("334 {}", B64.encode(DIGEST_CHALLENGE)))?;
                let resp = String::from_utf8(B64.decode(self.read_line()?).unwrap()).unwrap();
                let get = |k: &str| directive(&resp, k);
                let uri = get("digest-uri");
                let expected = digest(&s, &get("cnonce"), &uri, "AUTHENTICATE");
                if get("response") != expected
                    || get("nonce") != "OA6MG9tEQGm2hh"
                    || get("username") != s.user
                    || get("qop") != "auth"
                {
                    return self.auth_ok(false);
                }
                let rspauth = if s.bad_rspauth {
                    "0".repeat(32)
                } else {
                    digest(&s, &get("cnonce"), &uri, "")
                };
                self.send(&format!("334 {}", B64.encode(format!("rspauth={rspauth}"))))?;
                let last = self.read_line()?;
                if last == "*" {
                    return self.send("501 5.0.0 AUTH aborted");
                }
                self.auth_ok(last.is_empty())
            }
            "XOAUTH2" => {
                let payload = if initial.is_empty() {
                    self.send("334 ")?;
                    self.read_line()?
                } else {
                    initial.to_owned()
                };
                let decoded = B64.decode(payload).unwrap_or_default();
                let expected = format!("user={}\x01auth=Bearer {}\x01\x01", s.user, s.password);
                if let Some(reply) = s.xoauth2_error {
                    self.send("334 eyJzdGF0dXMiOiI0MDEifQ==")?;
                    assert_eq!(self.read_line()?, "");
                    return self.send(reply);
                }
                self.auth_ok(decoded == expected.as_bytes())
            }
            _ => self.send("504 5.5.4 Unrecognized authentication type"),
        }
    }
}

fn directive(resp: &str, key: &str) -> String {
    for part in resp.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            if k.trim() == key {
                return v.trim().trim_matches('"').to_owned();
            }
        }
    }
    String::new()
}

fn md5_hex(data: &[u8]) -> String {
    Md5::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// RFC 2831 §2.1.2.1, computed independently of the client.
fn digest(s: &Script, cnonce: &str, uri: &str, method: &str) -> String {
    let inner = Md5::digest(format!("{}:elwood.innosoft.com:{}", s.user, s.password));
    let mut a1 = inner.to_vec();
    a1.extend_from_slice(format!(":OA6MG9tEQGm2hh:{cnonce}").as_bytes());
    let ha1 = md5_hex(&a1);
    let ha2 = md5_hex(format!("{method}:{uri}").as_bytes());
    md5_hex(format!("{ha1}:OA6MG9tEQGm2hh:00000001:{cnonce}:auth:{ha2}").as_bytes())
}

fn client(server: &Server, security: Security, user: &str, auth: SmtpAuth) -> SmtpClient {
    SmtpClient::new(SmtpConfig {
        host: "127.0.0.1".into(),
        port: server.port,
        security,
        username: user.into(),
        auth,
        allow_obsolete_tls: false,
        outlook: false,
        timeout: Duration::from_secs(5),
        logger: None,
        tls_config: Some(client_tls()),
    })
}

fn password(p: &str) -> SmtpAuth {
    SmtpAuth::Password(p.into())
}

#[test]
fn cram_md5_rfc2195() {
    let server = Server::start(Script {
        auth: "CRAM-MD5",
        user: "tim",
        password: "tanstaaftanstaaf",
        ..Script::default()
    });
    let mut smtp = client(
        &server,
        Security::Clear,
        "tim",
        password("tanstaaftanstaaf"),
    );
    smtp.login().expect("CRAM-MD5 login");
    let lines = server.lines();
    assert!(lines.contains(&"AUTH CRAM-MD5".to_owned()), "{lines:?}");
    assert!(lines.contains(&"dGltIGI5MTNhNjAyYzdlZGE3YTQ5NWI0ZTZlNzMzNGQzODkw".to_owned()));

    let mut wrong = client(&server, Security::Clear, "tim", password("nope"));
    assert_eq!(wrong.login().unwrap_err().code, ErrorCode::Authentication);
}

#[test]
fn digest_md5_rfc2831() {
    let script = Script {
        auth: "DIGEST-MD5 CRAM-MD5",
        user: "chris",
        password: "secret",
        ..Script::default()
    };
    let server = Server::start(script.clone());
    let log = Arc::new(AccumulatingLogger::default());
    let mut smtp = client(&server, Security::Clear, "chris", password("secret"));
    let mut cfg = smtp.config().clone();
    cfg.logger = Some(log.clone());
    smtp = SmtpClient::new(cfg);
    smtp.login().expect("DIGEST-MD5 login");
    assert!(server.lines().contains(&"AUTH DIGEST-MD5".to_owned()));
    // Neither the response nor the empty acknowledgement reaches the transcript.
    assert!(!log.contents().contains("response="));
    assert!(!log.contents().lines().any(|l| l.starts_with("AUTH")));

    let bad = Server::start(Script {
        bad_rspauth: true,
        ..script
    });
    let mut smtp = client(&bad, Security::Clear, "chris", password("secret"));
    assert_eq!(smtp.login().unwrap_err().code, ErrorCode::Authentication);
    assert!(bad.lines().contains(&"*".to_owned()));
}

#[test]
fn login_mechanism_and_preference_order() {
    let server = Server::start(Script {
        auth: "CRAM-MD5 LOGIN",
        ..Script::default()
    });
    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    smtp.login().expect("LOGIN");
    assert!(server.lines().contains(&"AUTH LOGIN".to_owned()));
}

#[test]
fn only_unsupported_mechanisms() {
    let server = Server::start(Script {
        auth: "GSSAPI NTLM",
        ..Script::default()
    });
    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    let err = smtp.login().unwrap_err();
    assert_eq!(err.code, ErrorCode::NoImplementedAuthMethods);
    assert!(!server.lines().iter().any(|l| l.starts_with("AUTH")));
}

#[test]
fn helo_fallback_defaults_to_plain() {
    let server = Server::start(Script {
        ehlo_ok: false,
        ..Script::default()
    });
    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    smtp.send(RAW).expect("send after HELO");
    let lines = server.lines();
    assert!(lines.iter().any(|l| l.starts_with("HELO ")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("AUTH PLAIN ")));
    // Without ESMTP there is no SIZE parameter.
    assert!(lines.contains(&"MAIL FROM:<me@example.test>".to_owned()));
}

#[test]
fn tiscali_simple_password() {
    let server = Server::start(Script {
        auth_reply: Some("535 5.7.8 your password is too simple"),
        ..Script::default()
    });
    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    assert_eq!(
        smtp.login().unwrap_err().code,
        ErrorCode::TiscaliSimplePassword
    );
}

#[test]
fn xoauth2_gmail_style() {
    let server = Server::start(Script {
        auth: "XOAUTH2 PLAIN",
        user: "me@gmail.test",
        password: "ya29.token",
        ..Script::default()
    });
    let mut smtp = client(
        &server,
        Security::Clear,
        "me@gmail.test",
        SmtpAuth::XOAuth2("ya29.token".into()),
    );
    smtp.login().expect("XOAUTH2");
    let auth = server
        .lines()
        .into_iter()
        .find(|l| l.starts_with("AUTH"))
        .unwrap();
    assert!(auth.starts_with("AUTH XOAUTH2 "), "{auth}");

    let refused = Server::start(Script {
        auth: "XOAUTH2",
        xoauth2_error: Some("535 5.7.8 Username and Password not accepted"),
        ..Script::default()
    });
    let mut smtp = client(
        &refused,
        Security::Clear,
        "me@gmail.test",
        SmtpAuth::XOAuth2("expired".into()),
    );
    assert_eq!(smtp.login().unwrap_err().code, ErrorCode::Authentication);
}

#[test]
fn xoauth2_outlook_style() {
    let server = Server::start(Script {
        auth: "LOGIN XOAUTH2",
        user: "me@outlook.test",
        password: "eyJ0.token",
        ..Script::default()
    });
    let mut cfg = client(
        &server,
        Security::Clear,
        "me@outlook.test",
        SmtpAuth::XOAuth2("eyJ0.token".into()),
    )
    .config()
    .clone();
    cfg.outlook = true;
    let mut smtp = SmtpClient::new(cfg);
    smtp.login().expect("Outlook XOAUTH2");
    assert!(server.lines().contains(&"AUTH XOAUTH2".to_owned()));
}

#[test]
fn outlook_keeps_bcc_for_the_server() {
    let server = Server::start(Script::default());
    let raw = b"From: me@example.test\r\nBcc: hidden@example.test\r\nSubject: s\r\n\r\nbody\r\n";
    let mut cfg = client(&server, Security::Clear, "user", password("pass"))
        .config()
        .clone();
    cfg.outlook = true;
    SmtpClient::new(cfg).send(raw).expect("send");
    let lines = server.lines();
    assert!(lines.contains(&"Bcc: hidden@example.test".to_owned()));
    assert!(lines.contains(&"RCPT TO:<hidden@example.test>".to_owned()));

    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    smtp.send(raw).expect("send");
    let lines = server.lines();
    assert_eq!(lines.iter().filter(|l| l.starts_with("Bcc:")).count(), 1);
    assert!(lines.contains(&"To: Undisclosed recipients:;".to_owned()));
}

#[test]
fn implicit_tls() {
    let server = Server::start(Script {
        tls: ServerTls::Implicit,
        ..Script::default()
    });
    let mut smtp = client(&server, Security::Tls, "user", password("pass"));
    smtp.send(RAW).expect("send over TLS");
    assert!(server.lines().iter().any(|l| l == "body"));

    // The same server with the platform trust store: the test CA is not in it.
    let mut cfg = smtp.config().clone();
    cfg.tls_config = None;
    cfg.host = "localhost".into();
    let mut untrusted = SmtpClient::new(cfg);
    assert_eq!(untrusted.login().unwrap_err().code, ErrorCode::Certificate);
    assert_eq!(untrusted.last_tls_error(), None);
}

#[test]
fn implicit_tls_against_plain_server_is_a_connection_error() {
    let server = Server::start(Script::default());
    let mut smtp = client(&server, Security::Tls, "user", password("pass"));
    let err = smtp.login().unwrap_err();
    assert_eq!(err.code, ErrorCode::Connection);
    assert!(smtp.last_tls_error().is_some());
}

#[test]
fn starttls_by_dns_name() {
    let server = Server::start(Script {
        tls: ServerTls::StartTls,
        advertise_starttls: true,
        ..Script::default()
    });
    let mut cfg = client(&server, Security::StartTls, "user", password("pass"))
        .config()
        .clone();
    cfg.host = "localhost".into();
    let log = Arc::new(AccumulatingLogger::default());
    cfg.logger = Some(log.clone());
    let mut smtp = SmtpClient::new(cfg);
    smtp.send(RAW).expect("send after STARTTLS");
    let lines = server.lines();
    let ehlos = lines.iter().filter(|l| l.starts_with("EHLO")).count();
    assert_eq!(ehlos, 2, "EHLO is repeated after STARTTLS: {lines:?}");
    // The transcript is plaintext on both sides of the handshake.
    assert!(log.contents().contains("STARTTLS\r\n"));
    assert!(log.contents().contains("MAIL FROM:<me@example.test>"));
}

#[test]
fn starttls_failures() {
    let missing = Server::start(Script {
        tls: ServerTls::StartTls,
        advertise_starttls: false,
        ..Script::default()
    });
    let mut smtp = client(&missing, Security::StartTls, "user", password("pass"));
    assert_eq!(
        smtp.login().unwrap_err().code,
        ErrorCode::StartTLSNotAvailable
    );

    let injected = Server::start(Script {
        tls: ServerTls::StartTls,
        advertise_starttls: true,
        starttls_inject: true,
        ..Script::default()
    });
    let mut smtp = client(&injected, Security::StartTls, "user", password("pass"));
    assert_eq!(
        smtp.login().unwrap_err().code,
        ErrorCode::StartTLSNotAvailable
    );
}

#[test]
fn greeting_refusal_is_a_connection_error() {
    let server = Server::start(Script {
        greeting: "554 5.3.2 no service",
        ..Script::default()
    });
    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    let err = smtp.login().unwrap_err();
    assert_eq!(err.code, ErrorCode::Connection);
    assert_eq!(smtp.last_response_code(), 554);
}

fn send_error(script: Script) -> (ErrorCode, Vec<String>) {
    let server = Server::start(script);
    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    let code = smtp.send(RAW).unwrap_err().code;
    (code, server.lines())
}

#[test]
fn send_error_mapping() {
    let cases: [(Script, ErrorCode); 9] = [
        (
            Script {
                data_end_reply: "550 5.3.4 Requested action not taken; To continue sending messages, please sign in to your account.",
                ..Script::default()
            },
            ErrorCode::NeedsConnectToWebmail,
        ),
        (
            Script {
                data_end_reply: "550 5.7.1 Client does not have permissions to send as this sender",
                ..Script::default()
            },
            ErrorCode::SendMessageNotAllowed,
        ),
        (
            Script {
                rcpt_reply: "550 5.1.1 No such user",
                ..Script::default()
            },
            ErrorCode::SendMessageNotAllowed,
        ),
        (
            Script {
                mail_reply: "550 5.7.60 SMTP; Client does not have permissions",
                ..Script::default()
            },
            ErrorCode::SendMessageNotAllowed,
        ),
        (
            Script {
                data_end_reply: "552 5.7.0 This message contains an illegal attachment",
                ..Script::default()
            },
            ErrorCode::SendMessageIllegalAttachment,
        ),
        (
            Script {
                data_end_reply: "552 5.3.4 Message size exceeds fixed limit",
                ..Script::default()
            },
            ErrorCode::StorageLimit,
        ),
        (
            Script {
                data_end_reply: "554 5.7.9 Message not accepted for policy reasons: spam",
                ..Script::default()
            },
            ErrorCode::YahooSendMessageSpamSuspected,
        ),
        (
            Script {
                data_end_reply: "521 5.7.1 You are over the limit for the day",
                ..Script::default()
            },
            ErrorCode::YahooSendMessageDailyLimitExceeded,
        ),
        (
            Script {
                data_end_reply: "451 4.3.0 Temporary failure",
                ..Script::default()
            },
            ErrorCode::SendMessage,
        ),
    ];
    for (script, expected) in cases {
        let what = format!(
            "{} / {} / {}",
            script.mail_reply, script.rcpt_reply, script.data_end_reply
        );
        let (code, _) = send_error(script);
        assert_eq!(code, expected, "{what}");
    }
}

#[test]
fn advertised_size_limit_is_checked_before_mail() {
    let (code, lines) = send_error(Script {
        size: Some(10),
        ..Script::default()
    });
    assert_eq!(code, ErrorCode::StorageLimit);
    assert!(!lines.iter().any(|l| l.starts_with("MAIL")));
}

#[test]
fn check_account_error_mapping() {
    let server = Server::start(Script {
        rcpt_reply: "554 5.7.1 Relay access denied",
        ..Script::default()
    });
    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    assert_eq!(
        smtp.check_account("me@example.test").unwrap_err().code,
        ErrorCode::InvalidRelaySMTP
    );

    let server = Server::start(Script {
        mail_reply: "553 5.7.1 Sender address rejected",
        ..Script::default()
    });
    let mut smtp = client(&server, Security::Clear, "user", password("pass"));
    assert_eq!(
        smtp.check_account("me@example.test").unwrap_err().code,
        ErrorCode::InvalidAccount
    );
}

/// The timeout is per socket write, as libetpan's was, so an upload slower than the timeout
/// in total but still making progress goes through. 16 MB is more than the socket buffers
/// plus what the slow phase reads, so the client is still writing when it ends.
#[test]
fn slow_upload_outlasting_the_timeout() {
    let server = Server::start(Script {
        slow_data: true,
        ..Script::default()
    });
    let mut smtp = SmtpClient::new(SmtpConfig {
        timeout: Duration::from_secs(1),
        ..client(&server, Security::Clear, "user", password("pass"))
            .config()
            .clone()
    });
    let mut raw = RAW.to_vec();
    let line = [b'x'; 998];
    while raw.len() < 16 << 20 {
        raw.extend_from_slice(&line);
        raw.extend_from_slice(b"\r\n");
    }
    let started = std::time::Instant::now();
    smtp.send(&raw).unwrap();
    assert!(started.elapsed() > Duration::from_millis(1500));
}
