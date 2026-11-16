# Product model

Agreed 2026-10-19. How Airpost looks to the people who use it, across the desktop, local
agents and the web. Where this disagrees with `docs/architecture.md`, this file wins; section 7 lists
what changes there.

## 1. Principles

- Airpost is a mass-market mail client. Users never see "engine", "gateway" or "backend".
  They see the Airpost app, Airpost on the web, and an Airpost server.
- The only question a user has to answer is where their mail syncs: on this computer, or
  on an Airpost server as well.
- The mail provider (Gmail, Outlook, any IMAP server) is the source of truth for mail.
  Every Airpost client syncs with it directly. An Airpost server syncs the rest: settings,
  signatures, templates, rules, snoozes, scheduled sends, tracking.
- Nothing needs an account or a server to start. A server adds features; it never
  unlocks the basics.
- A feature that needs a server says so ("Needs an Airpost server") and links to the setup.
  It never looks broken.

## 2. Scope now

| Use case | Now | Later |
|---|---|---|
| Desktop app, local only | yes | |
| Local agents and CLI on the same computer | yes | |
| Self-hosted Airpost server: web mail, always-on features, sync for the desktop | yes, one person per server | several people per server |
| Hosted Airpost (airpost.app) | no | the same server, run by us |
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
devices) show "Needs an Airpost server" in place of their controls.

**Background mode.** Closing the window leaves Airpost running (on macOS always, with "Show
Airpost in the menu bar" only showing or hiding the icon; elsewhere while the tray icon is
on), so scheduled sends go out and agents keep working. Quitting stops the app.

**Services on this computer.** Preferences > General has a section for what runs next to
the app, in plain labels with the program's name in the description (agreed 2026-10-21, an
exception to section 1's "never show engine/gateway"). Each service is a launchd or systemd
user unit owned by the `airpost` CLI (`airpost <engine|gateway|mcp-http> install|uninstall|start|
stop`, state in `airpost --json status`), so it works the same from a terminal and keeps
running after the app quits; the desktop only runs those commands.

- "Keep Airpost running in the background": the engine unit, written with `--no-load` so it
  doesn't fight the app's own engine for the socket. It starts when the app quits and at
  login, and the app joins it at launch.
- "Web access from this computer": the gateway unit (`airpost gateway install --bind
  127.0.0.1:5180 --web-root <bundled web client>`), and "Pair a browser" shows a one-time
  code and link (`airpost gateway pair`).
- "airpost command": puts `airpost` on PATH, as a link to the bundled binary, or a wrapper that
  pins the profile when the app runs on another one.

The backend is a server component and has no row here.

## 4. Local agents and the CLI

A Preferences tab, "MCP", replaces the MCP section in Integrations:

- "Local apps (stdio)": a switch ("Allow apps over stdio", `enabled` in mcp.json) and
  Connect buttons per client: Claude Desktop, Claude Code, Codex (`airpost-mcp install`).
- "HTTP": a switch (`http.enabled` in mcp.json, and the `mcp-http` unit through `airpost
  mcp-http install --port N|uninstall`), the port, the access key and the server's state.
- One access level for every app, in plain words: "Read mail", "Read and draft", "Read,
  draft and send" (`read-only`, `read-write`, `read-write-send`), and the accounts and
  folders apps can see.
- An activity list: what each agent read, drafted or sent, and when.

The `airpost` command is installed from General (section 3). `airpost` and `airpost-mcp` talk to the
local engine over its socket. There is no network hop and no login, because being the same
user on the same computer is the trust check.

An agent uses one of the two. MCP follows the access level and hidden accounts above and
has no skill; its server instructions carry the safety rules. The CLI is the user at their
terminal, with full access, and comes with a skill built into the binary (`airpost skill
install`).

## 5. Self-hosted Airpost server

For the technical user who wants web mail and always-on features on hardware they
control.

**One thing to run.** `airpost-server` is one image (and one tarball for non-Docker
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

**Desktop and server together.** In the desktop, Preferences > Accounts gets "Airpost
server": enter the URL and sign in. Then:

- The desktop keeps syncing mail itself, so it stays fast and works offline. It doesn't
  become a thin client of the server.
- Settings, signatures, templates, rules and plugin metadata sync through the server.
- Server features turn on: tracking, sharing, and always-on scheduled sends and rules.
- The desktop asks once per mail account: "Also sync this account on your server? Needed
  for Airpost on the web and for sends while this computer is off." Yes hands the account's
  credentials to the server's engine over the signed-in connection. Mail accounts are
  added once, wherever the user starts.

**CLI.** `airpost` stays local. Pointing it at a server (`airpost login <url>`) comes later,
with remote MCP.

## 6. Keeping hosted possible

Hosted Airpost is this server, run by us with many users and sign-up. To keep that a change
in operations and not a rewrite:

- Everything on the server is keyed by user id: storage, sessions, settings, engine
  state. One person per server is a limit in setup and in the engine supervisor, not in
  the data model.
- Each user's mail credentials are encrypted at rest under a per-user key and are only
  ever decrypted inside that user's engine process. The account and web layers never
  hold them.
- The supervisor owns "start an engine for user X". Today it starts one. Later it starts
  one per user, possibly on other machines.
- Clients find out what a server can do from its feature list (`/airpost/v1/server`), never
  from "is this hosted". A self-hosted server and airpost.app answer the same way.
- The desktop's server field takes any URL. Hosted later adds airpost.app as the first
  choice in the same field.
- Wording says "your Airpost server", never "your self-hosted server", so the same screens
  serve both.
- OAuth for Gmail and Microsoft uses Airpost's own client ids, registered for both desktop
  and server use, so hosted needs no new consent flow.

## 7. What this changes

| Doc | Change |
|---|---|
| `docs/architecture.md` | The trust boundary holds between processes: the account and web layers never hold mail credentials; the engine does, on the desktop and inside `airpost-server`. "The backend never holds mail credentials" becomes "the backend process never does". Gateway and backend ship together in `airpost-server`. |
| `docs/backend-plan.md` | B1-C (desktop sign-in) is back in scope as part of section 5. Users and sessions stay; per-user credential storage for the server's engine is new. |

## 8. Order

1. **Desktop:** the Agents tab, the command-line tool install, background mode with the
   tray icon. No server work.
2. **`airpost-server`:** one image with engine, web and account layers; setup on first
   visit; web sign-in with the server login; adding mail accounts from the web.
3. **Desktop with server:** sign in, settings and metadata sync, server features
   lighting up, "also sync on server".
4. **Later:** remote MCP, `airpost login`, several people per server, hosted.

## 9. Decided

2026-10-19, the user's answers:

1. One person per server for now. Several people per server, and hosted, come later on
   the same data model.
2. Background mode is off by default and turns on when the user enables agent access.
3. Agents is its own Preferences tab; MCP moves there from Integrations.
