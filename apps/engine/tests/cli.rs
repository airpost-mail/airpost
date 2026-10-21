// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Black-box checks of the process contract on the
//! debug binary, which skips the argv[0] check.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ACCOUNT: &str = r#"{"id":"c0ffee-harness","emailAddress":"me@example.com","provider":"imap","settings":{"imap_host":"127.0.0.1","imap_port":1,"imap_username":"me","imap_password":"x","imap_security":"none","smtp_host":"127.0.0.1","smtp_port":1,"smtp_username":"me","smtp_password":"x","smtp_security":"none","imap_allow_insecure_ssl":false,"smtp_allow_insecure_ssl":false}}"#;

fn tern_engine(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tern-engine"));
    c.env("CONFIG_DIR_PATH", config)
        .env("IDENTITY_SERVER", "http://127.0.0.1:9/unreachable")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

fn wait_for_log(path: &Path, pattern: &regex::Regex, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if pattern.is_match(&text) {
            return text;
        }
        assert!(start.elapsed() < timeout, "{pattern} not in log:\n{text}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn migrate_creates_v12() {
    let dir = tempfile::tempdir().unwrap();
    let out = tern_engine(dir.path())
        .args([
            "--mode",
            "migrate",
            "--identity",
            "null",
            "--account",
            ACCOUNT,
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.ends_with("\n{\"error\":null}"), "{stdout:?}");
    assert!(out.stderr.is_empty());
    let conn = rusqlite::Connection::open(dir.path().join("tern.db")).unwrap();
    let v: i32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 12);
    let engine: String = conn
        .query_row("SELECT value FROM _State WHERE id = 'engine'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(engine, "rust");
}

/// One-shot contract: no account is read, success is `{"error":null}` after `info:` lines,
/// a failure is `{"error": what}` with exit 1, and stderr stays empty either way.
#[test]
fn import_legacy_one_shot_output() {
    let dir = tempfile::tempdir().unwrap();
    let run = |dir: &Path| {
        let out = tern_engine(dir)
            .args(["--mode", "import-legacy"])
            .output()
            .unwrap();
        assert!(out.stderr.is_empty(), "{:?}", out.stderr);
        (out.status.code(), String::from_utf8(out.stdout).unwrap())
    };

    // Before migrate, with a legacy database: the new database must exist first.
    std::fs::write(dir.path().join("tern.legacy.db"), "").unwrap();
    let (code, stdout) = run(dir.path());
    assert_eq!(code, Some(1));
    assert!(
        stdout.ends_with(
            "\n{\"error\":\"tern.db does not exist; run --mode migrate before --mode import-legacy\"}"
        ),
        "{stdout:?}"
    );
    std::fs::remove_file(dir.path().join("tern.legacy.db")).unwrap();

    let migrated = tern_engine(dir.path())
        .args(["--mode", "migrate"])
        .output()
        .unwrap();
    assert!(migrated.status.success());
    let (code, stdout) = run(dir.path());
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "info: No legacy database to import.\n\n{\"error\":null}"
    );
}

#[test]
fn usage_and_config_dir() {
    let out = Command::new(env!("CARGO_BIN_EXE_tern-engine"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("USAGE:"));

    let out = Command::new(env!("CARGO_BIN_EXE_tern-engine"))
        .env_remove("CONFIG_DIR_PATH")
        .args(["--mode", "migrate"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn account_handshake_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = tern_engine(dir.path())
        .args(["--mode", "sync", "--info", "me@example.com"])
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut prompt = [0u8; 27];
    stdout.read_exact(&mut prompt).unwrap();
    assert_eq!(&prompt, b"\nWaiting for Account JSON:\n");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not json\n")
        .unwrap();
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(1));
    assert!(
        rest.starts_with("\n{\"error\":\"Invalid Account JSON: "),
        "{rest:?}"
    );

    let out = tern_engine(dir.path())
        .args(["--mode", "sync", "--account", r#"{"id":"x"}"#])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "\n{\"error\":\"Account is missing required fields:id or settings\"}"
    );

    // An unknown security value is refused, never taken as cleartext.
    let blank = ACCOUNT.replace(r#""imap_security":"none""#, r#""imap_security":"""#);
    let out = tern_engine(dir.path())
        .args(["--mode", "sync", "--account", &blank])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        r#"
{"error":"Account has an invalid imap_security: \"\" (expected \"SSL / TLS\", \"STARTTLS\" or \"none\")"}"#
    );

    let out = tern_engine(dir.path())
        .args(["--mode", "sync", "--account", ACCOUNT, "--identity", "{}"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "\n{\"error\":\"ErrorIdentityMissingFields\"}"
    );
}

#[test]
fn sync_starts_logs_and_takes_commands() {
    let dir = tempfile::tempdir().unwrap();
    let status = tern_engine(dir.path())
        .args(["--mode", "migrate"])
        .output()
        .unwrap()
        .status;
    assert!(status.success());

    let mut child = tern_engine(dir.path())
        .args([
            "--mode",
            "sync",
            "--verbose",
            "--identity",
            "null",
            "--account",
            ACCOUNT,
        ])
        .spawn()
        .unwrap();
    let log = dir.path().join("engine-c0ffee-harness.log");
    let line = regex::Regex::new(
        r"(?m)^\d+ \[\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d{3}\] \[main\] \[info\] ------------- Starting Sync \(me@example.com\) ---------------\r?$",
    )
    .unwrap();
    wait_for_log(&log, &line, Duration::from_secs(10));

    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(b"garbage\n\n{\"type\":\"wake-workers\"}\n{\"type\":\"queue-task\",\"task\":{\"id\":\"t1\"}}\n{\"type\":\"need-bodies\",\"ids\":[\"m1\"]}\n")
        .unwrap();
    stdin.flush().unwrap();
    let woke = regex::Regex::new(r"(?m)\[main\] \[info\] Waking all workers\.\.\.\r?$").unwrap();
    wait_for_log(&log, &woke, Duration::from_secs(10));

    assert!(child.try_wait().unwrap().is_none(), "exited early");
    child.kill().unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The identity line picks the metadata backend (docs/backend-plan.md 4.1): a valid Tern
/// server object the HTTP one, `null`, a Mailspring identity or a bad Tern object the local
/// one. None of them stops mail sync.
#[test]
fn identity_line_selects_the_metadata_backend() {
    let tern = |url: &str, version: u32| {
        format!(
            r#"{{"type":"tern-server","version":{version},"url":"{url}","token":"tern_secret_token","userId":"u1","serverId":"s1"}}"#
        )
    };
    let cases = [
        (tern("http://127.0.0.1:9", 1), r"Metadata backend: tern-server 127\.0\.0\.1:9", true),
        (tern("http://tern.example.com", 1), r"Metadata backend: local only \(invalid url\)", false),
        (tern("https://tern.example.com", 2), r"Metadata backend: local only \(invalid version\)", false),
        (
            r#"{"emailAddress":"me@example.com","token":"t","createdAt":"2018-02-16T00:00:00.000Z"}"#.to_owned(),
            r"Metadata backend: local only \(no server\)",
            false,
        ),
        ("null".to_owned(), r"Metadata backend: local only \(no server\)", true),
    ];
    for (i, (identity, expected, via_stdin)) in cases.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        assert!(tern_engine(dir.path())
            .args(["--mode", "migrate"])
            .output()
            .unwrap()
            .status
            .success());
        let mut cmd = tern_engine(dir.path());
        cmd.args(["--mode", "sync", "--account", ACCOUNT]);
        if !via_stdin {
            cmd.args(["--identity", &identity]);
        }
        let mut child = cmd.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        if via_stdin {
            stdin.write_all(format!("{identity}\n").as_bytes()).unwrap();
            stdin.flush().unwrap();
        }
        let log = dir.path().join("engine-c0ffee-harness.log");
        let text = wait_for_log(
            &log,
            &regex::Regex::new(&format!("(?m){expected}")).unwrap(),
            Duration::from_secs(10),
        );
        assert!(
            !text.contains("tern_secret_token"),
            "case {i} logged the token"
        );
        assert!(child.try_wait().unwrap().is_none(), "case {i} exited");
        child.kill().unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            !String::from_utf8_lossy(&out.stdout).contains("tern_secret_token"),
            "case {i}"
        );
    }
}

/// Takes 30 s: `cargo test -p tern-engine -- --ignored`.
#[test]
#[ignore]
fn exits_141_after_stdin_closes() {
    let dir = tempfile::tempdir().unwrap();
    assert!(tern_engine(dir.path())
        .args(["--mode", "migrate"])
        .output()
        .unwrap()
        .status
        .success());
    let mut child = tern_engine(dir.path())
        .args(["--mode", "sync", "--identity", "null", "--account", ACCOUNT])
        .spawn()
        .unwrap();
    drop(child.stdin.take());
    let start = Instant::now();
    let status = wait_with_timeout(&mut child, Duration::from_secs(45)).expect("did not exit");
    assert_eq!(status.code(), Some(141));
    assert!(start.elapsed() >= Duration::from_secs(30));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.is_empty(), "{stderr}");
}
