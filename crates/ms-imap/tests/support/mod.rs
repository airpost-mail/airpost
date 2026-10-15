//! Starts a harness IMAP server (`serve.py`) for the integration tests and the spike.
//!
//! The fake server needs only Python 3; Dovecot and Cyrus need Docker. `MS_IMAP_SERVERS`
//! (comma-separated, default `fake,dovecot,cyrus`) narrows the kinds the tests run against.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::Value;

pub struct ServerProc {
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// PEM of the CA that signed the server certificate, when started with TLS.
    pub cert: Option<PathBuf>,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

pub fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/serve.py")
}

/// The server kinds to run against.
pub fn kinds() -> Vec<String> {
    std::env::var("MS_IMAP_SERVERS")
        .unwrap_or_else(|_| "fake,dovecot,cyrus".into())
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

impl ServerProc {
    pub fn start(kind: &str, profile: &str, populate: bool) -> ServerProc {
        Self::start_with(kind, profile, populate, "")
    }

    /// `tls` is `""`, `"tls"` (implicit) or `"starttls"`; fake server only.
    pub fn start_with(kind: &str, profile: &str, populate: bool, tls: &str) -> ServerProc {
        let mut cmd = Command::new("python3");
        cmd.arg(script()).arg(kind);
        if !profile.is_empty() {
            cmd.arg(profile);
        }
        if populate {
            cmd.arg("--populate");
        }
        if !tls.is_empty() {
            cmd.arg(format!("--{tls}"));
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python3 serve.py");
        let stdin = child.stdin.take();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).expect("server address");
        let v: Value = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("{kind} server did not start ({e}): {line:?}"));
        ServerProc {
            kind: kind.to_owned(),
            host: v["host"].as_str().unwrap().to_owned(),
            port: v["port"].as_u64().unwrap() as u16,
            user: v["user"].as_str().unwrap().to_owned(),
            password: v["password"].as_str().unwrap().to_owned(),
            cert: v["cert"].as_str().map(PathBuf::from),
            child,
            stdin,
            stdout,
        }
    }

    /// Calls a method of the harness `Server` object (`append`, `expunge`, `set_flags`, …).
    /// Byte arguments are passed as `"b64:<base64>"` strings.
    pub fn call(&mut self, op: &str, args: Value) -> Value {
        let req = serde_json::json!({ "op": op, "args": args });
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        if let Some(e) = v.get("error") {
            panic!("{op} failed on {}: {e}", self.kind);
        }
        v["ok"].clone()
    }
}

impl Drop for ServerProc {
    fn drop(&mut self) {
        // Closing stdin stops the server (and removes its container).
        self.stdin.take();
        let _ = self.child.wait();
    }
}
