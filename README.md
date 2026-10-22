# Tern

Tern is a desktop email client for macOS and Linux, forked from Mailspring, with its sync
engine rewritten in Rust. It works with Gmail, Microsoft 365, Outlook.com and any
IMAP/SMTP server, and it needs no account or server of its own: download it, add a mail
account, and your mail syncs directly between your computer and your provider. On top of the
desktop app, the same engine can be driven from a terminal, by AI agents over MCP, and from a
browser through a web client you host yourself.

<!-- TODO: capture assets/screenshots/main.png (see the app-screenshots skill) -->
![Tern's main window](assets/screenshots/main.png)

## Features

- **Mail, calendar and contacts.** Unified inbox, threads, labels and folders, search with
  `from:`, `in:`, `is:`, `has:attachment`, `before:`/`after:` and boolean operators.
  Calendars and contacts sync over CalDAV and CardDAV (Google contacts through the People
  API).
- **Local features that need no server.** Snooze, send later, send reminders, templates,
  mail rules, one-click unsubscribe, undo send, signatures.
- **Privacy defaults.** Remote images are blocked until you load them, tracking pixels from
  known trackers are stripped from incoming mail, contact photos from Gravatar are off, and
  mail whose reply-to address differs from the sender's gets a phishing warning.
- **Rust sync engine** (`tern-engine`). One process holds all your accounts and serves the
  desktop, the CLI, the MCP server and the gateway over a local socket. It can keep running
  after the app quits, so scheduled sends still go out.
- **The `tern` command.** Status, accounts, threads, search, show, snooze, unsubscribe,
  contacts, calendar, tasks, logs and a live `watch`, with `--json` on everything. It also
  installs and manages Tern's background services as launchd or systemd user units
  (`tern engine|gateway|mcp-http install`).
- **MCP server for AI agents** (`tern-mcp`). Over stdio for Claude Desktop, Claude Code and
  Codex (`tern-mcp install`), or streamable HTTP with a bearer token. Three access levels
  (read; read and draft; read, draft and send), per-account and per-folder limits, and an
  audit log. Configured in Preferences > MCP. See [docs/mcp.md](docs/mcp.md).
- **Web client and gateway.** `tern-gateway` serves the web client (`apps/web`) and relays it
  to an engine, on your own computer or on a server. Browsers pair with a one-time code.
- **AI**, optional and off until you set it up: thread summaries, reply suggestions and
  inbox sorting, with your own key for an OpenAI-compatible or Anthropic API, or a local
  model through Ollama, LM Studio or llama.cpp.
- **Plugins and themes.** The plugin system from Mailspring, with built-in features and
  bundled plugins loaded the same way, six bundled themes, and service plugins that run as
  their own process next to the engine ([examples/service-plugin](examples/service-plugin)).
  Integrations for translation (LibreTranslate, DeepL) and grammar checking (LanguageTool)
  use your own servers or keys.

Read receipts, link tracking, thread sharing and settings sync between computers need a Tern
server and aren't available yet; the desktop shows them as "Needs a Tern server".

## Install

No release has been published yet. Until the first `vX.Y.Z` tag, build from source (below or
[CONTRIBUTING.md](CONTRIBUTING.md)).

Once released, the desktop app on macOS is `brew install --cask ternmail/tap/tern`, and the
command-line programs (`tern`, `tern-engine`, `tern-mcp`, `tern-gateway` and the web client,
for macOS and Linux) come from an install script, Homebrew, .deb/.rpm, the AUR or Nix.
Every method is in [docs/install.md](docs/install.md). There is no Windows build of the
engine yet, so Windows isn't supported.

## Quick start

### Desktop, from source

You need the Node version in `.node-version`, Rust (pinned by `rust-toolchain.toml`), Python 3
and a C/C++ toolchain.

```sh
git clone https://github.com/ternmail/ternmail.git
cd ternmail
make setup     # npm ci, Python deps, debug Rust builds
make desktop   # the app on a sandbox profile in .dev/profile
make mail      # optional: fake IMAP and SMTP accounts to test against
```

`make desktop` never touches your real profile. `make help` lists every target.

### Command line, without the desktop

```sh
tern engine install            # the engine in the background, started at login
tern account add you@example.com
tern status
tern search from:alice has:attachment
```

To give an AI agent your mail, pick one: `tern-mcp install --client claude-code` (MCP, with the
access level and hidden folders you set in Tern), or `tern skill install` (the CLI with a skill;
full access, as you have at your terminal). [docs/mcp.md](docs/mcp.md) has both.

For the web client on the same computer, `tern gateway install` and `tern gateway pair`. In
the desktop app the same services are switches in Preferences > General.

### Self-hosted Tern server

`deploy/server/` runs the engine headless with the gateway, the web client and a TLS proxy,
and optionally the backend and MCP over HTTP, from one image and one compose file:

```sh
cd deploy/server
cp .env.example .env            # TERN_DOMAIN, TERN_VERSION, COMPOSE_PROFILES
TERN_VERSION=dev docker compose -f compose.yaml -f compose.build.yaml up -d --build
docker compose exec engine tern account add you@example.com
docker compose exec gateway tern-gateway pair
```

The server holds one person's mail for now. Setup, accounts, backups and upgrades are in
[docs/self-hosting.md](docs/self-hosting.md).

## Architecture

The engine is the only process that touches mail: it holds the credentials, the mail
database and search, and speaks JSON-RPC on a Unix socket that only the same user can reach.
The desktop, `tern`, `tern-mcp` and `tern-gateway` are separate clients of that socket, and
the web client reaches it only through the gateway. The backend is a separate server for
metadata sync between devices and never holds mail credentials or content. launchd, systemd
or Docker supervise the processes; there is no orchestrator. More in
[docs/architecture.md](docs/architecture.md) and [docs/product-model.md](docs/product-model.md).

| Path | Contents |
|---|---|
| `apps/desktop/` | Electron client (TypeScript, React); see its README.md and CLAUDE.md |
| `apps/engine/` | Rust sync engine, `tern-engine`, and its black-box test harness |
| `apps/cli/` | The `tern` command |
| `apps/mcp/` | The MCP server, `tern-mcp` |
| `apps/gateway/` | `tern-gateway`: serves the web client and relays it to an engine |
| `apps/web/` | Web client (Vite, React, shadcn/ui); runs against a gateway or an in-browser mock engine for development |
| `apps/backend/` | `tern-backend`: users, device tokens and metadata sync; tracking and sharing not yet implemented |
| `crates/` | Rust libraries: IMAP, SMTP, MIME, storage, sync, tasks, CalDAV/CardDAV, the socket protocol, AI, secrets |
| `packages/` | TypeScript libraries shared by the apps (`email-dark`) |
| `docs/` | Architecture, plans and guides; desktop docs (plugins, themes, localization) in `docs/desktop/` |
| `packaging/` | Release archives, install script, Homebrew, AUR, Nix and Linux package definitions |
| `deploy/` | Compose files for the Tern server and the backend, systemd units, a local dev mail stack |
| `examples/` | A sample service plugin |
| `skills/` | The agent skill for working with mail through the `tern` CLI (`tern skill install`) |
| `fuzz/` | Fuzz targets for the MIME, IMAP, iCalendar and vCard parsers |
| `assets/brand/` | Tern logos (trademarks, see below) |

## Relationship to Mailspring

Tern started as a fork of [Mailspring](https://github.com/Foundry376/Mailspring) by Foundry
376, LLC and contributors. The desktop app is Mailspring's, modified since, and the engine and
`crates/` are a Rust translation of [Mailspring-Sync](https://github.com/Foundry376/Mailspring-Sync),
including the parts of mailcore2, libetpan and iCalendarLib it carries. Thanks to everyone who
built them. Tern removes the Mailspring ID and its subscription features, and the v1 plugin
API keeps Mailspring's module names so existing plugins still load. An existing Mailspring
profile can be brought over with Preferences > Accounts > Import from Mailspring.

Tern is not affiliated with or endorsed by Foundry 376 or the Mailspring project. Credits and
upstream notices are in [NOTICE.md](NOTICE.md).

## Contributing, security, license

- [CONTRIBUTING.md](CONTRIBUTING.md): building, tests, pull requests.
  [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) applies.
- [SECURITY.md](SECURITY.md): report vulnerabilities privately through GitHub, not in an issue.
- Tern is licensed under GPL-3.0-only ([LICENSE.md](LICENSE.md), [NOTICE.md](NOTICE.md)).

The Tern name and logos are trademarks of the Tern project and are not covered by the GPL. A
fork or modified version that you distribute must use its own name and logos; see
[assets/brand/LICENSE.md](assets/brand/LICENSE.md).
