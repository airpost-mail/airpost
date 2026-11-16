# Engine, backend and clients

Decided 2026-10-16. There will be a hosted Airpost service, so the split between the engine and
the backend is a trust boundary, not only a deployment choice.

| | Engine (`apps/engine`) | Backend (`apps/backend`) |
|---|---|---|
| Serves | one person: their accounts on one device (or their own server, headless) | many users: self-hosted for a team or family, or the hosted Airpost service |
| Holds | IMAP/SMTP/OAuth credentials, the mail database, bodies, attachments, search | users, device tokens, plugin metadata, tracking events, shared pages and assets |
| Never holds | other users' data | mail credentials or mail content |
| Reachable from | the local user only (Unix socket, peer uid check); remote clients only through `apps/gateway` | the public internet |

**One hub, many clients.** The engine is the only process that touches mail. Everything else
is a separate program that speaks the engine protocol and
owns only its own surface:

| App | Talks to | Owns |
|---|---|---|
| `apps/desktop` | engine socket | UI, plugins' UI |
| `apps/cli` | engine socket | terminal UX |
| `apps/mcp` | engine socket | MCP over stdio (and streamable HTTP), per-account access policy, audit log |
| `apps/gateway` | engine socket | authenticated remote access (WebSocket/HTTP) for `apps/web` and remote clients |
| `apps/web` | gateway; backend for its own features | browser UI |
| `apps/backend` | nothing on the device | multi-user server features (below) |

No client embeds another one, and no client reads the engine's database directly once the
query API covers it. Supervision is the OS's job: the desktop starts an engine when none is
running; a headless install runs the engine and gateway as launchd/systemd services or in a
compose file (`deploy/server`, one service per component from the `airpost-server` image).
There is no separate orchestrator process.

**Engine:** mail sync, tasks, send later, snooze, reminders, mailbox insights, integrations
(LibreTranslate, DeepL, LanguageTool, People Data Labs, called with the user's own keys),
the query API, and the protocol that the desktop, CLI, web client and plugins use.

**Backend:** metadata sync between a user's devices, open and link tracking (public pixel and
redirect host), thread and page sharing, signature images, expiration claims. It sees thread
and message metadata that plugins write (snooze times, tracking ids), never mail.

**Always-on features** (send later while every device is off, web access from anywhere) come
from the same engine running headless on a server the user controls, behind `apps/gateway`. They
don't come from the backend, which would then need credentials. A hosted mail-access tier,
if Airpost ever offers one, is a separate per-user engine deployment with its own isolation,
never part of the backend process.

**Packaging for self-hosters** who run both: one compose file, or later an option for a
headless engine to embed the backend crate. The code shares only `crates/airpost-wire`, so that
stays possible.

Consequences:
- `apps/web` reaches an engine through `apps/gateway` (local or on the user's server),
  and the backend only for its own features. The backend can relay to an engine behind NAT
  without reading the traffic (end-to-end between client and engine).
- The headless engine owns account settings and credentials. Current choice (2026-10-21):
  the engine's file store (`crates/airpost-secrets`), `secrets.enc` in the profile sealed with
  a key file named by `AIRPOST_ENGINE_KEY_FILE` (or systemd's `LoadCredential`), next to
  `accounts.json`. In `deploy/server` the key is on a volume only the engine's container
  mounts; the gateway and MCP containers share the profile for the socket but can't decrypt
  the credentials. It needs nobody at boot. It keeps the credentials out of a leaked profile
  and out of a client container's reach on disk; it doesn't stop root on the host, or a
  compromised client container acting through the socket, which takes every local
  connection as the engine's owner. A passphrase-unlocked store,
  and per-user keys for several people on one server (`docs/product-model.md` section 6),
  come later. Accounts are added with `airpost account add` (or `--paste` for OAuth from
  another machine's browser) or from an admin-paired browser; `docs/self-hosting.md`, "Airpost
  server".
- The desktop's `mcp-server` plugin (from upstream Mailspring, about 4k lines over
  `DatabaseStore` reads and `Actions.queueTask`) moves to `apps/mcp` on the read API and
  `task.queue`; its preferences pane stays in the desktop as a thin editor of the MCP policy
  file in the profile dir.
