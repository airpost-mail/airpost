// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `tern-backend` binary: user and token commands, migrate, backup, and `serve`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_tern-backend");

fn cmd(data_dir: &Path) -> Command {
    let mut c = Command::new(BIN);
    for k in [
        "TERN_CONFIG",
        "TERN_BIND",
        "TERN_PUBLIC_URL",
        "TERN_SIGNUP",
        "TERN_TRUSTED_PROXIES",
        "TERN_BACKUP_DIR",
        "TERN_BACKUP_INTERVAL",
        "TERN_BACKUP_KEEP",
        "TERN_LOG_FORMAT",
        "RUST_LOG",
    ] {
        c.env_remove(k);
    }
    c.env("TERN_DATA_DIR", data_dir);
    c
}

fn run(data_dir: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut c = cmd(data_dir);
    c.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(input) = stdin {
            pipe.write_all(input.as_bytes()).unwrap();
        }
    }
    child.wait_with_output().unwrap()
}

fn ok(data_dir: &Path, args: &[&str], stdin: Option<&str>) -> String {
    let out = run(data_dir, args, stdin);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn fails(data_dir: &Path, args: &[&str]) -> String {
    let out = run(data_dir, args, None);
    assert!(!out.status.success(), "{args:?} succeeded");
    String::from_utf8(out.stderr).unwrap()
}

#[test]
fn users_and_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();

    let out = ok(d, &["migrate"], None);
    assert!(out.contains("schema version 1"), "{out}");
    assert!(out.contains("server id "), "{out}");

    let out = ok(
        d,
        &[
            "user",
            "add",
            "Ada@Example.com",
            "--first-name",
            "Ada",
            "--password-stdin",
        ],
        Some("correct horse\n"),
    );
    assert!(out.starts_with("created user "), "{out}");
    assert!(!out.contains("password:"));
    let user_id = out.split_whitespace().nth(2).unwrap().to_owned();
    assert_eq!(user_id.len(), 24);

    let out = ok(d, &["user", "add", "bob@example.com"], None);
    let generated = out
        .lines()
        .find_map(|l| l.strip_prefix("password: "))
        .unwrap();
    assert_eq!(generated.len(), 24);

    assert!(fails(d, &["user", "add", "ada@example.com"]).contains("already exists"));
    assert!(fails(d, &["user", "add", "not-an-email"]).contains("valid email"));

    let list = ok(d, &["user", "list"], None);
    assert!(
        list.contains(&format!("{user_id}\tada@example.com\tAda\t")),
        "{list}"
    );
    assert!(list.contains("bob@example.com"));

    let token = ok(
        d,
        &["token", "create", "ada@example.com", "--name", "harness"],
        None,
    );
    let token = token.trim();
    assert!(token.starts_with("tern_") && token.len() == 48, "{token}");
    let tokens = ok(d, &["token", "list", "--user", "ada@example.com"], None);
    assert!(tokens.contains("\tada@example.com\tharness\t"), "{tokens}");
    let token_id = tokens
        .lines()
        .nth(1)
        .unwrap()
        .split('\t')
        .next()
        .unwrap()
        .to_owned();

    ok(d, &["user", "disable", "ada@example.com"], None);
    assert!(fails(d, &["token", "create", "ada@example.com"]).contains("disabled"));
    ok(d, &["user", "enable", "ada@example.com"], None);

    ok(d, &["token", "revoke", &token_id], None);
    assert!(fails(d, &["token", "revoke", &token_id]).contains("no live token"));
    let tokens = ok(d, &["token", "list"], None);
    assert!(
        !tokens.lines().nth(1).unwrap().ends_with("\t-"),
        "revoked time shown: {tokens}"
    );

    let backup = d.join("backup.db");
    ok(d, &["backup", backup.to_str().unwrap()], None);
    assert!(backup.exists());
    assert!(fails(d, &["backup", backup.to_str().unwrap()]).contains("exists"));

    let server_id = |out: &str| out.split("server id ").nth(1).unwrap().trim().to_owned();
    let before = server_id(&ok(d, &["migrate"], None));
    let rotated = server_id(&ok(d, &["rotate-server-id"], None));
    assert_ne!(before, rotated);
    assert_eq!(server_id(&ok(d, &["migrate"], None)), rotated);

    assert!(fails(d, &["user", "delete", "ada@example.com"]).contains("--yes"));
    ok(d, &["user", "delete", "ada@example.com", "--yes"], None);
    let list = ok(d, &["user", "list"], None);
    assert!(!list.contains("ada@example.com"));
    assert!(
        ok(d, &["token", "list"], None).lines().count() == 1,
        "tokens deleted with the user"
    );
    assert!(fails(d, &["user", "delete", "ada@example.com", "--yes"]).contains("no user"));
}

#[test]
fn serve_refuses_plain_http_public_url() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = cmd(dir.path());
    c.env("TERN_PUBLIC_URL", "http://mail.example.com")
        .arg("serve");
    let out = c.output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--insecure-public-url"), "{err}");

    let mut c = cmd(dir.path());
    c.env("TERN_BIND", "0.0.0.0:5101").arg("serve");
    let out = c.output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("TERN_PUBLIC_URL"));
}

fn http_get(addr: &str, path: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).unwrap();
    buf
}

/// Reads stderr lines until one contains `needle`; returns the lines read.
fn read_until(stderr: &mut impl BufRead, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = String::new();
    loop {
        assert!(Instant::now() < deadline, "no {needle:?} in {seen}");
        let mut line = String::new();
        assert!(
            stderr.read_line(&mut line).unwrap() > 0,
            "server exited: {seen}"
        );
        seen.push_str(&line);
        if line.contains(needle) {
            return seen;
        }
    }
}

#[test]
fn serve_answers_and_stops_on_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let backups = dir.path().join("backups");
    let mut c = cmd(&dir.path().join("data"));
    c.env("TERN_BIND", "127.0.0.1:0")
        .env("TERN_PUBLIC_URL", "https://tern.test")
        .env("TERN_BACKUP_DIR", &backups)
        .env("TERN_BACKUP_INTERVAL", "1h")
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        .arg("serve")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());

    let seen = read_until(&mut stderr, "listening");
    assert!(seen.contains("tern-backend user add"), "{seen}");
    let addr = seen
        .split("addr=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();

    let reply = http_get(&addr, "/ping");
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("{}"), "{reply}");

    // No backup existed, so the first one is taken at startup.
    read_until(&mut stderr, "backup written");
    let written: Vec<_> = std::fs::read_dir(&backups).unwrap().collect();
    assert_eq!(written.len(), 1);
    let backup = written.into_iter().next().unwrap().unwrap().path();
    let data = dir.path().join("data");
    let backup_arg = backup.to_str().unwrap();
    let err = fails(&data, &["restore", backup_arg]);
    assert!(err.contains("running"), "{err}");
    let err = fails(&data, &["rotate-server-id"]);
    assert!(err.contains("running"), "{err}");

    let health = |bind: &str| {
        let mut c = cmd(&dir.path().join("data"));
        c.env("TERN_BIND", bind)
            .args(["healthcheck", "--timeout", "2"])
            .output()
            .unwrap()
    };
    let out = health(&addr);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let port = addr.rsplit(':').next().unwrap();
    // A bind on every interface is checked on loopback.
    assert!(health(&format!("0.0.0.0:{port}")).status.success());

    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let exit = child.wait().unwrap();
    assert!(exit.success(), "{exit:?}");
    let mut rest = String::new();
    stderr.read_to_string(&mut rest).unwrap();
    assert!(rest.contains("stopped"), "{rest}");
    let out = health(&addr);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("connecting to"));

    let out = ok(&data, &["restore", backup_arg], None);
    assert!(out.contains("server id"), "{out}");
}

#[test]
fn json_logs() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = cmd(dir.path());
    c.env("TERN_BIND", "127.0.0.1:0")
        .env("TERN_PUBLIC_URL", "https://tern.test")
        .env("TERN_LOG_FORMAT", "json")
        .arg("serve")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let seen = read_until(&mut stderr, "listening");
    for line in seen.lines() {
        let v: serde_json::Value = serde_json::from_str(line).expect(line);
        assert!(v["level"].is_string(), "{line}");
    }
    child.kill().unwrap();
    child.wait().unwrap();
}
