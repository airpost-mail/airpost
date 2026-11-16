// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `airpost-mcp install` / `uninstall`: registers this binary as the `airpost` stdio server in
//! Claude Desktop (`claude_desktop_config.json`), Claude Code (`claude mcp add-json`, or
//! the JSON file given with `--config-file`) and Codex (`~/.codex/config.toml`). Only the
//! `airpost` entry is ever written or removed; other entries are left alone, and so is a `airpost`
//! entry that doesn't run airpost-mcp unless `--force` is given. A config file that exists but doesn't parse is an
//! error, never rebuilt, so the user's other servers can't be lost.
//!
//! The desktop's removed `mcp-server` plugin registered `airpost` as Airpost's Electron binary
//! running `mcp-stdio-bridge.js` from `app.asar.unpacked`. That file no longer ships, so such
//! an entry is treated as airpost-mcp's own: replaced without `--force` and removable.
//! `--if-stale` replaces only that entry, and only when it is pinned to the same profile,
//! which is what the desktop runs once after the upgrade.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Map, Value};

pub const ENTRY_NAME: &str = "airpost";
const BINARY_NAME: &str = "airpost-mcp";
/// The stdio bridge the desktop's old `mcp-server` plugin registered.
const BRIDGE_SCRIPT: &str = "mcp-stdio-bridge.js";

/// What `install` may do to an existing `airpost` entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replace {
    /// Replace it when it runs airpost-mcp or is the old plugin's bridge.
    Ours,
    /// Replace whatever it runs (`--force`).
    Any,
    /// Replace only the old plugin's bridge pinned to this profile, else change nothing
    /// (`--if-stale`).
    OnlyStale,
}

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
        ".{}.airpost-mcp-{}",
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

fn string_args(entry: &Value) -> Vec<&str> {
    entry
        .get("args")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The old plugin's entry: an argument naming `mcp-stdio-bridge.js`, with either separator
/// since the config may come from Windows.
fn is_stale_bridge(entry: &Value) -> bool {
    string_args(entry)
        .iter()
        .any(|a| a.rsplit(['/', '\\']).next() == Some(BRIDGE_SCRIPT))
}

fn config_dir_arg<'a>(args: &[&'a str]) -> Option<&'a str> {
    args.iter()
        .position(|a| *a == "--config-dir")
        .and_then(|i| args.get(i + 1).copied())
}

/// A stale bridge entry for the profile `entry` is pinned to. The Claude Desktop config is
/// shared by every profile, so a dev or second profile never takes over another's entry.
fn is_stale_bridge_for(existing: &Value, entry: &ServerEntry) -> bool {
    let ours: Vec<&str> = entry.args.iter().map(String::as_str).collect();
    is_stale_bridge(existing)
        && match (
            config_dir_arg(&string_args(existing)),
            config_dir_arg(&ours),
        ) {
            (Some(a), Some(b)) => Path::new(a) == Path::new(b),
            _ => false,
        }
}

fn replaceable(existing: &Value, entry: &ServerEntry, replace: Replace) -> bool {
    match replace {
        Replace::Any => true,
        Replace::Ours => is_ours(existing) || is_stale_bridge(existing),
        Replace::OnlyStale => is_stale_bridge_for(existing, entry),
    }
}

fn no_stale_entry(place: &str) -> String {
    format!(
        "No stale '{ENTRY_NAME}' entry from the old desktop plugin in {place}; nothing changed."
    )
}

fn foreign_entry(place: &str) -> String {
    format!(
        "The '{ENTRY_NAME}' server in {place} doesn't run {BINARY_NAME}; left as it is. Give \
         --force to replace it."
    )
}

pub fn install_json(
    path: &Path,
    entry: &ServerEntry,
    with_type: bool,
    replace: Replace,
) -> Result<String, String> {
    let (real, mut config) = read_json(path)?;
    let place = real.display().to_string();
    if replace == Replace::OnlyStale
        && !config
            .get("mcpServers")
            .and_then(|s| s.get(ENTRY_NAME))
            .is_some_and(|e| is_stale_bridge_for(e, entry))
    {
        return Ok(no_stale_entry(&place));
    }
    let servers = config
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err(format!("{place}: mcpServers is not an object"));
    };
    if servers
        .get(ENTRY_NAME)
        .is_some_and(|e| !replaceable(e, entry, replace))
    {
        return Err(foreign_entry(&place));
    }
    servers.insert(ENTRY_NAME.into(), entry.json(with_type));
    let text = serde_json::to_string_pretty(&Value::Object(config)).expect("JSON");
    write_atomic(&real, &(text + "\n"))?;
    Ok(format!(
        "Added the '{ENTRY_NAME}' server to {}.",
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
        Some(e) if !is_ours(e) && !is_stale_bridge(e) => {
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

/// Whether a TOML table header line opens `[mcp_servers.airpost]` or one of its subtables.
fn is_our_table(line: &str) -> bool {
    let l = line.trim();
    let Some(inner) = l.strip_prefix('[').and_then(|r| r.split(']').next()) else {
        return false;
    };
    let inner: String = inner.chars().filter(|c| !c.is_whitespace()).collect();
    ["mcp_servers.airpost", "mcp_servers.\"airpost\""]
        .iter()
        .any(|t| inner == *t || inner.starts_with(&format!("{t}.")))
}

/// The text without the `airpost` server's tables, and whether it had them.
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

/// The `airpost` server in a parsed config.toml, in whatever form it was written.
fn codex_entry(config: &toml::Table) -> Option<&toml::Value> {
    config.get("mcp_servers")?.as_table()?.get(ENTRY_NAME)
}

fn codex_entry_is_ours(entry: &toml::Value) -> bool {
    is_our_command(entry.get("command").and_then(toml::Value::as_str))
}

/// The text without the `airpost` server. Only `[mcp_servers.airpost]` tables are removed; an
/// inline `airpost = {…}` or dotted keys are left for the user, never rewritten.
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

/// The old plugin never wrote a bridge entry to Codex, so `Replace::OnlyStale` changes
/// nothing here.
pub fn install_codex(path: &Path, entry: &ServerEntry, replace: Replace) -> Result<String, String> {
    if replace == Replace::OnlyStale {
        return Ok(no_stale_entry(&path.display().to_string()));
    }
    let (real, content) = read_text(path)?;
    let config = parse_toml(&real, &content)?;
    if codex_entry(&config).is_some_and(|e| !codex_entry_is_ours(e)) && replace != Replace::Any {
        return Err(foreign_entry(&real.display().to_string()));
    }
    let mut text = without_codex_entry(&real, &content)?;
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&codex_block(entry));
    // E.g. an inline `mcp_servers = {…}` can't take a `[mcp_servers.airpost]` table.
    if !parse_toml(&real, &text).is_ok_and(|c| codex_entry(&c).is_some()) {
        return Err(format!(
            "could not add the '{ENTRY_NAME}' server to {}; add it by hand",
            real.display()
        ));
    }
    write_atomic(&real, &text)?;
    Ok(format!(
        "Added the '{ENTRY_NAME}' server to {}.",
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
/// in the home directory. Only read, to see what a `airpost` entry runs; `claude mcp` writes.
fn claude_code_user_config() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(d) => Some(PathBuf::from(d).join(".claude.json")),
        None => home().map(|h| h.join(".claude.json")),
    }
}

/// The user-scope `airpost` entry Claude Code has, if any.
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
    replace: Replace,
    claude: &mut dyn FnMut(&[&str]) -> Result<(), String>,
) -> Result<String, String> {
    const PLACE: &str = "Claude Code (user scope)";
    match claude_code_entry(user_config)? {
        Some(e) if replace == Replace::OnlyStale && !is_stale_bridge_for(&e, entry) => {
            return Ok(no_stale_entry(PLACE));
        }
        None if replace == Replace::OnlyStale => return Ok(no_stale_entry(PLACE)),
        Some(e) if !replaceable(&e, entry, replace) => {
            return Err(foreign_entry(PLACE));
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
        Some(e) if !is_ours(&e) && !is_stale_bridge(&e) => Err(format!(
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

/// Registers `entry`, replacing an existing `airpost` entry as `replace` allows.
pub fn install(
    client: McpClient,
    config_file: Option<&Path>,
    entry: &ServerEntry,
    replace: Replace,
) -> Result<String, String> {
    match (client, config_file) {
        (McpClient::ClaudeDesktop, f) => {
            let path = f
                .map(Path::to_path_buf)
                .or_else(claude_desktop_config_path)
                .ok_or("could not work out Claude Desktop's config path; give --config-file")?;
            install_json(&path, entry, false, replace)
        }
        (McpClient::ClaudeCode, Some(f)) => install_json(f, entry, true, replace),
        (McpClient::ClaudeCode, None) => install_claude_code(
            claude_code_user_config().as_deref(),
            entry,
            replace,
            &mut claude_cli,
        ),
        (McpClient::Codex, f) => {
            let path = f
                .map(Path::to_path_buf)
                .or_else(codex_config_path)
                .ok_or("could not work out Codex's config path; give --config-file")?;
            install_codex(&path, entry, replace)
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
            PathBuf::from("/opt/airpost/bin/airpost-mcp"),
            Path::new("/Users/me/Airpost \"x\""),
        )
    }

    #[test]
    fn claude_desktop_keeps_other_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("claude_desktop_config.json");
        let mail = json!({"command": "/Applications/Mail.app/Contents/MacOS/Mail",
            "args": ["/x/mcp-stdio-bridge.js", "--config-dir", "/y"], "env": {"ELECTRON_RUN_AS_NODE": "1"}});
        std::fs::write(
            &path,
            json!({"mcpServers": {"mail": mail, "other": {"command": "x"}},
                                      "globalShortcut": "Cmd+K"})
            .to_string(),
        )
        .unwrap();
        install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::Ours,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            v["mcpServers"]["airpost"],
            json!({"command": "/opt/airpost/bin/airpost-mcp",
            "args": ["--config-dir", "/Users/me/Airpost \"x\""]})
        );
        assert_eq!(v["mcpServers"]["mail"], mail);
        assert_eq!(v["globalShortcut"], "Cmd+K");
        // Reinstalling replaces the entry rather than adding another.
        install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::Ours,
        )
        .unwrap();
        uninstall(McpClient::ClaudeDesktop, Some(&path)).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v["mcpServers"].get("airpost").is_none());
        assert_eq!(v["mcpServers"]["mail"], mail);
        assert!(uninstall(McpClient::ClaudeDesktop, Some(&path))
            .unwrap()
            .starts_with("No 'airpost'"));
    }

    #[test]
    fn refuses_bad_json_and_foreign_airpost_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("c.json");
        std::fs::write(&path, "{broken").unwrap();
        assert!(install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::Ours
        )
        .unwrap_err()
        .contains("not valid JSON"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        std::fs::write(
            &path,
            json!({"mcpServers": {"airpost": {"command": "/usr/bin/other"}}}).to_string(),
        )
        .unwrap();
        assert!(uninstall(McpClient::ClaudeDesktop, Some(&path)).is_err());
        let e = install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::Ours,
        )
        .unwrap_err();
        assert!(e.contains("--force"), "{e}");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["airpost"]["command"], "/usr/bin/other");
        install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::Any,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            v["mcpServers"]["airpost"]["command"],
            "/opt/airpost/bin/airpost-mcp"
        );
        // A missing file is created.
        let fresh = tmp.path().join("sub/new.json");
        install(McpClient::ClaudeCode, Some(&fresh), &entry(), Replace::Ours).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&fresh).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["airpost"]["type"], "stdio");
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
        install_claude_code(Some(&user), &entry(), Replace::Ours, &mut claude).unwrap();
        assert!(uninstall_claude_code(Some(&user), &mut claude)
            .unwrap()
            .starts_with("No 'airpost'"));
        std::fs::write(
            &user,
            json!({"mcpServers": {"airpost": {"type": "http", "url": "http://x/mcp"}}}).to_string(),
        )
        .unwrap();
        let e = install_claude_code(Some(&user), &entry(), Replace::Ours, &mut claude).unwrap_err();
        assert!(e.contains("--force"), "{e}");
        assert!(uninstall_claude_code(Some(&user), &mut claude).is_err());
        install_claude_code(Some(&user), &entry(), Replace::Any, &mut claude).unwrap();
        std::fs::write(
            &user,
            json!({"mcpServers": {"airpost": {"command": "/old/airpost-mcp"}}}).to_string(),
        )
        .unwrap();
        install_claude_code(Some(&user), &entry(), Replace::Ours, &mut claude).unwrap();
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

    /// The entry the desktop's old mcp-server plugin wrote (claude-desktop-config.ts).
    fn bridge_entry(profile: &str) -> Value {
        json!({"command": "/Applications/Airpost.app/Contents/MacOS/Airpost",
            "args": ["/Applications/Airpost.app/Contents/Resources/app.asar.unpacked/internal_packages/mcp-server/lib/mcp-stdio-bridge.js",
                     "--config-dir", profile],
            "env": {"ELECTRON_RUN_AS_NODE": "1"}})
    }

    fn servers(path: &Path) -> Value {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        v["mcpServers"].clone()
    }

    #[test]
    fn replaces_the_old_plugin_bridge_without_force_and_still_refuses_foreign_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("claude_desktop_config.json");
        let profile = "/Users/me/Airpost \"x\"";
        let write = |airpost: Value| {
            std::fs::write(
                &path,
                json!({"mcpServers": {"airpost": airpost, "other": {"command": "x"}}}).to_string(),
            )
            .unwrap();
        };

        write(bridge_entry(profile));
        install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::Ours,
        )
        .unwrap();
        assert_eq!(
            servers(&path)["airpost"]["command"],
            "/opt/airpost/bin/airpost-mcp"
        );
        assert_eq!(servers(&path)["other"], json!({"command": "x"}));

        // A Windows path, and uninstall removes a bridge entry too.
        write(json!({"command": "C:\\Airpost\\Airpost.exe",
            "args": ["C:\\Airpost\\resources\\app.asar.unpacked\\mcp-stdio-bridge.js", "--config-dir", profile]}));
        assert!(uninstall(McpClient::ClaudeDesktop, Some(&path))
            .unwrap()
            .starts_with("Removed"));
        assert!(servers(&path).get("airpost").is_none());

        // Anything else is still refused without --force.
        let foreign = json!({"command": "/usr/local/bin/node", "args": ["/srv/my-bridge.js"]});
        write(foreign.clone());
        let e = install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::Ours,
        )
        .unwrap_err();
        assert!(e.contains("--force"), "{e}");
        assert_eq!(servers(&path)["airpost"], foreign);
    }

    #[test]
    fn if_stale_only_replaces_the_bridge_for_this_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("claude_desktop_config.json");
        let profile = "/Users/me/Airpost \"x\"";

        // No file, no entry: nothing written.
        let msg = install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::OnlyStale,
        )
        .unwrap();
        assert!(msg.starts_with("No stale"), "{msg}");
        assert!(!path.exists());

        // Another profile's bridge, an up-to-date entry and a foreign one stay as they are.
        for airpost in [
            bridge_entry("/Users/me/Library/Application Support/Airpost-dev"),
            entry().json(false),
            json!({"command": "/usr/bin/other"}),
        ] {
            let before = json!({"mcpServers": {"airpost": airpost}}).to_string();
            std::fs::write(&path, &before).unwrap();
            let msg = install(
                McpClient::ClaudeDesktop,
                Some(&path),
                &entry(),
                Replace::OnlyStale,
            )
            .unwrap();
            assert!(msg.starts_with("No stale"), "{msg}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        }

        // This profile's bridge is replaced, trailing slash or not.
        std::fs::write(
            &path,
            json!({"mcpServers": {"airpost": bridge_entry(&format!("{profile}/"))}}).to_string(),
        )
        .unwrap();
        install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::OnlyStale,
        )
        .unwrap();
        assert_eq!(servers(&path)["airpost"], entry().json(false));

        let codex = tmp.path().join("config.toml");
        assert!(
            install(McpClient::Codex, Some(&codex), &entry(), Replace::OnlyStale)
                .unwrap()
                .starts_with("No stale")
        );
        assert!(!codex.exists());

        let user = tmp.path().join(".claude.json");
        let mut calls = 0;
        let mut claude = |_: &[&str]| {
            calls += 1;
            Ok(())
        };
        install_claude_code(Some(&user), &entry(), Replace::OnlyStale, &mut claude).unwrap();
        std::fs::write(
            &user,
            json!({"mcpServers": {"airpost": bridge_entry(profile)}}).to_string(),
        )
        .unwrap();
        install_claude_code(Some(&user), &entry(), Replace::OnlyStale, &mut claude).unwrap();
        assert_eq!(calls, 2, "remove, then add-json");
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
            .join(format!(".c.json.airpost-mcp-{}", std::process::id()));
        std::fs::write(&stale, "old").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();
        install(
            McpClient::ClaudeDesktop,
            Some(&path),
            &entry(),
            Replace::Ours,
        )
        .unwrap();
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
                "[mcp_servers]\ntern = { command = \"/opt/airpost/bin/airpost-mcp\" }\n",
                "inline table",
            ),
            (
                "mcp_servers.airpost.command = \"/opt/airpost/bin/airpost-mcp\"\n",
                "inline table",
            ),
            (
                "[mcp_servers.airpost]\ncommand = \"/usr/local/bin/wrapper\"\n",
                "--force",
            ),
            (
                "mcp_servers = { other = { command = \"x\" } }\n",
                "add it by hand",
            ),
            ("model = \n", "not valid TOML"),
        ] {
            std::fs::write(&path, before).unwrap();
            let e = install(McpClient::Codex, Some(&path), &entry(), Replace::Ours).unwrap_err();
            assert!(e.contains(needle), "{before}: {e}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        }
        std::fs::write(
            &path,
            "[mcp_servers.airpost]\ncommand = \"/usr/local/bin/wrapper\"\n",
        )
        .unwrap();
        assert!(uninstall(McpClient::Codex, Some(&path)).is_err());
        install(McpClient::Codex, Some(&path), &entry(), Replace::Any).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("wrapper") && text.contains("airpost-mcp"),
            "{text}"
        );
        // Another server inline in [mcp_servers] can sit next to our table.
        std::fs::write(&path, "[mcp_servers]\nother = { command = \"x\" }\n").unwrap();
        install(McpClient::Codex, Some(&path), &entry(), Replace::Ours).unwrap();
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
        install(
            McpClient::ClaudeDesktop,
            Some(&link),
            &entry(),
            Replace::Ours,
        )
        .unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::read_to_string(&real)
            .unwrap()
            .contains("airpost-mcp"));
    }

    #[test]
    fn codex_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let before = "model = \"o4\"\n\n[mcp_servers.mail]\nurl = \"http://127.0.0.1:2587/mcp\"\n\n[mcp_servers.airpost]\ncommand = \"/old/airpost-mcp\"\n\n[mcp_servers.airpost.env]\nA = \"1\"\n\n[profiles.x]\nmodel = \"y\"\n";
        std::fs::write(&path, before).unwrap();
        install(McpClient::Codex, Some(&path), &entry(), Replace::Ours).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[mcp_servers.mail]\nurl = \"http://127.0.0.1:2587/mcp\""));
        assert!(text.contains("[profiles.x]\nmodel = \"y\""));
        assert!(!text.contains("/old") && !text.contains("[mcp_servers.airpost.env]"));
        assert!(text.ends_with(
            "[mcp_servers.airpost]\ncommand = \"/opt/airpost/bin/airpost-mcp\"\nargs = [\"--config-dir\", \"/Users/me/Airpost \\\"x\\\"\"]\n"
        ), "{text}");
        assert_eq!(text.matches("[mcp_servers.airpost]").count(), 1);
        uninstall(McpClient::Codex, Some(&path)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("mcp_servers.airpost"));
        assert!(text.contains("[mcp_servers.mail]") && text.starts_with("model = \"o4\""));
    }
}
