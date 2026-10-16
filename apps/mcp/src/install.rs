// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern-mcp install` / `uninstall`: registers this binary as the `tern` stdio server in
//! Claude Desktop (`claude_desktop_config.json`), Claude Code (`claude mcp add-json`, or
//! the JSON file given with `--config-file`) and Codex (`~/.codex/config.toml`). Only the
//! `tern` entry is ever written or removed; the `mailspring` and `mailspring-mcp` entries
//! Mailspring.app registers are left alone, and so is a `tern` entry that doesn't run
//! tern-mcp unless `--force` is given. A config file that exists but doesn't parse is an
//! error, never rebuilt, so the user's other servers can't be lost.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Map, Value};

pub const ENTRY_NAME: &str = "tern";
/// Entries Mailspring.app's MCP plugin registers.
const MAILSPRING_ENTRIES: &[&str] = &["mailspring", "mailspring-mcp"];
const BINARY_NAME: &str = "tern-mcp";

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum McpClient {
    ClaudeDesktop,
    ClaudeCode,
    Codex,
}

/// The server entry: this binary, pinned to one profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerEntry {
    pub command: PathBuf,
    pub args: Vec<String>,
}

impl ServerEntry {
    pub fn for_profile(binary: PathBuf, config_dir: &Path) -> ServerEntry {
        ServerEntry {
            command: binary,
            args: vec!["--config-dir".into(), config_dir.display().to_string()],
        }
    }

    fn json(&self, with_type: bool) -> Value {
        let mut v = json!({"command": self.command.display().to_string(), "args": self.args});
        if with_type {
            v["type"] = Value::from("stdio");
            v["env"] = json!({});
        }
        v
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `claudeDesktopConfigPath()` of the plugin.
pub fn claude_desktop_config_path() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Claude")
    } else if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support/Claude")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".config")))?
            .join("Claude")
    };
    Some(dir.join("claude_desktop_config.json"))
}

pub fn codex_config_path() -> Option<PathBuf> {
    let dir = std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".codex")))?;
    Some(dir.join("config.toml"))
}

/// Reads a JSON config, following a symlink so the write replaces the real file. Absent or
/// empty is `{}`.
fn read_json(path: &Path) -> Result<(PathBuf, Map<String, Value>), String> {
    if !path.exists() {
        return Ok((path.to_path_buf(), Map::new()));
    }
    let real = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let raw = std::fs::read_to_string(&real).map_err(|e| format!("{}: {e}", real.display()))?;
    if raw.trim().is_empty() {
        return Ok((real, Map::new()));
    }
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(m)) => Ok((real, m)),
        Ok(_) => Err(format!("{} is not a JSON object", real.display())),
        Err(e) => Err(format!("{} is not valid JSON ({e})", real.display())),
    }
}

/// Writes through a temporary file in the same directory and renames it into place. The
/// temporary file is owner-only from the start, since client configs hold other servers'
/// secrets; it gets the original's mode just before the rename.
fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.tern-mcp-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config"),
        std::process::id()
    ));
    // A copy left by an earlier run with the same pid.
    let _ = std::fs::remove_file(&tmp);
    let result = crate::platform::create_private(&tmp, contents.as_bytes())
        .map_err(|e| format!("{}: {e}", tmp.display()))
        .and_then(|()| {
            if let Ok(meta) = std::fs::metadata(path) {
                let _ = std::fs::set_permissions(&tmp, meta.permissions());
            }
            std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
        });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn is_our_command(command: Option<&str>) -> bool {
    command
        .and_then(|c| Path::new(c).file_stem())
        .and_then(|s| s.to_str())
        .is_some_and(|s| s == BINARY_NAME)
}

fn is_ours(entry: &Value) -> bool {
    is_our_command(entry.get("command").and_then(Value::as_str))
}

fn foreign_entry(place: &str) -> String {
    format!(
        "The '{ENTRY_NAME}' server in {place} doesn't run {BINARY_NAME}; left as it is. Give \
         --force to replace it."
    )
}

fn mailspring_note(servers: &Map<String, Value>) -> String {
    let found: Vec<&str> = MAILSPRING_ENTRIES
        .iter()
        .copied()
        .filter(|n| servers.contains_key(*n))
        .collect();
    if found.is_empty() {
        String::new()
    } else {
        format!(
            " The existing {} entry belongs to Mailspring.app and was left as it is.",
            found.join(", ")
        )
    }
}

pub fn install_json(
    path: &Path,
    entry: &ServerEntry,
    with_type: bool,
    force: bool,
) -> Result<String, String> {
    let (real, mut config) = read_json(path)?;
    let servers = config
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err(format!("{}: mcpServers is not an object", real.display()));
    };
    if servers.get(ENTRY_NAME).is_some_and(|e| !is_ours(e)) && !force {
        return Err(foreign_entry(&real.display().to_string()));
    }
    servers.insert(ENTRY_NAME.into(), entry.json(with_type));
    let note = mailspring_note(servers);
    let text = serde_json::to_string_pretty(&Value::Object(config)).expect("JSON");
    write_atomic(&real, &(text + "\n"))?;
    Ok(format!(
        "Added the '{ENTRY_NAME}' server to {}.{note}",
        real.display()
    ))
}

pub fn uninstall_json(path: &Path) -> Result<String, String> {
    let (real, mut config) = read_json(path)?;
    let Some(Value::Object(servers)) = config.get_mut("mcpServers") else {
        return Ok(format!("No '{ENTRY_NAME}' server in {}.", real.display()));
    };
    match servers.get(ENTRY_NAME) {
        None => return Ok(format!("No '{ENTRY_NAME}' server in {}.", real.display())),
        Some(e) if !is_ours(e) => {
            return Err(format!(
                "The '{ENTRY_NAME}' server in {} doesn't run {BINARY_NAME}; left as it is.",
                real.display()
            ))
        }
        Some(_) => {}
    }
    servers.remove(ENTRY_NAME);
    let text = serde_json::to_string_pretty(&Value::Object(config)).expect("JSON");
    write_atomic(&real, &(text + "\n"))?;
    Ok(format!(
        "Removed the '{ENTRY_NAME}' server from {}.",
        real.display()
    ))
}

fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether a TOML table header line opens `[mcp_servers.tern]` or one of its subtables.
fn is_our_table(line: &str) -> bool {
    let l = line.trim();
    let Some(inner) = l.strip_prefix('[').and_then(|r| r.split(']').next()) else {
        return false;
    };
    let inner: String = inner.chars().filter(|c| !c.is_whitespace()).collect();
    ["mcp_servers.tern", "mcp_servers.\"tern\""]
        .iter()
        .any(|t| inner == *t || inner.starts_with(&format!("{t}.")))
}

/// The text without the `tern` server's tables, and whether it had them.
fn remove_codex_tables(content: &str) -> (String, bool) {
    let mut out = Vec::new();
    let (mut skipping, mut found) = (false, false);
    for line in content.lines() {
        if line.trim_start().starts_with('[') {
            skipping = is_our_table(line);
            found |= skipping;
        }
        if !skipping {
            out.push(line);
        }
    }
    let mut text = out.join("\n").trim_end().to_owned();
    if !text.is_empty() {
        text.push('\n');
    }
    (text, found)
}

fn codex_block(entry: &ServerEntry) -> String {
    let args: Vec<String> = entry.args.iter().map(|a| toml_string(a)).collect();
    format!(
        "[mcp_servers.{ENTRY_NAME}]\ncommand = {}\nargs = [{}]\n",
        toml_string(&entry.command.display().to_string()),
        args.join(", ")
    )
}

fn read_text(path: &Path) -> Result<(PathBuf, String), String> {
    if !path.exists() {
        return Ok((path.to_path_buf(), String::new()));
    }
    let real = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = std::fs::read_to_string(&real).map_err(|e| format!("{}: {e}", real.display()))?;
    Ok((real, text))
}

fn parse_toml(path: &Path, text: &str) -> Result<toml::Table, String> {
    text.parse::<toml::Table>()
        .map_err(|e| format!("{} is not valid TOML ({})", path.display(), e.message()))
}

/// The `tern` server in a parsed config.toml, in whatever form it was written.
fn codex_entry(config: &toml::Table) -> Option<&toml::Value> {
    config.get("mcp_servers")?.as_table()?.get(ENTRY_NAME)
}

fn codex_entry_is_ours(entry: &toml::Value) -> bool {
    is_our_command(entry.get("command").and_then(toml::Value::as_str))
}

/// The text without the `tern` server. Only `[mcp_servers.tern]` tables are removed; an
/// inline `tern = {…}` or dotted keys are left for the user, never rewritten.
fn without_codex_entry(real: &Path, content: &str) -> Result<String, String> {
    let (text, _) = remove_codex_tables(content);
    if codex_entry(&parse_toml(real, &text)?).is_some() {
        return Err(format!(
            "The '{ENTRY_NAME}' server in {} is written as an inline table or dotted keys; \
             remove it by hand and run this again.",
            real.display()
        ));
    }
    Ok(text)
}

pub fn install_codex(path: &Path, entry: &ServerEntry, force: bool) -> Result<String, String> {
    let (real, content) = read_text(path)?;
    let config = parse_toml(&real, &content)?;
    if codex_entry(&config).is_some_and(|e| !codex_entry_is_ours(e)) && !force {
        return Err(foreign_entry(&real.display().to_string()));
    }
    let mut text = without_codex_entry(&real, &content)?;
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&codex_block(entry));
    // E.g. an inline `mcp_servers = {…}` can't take a `[mcp_servers.tern]` table.
    if !parse_toml(&real, &text).is_ok_and(|c| codex_entry(&c).is_some()) {
        return Err(format!(
            "could not add the '{ENTRY_NAME}' server to {}; add it by hand",
            real.display()
        ));
    }
    write_atomic(&real, &text)?;
    let note = if content.contains("[mcp_servers.mailspring]") {
        " The existing mailspring entry belongs to Mailspring.app and was left as it is."
    } else {
        ""
    };
    Ok(format!(
        "Added the '{ENTRY_NAME}' server to {}.{note}",
        real.display()
    ))
}

pub fn uninstall_codex(path: &Path) -> Result<String, String> {
    let (real, content) = read_text(path)?;
    match codex_entry(&parse_toml(&real, &content)?) {
        None => return Ok(format!("No '{ENTRY_NAME}' server in {}.", real.display())),
        Some(e) if !codex_entry_is_ours(e) => {
            return Err(format!(
                "The '{ENTRY_NAME}' server in {} doesn't run {BINARY_NAME}; left as it is.",
                real.display()
            ))
        }
        Some(_) => {}
    }
    let text = without_codex_entry(&real, &content)?;
    write_atomic(&real, &text)?;
    Ok(format!(
        "Removed the '{ENTRY_NAME}' server from {}.",
        real.display()
    ))
}

/// The `claude` CLI: on PATH, else through the login shell, whose profile GUI-less
/// environments don't load (the plugin's `_findClaudeBinary`).
fn find_claude() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join("claude");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let out = Command::new(shell)
        .args(["-lic", "command -v claude"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('/') && Path::new(l).is_file())
        .map(PathBuf::from)
}

fn claude_cli(args: &[&str]) -> Result<(), String> {
    let claude = find_claude()
        .ok_or("the Claude Code CLI (claude) isn't on PATH; install it, or give --config-file")?;
    let out = Command::new(&claude)
        .args(args)
        .output()
        .map_err(|e| format!("{}: {e}", claude.display()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "claude {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Where Claude Code keeps user-scope servers: `.claude.json` in `CLAUDE_CONFIG_DIR`, else
/// in the home directory. Only read, to see what a `tern` entry runs; `claude mcp` writes.
fn claude_code_user_config() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(d) => Some(PathBuf::from(d).join(".claude.json")),
        None => home().map(|h| h.join(".claude.json")),
    }
}

/// The user-scope `tern` entry Claude Code has, if any.
fn claude_code_entry(user_config: Option<&Path>) -> Result<Option<Value>, String> {
    let Some(path) = user_config else {
        return Ok(None);
    };
    let (_, config) = read_json(path)?;
    Ok(config
        .get("mcpServers")
        .and_then(|s| s.get(ENTRY_NAME))
        .cloned())
}

fn install_claude_code(
    user_config: Option<&Path>,
    entry: &ServerEntry,
    force: bool,
    claude: &mut dyn FnMut(&[&str]) -> Result<(), String>,
) -> Result<String, String> {
    match claude_code_entry(user_config)? {
        Some(e) if !is_ours(&e) && !force => {
            return Err(foreign_entry("Claude Code (user scope)"));
        }
        // `claude mcp add` doesn't replace an entry of the same name.
        Some(_) => claude(&["mcp", "remove", ENTRY_NAME, "--scope", "user"])?,
        None => {}
    }
    let json = entry.json(true).to_string();
    claude(&["mcp", "add-json", ENTRY_NAME, &json, "--scope", "user"])?;
    Ok(format!(
        "Added the '{ENTRY_NAME}' server to Claude Code (user scope)."
    ))
}

fn uninstall_claude_code(
    user_config: Option<&Path>,
    claude: &mut dyn FnMut(&[&str]) -> Result<(), String>,
) -> Result<String, String> {
    match claude_code_entry(user_config)? {
        None => Ok(format!(
            "No '{ENTRY_NAME}' server in Claude Code (user scope)."
        )),
        Some(e) if !is_ours(&e) => Err(format!(
            "The '{ENTRY_NAME}' server in Claude Code (user scope) doesn't run {BINARY_NAME}; \
             left as it is."
        )),
        Some(_) => {
            claude(&["mcp", "remove", ENTRY_NAME, "--scope", "user"])?;
            Ok(format!(
                "Removed the '{ENTRY_NAME}' server from Claude Code (user scope)."
            ))
        }
    }
}

/// Registers `entry`. An existing `tern` entry that doesn't run tern-mcp is refused unless
/// `force`.
pub fn install(
    client: McpClient,
    config_file: Option<&Path>,
    entry: &ServerEntry,
    force: bool,
) -> Result<String, String> {
    match (client, config_file) {
        (McpClient::ClaudeDesktop, f) => {
            let path = f
                .map(Path::to_path_buf)
                .or_else(claude_desktop_config_path)
                .ok_or("could not work out Claude Desktop's config path; give --config-file")?;
            install_json(&path, entry, false, force)
        }
        (McpClient::ClaudeCode, Some(f)) => install_json(f, entry, true, force),
        (McpClient::ClaudeCode, None) => install_claude_code(
            claude_code_user_config().as_deref(),
            entry,
            force,
            &mut claude_cli,
        ),
        (McpClient::Codex, f) => {
            let path = f
                .map(Path::to_path_buf)
                .or_else(codex_config_path)
                .ok_or("could not work out Codex's config path; give --config-file")?;
            install_codex(&path, entry, force)
        }
    }
}

pub fn uninstall(client: McpClient, config_file: Option<&Path>) -> Result<String, String> {
    match (client, config_file) {
        (McpClient::ClaudeDesktop, f) => {
            let path = f
                .map(Path::to_path_buf)
                .or_else(claude_desktop_config_path)
                .ok_or("could not work out Claude Desktop's config path; give --config-file")?;
            uninstall_json(&path)
        }
        (McpClient::ClaudeCode, Some(f)) => uninstall_json(f),
        (McpClient::ClaudeCode, None) => {
            uninstall_claude_code(claude_code_user_config().as_deref(), &mut claude_cli)
        }
        (McpClient::Codex, f) => {
            let path = f
                .map(Path::to_path_buf)
                .or_else(codex_config_path)
                .ok_or("could not work out Codex's config path; give --config-file")?;
            uninstall_codex(&path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> ServerEntry {
        ServerEntry::for_profile(
            PathBuf::from("/opt/tern/bin/tern-mcp"),
            Path::new("/Users/me/Tern \"x\""),
        )
    }

    #[test]
    fn claude_desktop_keeps_other_servers_and_mailspring() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("claude_desktop_config.json");
        let mailspring = json!({"command": "/Applications/Mailspring.app/Contents/MacOS/Mailspring",
            "args": ["/x/mcp-stdio-bridge.js", "--config-dir", "/y"], "env": {"ELECTRON_RUN_AS_NODE": "1"}});
        std::fs::write(
            &path,
            json!({"mcpServers": {"mailspring": mailspring, "other": {"command": "x"}},
                                      "globalShortcut": "Cmd+K"})
            .to_string(),
        )
        .unwrap();
        let msg = install(McpClient::ClaudeDesktop, Some(&path), &entry(), false).unwrap();
        assert!(msg.contains("belongs to Mailspring.app"), "{msg}");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            v["mcpServers"]["tern"],
            json!({"command": "/opt/tern/bin/tern-mcp",
            "args": ["--config-dir", "/Users/me/Tern \"x\""]})
        );
        assert_eq!(v["mcpServers"]["mailspring"], mailspring);
        assert_eq!(v["globalShortcut"], "Cmd+K");
        // Reinstalling replaces the entry rather than adding another.
        install(McpClient::ClaudeDesktop, Some(&path), &entry(), false).unwrap();
        uninstall(McpClient::ClaudeDesktop, Some(&path)).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v["mcpServers"].get("tern").is_none());
        assert_eq!(v["mcpServers"]["mailspring"], mailspring);
        assert!(uninstall(McpClient::ClaudeDesktop, Some(&path))
            .unwrap()
            .starts_with("No 'tern'"));
    }

    #[test]
    fn refuses_bad_json_and_foreign_tern_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("c.json");
        std::fs::write(&path, "{broken").unwrap();
        assert!(
            install(McpClient::ClaudeDesktop, Some(&path), &entry(), false)
                .unwrap_err()
                .contains("not valid JSON")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        std::fs::write(
            &path,
            json!({"mcpServers": {"tern": {"command": "/usr/bin/other"}}}).to_string(),
        )
        .unwrap();
        assert!(uninstall(McpClient::ClaudeDesktop, Some(&path)).is_err());
        let e = install(McpClient::ClaudeDesktop, Some(&path), &entry(), false).unwrap_err();
        assert!(e.contains("--force"), "{e}");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["tern"]["command"], "/usr/bin/other");
        install(McpClient::ClaudeDesktop, Some(&path), &entry(), true).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["tern"]["command"], "/opt/tern/bin/tern-mcp");
        // A missing file is created.
        let fresh = tmp.path().join("sub/new.json");
        install(McpClient::ClaudeCode, Some(&fresh), &entry(), false).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&fresh).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["tern"]["type"], "stdio");
    }

    #[test]
    fn claude_code_cli_leaves_a_foreign_entry_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join(".claude.json");
        let mut calls: Vec<String> = Vec::new();
        let mut claude = |args: &[&str]| {
            calls.push(args[..2].join(" "));
            Ok(())
        };
        // Nothing there: add only.
        install_claude_code(Some(&user), &entry(), false, &mut claude).unwrap();
        assert!(uninstall_claude_code(Some(&user), &mut claude)
            .unwrap()
            .starts_with("No 'tern'"));
        std::fs::write(
            &user,
            json!({"mcpServers": {"tern": {"type": "http", "url": "http://x/mcp"}}}).to_string(),
        )
        .unwrap();
        let e = install_claude_code(Some(&user), &entry(), false, &mut claude).unwrap_err();
        assert!(e.contains("--force"), "{e}");
        assert!(uninstall_claude_code(Some(&user), &mut claude).is_err());
        install_claude_code(Some(&user), &entry(), true, &mut claude).unwrap();
        std::fs::write(
            &user,
            json!({"mcpServers": {"tern": {"command": "/old/tern-mcp"}}}).to_string(),
        )
        .unwrap();
        install_claude_code(Some(&user), &entry(), false, &mut claude).unwrap();
        uninstall_claude_code(Some(&user), &mut claude).unwrap();
        assert_eq!(
            calls,
            [
                "mcp add-json",
                "mcp remove",
                "mcp add-json",
                "mcp remove",
                "mcp add-json",
                "mcp remove"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn written_configs_keep_their_mode_and_leave_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("c.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        // A temp file left by a crash is replaced, not reused with its mode.
        let stale = tmp
            .path()
            .join(format!(".c.json.tern-mcp-{}", std::process::id()));
        std::fs::write(&stale, "old").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();
        install(McpClient::ClaudeDesktop, Some(&path), &entry(), false).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn codex_inline_and_foreign_entries_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        for (before, needle) in [
            (
                "[mcp_servers]\ntern = { command = \"/opt/tern/bin/tern-mcp\" }\n",
                "inline table",
            ),
            (
                "mcp_servers.tern.command = \"/opt/tern/bin/tern-mcp\"\n",
                "inline table",
            ),
            (
                "[mcp_servers.tern]\ncommand = \"/usr/local/bin/wrapper\"\n",
                "--force",
            ),
            (
                "mcp_servers = { other = { command = \"x\" } }\n",
                "add it by hand",
            ),
            ("model = \n", "not valid TOML"),
        ] {
            std::fs::write(&path, before).unwrap();
            let e = install(McpClient::Codex, Some(&path), &entry(), false).unwrap_err();
            assert!(e.contains(needle), "{before}: {e}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        }
        std::fs::write(
            &path,
            "[mcp_servers.tern]\ncommand = \"/usr/local/bin/wrapper\"\n",
        )
        .unwrap();
        assert!(uninstall(McpClient::Codex, Some(&path)).is_err());
        install(McpClient::Codex, Some(&path), &entry(), true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("wrapper") && text.contains("tern-mcp"),
            "{text}"
        );
        // Another server inline in [mcp_servers] can sit next to our table.
        std::fs::write(&path, "[mcp_servers]\nother = { command = \"x\" }\n").unwrap();
        install(McpClient::Codex, Some(&path), &entry(), false).unwrap();
        let config = parse_toml(&path, &std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(codex_entry(&config).is_some_and(codex_entry_is_ours));
        assert!(config["mcp_servers"].get("other").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.json");
        let link = tmp.path().join("link.json");
        std::fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        install(McpClient::ClaudeDesktop, Some(&link), &entry(), false).unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::read_to_string(&real).unwrap().contains("tern-mcp"));
    }

    #[test]
    fn codex_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let before = "model = \"o4\"\n\n[mcp_servers.mailspring]\nurl = \"http://127.0.0.1:2587/mcp\"\n\n[mcp_servers.tern]\ncommand = \"/old/tern-mcp\"\n\n[mcp_servers.tern.env]\nA = \"1\"\n\n[profiles.x]\nmodel = \"y\"\n";
        std::fs::write(&path, before).unwrap();
        install(McpClient::Codex, Some(&path), &entry(), false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[mcp_servers.mailspring]\nurl = \"http://127.0.0.1:2587/mcp\""));
        assert!(text.contains("[profiles.x]\nmodel = \"y\""));
        assert!(!text.contains("/old") && !text.contains("[mcp_servers.tern.env]"));
        assert!(text.ends_with(
            "[mcp_servers.tern]\ncommand = \"/opt/tern/bin/tern-mcp\"\nargs = [\"--config-dir\", \"/Users/me/Tern \\\"x\\\"\"]\n"
        ), "{text}");
        assert_eq!(text.matches("[mcp_servers.tern]").count(), 1);
        uninstall(McpClient::Codex, Some(&path)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("mcp_servers.tern"));
        assert!(text.contains("[mcp_servers.mailspring]") && text.starts_with("model = \"o4\""));
    }
}
