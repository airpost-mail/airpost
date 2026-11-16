// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The audit log: `<config-dir>/mcp-audit.log`, one JSON object per line, appended for
//! every tool call and resource read, owner-readable only. When it passes [`ROTATE_BYTES`]
//! it is renamed to `mcp-audit.log.1` (replacing the previous one) and a new file starts.
//!
//! Each entry has the fields the desktop's audit pane shows (`AuditEntry` in the plugin's
//! `mcp-tools.ts`: `timestamp`, `toolName`, `params`, `resultSummary`, `durationMs`), plus
//! `client`, `accountId` and `outcome` (`ok`, `error` or `denied`). `params` is a summary of
//! the arguments: message bodies are replaced by their length and keys that look like
//! credentials are dropped, then it is cut to 200 characters.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

pub const AUDIT_FILE: &str = "mcp-audit.log";
pub const ROTATE_BYTES: u64 = 4 * 1024 * 1024;
const PARAMS_MAX: usize = 200;
const RESULT_MAX: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// Milliseconds since the epoch, when the call finished.
    pub timestamp: i64,
    /// `clientInfo.name` (and version) from the client's `initialize`.
    pub client: String,
    pub tool_name: String,
    pub account_id: Option<String>,
    pub outcome: &'static str,
    pub params: String,
    pub result_summary: String,
    pub duration_ms: u64,
}

impl AuditEntry {
    pub fn new(
        client: &str,
        tool: &str,
        args: &Value,
        account_id: Option<String>,
        outcome: &'static str,
        result_summary: &str,
        duration_ms: u64,
    ) -> AuditEntry {
        AuditEntry {
            timestamp: chrono::Utc::now().timestamp_millis(),
            client: client.to_owned(),
            tool_name: tool.to_owned(),
            account_id,
            outcome,
            params: summarize_params(args),
            result_summary: cut(result_summary, RESULT_MAX),
            duration_ms,
        }
    }
}

fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_owned(),
        None => s.to_owned(),
    }
}

/// Keys whose values never reach the log.
fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "password",
        "token",
        "secret",
        "credential",
        "authorization",
        "apikey",
        "api_key",
    ]
    .iter()
    .any(|s| k.contains(s))
}

fn redact(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = Map::new();
            for (k, v) in o {
                if is_secret_key(k) {
                    continue;
                }
                let v = match (k.as_str(), v) {
                    ("body", Value::String(s)) => {
                        Value::from(format!("<{} chars>", s.chars().count()))
                    }
                    _ => redact(v),
                };
                out.insert(k.clone(), v);
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(redact).collect()),
        other => other.clone(),
    }
}

/// The arguments as the log records them.
pub fn summarize_params(args: &Value) -> String {
    cut(&redact(args).to_string(), PARAMS_MAX)
}

pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(config_dir: &Path) -> AuditLog {
        AuditLog {
            path: config_dir.join(AUDIT_FILE),
        }
    }

    /// Appends one line. A log that can't be written doesn't fail the call: the error goes
    /// to stderr, which MCP clients keep as the server's log.
    pub fn append(&self, entry: &AuditEntry) {
        if let Err(e) = self.try_append(entry) {
            crate::log(&format!(
                "could not write the audit log {}: {e}",
                self.path.display()
            ));
        }
    }

    fn try_append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() >= ROTATE_BYTES) {
            let mut rotated = self.path.clone().into_os_string();
            rotated.push(".1");
            std::fs::rename(&self.path, rotated)?;
        }
        let mut line = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
        line.push(b'\n');
        crate::platform::open_private_append(&self.path)?.write_all(&line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn params_never_carry_bodies_or_credentials() {
        let args = json!({
            "to": [{"email": "bob@x.test"}],
            "subject": "Hi",
            "body": "<p>the secret plan</p>",
            "token": "abc",
            "nested": {"apiKey": "k", "body": "also hidden"}
        });
        let s = summarize_params(&args);
        assert!(!s.contains("secret plan") && !s.contains("abc") && !s.contains("also hidden"));
        assert!(!s.contains("apiKey"));
        assert!(s.contains("<22 chars>") && s.contains("bob@x.test"), "{s}");
        let long = json!({"query": "x".repeat(500)});
        assert_eq!(summarize_params(&long).chars().count(), PARAMS_MAX);
    }

    #[test]
    fn appends_jsonl_and_rotates() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AuditLog::new(tmp.path());
        let e = AuditEntry::new(
            "claude-ai 1.0",
            "set_starred",
            &json!({"threadIds": ["t1"], "starred": true}),
            Some("a1".into()),
            "ok",
            "ok",
            12,
        );
        log.append(&e);
        log.append(&e);
        let text = std::fs::read_to_string(&log.path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        let mut keys: Vec<&str> = lines[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "accountId",
                "client",
                "durationMs",
                "outcome",
                "params",
                "resultSummary",
                "timestamp",
                "toolName"
            ]
        );
        assert_eq!(lines[0]["toolName"], "set_starred");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&log.path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let f = std::fs::OpenOptions::new()
            .append(true)
            .open(&log.path)
            .unwrap();
        f.set_len(ROTATE_BYTES).unwrap();
        log.append(&e);
        assert_eq!(
            std::fs::read_to_string(&log.path).unwrap().lines().count(),
            1
        );
        assert!(tmp.path().join("mcp-audit.log.1").exists());
    }
}
