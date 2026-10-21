# Product model

Agreed 2026-10-19. How Tern looks to the people who use it, across the desktop, local
agents and the web. Where this disagrees with `docs/architecture.md`, this file wins; section 7 lists
what changes there.

## 1. Principles

- Tern is a mass-market mail client. Users never see "engine", "gateway" or "backend".
  They see the Tern app, Tern on the web, and a Tern server.
- The only question a user has to answer is where their mail syncs: on this computer, or
  on a Tern server as well.
- The mail provider (Gmail, Outlook, any IMAP server) is the source of truth for mail.
  Every Tern client syncs with it directly. A Tern server syncs the rest: settings,
  signatures, templates, rules, snoozes, scheduled sends, tracking.
- Nothing needs an account or a server to start. A server adds features; it never
  unlocks the basics.
- A feature that needs a server says so ("Needs a Tern server") and links to the setup.
  It never looks broken.

## 2. Scope now

| Use case | Now | Later |
|---|---|---|
| Desktop app, local only | yes | |
| Local agents and CLI on the same computer | yes | |
| Self-hosted Tern server: web mail, always-on features, sync for the desktop | yes, one person per server | several people per server |
| Hosted Tern (tern.app) | no | the same server, run by us |
| Cloud agents (claude.ai, ChatGPT) | no | remote MCP on the server |

Hosted is out of scope, but nothing built now may rule it out. Section 6 lists the rules
that keep it possible.

## 3. Desktop, local only

What the user does: download, open, add a mail account. No sign-up. The engine is
bundled and started by the app, and it appears in the UI only by name in the descriptions
under "Services on this computer".

Everything that can run locally does: mail, search, calendar, contacts, snooze, send
later, reminders, templates, rules, and AI with the user's own key or a local model.
Features that need a server (read receipts, link tracking, sharing, sync between
devices) show "Needs a Tern server" in place of their controls.

**Background mode.** Closing the window leaves Tern running (on macOS always, with "Show
Tern in the menu bar" only showing or hiding the icon; elsewhere while the tray icon is
on), so scheduled sends go out and agents keep working. Quitting stops the app.

**Services on this computer.** Preferences > General has a section for what runs next to
the app, in plain labels with the program's name in the description (agreed 2026-10-21, an
exception to section 1's "never show engine/gateway"):

- "Keep Tern running in the background": installs the engine as a launchd or systemd user
  unit (`tern engine install --no-load`, then enabled on Linux). It starts when the app
  quits and at login, and the app joins it at launch, so mail keeps syncing with Tern
  closed. Uninstall removes it.
- "Web access from this computer": the app runs `tern-gateway serve` on 127.0.0.1:5180
  with the bundled web client while it is open, and "Pair a browser" shows a one-time code
  and link (`tern-gateway pair`).
- "MCP over HTTP": the state of the MCP tab's HTTP server, with a link there.
- "tern command": puts `tern` on PATH, as a link to the bundled binary, or a wrapper that
  pins the profile when the app runs on another one.

## 4. Local agents and the CLI

A Preferences tab, "MCP", replaces the MCP section in Integrations:

- "Local apps (stdio)": a switch ("Allow apps over stdio", `enabled` in mcp.json) and
  Connect buttons per client: Claude Desktop, Claude Code, Codex (`tern-mcp install`).
- "HTTP": a switch (`http.enabled`), the port, the access key and the server's state. The
  app runs `tern-mcp http` while it is open and HTTP is on.
- One access level for every app, in plain words: "Read mail", "Read and draft", "Read,
  draft and send" (`read-only`, `read-write`, `read-write-send`), and the accounts and
  folders apps can see.
- An activity list: what each agent read, drafted or sent, and when.

The `tern` command is installed from General (section 3). `tern` and `tern-mcp` talk to the
local engine over its socket. There is no network hop and no login, because being the same
user on the same computer is the trust check.

## 5. Self-hosted Tern server

For the technical user who wants web mail and always-on features on hardware they
control.

**One thing to run.** `tern-server` is one image (and one tarball for non-Docker
installs) with one compose file. Inside are the server's account and sync layer (today's
`apps/backend`), web access (today's `apps/gateway` with the built `apps/web`), and the
engine, supervised by the server. They stay separate processes, so the code that faces
the internet never shares memory with mail credentials, but the operator installs,
configures, updates and backs up one thing.

**One person per server for now.** The first visit to the web address runs setup: create
the owner's login (email and password), then add mail accounts. The server runs one
engine, for the owner. Several people per server waits; the data model is per user from
the start (section 6), so it adds without a migration.

**Web.** Sign in with the server login. Each browser is a session the owner can see and
revoke in the web settings. The gateway's pairing codes stay only as a fallback for
signing in a device without a keyboard.

**Desktop and server together.** In the desktop, Preferences > Accounts gets "Tern
server": enter the URL and sign in. Then:

- The desktop keeps syncing mail itself, so it stays fast and works offline. It doesn't
  become a thin client of the server.
- Settings, signatures, templates, rules and plugin metadata sync through the server.
- Server features turn on: tracking, sharing, and always-on scheduled sends and rules.
- The desktop asks once per mail account: "Also sync this account on your server? Needed
  for Tern on the web and for sends while this computer is off." Yes hands the account's
  credentials to the server's engine over the signed-in connection. Mail accounts are
  added once, wherever the user starts.

**CLI.** `tern` stays local. Pointing it at a server (`tern login <url>`) comes later,
with remote MCP.

## 6. Keeping hosted possible

Hosted Tern is this server, run by us with many users and sign-up. To keep that a change
in operations and not a rewrite:

- Everything on the server is keyed by user id: storage, sessions, settings, engine
  state. One person per server is a limit in setup and in the engine supervisor, not in
  the data model.
- Each user's mail credentials are encrypted at rest under a per-user key and are only
  ever decrypted inside that user's engine process. The account and web layers never
  hold them.
- The supervisor owns "start an engine for user X". Today it starts one. Later it starts
  one per user, possibly on other machines.
- Clients find out what a server can do from its feature list (`/tern/v1/server`), never
  from "is this hosted". A self-hosted server and tern.app answer the same way.
- The desktop's server field takes any URL. Hosted later adds tern.app as the first
  choice in the same field.
- Wording says "your Tern server", never "your self-hosted server", so the same screens
  serve both.
- OAuth for Gmail and Microsoft uses Tern's own client ids, registered for both desktop
  and server use, so hosted needs no new consent flow.

## 7. What this changes

| Doc | Change |
|---|---|
| `docs/architecture.md` | The trust boundary holds between processes: the account and web layers never hold mail credentials; the engine does, on the desktop and inside `tern-server`. "The backend never holds mail credentials" becomes "the backend process never does". Gateway and backend ship together in `tern-server`. |
| `docs/backend-plan.md` | B1-C (desktop sign-in) is back in scope as part of section 5. Users and sessions stay; per-user credential storage for the server's engine is new. |

## 8. Order

1. **Desktop:** the Agents tab, the command-line tool install, background mode with the
   tray icon. No server work.
2. **`tern-server`:** one image with engine, web and account layers; setup on first
   visit; web sign-in with the server login; adding mail accounts from the web.
3. **Desktop with server:** sign in, settings and metadata sync, server features
   lighting up, "also sync on server".
4. **Later:** remote MCP, `tern login`, several people per server, hosted.

## 9. Decided

2026-10-19, the user's answers:

1. One person per server for now. Several people per server, and hosted, come later on
   the same data model.
2. Background mode is off by default and turns on when the user enables agent access.
3. Agents is its own Preferences tab; MCP moves there from Integrations.
