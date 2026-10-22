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
| `tern status` | running engines and their accounts' state, then the engine, gateway and mcp-http units | `{engines: [...], services: [{name, installed, running, pid, url, port, unit, bin, ...}]}` |
| `tern accounts` | the accounts | `{accounts: [{accountId, email, state, error, pid, socket}]}` |
| `tern folders`, `tern labels` | folders or labels with unread and total counts | |
| `tern threads [--role inbox] [--folder F] [--label L] [-n N] [--all] [--before CURSOR]` | threads, newest first (default 50) | `{threads, next, more}` |
| `tern search <query> [-n N]` | threads matching the search grammar (see search.md) | `{threads}` |
| `tern show <thread-id>` | a thread and its messages, without bodies | `{kind: "thread", thread, messages}` |
| `tern show <message-id> --body` | a message with its body (downloads it if needed) | `{kind: "message", message, body, files}` |
| `tern contacts <query> [-n N]` | contacts whose name or address starts with the words | `{contacts}`, each with `id`, `name`, `email`, `refs` |
| `tern contact get <id>` | a contact's details | |
| `tern calendar list [--from WHEN] [--to WHEN] [--calendar ID]` | events, now to a week ahead by default | `{occurrences}` |
| `tern draft list [-n N]` | unsent drafts, newest first, with their attachments and thread | `{drafts}`, each a Message with `id`, `hMsgId`, `threadId`, `files` |
| `tern task list [--status ...] [--class SendDraftTask]` | queued and finished tasks | |
| `tern logs [ACCOUNT\|engine\|gateway\|mcp-http] [-n N]` | an account's engine log, or a service's (works without an engine) | |

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

## Drafts and sending

The draft commands need the serve engine. Drafts first, as with the MCP tools: write the
draft, show it to the user, and send only when they say so.

| Command | What it does |
| --- | --- |
| `tern draft new [--to A] [--cc A] [--bcc A] [--subject S] [--body T] [--attach PATH]...` | a new message, from `--account` or the default sending account |
| `tern draft reply <message-id\|thread-id> [--body T] [--attach PATH]...` | a reply to the sender (a thread id answers its newest message that isn't a draft) |
| `tern draft reply-all <message-id\|thread-id> [--body T] [--attach PATH]...` | a reply to everyone |
| `tern draft forward <message-id\|thread-id> --to A [--body T] [--attach PATH]...` | a forward, with the attachments |
| `tern draft edit <draft-id\|Message-ID> [--to A] [--cc A] [--bcc A] [--subject S] [--body T] [--attach PATH]... [--detach NAME\|ID]...` | changes the draft in place: same id, thread and Message-ID |
| `tern draft delete <draft-id\|Message-ID>` | deletes the draft |
| `tern draft send <draft-id\|Message-ID> [--archive] [--wait [--timeout SECS]]` | sends now; it can't be called back |

- The text (`--body T`, `--body-file FILE`, or `--body-file -` for stdin) goes above the
  signature and the quoted message. It is plain text unless `--html`; `--plain` makes a
  plain-text draft. `--to`, `--cc`, `--bcc` take `a@x.com` or `"Name <a@x.com>"`,
  comma-separated or repeated, and replace the draft's recipients (the ones a reply starts
  with).
- To change a draft, use `tern draft edit` instead of writing a new one and leaving the old
  behind. Its `--body` replaces the text above the signature and quote and keeps those;
  the other options change only what they name. Running `tern draft reply` again on the
  same message also revises the draft it made; `--no-prefer-existing` starts a new one.
- `--attach PATH` (repeatable) attaches a file from this computer; Tern copies it, so the
  original can change or go afterwards. A file the draft already has (same name and
  contents) isn't attached twice, so a command run again is safe. `--detach` takes a file
  name from `tern draft list` or `tern show <draft-id>`, or a file id from `--json`. At
  most 25 MB of attachments go with a draft. Attach only files the user asked for.
- `tern draft edit` is refused while the draft is being sent or has a scheduled send
  (cancel it in Tern first). `tern draft delete` cancels a scheduled send with the draft,
  and is refused once the send has started. Deleting can't be undone: ask first.
- `--json` prints `{draft, updated}`: the saved Message (`id`, `hMsgId`, `threadId`, `to`,
  `subject`, `body`, `files`) and whether an existing draft was edited. `tern draft delete
  --json` prints `{id, headerMessageId, taskId, cancelledSend}`.
- `tern draft send` refuses a draft the checks stop (no recipients, a bad address) and
  prints the warnings (no subject, a forgotten attachment) before it sends.

There are no CLI commands for archiving, moving or labelling. Use the MCP tools for those.
`tern task queue` and `tern rpc` send raw engine requests; don't use them for mail changes
unless the user asks for exactly that.

## Tern's own AI

`tern ai summarize <thread-id>` and `tern ai triage` run through the AI provider the user
set up in Tern's Preferences > AI, which sends the mail to that provider and uses their
tokens. Use them only when the user asks for Tern's summaries or triage.
