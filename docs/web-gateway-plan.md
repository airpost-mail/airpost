# Web client on a real engine: gateway and engine work

Confirmed by the user 2026-10-16, after an independent review (folded in below). Goal:
`apps/web` runs against a real engine through `apps/gateway` instead of `MockEngine`. In v1
the browser doesn't call `apps/backend` at all (G8). Written against branch
`roadmap/web-ready` (main d4a630c3b). Background: `docs/architecture.md`,
`docs/desktop-client-plan.md` (cited as DCP; its decisions T1-T23, method catalogue section 3,
stages section 7), `docs/engine-protocol-plan.md` (EPP), `docs/backend-plan.md` (BP),
`apps/web/TODO-engine.md`.

Marks: **V** = read or grepped in this tree; **I** = inferred, to check when the work runs.

**Scope until DCP stage 11.** `serve` takes accounts only on its parent's stdin and exits 141
when the stdin closes (`apps/engine/src/serve.rs:8-13`, V). Until stage 11 (`--detached`,
engine-owned credentials) the gateway gives a browser access to a running desktop's engine:
the accounts are the desktop's, and the browser's tabs drop and reconnect whenever the
desktop restarts its engine. A headless, internet-facing server waits for stage 11.

## 0. Decisions

Settled before this plan (DCP 0.1 items 22-23):
- Gateway auth is **device tokens from a local pairing flow**, not backend sign-in. The
  backend stays optional.
- An **admin scope**, granted at pairing, lets a relayed connection call the local-only
  method class (T13), so a headless server can be set up from the web.

Confirmed with this plan:

| # | Decision |
|---|---|
| G1 | `apps/gateway` (binary `tern-gateway`) is a separate process running as the engine's uid. It is a socket client like the CLI (S6 peer-uid check passes), built on `crates/tern-client` (T20) for discovery and types. It never starts the engine; the desktop or a launchd/systemd unit does. |
| G2 | One engine socket connection per WebSocket for browser traffic, so per-connection order, subscription filters and the slow-subscriber drop (EPP 1.4) apply per tab. The gateway also keeps its own non-relayed connection for health and the file routes. |
| G3 | Same origin. The gateway serves the built `apps/web` (`dist/`) at `/` with an SPA fallback, and its API under `/gateway/v1/`. No CORS layer. In development Vite proxies `/gateway` to a local gateway. |
| G4 | The gateway owns device tokens: `<config>/gateway/devices.json` (0600, atomic writes, file lock), holding sha256 of each 256-bit token, name, scopes, created and last-used times. The engine never sees tokens; a relayed connection's `hello` carries `relayed: true`, `scopes` and a device id. |
| G5 | Browsers authenticate with an httpOnly cookie set by the pairing response. No bearer tokens in v1. Every WebSocket upgrade and every non-GET request must carry an `Origin` equal to the public origin, and every request a `Host` matching the public URL (DNS rebinding against a gateway on `127.0.0.1`). |
| G6 | One method-class table in `crates/ms-ipc` (`method_class(name) -> Open | LocalOnly | ParentOnly | Debug`), used by the engine and the gateway. Unknown names are `LocalOnly`. The gateway refuses `LocalOnly` without the admin scope, and `ParentOnly`/`Debug` always; the engine refuses the same on a relayed connection (T13, both sides enforce). `files.*` are `LocalOnly`: browsers use the gateway's HTTP routes. |
| G7 | No built-in TLS. The gateway binds `127.0.0.1` by default and expects a TLS reverse proxy (Caddy, nginx) or a private network (Tailscale) for remote use. Like the backend, it refuses a non-loopback `http` public URL unless given `--insecure-public-url`. |
| G8 | In v1 the browser talks only to the gateway. The engine holds the Tern-server (backend) connection and token (`server.*`, DCP 3.6). Connecting the engine to a backend from the web is a later admin method, `server.login` (to add to DCP 3.6, local-only class), which calls a JSON variant of the backend's login; both are built when that feature is. The backend's CORS and cookie work drops out. |
| G9 | The gateway ships once stage 1 (`tern-client`) and the engine's relay hardening (section 3, G0) are on main. It doesn't wait for `query` (7), credentials (11) or files (16), as DCP's "later" row assumed: v1 web reads use the phase B read API, and the file routes arrive with stage 16. DCP's stage table is amended to say so. |
| G10 | A relayed connection without admin can't reach the file system through tasks. The engine accepts only an allowlist of task classes from it and refuses client-chosen paths; File ids are validated for every caller (section 3). |

## 1. Where the web client's assumptions conflict with the decisions

1. **Sign-in is backend sign-in** (`src/features/auth/session.ts:1-30`, V). The session is a
   backend device token from `POST /tern/v1/login` or a pasted `tern_…` token, re-checked on
   `GET /tern/v1/me`, and sent in `hello.token`. Under G4/G5 the engine connection is
   authorised by a pairing cookie the client can't read, and the backend isn't involved. The
   login screen becomes a pairing screen, and "who am I" comes from `GET /gateway/v1/session`.
2. **`hello.token`** (`src/engine/rpc.ts:80`, `ws.ts:61-66`, V). Removed. Auth happens on the
   upgrade; the gateway rewrites `hello` (2.4).
3. **`VITE_ENGINE_URL`** decides mock vs real at build time and doubles as the server URL
   (`connect.ts:20-28`, `server-url.ts:8-20`, V). The socket is `wss://<page
   origin>/gateway/v1/ws`. A setting `engine: "mock" | "gateway"` replaces it: build default
   `gateway` for the gateway's bundle, `mock` for `npm run dev` without a gateway, overridable
   in dev.
4. **`accounts.list` returning `AccountJSON[]`** (`rpc.ts:239`, V). The engine has
   `account.list` (phase C) returning `{accounts: [{accountId, email, state,
   connectionError, error?}]}` with no settings (`crates/ms-ipc/src/methods.rs:104-119`, V).
   G0 adds an interim allowlisted copy of the account settings; stage 11's registry replaces
   the source, not the shape.
5. **`account.test {account}` with passwords from the browser** (`rpc.ts`, `actions.ts:30`,
   V). An admin-scoped call only, with what the user just typed. Testing a stored account is
   `account.test {accountId}` (stage 12). Stored secrets are never read back (T13).
6. **Preferences, signatures, templates "in the engine or backend"** (`preferences.ts:2`,
   `signatures/store.ts:3`, `templates.ts:3`, V). Engine `settings` (stage 3, T11), never the
   backend. UI-only prefs (density, reading pane, recent searches) stay per browser in v1.
7. **Server-sourced capabilities from the web's own backend session**
   (`capabilities.ts:126`, V). From the engine's `capabilities` (stage 5).
8. **`identity-refresh-needed`** (`session.ts:25`, V). A delta class,
   `ProcessIdentityRefreshNeeded` (`crates/ms-store/src/delta.rs:298-303`, V), relayed like
   any delta. It now means "the engine's Tern server rejected its token"; the web shows it in
   Settings and doesn't sign the browser out.
9. **One origin serving `/tern/v1/*` and the socket** (`session.ts:23`, V). One origin serves
   the web app and `/gateway/v1/*`; the browser doesn't call the backend in v1.

## 2. The gateway

### 2.1 Process model

- `apps/gateway`, binary `tern-gateway`, tokio + axum 0.8 (already in the workspace via
  `apps/backend`, V). axum's `ws` feature is new to the workspace (tokio-tungstenite,
  MIT/Apache). Compression (permessage-deflate) is not in axum's `ws` (I) and is left out of
  v1.
- Finds the engine through `tern-client` discovery (`--config-dir`, `TERN_CONFIG_DIR`, the
  profile default; `run/engine.json`) on every new WebSocket, so an engine restarted by the
  desktop is picked up without restarting the gateway.
- Each WebSocket opens one Unix-socket connection to the engine. The socket half uses
  `tokio::net::UnixStream` with ms-ipc's types (`ms-ipc`'s client is blocking, V; `tern-client`
  doesn't exist yet, V).
- Subcommands of the same binary: `serve` (default), `pair`, `devices list|revoke|rename`.
  `pair` and `devices revoke` write `devices.json` under the lock, then send `reload` on the
  gateway's control socket (`<config>/gateway/control.sock`, 0600, same-uid check) so the
  running gateway picks up the change at once and closes a revoked device's WebSockets. The
  gateway reads the file at start.
- Supervision: launchd LaunchAgent / systemd `--user` unit examples next to the engine's,
  useful once `serve --detached` exists (stage 11).

### 2.2 HTTP routes

| Route | Auth | Purpose |
|---|---|---|
| `GET /`, `/assets/*`, SPA fallback | none | the built web app; immutable caching for hashed assets, `no-cache` for `index.html` |
| `GET /gateway/v1/session` | cookie, else 401 | `{device: {id, name, scopes, pairedAt}, engine: {state: "up"|"down", protocol?, engine?}, gateway: {version}}`; the client's health route (2.6) |
| `POST /gateway/v1/pair` | pairing code + Origin | `{code, deviceName}` → sets the cookie, `{device}`; 401 for a bad or used code, 429 when limited |
| `POST /gateway/v1/unpair` | cookie + Origin | revokes the calling device, clears the cookie |
| `GET /gateway/v1/ws` | cookie + Origin | the JSON-RPC WebSocket (2.4) |
| `GET /gateway/v1/files/{accountId}/{fileId}` | cookie | download, stage 16 (2.5) |
| `POST /gateway/v1/files/{accountId}` | cookie + Origin | upload, stage 16 (2.5) |

Every response: `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`. The app:
CSP `default-src 'self'`, `frame-ancestors 'none'`, `connect-src 'self'`, and `img-src`
allowing `'self'` (rewritten `cid:` images, 2.5) besides what the reader needs today. The
reader's srcdoc iframe has its own CSP with `img-src data: blob:`
(`src/components/email/sanitize.ts:13`, per the review, I) and inherits the app's, so both
gain `'self'`.

### 2.3 Pairing, device tokens and the admin scope

- **Mint a code.** `tern-gateway pair [--admin] [--name <device>] [--ttl 10m]`, run as the
  engine's user (over SSH on a server). It writes a pending entry (sha256 of the code,
  scopes, name, expiry) and prints the code, 12 base32 characters in groups of four (60
  bits), and a URL `<public-url>/pair#<code>`. The fragment keeps the code out of server logs
  and `Referer`; the web clears it with `history.replaceState` once read.
- **Redeem it.** The web's `/pair` route posts `POST /gateway/v1/pair`. The gateway looks up
  the hash, consumes the entry, mints a 256-bit token, stores its sha256 and sets the cookie
  with a 400-day rolling `Max-Age`. The token is never in a response body.
- **Cookie.** `__Host-tern_device; Secure; HttpOnly; SameSite=Strict; Path=/` when the public
  URL is https. On plain http (loopback, or `--insecure-public-url`) browsers drop `Secure`
  cookies off https except on localhost in some browsers (I: Safari), so it is
  `tern_device; HttpOnly; SameSite=Strict; Path=/` without `Secure`, and the gateway logs a
  warning for the non-loopback case.
- **Limits.** Pairing: 5 attempts a minute per client IP (IPv6 per /64, `X-Forwarded-For`
  only from `trusted_proxies`, as the backend does, V `apps/backend/src/http/mod.rs:180-213`).
  A global limit of 30 an hour applies only while a code is pending, and never to loopback
  clients, so strangers can't lock out pairing when nobody is pairing. 60 bits at 5 a
  minute per IP needs no per-code counter. Upgrades: 10 a minute per device. Frames: 50 a
  second per connection, burst 200. At most 8 open WebSockets per device and 64 overall.
- **Scopes.** `admin` or none, granted only at pairing; a device is never elevated later.
  Devices unused for 180 days are dropped; admin devices after 30.
- **Revocation.** `tern-gateway devices revoke <id>`, or Sign out on the device itself.
  Revoking closes that device's WebSockets with 4401. A devices page in the web waits.
- **Bootstrap of a headless server** (from stage 11): `tern-gateway pair --admin` over SSH,
  then accounts are added from the web (stages 11/12). Later: an admin device approving a
  code shown by a new browser, and the desktop's "Open in browser".

### 2.4 WebSocket relay

- The gateway accepts every upgrade that reaches the route and closes at once with the right
  code (4401, 4403, 4429) when the device, Origin or limits fail, because a refused upgrade
  only shows up in the browser as 1006.
- **`hello`.** The first frame must be `hello`; a later `hello` closes the connection with
  4400 (the engine re-runs `hello` at any point, `apps/engine/src/socket.rs:811-814`, V).
  The gateway forwards it with `client` prefixed (`tern-gateway/<v> (tern-web/0.1.0)`),
  drops any `relayed`, `scopes` or `device` the browser sent, and adds its own. The engine's
  result must list the `relayed` feature (section 3); without it the gateway closes with 4502
  and logs a version mismatch, since an older engine would ignore `relayed` (serde drops
  unknown fields). It adds `gateway: {version, scopes}` to the result.
- **Policy.** Each frame from the browser is parsed: one JSON-RPC request object, with or
  without `id` (no batches), at most 8 MiB, a method the class table allows for this
  device. Refusals are answered with `-32008` and never reach the engine. Allowed requests
  are **re-encoded** with ms-ipc's encoder before they're written: the engine splits on
  `\n` (`crates/ms-ipc/src/frame.rs:56`, V), and valid JSON may carry raw newlines between
  tokens, so forwarding the browser's bytes would let a second request ride inside the
  first.
- **Backpressure.** Each WebSocket has a bounded outgoing queue (by bytes). When it is full
  the gateway stops reading that engine connection, so the engine's own slow-subscriber
  drop (EPP 1.4) applies and the gateway's memory stays bounded. The web subscribes only to
  the classes it renders.
- **Logs.** Method name (known names only, anything else as `unknown(<len>)`), device id,
  response code and duration; connection open and close with the close code. Never params,
  results, cookies, pairing codes or query strings.
- **Engine drops.** When the engine connection ends, the gateway closes the WebSocket with
  4502, or 4008 when the engine sent `disconnect` for a slow subscriber. It doesn't reconnect
  behind the browser's back.

### 2.5 Files (with stage 16)

- **Download.** The gateway calls `files.get {accountId, fileId}` (DCP 3.2) on its own
  connection; the engine fetches the part when missing and answers `{path, size,
  contentType}`. The gateway checks that the path is inside the engine's files directory and
  streams it (`Range` supported), with `Content-Disposition: attachment` and
  `Content-Security-Policy: sandbox`. With `?inline=1` it serves inline only for an
  allowlist of raster image types (png, jpeg, gif, webp), sniffed from the bytes, with the
  `Content-Type` it chose itself, never the sender's (which can be svg or html). The cookie
  authorises `<img>` and download links.
- **Upload.** `POST /gateway/v1/files/{accountId}` streams the body (25 MiB cap) to a temp
  file in `<config>/gateway/incoming/`, calls `files.put {accountId, name, contentType,
  path}` → `{fileId}`, and deletes the temp file. A per-device quota (1 GiB of uploads not
  yet attached to a sent draft, I: how to tell) and removal of temp files older than a day.
- **Inline images.** The web rewrites `cid:` references to the download route with
  `?inline=1`.
- **Forwarding.** `SendDraftTask` attaches files by id from the files directory
  (`crates/ms-tasks/src/send.rs:72-82,162`, V), so a forward that re-attaches the original's
  File records works once they are downloaded. The web calls the download route's HEAD (which
  triggers the fetch) for each before queueing the send, until `draft.create {kind:
  "forward"}` (stage 13).

### 2.6 Failures and the reconnect contract

| Code | Meaning | Client |
|---|---|---|
| 1000 | client closed | — |
| 1001, 1006, 1011, 1012 | gateway going away, network drop, gateway error or restart | reconnect with backoff |
| 4008 | the engine dropped this subscriber for falling behind | reconnect with backoff (no immediate retry), then re-read |
| 4400 | protocol error (bad frame, `hello` not first or repeated, frame too big) | stop; log; bug |
| 4401 | device unknown or revoked | stop; go to `/pair` with "This browser was unpaired" |
| 4403 | Origin or Host not allowed | stop; bug or misconfigured proxy |
| 4429 | too many connections or frames | reconnect with backoff starting at 5 s |
| 4502 | engine unreachable, gone, or too old for the gateway | reconnect with backoff; "Couldn't reach the engine" |

Health: before each connect the client calls `GET /gateway/v1/session`. A network error is
"Couldn't reach the server"; 401 is unpaired; 200 with `engine.state: "down"` is "The engine
isn't running", and no WebSocket is opened until it's up (polled with the same backoff).

Client contract (`WsEngineClient`), matching DCP 4.1 step 2:
- Backoff 0.5, 1, 2, 4, 8, 15, 30 s (cap), ±20% jitter, reset after 30 s connected; state
  `reconnecting` while waiting.
- On reconnect: `hello`, `subscribe`, then invalidate every query and re-seed process state.
- A call made while reconnecting waits up to 10 s, then rejects with `EngineUnavailable`.
  Calls in flight when the socket drops reject at once. `task.queue` is not retried (I:
  whether the engine dedupes a re-queued task id).
- Tabs reconnect independently; a revoke reaches every tab through 4401 or `/session`.

### 2.7 Packaging (deployment scenario 3, `docs/deployment-plan.md` 4 and 7)

- One `tern-headless` image with `mailsync`, `tern`, `tern-mcp` and `tern-gateway`; the
  engine and the gateway run as two compose services from it, as the same non-root uid, so
  the engine's peer-uid check passes. They share the whole profile volume, not only the
  socket: the gateway finds the engine through `<config>/run/engine.json` and keeps its own
  state in `<config>/gateway/` (`devices.json`, `control.sock`, `gateway.json`). Both get
  `TERN_CONFIG_DIR` pointing at it.
- `apps/web/dist` is baked into the image and served with `TERN_GATEWAY_WEB_ROOT`, so web and
  gateway always ship the same version.
- Health: `GET /healthz` (no auth, no Host check, says nothing about the engine) and
  `tern-gateway healthcheck` (connects to `TERN_GATEWAY_BIND`, 0.0.0.0 mapped to loopback),
  for an image without curl. A down engine doesn't fail the gateway's healthcheck.
- Pairing: `docker compose exec gateway tern-gateway pair [--admin]`; it reaches the running
  gateway over the control socket in the shared volume.
- Environment: `TERN_CONFIG_DIR`, `TERN_GATEWAY_BIND` (default `127.0.0.1:5180`; a container
  sets `0.0.0.0:5180`), `TERN_GATEWAY_PUBLIC_URL` (required off loopback),
  `TERN_GATEWAY_INSECURE_PUBLIC_URL`, `TERN_GATEWAY_WEB_ROOT`, `TERN_GATEWAY_ALLOW_ORIGINS`,
  and `TERN_TRUSTED_PROXIES` (the backend's name; default `127.0.0.0/8,::1`).
- Hostnames: the backend can be public and the gateway VPN-only. The browser never calls the
  backend (G8), so the backend needs no web origin in its CORS list. On a VPN, give the
  gateway an https name (e.g. `tailscale cert`) rather than plain http, which needs
  `TERN_GATEWAY_INSECURE_PUBLIC_URL` and sends the cookie without `Secure`.
- Until DCP stage 11 there is no headless engine (scope note at the top), so the image's
  engine service waits for `serve --detached` and engine-owned credentials.

## 3. Engine work for the gateway (G0)

After stage 1 is on main, since it touches `crates/ms-ipc`, `crates/ms-store`,
`crates/ms-tasks` and the socket code:
- **Relayed hello.** `HelloParams` gains `relayed: bool`, `scopes: [string]`, `device: {id,
  name}?`. The first `hello` fixes them for the connection; a later `hello` can't change them.
  `hello` gains `features: ["relayed"]` (with stage 1's `hello.features`). `engine.log`
  records the device on connect.
- **Method classes** (G6) for every current method; the socket refuses `LocalOnly` on a
  relayed connection without `admin` with new code `-32008`.
- **Task policy** (G10). On a relayed connection without `admin`, `task.queue` accepts only
  the classes the web uses: `ChangeUnreadTask`, `ChangeStarredTask`, `ChangeFolderTask`,
  `ChangeLabelsTask`, `SyncbackDraftTask`, `SendDraftTask`, `DestroyDraftTask`,
  `SyncbackMetadataTask`, `ExpungeAllInFolderTask`, `SyncbackCategoryTask`,
  `DestroyCategoryTask` (`apps/web/src/engine/tasks.ts`, V), and later the event and contact
  tasks once they are reviewed. `GetMessageRFC2822Task` (writes to a client `filepath`,
  `crates/ms-tasks/src/export.rs:90`, V) and `GetManyRFC2822Task` (client `outputDir`, `:149`,
  V) are refused; stage 16's file-id mode serves the web instead.
- **File ids.** `File::dir_in` joins the id onto the files root unchecked
  (`crates/ms-store/src/models/file.rs:136-141`, V), and `Path::join` with an absolute
  segment replaces the root, so a draft's `files[].id` of `/Users/me/.ssh` would attach a
  file from outside it. Every File id from a client is validated (I: the id alphabet; likely
  `[A-Za-z0-9_-]`), and `dir_in` refuses anything else, for every caller, relayed or not.
- **Backlog.** A relayed connection never takes the first-subscriber backlog
  (`socket.rs` `takes_backlog`, per the review), so a browser tab can't take the desktop's
  start-up deltas after an engine restart.
- **Connection cap** of 128 (`SOCKET_MAX_CONNECTIONS`; the gateway keeps at most 64 for
  browsers), refused with `-32009`.
- **Interim `account.list` field `account`**: the Account JSON the engine received in
  `account.add`, copied through an allowlist of fields (id, name, emailAddress, provider,
  label, aliases, defaultAlias, color, IMAP/SMTP host, port, username and security), never
  a denylist.
- Tests: `tests/socket.rs` for hello with and without `relayed`, a second hello, the method
  refusal, each refused task class and path, a File id with `/` or `..` in a `SendDraftTask`,
  the backlog rule, the cap, the redaction (a secret-shaped fixture never appears).

~600 Rust.

## 4. Every TODO(engine) item mapped

Destinations: **DCP n** = desktop-client-plan stage n; **G0** = section 3; **GW** = the
gateway (section 2); **Web** = web-only.

### Transport and auth

| Item | Destination |
|---|---|
| `ws.ts:2` HTTP/WebSocket server | GW |
| `ws.ts:58`, `rpc.ts:80` auth scheme | GW (cookie on upgrade, G5); Web removes `hello.token` |
| `ws.ts:133` unreachable vs auth rejected | GW (`/session`, close codes 2.6); Web |
| `ws.ts:141` reconnect | Web (2.6 contract); GW close codes |
| `provider.tsx:22` `account.state` banners | exists (phase C), relayed by GW; Web |
| `session.ts:23` one origin, same token | GW (G3, G4); Web replaces backend sign-in with pairing (1.1) |
| `session.ts:25` identity-refresh-needed | relayed delta (1.8); Web shows it in Settings |
| `session.ts:72` httpOnly cookie | GW (G5), for the device token |
| `transport.ts:115` CORS | not needed (G3, G8) |
| `transport.ts:160` JSON login | not in v1 (G8): built with `server.login` |

### Read API

| Item | Destination |
|---|---|
| `rpc.ts:239` `accounts.list` | `account.list` (exists) + G0's `account` field; DCP 11 registry later |
| `hooks.ts:192` starred/unread filters | DCP 7 `query` (`where` on `starred`/`unread`) |
| `model.ts:104` sort key | DCP 7 `query` `order` |
| `use-unified-threads.ts:20` unified paging | DCP 7 `query` with `accounts: null` |
| `senders.ts:3` message count, per-message senders | DCP 7: a Thread `include` with a message summary, new to DCP 3.2 (I: which fields the desktop row reads) |
| `hooks.ts:389` `search` across accounts | DCP 9 (`search` gains `accounts`) or `query.search` |
| `hooks.ts:416` `contacts.search` across accounts | DCP 7 `query` on Contact with `search {grammar: false}` |
| `query.ts:172`, `mock/search.ts:3`, `SearchView.tsx:225` operators and dates | DCP 9 |
| `cache.ts:218` calendars, events, contact groups | DCP 7 `query` classes; views Web; writes DCP 18 |
| `mock/engine.ts:342` calendar fixtures | Web (mock fixtures) |

### Tasks

| Item | Destination |
|---|---|
| `mock/apply-task.ts:3` mock stands in for ms-tasks | Web: keep for the mock and tests, drop the "delete" wording |
| `tasks.ts:98` no local phase for expunge/category tasks | Web (already waits for deltas) |
| `apply-task.ts:142` RFC2822, RSVP, contact tasks in the mock | Web (mock); RFC2822 via DCP 16's file-id mode; RSVP and contacts DCP 18 |
| `mock/engine.ts:314` cancel semantics | Web (document) |
| `useComposerSession.ts:205` undo-send survives a closed tab | DCP 4: the web writes send-later `{expiration, isUndoSend}` metadata and the executor fires it; DCP 13's `draft.send {delayMs}` later |
| `send-later-runner.ts:3` sends with no tab open | DCP 4 executor (runner gated on `expiration-executor`); with no desktop running, DCP 11 |

### Accounts

| Item | Destination |
|---|---|
| `mock/engine.ts:381` `account.add` | DCP 11/12, admin scope |
| `mock/engine.ts:397` `account.test` | DCP 12, admin scope |
| `actions.ts:30` test with stored credentials | DCP 12: `account.test {accountId}` (add to DCP 3.6) |
| `actions.ts:47` reset streams the resync | DCP 12 (`account.reset` on the socket), admin scope; Web invalidates that account's queries on `account.state` `starting` → `running` |
| `account-draft.ts:108` engine assigns the id | DCP 11/12 |
| `presets.ts:25` OAuth | deferred: app passwords; later a fixed redirect page on a Tern domain (T14, DCP 0.1 item 18) |
| `presets.ts:110` `account.discover` | DCP 12 |

### Attachments

| Item | Destination |
|---|---|
| `client.ts:28`, `ws.ts:116`, `AttachmentList.tsx:30` download | DCP 16 + GW route (2.5) |
| `useComposerSession.ts:132` upload | DCP 16 + GW route |
| `draft.ts:193` forward re-attaches by id | works once downloaded (2.5); DCP 13 later |

### Integrations

| Item | Destination |
|---|---|
| `rpc.ts:243`, `mock/engine.ts:430` shapes | DCP 5 (`docs/integrations-plan.md` section 4) |
| `useIntegrationTest.ts:10` `integrations.test` result | DCP 5 |
| `integrations/store.ts:61` keys stored by the engine | DCP 11/12 `secrets.set`, admin scope; per request (DCP 5) until then |
| `capabilities.ts:126` Tern server state | DCP 5 `capabilities` |

### Preferences

| Item | Destination |
|---|---|
| `preferences.ts:2`, `GeneralSettings.tsx:16` UI prefs | per browser in v1; `core.sending.undoSend` from DCP 3 |
| `signatures/store.ts:3` | DCP 3 |
| `templates.ts:3` | DCP 3 |
| `search/recent.ts:2` | per browser in v1 |

## 5. Web work

- **Engine setting** (1.3) and the WebSocket URL from the page origin. The mock stays the
  default for `npm run dev` without a gateway and for tests.
- **Pairing instead of sign-in**: `/pair` (code from the fragment, then `replaceState`, or
  typed), the session store on `/gateway/v1/session`, Sign out → `POST
  /gateway/v1/unpair`. The backend login form, the token paste and `transport.ts`'s HTML
  scraping go.
- **`WsEngineClient`**: no `hello.token`; reconnect contract and close codes (2.6);
  `fileUrl` on the files route; `account.list`; subscribe to the rendered classes only.
- **Reader**: `cid:` rewrite and the CSPs (2.2, 2.5).
- Each later engine piece switches its hook from the workaround to the engine method behind
  `hello.features`, removes its `TODO(engine)` markers and updates `TODO-engine.md`.
- Tests: the reconnect contract against a fake WebSocket server (vitest), pairing on a mock
  gateway, and the end-to-end run (section 6).

## 6. Tests

- Gateway unit tests: pairing (expiry, one-shot, rate limits, the pending-only global limit),
  cookie flags per public URL, Origin and Host checks, the method policy with and without
  admin, a second `hello`, newline smuggling (a frame with raw newlines between tokens
  reaches the engine as one line), frame limits, log redaction (a session with secrets in
  params leaves none in the log), the bounded queue.
- Gateway integration: a harness scenario (`MAILSYNC_TRANSPORT=serve` engine on the fake
  IMAP/SMTP server) plus `tern-gateway serve`, driven by a Python WebSocket client: pair,
  `hello`, `threads.list`, a `task.queue` with its Task delta, refused local-only calls and
  task classes, revoke closing with 4401, engine kill closing with 4502 and the reconnect, a
  download and upload (with stage 16).
- Web: vitest for the client contract; a Playwright smoke run against the harness gateway
  later (I: whether the repo wants a browser in CI).
- The gate adds `cargo build --release -p tern-gateway` and the gateway scenarios to the
  quick tier.

## 7. Order and sizes

| Step | Work | Waits for | Size |
|---|---|---|---|
| W1 | Web: engine setting, pairing screen and session on a mock gateway, reconnect contract and close codes, `account.list`, Vite proxy | — | ~700 TS |
| W2 | Gateway without the engine: HTTP server, static app, devices store, `pair`/`devices`, control socket, cookie auth, Origin/Host checks, `/session` (engine reported down), limits, logs. New crate. | — | ~1.2k Rust |
| W3 | G0 in the engine (section 3) | stage 1 on main | ~600 Rust |
| W4 | Gateway relay: discovery, WebSocket relay, policy, re-encoding, backpressure, close codes, harness scenarios, unit files, `docs/self-hosting.md` section; the gateway's bundle defaults to `gateway` | W2, W3 | ~900 Rust, ~150 Python |
| W5 | DCP 16 files + gateway file routes + web download, upload, inline images, CSPs | W4 | ~400 + 350 Rust, ~200 TS |
| W6 | DCP 7 `query` + parity tests; web lists, unified, starred/unread, sort, contacts across accounts, calendar read views | W5 | ~2k Rust, ~400 TS |
| W7 | DCP 9 search grammar; web drops client-side date filters and the mock-only grammar | W6 | ~1.3k Rust, −200 TS |
| after DCP 6 | DCP 3, 4, 5, 13, and the web's switch to each | stage 6 | per DCP 7 |
| after DCP 10 | DCP 11/12: headless, accounts from the web with the admin scope | DCP 10 | per DCP 7 |

W1 and W2 run in parallel now. After W5 the web runs against a real (desktop-owned) engine
for reading, triage, compose and attachments; W6 and W7 remove its client-side workarounds.

**Stages this plan takes on:** G0, the gateway, 16, 7 and 9 (engine halves, the CLI and
tern-mcp parts, and the desktop's `Query.toJSON()` capture for 7's parity tests; stage 8's
transport switch stays with the desktop work). Offered next once stage 6 is on main: 3 and
4. Not taken: 6, 10, 11, 12, 13 and the desktop halves of 8 and 9.

## 8. Answers (2026-10-16)

The user agreed to the recommended answers; the review's amendments are folded in.

1. Browser talks only to the gateway (G8). `server.login` goes in DCP 3.6 as local-only.
2. Device tokens in the gateway (G4).
3. Pairing by `tern-gateway pair [--admin]`, 10-minute one-time code, `/pair#code`; limits
   as 2.3.
4. Same origin, no CORS, no built-in TLS; cookie flags per public URL (2.3).
5. Gateway after stage 1 + G0, before 7, 11, 16; works on a desktop-owned engine until
   stage 11 (scope note at the top).
6. Stages G0, gateway, 16, 7, 9; 3 and 4 offered after stage 6.
7. The gateway's HTTP and pairing half (W2) starts before stage 1 lands.
8. Deferred: UI prefs stay per browser in v1. When a `ui.<client>.*` settings namespace
   comes, it caps key count and size, since devices without admin can write it.
9. OAuth from the web deferred; app passwords until a fixed redirect page exists.
10. Admin only at pairing; idle expiry 180 days, 30 for admin devices.
11. No second factor on admin calls in v1, given G10.

Cut from v1 after the review: bearer tokens, the backend JSON login, a devices page in the
web, a per-code attempt counter, the `ui.<client>.*` namespace.

## Status

2026-10-16: draft written, reviewed independently (three high findings: file-system access
through `task.queue`, newline smuggling past the policy, a second `hello`; all checked
against the code and fixed in the design), confirmed by the user. W1 and W2 starting.

2026-10-16, rebased on main with DCP stages 1 and 2 (4597493fa). Done, not merged:
- W1 (web): pairing instead of sign-in, the engine setting, the reconnect contract and close
  codes, `account.list`, subscription to rendered classes, a plain message for `-32008`.
- W2 (gateway HTTP): devices store, `pair` / `devices` / `healthcheck`, control socket,
  cookie, Host and Origin checks, limits, static app with CSP, `/healthz` (packaging, 2.7).
- W3 (G0): `hello.relayed` / `scopes` / `device` fixed by the first `hello`, `feature`
  `relayed`, `ms_ipc::policy` (method classes, task allowlist, account allowlist), `-32008`,
  no backlog for relayed subscribers, the connection cap, `account` in `account.list`, and
  `File::dir_in` mapping every character outside `[a-z0-9_-]` to `_`.
- W4 (relay): discovery of the `serve` engine, the relayed `hello` (refused when the engine
  lacks `relayed`), parse, policy and re-encode per frame, frame limits, revoke and shutdown
  closing open sockets, engine health in `/session`, `apps/gateway/README.md`. The backend
  image copies `apps/gateway`, now a workspace member.
- Tests: gateway unit and HTTP tests; `tests/relay.rs` against a real `serve` engine (hello,
  `account.list` without secrets, refused method and task, newline smuggling, second
  `hello`, revoke 4401, engine exit 4502, `/session` up and down); engine `tests/socket.rs`
  (relayed policy, admin scope, backlog, cap); `dir_in` unit test.

Open:
- No browser run yet: the web client has only met the gateway through its unit tests and
  the protocol-level relay test. A trial needs a desktop on the `serve` engine.
- The harness has no gateway scenario on the fake IMAP server (the relay test uses an
  account whose servers refuse connections); `test_gateway.py` with a small stdlib
  WebSocket client is the next test to write.
- No compression on the WebSocket (axum's `ws` has none); initial-sync deltas over a WAN are
  unmeasured.
- W5 (files, DCP 16) next; then W6/W7 (DCP 7 and 9). DCP stage 6 will need one more rebase.

2026-10-18, W6 (DCP 7 `query`), branch `roadmap/web-ready`, not merged:
- Engine: `query` in the DCP 3.2 shape on `ReadStore` (`crates/ms-store/src/model_query.rs`),
  plain search only (`grammar: true` is `-32602` with `data.unsupported: "search-grammar"`),
  keyset `cursor` paging, `hello.features: "query"`, `query` in the relayed Open class. The
  allowlist is generated from the desktop's `queryable` attributes. Parity: about 60 queries
  captured from desktop call sites agree row for row with `Query.sql()` (see DCP Status).
- Web: thread lists read with `query` when the engine announces it: Starred and unread
  filtered by the engine, lists sorted by last received (Sent by last sent), unified views
  paged by the engine in one list (`ThreadQuery.accountId: '*'`), contact autocomplete in
  one read across accounts, and a read-only two-week calendar agenda (`/calendar`). Engines
  without `query` keep the old paths. The mock implements `query` and has calendar fixtures;
  `MockEngine({features: []})` stands in for an older engine.
- Open: the row's message count and senders (`senders.ts`) need fields on Thread JSON, which
  no DCP stage has; recurring events show as stored (one master), expansion is DCP 18; a
  `query` without `limit` is unbounded, as on the desktop, including for relayed devices.
