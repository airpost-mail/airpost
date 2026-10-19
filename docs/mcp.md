# tern-mcp

`apps/mcp` builds `tern-mcp`, an MCP server for the mail in a Tern profile. It replaced the
desktop's `mcp-server` plugin, which ran inside the Electron app and has been removed. The
desktop ships tern-mcp (see "The desktop" below). tern-mcp talks to the running engine over its socket
(docs/engine-protocol-plan.md): reads go through the read API, and changes are queued as the
same task JSON the desktop sends (`__cls`, the `DraftFactory` fields), so the desktop and the
engine can't tell the two apart. With an engine that has the draft pipeline (`draft-pipeline`),
`create_draft` and `reply_to_thread` build the draft with `draft.create` and `send_draft` sends
with `draft.send`, so drafts get the same quoting, attribution line and signature as the
desktop's; against an older engine tern-mcp builds them itself with the same code
(`crates/ms-compose`). The desktop doesn't have to be open, but an engine does
(scheduled sends on an engine without the expiration executor are the exception, see
`schedule_send` below).

stdout carries MCP; stderr is the log, which MCP clients keep.

## Running it

```
tern-mcp [--config-dir DIR]                       # MCP on stdio (also: tern-mcp stdio)
tern-mcp http [--port N]                           # streamable HTTP on 127.0.0.1, see below
tern-mcp install   --client claude-desktop|claude-code|codex [--config-file PATH] [--force | --if-stale]
tern-mcp uninstall --client claude-desktop|claude-code|codex [--config-file PATH]
tern-mcp status [--json]
```

`--config-dir` (or `TERN_CONFIG_DIR`) picks the profile, the same way `tern` does
(apps/cli): without it, the first default profile with a live engine, else the first that
exists. `install` pins the profile it resolved into the client entry.

`install` writes one stdio entry named `tern` whose command is this binary (canonical path)
with `--config-dir DIR`:

- Claude Desktop: `mcpServers.tern` in `claude_desktop_config.json` (macOS
  `~/Library/Application Support/Claude`, Windows `%APPDATA%\Claude`, else
  `$XDG_CONFIG_HOME/Claude`).
- Claude Code: `claude mcp add-json tern … --scope user` (the `claude` binary from PATH, else a
  login shell), or with `--config-file` the file's top-level `mcpServers`.
- Codex: a `[mcp_servers.tern]` table in `$CODEX_HOME/config.toml` (default `~/.codex`).

It never writes or removes the `mailspring` / `mailspring-mcp` entries that Mailspring.app
registers. A config file that doesn't parse is an error and is left untouched; writes go
through an owner-only temporary file and a rename, following symlinks, and keep the file's
mode. An existing `tern` entry whose command isn't `tern-mcp` is left alone: `install` refuses
it unless given `--force`, and `uninstall` refuses it. The exception is the entry the removed
`mcp-server` plugin wrote: Tern's Electron binary running `…/mcp-stdio-bridge.js` (any argument
naming that file) with `--config-dir`. The script no longer ships, so `install` replaces that
entry without `--force` and `uninstall` removes it. `install --if-stale` replaces only that
entry, and only when its `--config-dir` is the profile being installed (Claude Desktop's config
is shared by every profile); otherwise it changes nothing and says so. The desktop runs
`install --client claude-desktop --if-stale` once per profile after the upgrade. For Claude Code without `--config-file`
the check reads the user-scope entry from `.claude.json` (in `$CLAUDE_CONFIG_DIR`, else the
home directory); only `claude mcp` writes it. In Codex's config.toml only `[mcp_servers.tern]`
tables are rewritten; a `tern` written as an inline table or dotted keys is refused, to be
removed by hand.

Engines are found through `<config-dir>/run/*.json`: the per-account files
(`{pid, socket, protocol, accountId}`) and phase C's shared `engine.json`
(`{pid, socket, protocol, accounts}`), which wins when both exist. Each tool call scans and
connects afresh (`hello` with client `tern-mcp/<version> (<MCP client name>)`), so an engine
restart doesn't need a server restart. With no engine the tools return an error that says to
open Tern.

## Policy: `<config-dir>/mcp.json`

```json
{
  "accessLevel": "read-only",
  "enabledAccounts": {
    "<accountId>": {"enabled": true, "excludedFolderIds": ["<folderId>"]}
  },
  "http": {"enabled": false, "port": 2587}
}
```

- `accessLevel`: `read-only` (default), `read-write` or `read-write-send`, with the plugin's
  tool categories: read tools need nothing, drafts and thread changes need `read-write`,
  `send_draft`, `schedule_send`, `unsubscribe` and `respond_to_event` (and events with
  guests) need `read-write-send`. An unknown value is read-only.
- `enabledAccounts`: empty means every account. Otherwise an account that is absent or has
  `enabled: false` is invisible: not listed, and its ids answer "not found". Threads and
  messages in an excluded folder are hidden the same way, and the folder isn't listed.
- The file is re-read on every call. A file that doesn't parse is an error for every tool, so
  a typo can't widen access.
- Without `mcp.json`, `accessLevel` and `enabledAccounts` come from the desktop's
  `config.json` keys `core.mcp.accessLevel` / `core.mcp.enabledAccounts`, which the removed
  plugin's pane wrote. The desktop moves them into `mcp.json` on the first launch after the
  upgrade (and when the pane opens) and deletes `core.mcp`, so the fallback only covers a
  profile the new desktop hasn't opened yet. HTTP stays off in that fallback and after the
  move (the plugin's port is kept, its token is not). A null `enabled` or
  `excludedFolderIds` reads as false or `[]`, as in the plugin; any other value that doesn't
  fit is an error, like a bad `mcp.json`, and the desktop leaves such keys where they are.

## HTTP

`tern-mcp http` serves MCP's streamable HTTP transport at `http://127.0.0.1:<port>/mcp`, only
when `http.enabled` is true. Every request needs `Authorization: Bearer <token>`; the token
is in `<config-dir>/mcp-token`, created owner-only (0600) on first use. Requests whose `Host`
isn't a loopback name get 403, which stops DNS rebinding. `install` only writes stdio entries.
Nothing starts `tern-mcp http` for you; the desktop no longer serves MCP over HTTP itself.

## Audit log: `<config-dir>/mcp-audit.log`

One JSON object per line, owner-only, rotated to `mcp-audit.log.1` at 4 MiB:

```json
{"timestamp": 1792137600000, "client": "claude-ai 0.1.0", "toolName": "update_draft",
 "accountId": "a1", "outcome": "ok", "params": "{\"body\":\"<120 chars>\",\"id\":\"d0\"}",
 "resultSummary": "ok", "durationMs": 41}
```

`outcome` is `ok`, `denied` (policy) or `error`. `params` has `body` replaced by its length,
secret-looking keys (`password`, `token`, `secret`, `authorization`, …) dropped, and is cut to
200 characters; `resultSummary` is cut to 100. `resources/read` is logged too.

## Tools

The plugin's tools keep their names, arguments and output shapes:

| Category | Tools |
| --- | --- |
| read | `list_accounts`, `list_folders`, `search_mail`, `list_threads`, `get_thread`, `get_message`, `get_message_tracking`, `list_tracked_messages`, `get_top_links`, `list_scheduled_sends`, `get_attachment`, `list_events`, `get_contact` |
| read-write | `create_draft`, `reply_to_thread`, `update_draft`, `delete_draft`, `archive_threads`, `trash_threads`, `move_to_folder`, `set_labels`, `set_unread`, `set_starred`, `cancel_scheduled_send`, `snooze_threads`, `unsnooze_threads`, `create_event`, `update_event`, `create_contact`, `update_contact` |
| read-write-send | `send_draft`, `schedule_send`, `unsubscribe`, `respond_to_event`; `create_event` and `update_event` with `attendees` |

Attachments are also served as the resource template `attachment://{messageId}/{fileId}`,
read from `<config-dir>/files/` (fetching the body first when the file isn't there yet).

Changes from the plugin:

- Every message in `get_thread` / `get_message` carries `draft`, and the server instructions say
  that `draft: true` is the user's unsent draft.
- `reply_to_thread` replies to the newest message that is neither a draft nor hidden, or to
  `messageId` (a draft is refused). The plugin replied to the newest message, which after one
  reply was the user's own draft, so a second reply quoted the draft instead of the mail.
  The result has `repliedTo`, `quoted`, `to`, `cc` and `existingDrafts`, with a note to use
  `update_draft` when one of them was meant to be revised.
- `update_draft` (new) changes a draft's subject, body, To, Cc or Bcc by `id` or
  `headerMessageId`. It saves through `SyncbackDraftTask` with the stored draft's identity
  (id, headerMessageId, thread, reply-to, files, metadata). It refuses a draft being sent: one
  with a pending send-later/undo-send, or one this server queued `SendDraftTask` for.
- `delete_draft` (new) queues `DestroyDraftTask`, with the same refusal.
- `send_draft` and `update_draft` wait for the draft's body (up to 15 s) and read the draft
  again before building the task. A draft synced from another device gets `plaintext` and its
  `files` only when its body is downloaded; the header-only copy would fail to send or drop
  the attachments.
- `schedule_send` only writes the `send-later` metadata. A `serve` engine (`hello.features`
  lists `expiration-executor`, docs/desktop-client-plan.md 3.3) sends the draft at `sendAt`
  itself, or when it next starts. An engine without it announces the due send as a
  `metadata-expiration` delta and the desktop's send-later plugin sends it, so the desktop
  has to be running at `sendAt`; the result's `note` says which applies. `sendAt` without an offset is local time, with or without
  seconds; a date alone is midnight UTC (both as `new Date()` read them).

- `snooze_threads` (new) and `unsnooze_threads` (new) call the engine's `threads.snooze` and
  `threads.unsnooze` (the `snooze` feature, docs/desktop-client-plan.md 3.3): the move to or
  from Snoozed and the `thread-snooze` metadata in one call. `until` reads as `sendAt` does.
  The engine's expiration executor brings the threads back; an engine without it leaves that
  to the desktop.
- `unsubscribe` (new) calls `messages.unsubscribe` (the `unsubscribe` feature): the engine
  makes the one-click POST or sends the mailto message from the account, and a web-only list
  comes back as a `url` with a note to have the user open it. It is in the send category
  because it can send mail.
- `list_events`, `create_event`, `update_event` and `respond_to_event` (new) call the engine's
  `events.expand`, `events.create`, `events.update` and `events.rsvp` (the `calendar-editing`
  feature, docs/desktop-client-plan.md 3.8); `get_contact`, `create_contact` and
  `update_contact` (new) its `contacts.get`, `contacts.create` and `contacts.update`
  (`contact-editing`). The engine builds the ICS or vCard and queues the tasks the desktop
  would. Times read as `sendAt` does, in the machine's zone, which is passed as `tz`.
  `respond_to_event` mails the organizer, and guests on an event are mailed by the calendar
  server, so those need `read-write-send`.

`list_tracked_messages`, `get_top_links` and `list_scheduled_sends` read through
`metadata.list` and fail with "needs a newer Tern engine" on an engine without it. With
`tasks.list` (the `tasks-list` feature), the send check before editing a draft sees every queued
`SendDraftTask`, including the desktop's; without it, only the sends this session queued.

`search_mail` hands the query to the engine's search grammar (the `search-grammar` feature):
words and quoted phrases, `from:`, `to:`, `subject:`, `in:`, `is:unread|read|starred|unstarred`,
`has:attachment`, `before:` and `after:`/`since:` (dates such as `2026-05-31`, `yesterday`,
`"3 days ago"`, `2w`, read in the local time zone), `AND`, `OR`, `NOT` and parentheses. tern-mcp
only applies the access policy to the results; a query the engine can't read is an error with
the engine's reason. On an engine without the grammar, tern-mcp parses a subset itself: plain
terms with `in:`, `is:`, `has:attachment` and absolute dates, refusing `OR`, `NOT`,
parentheses and relative dates, and passing `from:`-style values to the search as terms.

## The desktop

- The packaged app ships tern-mcp next to the engine, in `app.asar.unpacked/tern-mcp`
  (`apps/desktop/app/build/build.js`; `apps/engine/packaging/build-release.sh
  --install-desktop` builds and places it). A source checkout sets `TERN_MCP_BIN`.
- Preferences > Integrations > AI assistants edits `mcp.json` with the semantics above (access
  level, accounts and excluded folders, HTTP on/off and port), and refuses to touch a
  `mcp.json` that doesn't parse. It shows `tern-mcp status` (engines found), runs the bundled
  `tern-mcp install` / `uninstall` for Claude Desktop, Claude Code and Codex with their output
  (and `--force` behind a "Replace It" button when a foreign `tern` entry is in the way), and
  the newest entries of `mcp-audit.log`.
- On the main window's start the desktop moves `core.mcp.*` into `mcp.json` and runs
  `tern-mcp install --client claude-desktop --if-stale` once per profile
  (`core.ternMcp.staleBridgeEntryChecked`), never in specs or dev mode. On macOS it waits
  until the app runs from /Applications, since tern-mcp writes its own path into the entry
  and a copy on a DMG or in ~/Downloads would leave a path that later disappears.
- The plugin's HTTP registrations point at a server that no longer runs: Codex's
  `[mcp_servers.tern]` with a `url` (Add refuses it until "Replace It") and Claude Code's
  user-scope `tern-mcp` entry (another name, left alone; remove it with
  `claude mcp remove tern-mcp --scope user`).
