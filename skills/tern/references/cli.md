# The `tern` command

`tern` talks to the running Tern engine over its local socket. `tern --help` and
`tern <command> --help` list everything; this page covers what helps with mail.

**It doesn't apply Tern's MCP settings** (access level, hidden accounts and folders, the
"Let apps on this computer use Tern" switch). It runs with the user's full rights, so ask
before every change, and never use it to get around something MCP refused or hid.

## Global options

- `--json`: one JSON document (NDJSON for streams such as `tern watch`). Use it whenever
  you parse output.
- `-a, --account <ID|EMAIL>`: the account to act on. Read commands without it cover every
  account.
- `--config-dir <DIR>` or `TERN_CONFIG_DIR`: the Tern profile. The default is the desktop
  app's profile for this OS.

Exit codes: 0 ok, 1 the command failed, 2 usage error, 3 no engine running.

## Reading

| Command | What it does | `--json` shape |
| --- | --- | --- |
| `tern status` | running engines and their accounts' state | `{engines: [...]}` |
| `tern accounts` | the accounts | `{accounts: [{accountId, email, state, error, pid, socket}]}` |
| `tern folders`, `tern labels` | folders or labels with unread and total counts | |
| `tern threads [--role inbox] [--folder F] [--label L] [-n N] [--all] [--before CURSOR]` | threads, newest first (default 50) | `{threads, next, more}` |
| `tern search <query> [-n N]` | threads matching the search grammar (see search.md) | `{threads}` |
| `tern show <thread-id>` | a thread and its messages, without bodies | `{kind: "thread", thread, messages}` |
| `tern show <message-id> --body` | a message with its body (downloads it if needed) | `{kind: "message", message, body, files}` |
| `tern contacts <query> [-n N]` | contacts whose name or address starts with the words | `{contacts}`, each with `id`, `name`, `email`, `refs` |
| `tern contact get <id>` | a contact's details | |
| `tern calendar list [--from WHEN] [--to WHEN] [--calendar ID]` | events, now to a week ahead by default | `{occurrences}` |
| `tern task list [--status ...] [--class SendDraftTask]` | queued and finished tasks | |
| `tern logs [ACCOUNT] [-n N]` | an account's engine log (works without an engine) | |

The JSON is the engine's own model, not the MCP shapes: a thread's account is `aid`, its
last-received time `lmrt` (Unix seconds), a message's Message-ID `hMsgId`.

Quote the whole query for `tern search` so the shell keeps quotes and parentheses:
`tern search --json 'from:alice "project alpha" after:2w'`.

## Changing things

Ask the user first, as for the MCP tools.

| Command | What it does |
| --- | --- |
| `tern snooze --until WHEN <thread-id>...` | snooze; WHEN is `30m`, `2h`, `3d`, `1w`, `tomorrow` (9:00), a date (9:00 that day) or a date and time (`2026-11-02T14:30`, local unless it has an offset) |
| `tern unsnooze <thread-id>...` | back to the inbox now |
| `tern unsubscribe <message-id> [--method one-click\|mailto\|web]` | may send mail or make a web request |
| `tern calendar create --calendar ID --title T --start WHEN [--end WHEN] [--all-day] [--location L] [--description D] [--guest EMAIL]... [--repeat RRULE]` | with `--guest` the guests are invited by mail |
| `tern calendar update <event-id> [--start] [--end] [--title] [--location] [--description] [--delete] [--scope this\|following\|all] [--occurrence WHEN]` | |
| `tern calendar rsvp <event-id> accepted\|tentative\|declined [--comment C] [--propose-start WHEN --propose-end WHEN]` | mails the organizer |
| `tern contact create\|update ... [--name] [--email] [--phone] [--company] [--title] [--notes]` | |

There are no CLI commands for drafts, sending, archiving, moving or labelling. Use the MCP
tools for those. `tern task queue` and `tern rpc` send raw engine requests; don't use them
for mail changes unless the user asks for exactly that.

## Tern's own AI

`tern ai summarize <thread-id>` and `tern ai triage` run through the AI provider the user
set up in Tern's Preferences > AI, which sends the mail to that provider and uses their
tokens. Use them only when the user asks for Tern's summaries or triage.
