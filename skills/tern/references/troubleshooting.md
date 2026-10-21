# Troubleshooting Tern

Start with the status commands when a shell is available. They don't change anything:

```
tern status          # engines and each account's state (exit 3: no engine)
tern-mcp status      # the profile, engines and access settings the MCP server sees
```

## Tern isn't running

- MCP: `Tern's mail engine isn't running, so there is no mail to read: open Tern and try
  again (tern-mcp looked for engines in <profile>/run).`
- CLI: `no engine running: start Tern, or check --config-dir or TERN_CONFIG_DIR`, exit 3.

Ask the user to open Tern. Apps can reach the mail while Tern is open. In Tern's MCP
settings, "Connection to your mail" shows whether apps can reach it.

## Access refused

| Error | Tell the user |
| --- | --- |
| `Access denied: apps on this computer are not allowed to use Tern. ...` | Turn on "Let apps on this computer use Tern" in Tern's MCP settings. |
| `Access denied: 'write' operations require access level 'read-write' ...` | Set Access to "Read and draft" in Tern's MCP settings. |
| `Access denied: 'send' operations require access level 'read-write-send' ...` | Set Access to "Read, draft and send" in Tern's MCP settings. |
| `<profile>/mcp.json is not a valid policy file: ...` | The settings file is damaged; every tool is refused until it is fixed. Opening Tern's MCP settings shows the problem. |

Changes take effect on the next tool call. Ask only for the level the task needs.

## Nothing found, or "not found"

- `Account '<id>' not found`, `Thread '<id>' not found`, `Message '<id>' not found`,
  `Folder '<id>' not found`: the id is wrong, or the user hid that account or folder from
  apps in Tern's MCP settings. Hidden items look exactly like missing ones. Don't look for
  them another way.
- `list_accounts` returns `[]`, or the CLI says `the running engines hold no accounts: add
  one in Tern`: no accounts yet, or all hidden.
- An account's `state` in `tern status` that isn't running (for example
  `stopped: ErrorAuthentication`) means Tern can't sync it; the user fixes it in Tern.
- A message body that never arrives: the account may be offline. `tern status` shows it.

## Old engine

`The Tern engine for account '<id>' can't do this (<feature>); update Tern` or `<tool> needs
a newer Tern engine`: the user's Tern is too old for that tool.

## The tools or the command are missing

- No `tern` MCP tools: in Tern's MCP settings, Connect the app (Claude Desktop, Claude Code,
  Codex), or run `tern-mcp install --client claude-desktop` (or `claude-code`, `codex`).
  Then restart the app and start a new chat.
- `tern: command not found`: Tern's Preferences > General > Command line > Install. On
  macOS it links `/usr/local/bin/tern` (or `~/.local/bin/tern`), on Linux
  `~/.local/bin/tern`; that directory must be on PATH.

## Wrong profile

Without `--config-dir`, `tern` and `tern-mcp` use the first default profile with a running
engine, else the first that exists:

- macOS: `~/Library/Application Support/Tern`
- Windows: `%APPDATA%\Tern`
- Linux: `$XDG_CONFIG_HOME/Tern` (usually `~/.config/Tern`), then `~/snap/tern/common`

A development build of Tern uses `Tern-dev` instead. If the mail isn't the user's, check
`tern status` / `tern-mcp status` and pass `--config-dir DIR` or set `TERN_CONFIG_DIR=DIR`.
An MCP entry made by `tern-mcp install` is pinned to the profile it was installed for.
