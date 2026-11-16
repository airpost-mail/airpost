# Airpost backend

This is the brief for `apps/backend`, the self-hostable server behind the engine's
`MetadataBackend` trait and the desktop's server features. It maps every Mailspring
server dependency to a treatment, lists the decisions still open, and gives the architecture,
the protocol the engine and client expect, and a work breakdown sized for a multi-agent
workflow.

The Mailspring server is closed source. Every server behaviour here is reconstructed from the
client (`apps/desktop`), the C++ engine (upstream Mailspring-Sync's `MailSync/`) and the Rust
engine. Desktop paths are relative to `apps/desktop/`.

## 0. Inherited decisions, and the ones this plan changes

| # | Decision | Consequence for the backend |
|---|---|---|
| D2 | Every Pro feature stays, with no sign-in requirement | Snooze, send-later and reminders must work with no backend at all. The backend adds multi-device sync, tracking and sharing; it never gates a feature. |
| D3 | Metadata is local only; multi-device sync is out of scope; remote sync sits behind `MetadataBackend`; nothing calls `*.getmailspring.com` | **Changed.** Multi-device metadata sync is the core of this plan. `LocalOnly` stays the default when no server is configured. D3 and section 3 of the engine plan were updated in B1-E (O7). |
| D10, D12 | One monorepo, one Cargo workspace; the backend reuses engine crates (ids, metadata models) | **Changed.** The backend reuses no engine crate: it has its own schema and never computes ids (3.1). It joined the root workspace in B1-E; D12 in the engine plan was updated then. |
| D11 | Everything ships under GPL-3.0-only | Holds for the backend and the wire crate too (O6, changed 2026-10-16). They are written from scratch, so they could move to MIT later by a deliberate decision. |
| — | Mailspring ID removed from the desktop (4120f4351, 27f713bf6): no identity store, no sign-in, engines get a `null` identity line | **Changed (2026-10-16).** No Mailspring ID compatibility layer. Airpost-native login and an Airpost server object on the engine's identity line (3.4, 4.1). Only the metadata protocol stays Mailspring-compatible, for the C++ oracle. |

## 1. Scope

Every inventory item, with its treatment. **Backend**: the server implements it. **Client**:
replaced or removed in the desktop or engine, no server needed. **Dropped**: removed with no
replacement. **Later**: wanted, not in v1.

| Item | Treatment | Why |
|---|---|---|
| Plugin identity gate, Pro checks and upsell, login-link rewriting | Done (client) | Removed with Mailspring ID (4120f4351, 27f713bf6). Server features are disabled with a tooltip until a server is connected (`docs/pro-features-backend.md`). |
| Sign-in (was the Mailspring ID webview and `/api/me`) | Backend + client | Airpost login against a user-configured server URL, one device token per install (3.4). `GET /airpost/v1/me` replaces `/api/me`. |
| Feature quotas, `/api/feature_usage_event`, `/api/login-link`, `/payment`, `/dashboard` | Dropped | No plans or quotas; the desktop no longer queues `SendFeatureUsageEventTask`. |
| Open tracking | Backend, new Airpost URL format | Needs a public host that writes events into metadata. Mailspring's `/o/<token>.png` pixels point at `link.getmailspring.com` and can never reach an Airpost server, so the format is replaced (3.6). |
| Link tracking | Backend, new Airpost URL format | Same, plus a safe redirect. |
| Metadata push, pull, delta stream | Backend | The core of the server. v1 carries message metadata (tracking state) and, with the thread anchor in B1-E, thread metadata (snooze, reminders). Send-later never syncs: it lives on drafts, which are local-only (O1). |
| `identity-refresh-needed` backend event | Backend | Sent when the user's profile or the server feature set changes, so the client re-fetches `/airpost/v1/me`. |
| Thread sharing (`/api/save-public-asset`, public viewer) | Backend, phase B2 | Needs upload, public storage and a viewer page. Disabled with a tooltip today; B2-S enables it when `features.sharing` is on. |
| Activity report sharing (`/api/share-static-page`) | Backend, phase B2 | Same storage as sharing. |
| Signature photo upload | Backend, phase B2 | Same storage. The URL is baked into delivered mail, so it must stay up. |
| Company logo (`logo.getmailspring.com`) | Dropped | No free logo service to proxy. The client removes the option; existing URL entry still works. |
| Signature social icons (`www.getmailspring.com/signature-assets/*.gif`) | Backend static files, phase B2; client fallback | Data URIs are blocked by many mail clients, so the icons need a host. Without a server the client renders text links. The GIFs need Airpost-owned replacements. |
| Translation (`/api/translate`) | Client, in B1-C | Disabled today. Enable it on a user-configured LibreTranslate URL (`{q, source:"auto", target, format:"html"}` → `{translatedText}`), with the adapter in `translation/lib/service.ts`. |
| Contact enrichment (`/api/info-for-email-v2`) | Dropped | No free data source; the plugin makes no lookups (27f713bf6). |
| Grammar check (`/api/grammar/check`) | Client, in B1-C | Disabled today. The response is already LanguageTool's `/v2/check`; call a user-configured LanguageTool URL directly (form-encoded `text`, `language`). |
| Newsletter opt-in | Dropped | |
| Gmail / O365 OAuth | Client | Airpost's own Google and Azure app registrations; the loopback server shows its own "finished" page instead of redirecting to `id.getmailspring.com/oauth/finished`. |
| Auto-update (`updates.getmailspring.com`) | Client now; Airpost feed later | Disable first: an Airpost build can install Mailspring over itself today. A later feed can come from GitHub releases and is not this server's job. |
| Native crash reporter (`id.getmailspring.com/report-crash`) | Client | Disable. |
| Sentry DSN (Mailspring's project) | Client | Disable, or an Airpost DSN configured at build time. |
| Engine identity input (`--identity`, stdin) | Engine | Carries the Airpost server object that selects the HTTP backend in the Rust engine (4.1). C++ keeps getting `null` (O7). |
| `/api/resolve-dav-hosts` | Done in the engine | Rust resolves SRV locally. C++ only calls it with an identity (harness oracle runs), and reads the backend's default 404 as "no DAV, skip". |
| `/ping` | Backend | One line; also the container health check. |
| `@getmailspring.com` Message-IDs (`src/flux/stores/draft-factory.ts:66`) | Client | Use the sender's domain. |
| "Sent from Mailspring" signature, help links, `remove-tracking-pixels` host patterns | Client | Branding. |
| Snooze, reminders | Client (gate) + backend (mirror) | They fire locally from `metadata-expiration` deltas. The backend carries their metadata between devices and arbitrates which device acts on an expiration (3.8). |
| Send later | Client only | Fires locally from the draft's metadata. Not synced: `draft-store.ts:620-630` queues an undo-only task on purpose, and `SyncbackDraftTask` has no remote side. |

What v1 of the backend is, given the recommendations in section 2: accounts and device tokens,
the metadata protocol with the Airpost additions (backfill, claims, the thread anchor echo), Airpost
open and link tracking, the Airpost login and `/airpost/v1/me` (3.4), and `/ping`. Multi-device
in v1 means tracking state and, once B1-E's thread anchor lands, snooze and reminders; it never
covers send-later. Sharing and asset hosting follow in B2.

## 2. Open decisions

| # | Question | Recommendation | Reason |
|---|---|---|---|
| O1 | Features in v1 | Metadata sync and Airpost open/link tracking in v1, with the thread anchor (4.1) in B1-E so snooze and reminders sync. Thread sharing, activity sharing and signature assets in B2, same release if they are ready. Translation, grammar and enrichment never in the backend. Send-later stays single-device. | Without the anchor, thread metadata parks in `DetatchedPluginMetadata` on the second device for most threads (7), and v1 multi-device would only carry tracking state, which only the sending device writes anyway. The anchor is wire-compatible and small. Sharing needs storage, a viewer and HTML sanitizing, which roughly doubles the surface. |
| O2 | Wire compatibility with the Mailspring ID API, or a new API | **Decided (2026-10-16).** Compatible for the metadata protocol only: same paths, Basic auth, bodies and stream framing. Login, server info and every other Airpost endpoint live under `/airpost/v1/`; tracking uses a new URL format (3.6). | The C++ engine speaks the metadata protocol, so compatibility there keeps it as the backend's oracle in the harness. The desktop no longer has a Mailspring ID client, so identity compatibility buys nothing. |
| O3 | Auth and identity | **Decided (2026-10-16).** Multi-user data model; single-admin by default. Users are created with `airpost-backend user add`; open signup is a config switch, off by default. The user enters a server URL in Airpost; the desktop opens the server's login page in its own window, and the result page returns one revocable token per device in its body (3.4). The token lives in the keyset; the engine gets it in the Airpost server object on stdin (4.1). | Reuses B0's form, rate limits and token minting. The token never appears in a URL, a redirect or a log. A device-code flow needs a new table, polling and a code page; it is the later path for a CLI and fits the same tokens. |
| O4 | Storage | SQLite (rusqlite `bundled`, WAL) in v1, behind a `Repo` trait. Postgres only if someone runs a large shared instance. | One file to back up, same crate the engine uses. sqlx's SQLite driver would pull a second `libsqlite3-sys` and collide with rusqlite on `links = "sqlite3"`; a later Postgres backend should use `tokio-postgres`. |
| O5 | Deployment | One binary, `airpost-backend serve`, plain HTTP on `127.0.0.1:5101` behind a TLS reverse proxy (Caddy or similar), plus a Docker image with `/data` as the only volume and `AIRPOST_BIND=0.0.0.0:5101`. | No TLS stack to configure in-process, and no chance of pulling aws-lc-rs. |
| O6 | Licence | **Decided (2026-10-16): GPL-3.0-only** for `apps/backend` and `crates/airpost-wire`, like the rest of the repo; this replaces the MIT decision of 2026-10-15. Dependencies must be GPL-3.0-compatible. The backend still depends on no engine crate (3.1). No `/airpost/v1/source` endpoint: GPLv3 has no network-use clause. | The user chose one licence for the whole repo until a part is deliberately moved. Both crates are written from scratch and depend on no engine crate, so a later move to MIT stays possible while all their code is ours. |
| O7 | Engine switch for the HTTP backend | **Decided (2026-10-16).** Rust: the HTTP backend when the identity line is a valid Airpost server object (4.1); `null` or anything else stays `LocalOnly`. `IDENTITY_SERVER` is not read. Desktop: the server object only when the engine is `rust`; C++ always gets `null`. C++ stays a test oracle, driven by the harness with a Mailspring-shaped identity. | C++ `abort()`s on a revoked token, and the crash handler then marks the mail account `SYNC_STATE_AUTH_FAILED`; it also keeps the head-before-pages bug (7) and has none of the Airpost additions. The harness keeps `--identity null` by default, so existing scenarios stay local. This changes plan section 3 ("accepted and ignored"); update it when B1-E lands. |
| O8 | Public URL for tracking and shares | One `public_url` in server config, returned as `publicURL` by `GET /airpost/v1/server` and `/airpost/v1/me` (3.4). The client uses it for pixels, redirects and share links, falling back to the server URL. | Tracking and shares only work if recipients can reach the host; the API host may sit on a private network. |

## 3. Architecture

### 3.1 Crates and dependencies

| Crate | Role | Depends on |
|---|---|---|
| `apps/backend` (`airpost-backend`, GPL-3.0-only per O6) | Bin + lib: HTTP server, repo, auth, tracking, sharing, CLI. A member of the root workspace since B1-E. `airpost_backend::wire` re-exports `airpost_wire`. | `airpost-wire`; no engine crate, not even as a dev-dependency |
| `crates/airpost-wire` (GPL-3.0-only per O6) | Serde types for the wire protocol: metadata record, push body, stream line (`metadata` / `backend-event`), head response, backfill and claim bodies, the login result, `/airpost/v1/server` and `/airpost/v1/me` bodies and the engine's Airpost server object with its URL check (3.4, 4.1), the constants (heartbeat 5 s × 16 bytes, page size 500, low-speed 30 s, `v` max `u32::MAX`). | serde, serde_json |
| `airpost-net` | `HttpMetadataBackend` and its `Outbox` (4.1) on `airpost-wire`. | |

The backend does not compute message ids and has its own schema, so it depends on neither
`airpost-store` nor `airpost-core`; converting a wire record to `airpost_core::Metadata` is one function in
airpost-net. The point of `airpost-wire` is that the engine and server cannot drift on the wire format,
as `packages/` does for client protocol types.

The backend started in its own workspace so it could not break the engine workflow's cargo
runs; B1-E joined it to the root (`members = ["apps/engine", "apps/backend", "crates/*"]`, one
`Cargo.lock`, shared versions through `workspace = true`). Licence checks: the root `deny.toml`
covers the whole workspace with the GPL policy; `cargo deny --manifest-path
apps/backend/Cargo.toml check` checks airpost-backend's normal dependencies alone against
`apps/backend/deny.toml` (the same GPLv3-compatible licences, no TLS stack or HTTP client, no
engine crate). The Docker image builds from the repo root (`docker build -f
apps/backend/Dockerfile .`).

Backend dependencies (all permissive; confirm current versions and run `cargo deny` in
B0):

| Concern | Crate |
|---|---|
| HTTP server | `axum` on `tokio` + `hyper` |
| Middleware | `tower`, `tower-http` (body limits, trace, timeout, CORS) |
| Storage | `rusqlite` (same version as the workspace), one writer connection + a small reader pool, run via `spawn_blocking` |
| Passwords | `argon2` (RustCrypto) |
| Tokens and hashes | `rand`, `sha2`, `base64` |
| Rate limiting | `governor` behind a small tower layer |
| Multipart | axum's `multipart` feature |
| HTML templates | `maud` (escapes by default) |
| HTML sanitizing (share viewer) | `ammonia` |
| CLI and config | `clap`, `toml` |
| Logging | `tracing`, `tracing-subscriber` |

`tokio` with `rt-multi-thread`, `signal`, `macros`. No reqwest, rustls or OpenSSL in the
backend's normal dependencies.

### 3.2 Process and deployment

- `airpost-backend serve [--config airpost.toml]`; env overrides `AIRPOST_DATA_DIR` (default `/data`),
  `AIRPOST_BIND` (default `127.0.0.1:5101`; the Docker image sets `0.0.0.0:5101`, since loopback
  is unreachable from the proxy), `AIRPOST_PUBLIC_URL`, `AIRPOST_SIGNUP` (`closed`),
  `AIRPOST_TRUSTED_PROXIES` (default loopback; section 5).
- `airpost-backend user add|list|disable|enable|delete`, `token create|list|revoke`, `migrate`,
  `backup <path>` (`VACUUM INTO`), `rotate-server-id`. `user delete --yes` removes the user and
  every row and asset they own. `token create` prints a device token, for the harness fixture
  (6.1) and scripts.
- **Restores.** A restored database has an older seq counter than the cursors engines hold, so
  under the same server id they would skip every write until seq passed their cursor again.
  `backup` therefore gives the copy a new server id (engines treat it as a first pull, 4.1), and
  `rotate-server-id` does the same for a data dir restored any other way. The stream also
  replays from `0` for a cursor above the counter (3.5), which covers C++ and engines that
  restored without a rotation, as long as nothing was written since.
- Data dir: `airpost.db` (+ WAL) and `assets/` (content-addressed files, B2). The dir is created
  0700 and `airpost.db` (with its WAL and shm files) and backups 0600; an existing world-readable
  `airpost.db` is tightened on open.
- One process. Stream fan-out is in memory (a `tokio::sync::watch` per `(user, aid)` carrying
  the latest seq), so horizontal scaling is out of scope.
- Refuses to start when `public_url` is not `https` and its host is not loopback, unless
  `--insecure-public-url` is passed. Basic auth over plain HTTP leaks tokens.
- `docs/self-hosting.md` gives the proxy settings the stream needs (3.5): Caddy
  `reverse_proxy { flush_interval -1 }`, nginx `proxy_buffering off` and `proxy_http_version 1.1`
  for `/deltas/`.

### 3.3 Data model

| Table | Columns | Notes |
|---|---|---|
| `users` | `id` (random 24 chars), `email` unique, `first_name`, `last_name`, `password_hash`, `tracking_ns` (random 128-bit, base64url) unique, `created_at`, `disabled_at` | `id` is the `userId` of the login result and the engine's server object. `tracking_ns` scopes tracking URLs (3.6). |
| `tokens` | `id`, `user_id`, `hash` (sha256) unique, `name`, `created_at`, `last_used_at`, `revoked_at` | Format `airpost_` + base64url(32 random bytes). Only the hash is stored. |
| `accounts` | `user_id`, `aid`, `created_at`; PK `(user_id, aid)` | Created on first use; at most 50 per user. `aid` is the client's 8-hex id (`idForAccount`), not unique across users. |
| `metadata` | `id` (INTEGER PRIMARY KEY), `user_id`, `aid`, `object_type`, `object_id`, `plugin_id`, `v`, `value` (JSON text), `header_message_id`, `tracking_id`, `seq`, `created_at`, `updated_at`; UNIQUE `(user_id, aid, object_type, object_id, plugin_id)`, the upsert target | `id` aliases the rowid: SQLite only keeps rowids stable across `VACUUM` and `VACUUM INTO` (backups) when an INTEGER PRIMARY KEY aliases them, and pages are ordered by it (3.5). One row per plugin value, including tombstones (`{}` / `[]`). `header_message_id` is the last non-null pushed (thread anchor, 4.1). `tracking_id` is copied from `value.trackingId` for `open-tracking` / `link-tracking`. Indexes `(user_id, aid, seq)` and `(user_id, plugin_id, tracking_id)`. |
| `seq` | single row counter | Global and monotonic; every metadata write takes the next value inside its transaction. Cursors are its decimal string. |
| `tracking_events` | `id`, `user_id`, `tracking_id`, `kind` (`open`/`click`), `link_index`, `rid`, `ts`, `applied` | Events whose metadata row has not been pushed yet wait here (`applied = 0`), at most 500 per `tracking_id`. Purged by retention. |
| `claims` | `user_id`, `aid`, `object_id`, `plugin_id`, `expiration`, `token_id`, `created_at`; PK of the first five | Expiration claims (3.8). Purged after 30 days. |
| `assets` (B2) | `id`, `user_id`, `slug` (random 128-bit), `kind` (`file`/`thread`/`page`/`signature`), `client_key`, `display_name`, `content_type`, `size`, `sha256`, `created_at`, `revoked_at` | Files on disk under `assets/<sha256[0:2]>/<sha256>`. |

All writes to `metadata` are `INSERT … ON CONFLICT DO UPDATE`, never `INSERT OR REPLACE`:
replace deletes and reinserts, giving the row a new rowid, which moves it across page offsets
(3.5).

### 3.4 Auth, login and server info

- **Auth on every API call:** `Authorization: Basic base64("<token>:")` (what C++ sends; the
  Rust engine sends the same); `Bearer <token>` also accepted. 401 for unknown, revoked or
  disabled.
- **`GET /airpost/v1/server`** (no auth): `{"serverId", "version", "publicURL", "signup":
  "open"|"closed", "features": {"tracking", "sharing"}}`. The desktop calls it when the user
  enters a URL, to check it is an Airpost server before opening the login window. `serverId` is
  random per instance, created at `migrate` and changed by `backup` / `rotate-server-id` (3.2).
- **Login:** `GET /airpost/v1/login?device=<name>` renders B0's form (email, password; sign-up
  when open); it posts form-encoded `email`, `password`, `action`, `device` to
  `POST /airpost/v1/login`. Success is a `200` page, never a redirect, with
  `<div id="airpost-login-result" hidden>base64url(JSON)</div>` (no padding; the JSON is ASCII,
  non-ASCII as `\u` escapes) and JSON
  `{"type":"airpost-login","version":1,"token":"airpost_…","userId":"…","serverId":"…","email":"…"}`.
  Each login mints a device token named from `device` (at most 64 chars) or the User-Agent.
  Form and result pages carry `Cache-Control: no-store`, `Referrer-Policy: no-referrer` and
  `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'`;
  no scripts and no third-party resources. Rate limits and argon2 permits as B0 (5).
- **`GET /airpost/v1/me`** (auth): `{"userId", "email", "firstName", "lastName", "trackingNS",
  "serverId", "publicURL", "features": {"tracking", "sharing"}}`. The desktop reads it after
  login and on `ProcessIdentityRefreshNeeded`; the Rust engine reads it before each stream
  connect (4.1). 401 means the token is gone.
- **`DELETE /airpost/v1/tokens/current`** (auth): revokes the calling token, `204`. Sign-out.
- **`GET /ping`** → 200 `{}`, the container health check.
- **Account removal:** `DELETE /airpost/v1/accounts/<aid>` deletes the account's metadata, events
  and claims, and closes its streams.
- **Dead after the switch** (removed in B1-E): `/onboarding`, `/onboarding/skip`, `/api/me`,
  `/api/feature_usage_event`, and the 404 stubs for `/api/resolve-dav-hosts`,
  `/api/login-link`, `/api/newsletter/*`, `/api/translate`, `/api/grammar/check`,
  `/api/info-for-email-v2/*` (the fallback 404 covers them). With them go `wire::Identity`,
  `STRIPE_PLAN` and the `createdAt` formatting. B0's users, tokens, CLI, auth extractor, login
  limits and CORS layer stay.
- **CORS:** the renderer calls with `fetch` and an `Authorization` header; there is no
  `webSecurity` override and `main.js:419` only rewrites CSP, so Electron enforces CORS. The
  `tower-http` CORS layer echoes `Origin` only for `file://` and `null` (never `*`), allows credentials
  and the `Authorization` and `Content-Type` headers, and answers `OPTIONS` preflights.

### 3.5 Metadata sync protocol

This is the protocol `MetadataWorker.cpp`, `TaskProcessor.cpp` and the Rust `MetadataBackend`
expect, with the server-side rules made explicit. All paths are scoped to the token's user;
`<aid>` is created on first use.

**Ordering invariant.** One writer connection. Each write takes its seq inside its own
transaction, so commit order equals seq order and a reader never sees seq `n+1` before `n`.
`v` is clamped to `u32::MAX`, since C++ reads it with `get<uint32_t>()` (`MailStore.cpp:43`).

**Push.** `POST /metadata/<aid>/<objectId>/<pluginId>`, body
`{objectType, version, value, headerMessageId?}`.
- `objectType` is lowercase (`message`, `thread`, `contact`); reject others with 400.
- `version` may be `null` (C++, model missing locally) or `0` (Rust, same case); both mean
  "unknown".
- Server version rule, in one transaction:
  1. Load the row. `base = row ? row.v + 1 : 1`; `new_v = max(base, version or 0)`.
  2. For `open-tracking` and `link-tracking`, merge server-owned fields (3.6) from the stored
     row and from pending `tracking_events` into the pushed value.
  3. If the merge changed the pushed value, `new_v += 1`, so the pushing device (which holds
     `version`) accepts the echo instead of ignoring it as equal.
  4. Upsert `value`, `new_v`, `header_message_id` (when non-null), `tracking_id`,
     `seq = next()`; notify the `(user, aid)` watch.
- Arrival order at the server is the conflict rule: the last push wins, and `v` only grows.
  Clients apply only greater versions, so devices converge on the server's row as long as every
  push reaches the server. Neither engine retries a failed `SyncbackMetadataTask` (C++
  `TaskProcessor.cpp:996-1000`, Rust `fail()` in `crates/airpost-tasks/src/lib.rs`), so the Rust
  engine keeps an outbox (4.1); without it, a lost push leaves the device's `v` above the
  server's and it ignores every later echo.
- The response is `200 {"v": new_v}`; both engines ignore the body.
- `value` larger than 64 KiB → 413. Tombstones (`{}`, `[]`) are stored like any value and never
  compacted, so a device that was offline for months still sees the delete.

**Backfill (Airpost-only).** `POST /airpost/v1/backfill/<aid>`, body: an array of up to 500
`{objectType, objectId, pluginId, version, value, headerMessageId?}`. Per record, in one
transaction: no row → insert as a push would; row exists → keep the stored value, and if
`version >= row.v` set `v = version + 1`, take a seq and notify, so the device adopts the
server's value through the stream. Used once per account per server (4.1). A plain push would
let a device's stale `LocalOnly` state overwrite rows other devices have been syncing. The
response is `200 {"written": n, "rejected": [i, …]}`: the rows inserted or raised, and the
indices of records skipped as invalid (a `value` over 64 KiB, a bad id, an unknown
`objectType`), so one bad local value does not hold back the rest of the batch. More than 500
records, or a body that is not a JSON array, is 400; the body limit is 16 MiB, so a client
splits larger batches.

**Head.** `GET /deltas/<aid>/head` → `{"cursor": "<max seq for this aid, or 0>"}`. The C++
engine sends the 4-byte body `null` with `Content-Type: application/json` on this GET and on
the pages; the handlers must not try to parse a body.

**Pages.** `GET /metadata/<aid>?limit=500&offset=<n>` → JSON array of
`{object_type, object_id, aid, plugin_id, v, value, headerMessageId?}`, ordered by rowid. Rowids
are stable because writes upsert (3.3), so offsets stay put while rows are updated. Tombstones
are included: if one were omitted and a row became a tombstone mid-paging, every later offset
would shift and C++ would skip an unchanged row for good. Empty values are harmless on a fresh
device.

**Stream.** `GET /deltas/<aid>/streaming?p=&ih=&cursor=<c>`:
- HTTP/1.1, `200`, chunked, `Content-Type: application/x-ndjson`, `X-Accel-Buffering: no`,
  sent immediately.
- The handler subscribes to the `(user, aid)` watch before it runs the replay query, so a write
  committed during the replay is not missed. It then sends every row of the aid with `seq > c`,
  in seq order, in batches of 500, then each new write as it happens. `c` empty or `0` replays
  the whole current state, so `Store::reset_for_account` writing cursor `"0"` (gap 2 in section
  7) needs no special case. A `c` above the global seq counter was issued before a restore
  (3.2) and is treated as `0`.
- Line: `{"object":"metadata","attributes":{object_type,object_id,aid,plugin_id,v,value,headerMessageId?},"cursor":"<seq>"}`.
  `cursor` must be a JSON string; C++ throws on a number after applying the record.
  `headerMessageId` is present for thread rows that have one; C++ ignores it.
- `{"object":"backend-event","event":"identity-refresh-needed"}` when the user's name, email or
  the server feature set changes. B1-M checks name and email on every heartbeat. Features only
  change across a restart, which drops every stream, so that case needs the last advertised set
  persisted; it lands with B2, the first phase that can turn a feature on.
- Heartbeat: 16 `\n` bytes every 5 s, data or not. Clients drop the connection below 1 byte/s
  over 30 s, i.e. 30 bytes; at 10 s the window holds only 32 to 48 bytes (the
  `MetadataWorker.cpp` comment says as much), so 5 s gives 96 to 112.
- The server never closes after a batch (a clean end makes the client reconnect at once). It
  closes cleanly after 1 h, when the account is removed, on server shutdown, and when the token
  is revoked, so revocation takes effect. `token revoke` and `user disable|delete` run in another
  process, so each stream re-checks its token on every heartbeat: revocation ends it within 5 s.
- Dead clients. A send that blocks for 60 s ends the stream, but that only happens once the
  channel, hyper's buffer and both socket buffers are full, which at 16 bytes per 5 s takes
  hours, and never behind a proxy, which always reads. So a vanished peer (laptop sleep,
  network switch) would hold its slot until TCP gives up. Two things free it sooner: a stream
  is keyed by token as well, and a new stream from the same token for the same aid replaces
  that token's old one (the same device reconnecting, so no reconnect loop); and accepted
  sockets get TCP keepalive and, on Linux, `TCP_USER_TIMEOUT` 60 s, which ends a direct
  connection whose heartbeats go unacknowledged. One token per running engine is therefore a
  requirement: two processes streaming the same aid with one token would keep replacing each
  other.
- `p` and `ih` (platform, IMAP host) are accepted and neither stored nor logged. The Rust
  engine does not send `ih`.
- Errors before streaming: 401 for auth, 400 for a non-numeric cursor, 429 with an empty body
  when the `(user, aid)` already has 4 open streams of other tokens, the token already streams
  50 accounts, or the user has 200 open streams (the account limit times 4; `aid` is
  unchecked, so without these one token could open unbounded streams). Both engines treat 429
  as retryable and back off; closing the oldest stream of another device instead would make it
  reconnect at once, and five devices would loop forever.

Head-then-pages-then-stream (C++) and stream-from-0 (Rust, 4.1) both converge: the state is
replayed from current rows, not from an event log, and anything delivered twice is dropped by
the client's version check.

### 3.6 Tracking (Airpost format)

The engine generates, per tracked message, a random 128-bit `trackingId` (base64url, 22
chars), and per recipient a random 64-bit `rid` (11 chars), when it sends a draft (`draft.send`
or the expiration executor; `airpost_compose::tracking`). Both live in the
plugin value, which the send moves to the sent message and pushes as any metadata:
- `open-tracking`: `{trackingId, recipients: {<rid>: <email>}, open_count, open_data, uid}`.
- `link-tracking`: `{trackingId, recipients, links: [{url, click_count, click_data}], uid}`, one
  entry per distinct URL; `i` is its index. `uid` is the draft's Message-ID, as the desktop
  wrote it before.
The open-tracking toggle writes `{open_count: 0, open_data: []}` and the link-tracking one
`{tracked: true}`; the engine keeps those keys and adds the rest. One `trackingId` serves both
plugins of a message. Every To, Cc and Bcc recipient gets their own body (one SMTP submission
each); the copy saved to Sent keeps the untracked body, so the sender never trips their own
pixel or links.

URLs, with `<ns>` the sender's `trackingNS` from `/airpost/v1/me`:
- pixel `<public_url>/t/o/<ns>/<trackingId>.<rid>.png`, as `<img alt="" src="…">` before the
  first `.gmail_quote` element, or at the end of the body;
- link `<public_url>/t/l/<ns>/<trackingId>/<i>.<rid>?u=<url-encoded original>` (JavaScript's
  `encodeURIComponent`), for every `http`/`https` `<a href>` outside `.gmail_quote`, up to
  1000 distinct URLs of at most 2 KiB each; the rest stay as written.

The engine builds them only when `/airpost/v1/me` reports `features.tracking`, `trackingNS` is
base64url and `publicURL` (or, when it is empty, the server URL) passes the 4.1 URL check;
otherwise a tracked draft is sent untracked. Plaintext drafts are never tracked.

The server looks the row up by `(user of <ns>, plugin_id, tracking_id)`. Scoping by `ns` is what
stops hijacking: a recipient on the same server sees the `trackingId` in their copy and could
push a row with it before the sender's push lands (minutes on Exchange, see
`remove_late_sent_copies` in `crates/airpost-tasks/src/lib.rs`), but only under their own user, which
the sender's URLs never reach. The recipient's address is not in the URL, so a forwarded message
reveals only an opaque `rid`. There are no Mailspring `/o/` or `/link/` routes and no
`header_message_id` lookup.

**Input validation**, before anything is stored: `ns` and `trackingId` 22 base64url chars, `rid`
11, `i` an integer below 1000, `u` at most 2 KiB. Invalid input records nothing.

**Open pixel.**
1. Validate; on failure still return the pixel.
2. Find the `open-tracking` row. Append `{timestamp: unix seconds, recipient: recipients[rid]}`
   to `open_data` (keep the last 500), increment `open_count`, bump `v`, take a seq, notify. An
   unknown `rid` records `recipient: null`. Drop a repeat from the same `rid` within 30 s (proxy
   prefetch double hits).
3. No row yet: insert a pending `tracking_events` row; the push merges it.
4. Always answer `200 image/png`, a fixed 1×1 transparent PNG, `Cache-Control: no-store`, no
   cookies. The rate limit (5) applies to recording, never to the response: Gmail's image proxy
   concentrates hits on a few IPs.

**Link redirect.**
1. Validate; on failure 404.
2. Redirect with 302 only when the `link-tracking` row exists, `links[i].url == u`, and the
   scheme is `http` or `https`. Then append `{timestamp, recipient}` to `links[i].click_data`
   (last 100), increment `click_count`, bump `v`, notify.
3. Otherwise render an interstitial page, "This link goes to <escaped u>", with the link and no
   automatic redirect, and record a pending click if the row is missing. `u` stays in the URL
   for this case: a recipient who clicks before the sender's push lands still gets somewhere,
   and the endpoint is never an open redirect.

**Merge rule on push** (3.5 step 2): for `open-tracking`, `open_count = max(stored, pushed)`
and `open_data` is the stored list plus pending events, with `rid` mapped through the pushed
`recipients`. For `link-tracking`, per index `i`, the same for `click_count` and `click_data`.
Everything else in the value comes from the push. Pending events are marked applied.

What the client then sees: the metadata delta reaches the engine through the stream,
`apply_metadata` saves it on the Message, and the open-tracking icon and the Activity plugin
read `open_data` / `click_data` with an email `recipient` as they do today.

### 3.7 Sharing and assets (B2)

- Paths are Airpost-only (O2); the desktop's Mailspring upload code is gone, so nothing needs the
  old `/api/save-public-asset` and `/api/share-static-page` paths.
- `POST /airpost/v1/assets` (multipart `filename`, `file`) → `{link}`. The client's
  `filename` (`<fileId>/<displayName>`, `sig-<id>-<rand>.png`, or a thread key) is kept as
  `client_key` and `display_name` only; storage paths come from the sha256. Re-uploading a live
  `client_key` replaces the content and keeps the slug, which is how the client updates a shared
  thread.
- Unsharing: an upload of `{"shared":false}` for a thread key revokes the slug (deletes the
  content, sets `revoked_at`). The next upload of that key mints a new slug. The client keeps
  `key` after unsharing (`thread-sharing/lib/main.tsx:146-160`), so keeping the slug would bring
  an old link back on re-share.
- Thread ingest keeps only what the viewer shows: subject, from, to, cc, date, body, attachment
  names. `bcc`, headers and every other field of the messages' `toJSON()` are stripped before
  storage. The raw JSON is never served.
- Links: `<public_url>/a/<slug>/<display_name>` for files and signatures,
  `<public_url>/thread/<slug>` for threads (rendered by the viewer), `<public_url>/p/<slug>`
  for activity pages.
- `POST /airpost/v1/pages` `{key, html}` → `{link}`.
- Thread viewer: server-rendered from the stored fields, bodies through `ammonia`.
- Static signature icons at `/static/signature/<name>.gif`.
- Signature images: the client sets `width` / `height` on the `<img>` instead of Mailspring's
  `?msw=&msh=` resizer params; the server serves the file as uploaded.
- Limits: 25 MiB per file, 2 MiB per page, 1 GiB per user (config). A daily job deletes
  `kind = file` assets no longer referenced by any of the user's live shared threads (Mailspring
  never deleted them). It never touches `signature` (referenced from delivered mail) or `page`.
- The client stores the returned `link` as `url` in the thread-sharing metadata and reads it
  from there, so other devices get it too and the URL carries the server's random slug, not
  the guessable `<threadId>-<ts>` key. (Mailspring built `shared.getmailspring.com/thread/<identity.id>/<key>`
  itself; that code is gone since 27f713bf6.)

### 3.8 Expiration claims (Airpost-only)

Once thread metadata syncs, every running device receives the same `metadata-expiration`. For
send-reminders that means one reminder email per device (`send-reminders-store.ts`
`_sendReminderEmail`); for snooze, racing moves. `POST /airpost/v1/claim/<aid>/<objectId>/<pluginId>/<expiration>`
inserts into `claims`: the first caller gets 200, every later one 409. The device holding the
claim gets 200 again if it repeats the call (a retry after a lost response). When a server is
connected, the engine's expiration executor claims before acting and does nothing on 409. On a
network error, a refused token or any other answer it acts anyway (D2: no feature depends on
the server), accepting a possible duplicate.

The executor claims due `send-later` (on a draft, for actions it performs), `thread-snooze` and
`send-reminders` rows. `<aid>` is the account id; `<objectId>` is a Message-ID, since model ids
differ across devices: the draft's for `send-later`, and for a thread the Message-ID of its oldest
non-draft message (the anchor of 4.1 can differ between devices that first saw different
messages of the thread); `<expiration>` is the metadata's expiration in Unix seconds. The request
has no body and the engine reads only the status. A claim it lost is remembered until the
engine restarts, and the row is left for the winning device's change to sync in.

## 4. Engine and client changes

### 4.1 Engine (`crates/airpost-net`, `crates/airpost-sync`, `crates/airpost-store`, `apps/engine/src/modes.rs`)

- **Airpost server object.** The identity line the engine reads after the account line (stdin;
  `--identity` takes the same JSON, for tests only, since argv is visible to other users):
  `{"type":"airpost-server","version":1,"url":"https://airpost.example.com","token":"airpost_…","userId":"…","serverId":"…"}`,
  as `airpost_wire::AirpostServer`, whose `Debug` redacts `token`. `url` is the base URL the user
  configured: scheme, host, optional port and path prefix; no trailing slash, query, fragment or
  userinfo; `https`, or `http` only for a loopback host; never `*.getmailspring.com`.
  - `null`: `LocalOnly`, unchanged.
  - `type` `airpost-server` with a bad `url`, an empty field or another `version`: log an error
    naming the field (never the token), then `LocalOnly`. Mail sync carries on.
  - Any other object (a Mailspring-shaped identity): validated as today
    (`ErrorIdentityMissingFields`, `Invalid Identity JSON`), then ignored: `LocalOnly`.
- **`HttpMetadataBackend { server: AirpostServer, push_client, stream_client, outbox }`**
  in airpost-net on `airpost-wire`; credentials and the account id go into the constructor. The trait
  gained three defaulted methods the `metadata` thread calls before each pull: `binding()`
  (the `/airpost/v1/me` pair), `rebind(local)` (clear the outbox, queue backfill) and
  `drain_outbox()`; `MetadataDelta` gained `AnchoredThreadMetadata(Metadata, hmid)`.
  - `push_client` is `HttpClient::new()`. `stream_client` is a separate blocking reqwest client
    with `http1_only()` (the workspace reqwest enables `http2`, which would negotiate h2 through
    Caddy; C++ forces HTTP/1.1 in `MetadataWorker.cpp:131`) and `timeout(30 s)`. In blocking
    reqwest 0.13 the client timeout bounds the wait for headers and each `Response::read` call
    separately (`blocking/client.rs:1497`, `blocking/response.rs:439`), so it acts as a 30 s
    per-read timeout matching curl's low-speed abort, not a total one. The stream must be read
    through `Read`, never `.text()`/`.bytes()`, and must not use `RequestBuilder::timeout`,
    which is total. If a test shows otherwise, add a watchdog that drops the response.
  - `push`: the POST in 3.5, Basic auth. On a retryable error (connect, timeout, 429, 5xx) or
    401/403 it writes the record to the outbox and returns success, so the task semantics stay
    as they are.
  - **Outbox:** a `MetadataOutbox` table in `airpost.db` (account, object type and id,
    plugin, version, value, header Message-ID, backfill flag; latest per key wins), created by
    `airpost_net::Outbox` on first use so a migrated database keeps the C++ V12 schema. Ids are
    AUTOINCREMENT so removing a sent entry never drops a newer push for the same key. A push
    that reaches the server deletes what is queued for its key, and pushes and drained entries
    are sent one at a time under a lock, so an older queued value never lands after a newer one
    (the server keeps the last write). The metadata thread drains it on every stream
    (re)connect, after the binding check and before opening the stream; a value queued while a
    stream is open waits for the next connect (the server ends streams after an hour).
  - **Backfill:** on the first pull for an account against this server, queue every
    `ModelPluginMetadata` row of the account into the outbox flagged as backfill and send them
    through `/airpost/v1/backfill/<aid>` (3.5). This covers metadata created under `LocalOnly` and
    metadata brought in by `import-legacy`. Records listed in `rejected` are dropped from the
    outbox, not retried.
  - **Cursor binding:** before each stream connect the metadata thread calls
    `GET /airpost/v1/me` (3.4) and binds to the `(serverId, userId)` it returns, stored in
    `_State` as `metadata-binding-<aid>` = `"<serverId>:<userId>"` next to `cursor-<aid>`. A
    missing or different pair means "first pull": discard the cursor, clear the outbox (its
    records were meant for another server or user; the backfill re-sends local state), backfill,
    stream from `0`. If the pair differs from the server object's, the server's wins: log it and
    emit `IdentityRefreshNeeded` once so the desktop updates its copy. The server id matters
    because a restore rotates it (3.2). The user matters because seq is global and only filtered per
    user: after signing in as another user on the same server (or `user delete` and re-adding
    the user, which gives a new id), the old cursor would skip the new user's rows below it.
    `import-legacy` carries the C++ `cursor-<aid>` (`legacy_import.rs` header), which is a
    Mailspring server position and would otherwise make the engine skip everything below it.
  - `pull(None)` / first pull: stream from cursor `0` instead of head + pages. That avoids the
    lost-pages bug (gap 6) without a protocol change.
  - `pull(Some(c))`: stream from `c`; for each line emit `Metadata` then `Cursor(c)`; ignore
    0–1 byte lines; log and skip bad JSON. Return the last cursor on a clean end. 429 and 5xx
    are retryable.
  - **401/403** on any call (`/airpost/v1/me`, push, backfill, stream): emit
    `IdentityRefreshNeeded` (`ProcessIdentityRefreshNeeded`) once and go dormant for the rest of
    the process: no more requests, pushes go to the outbox, `pull` returns a retryable error
    without a request so the worker backs off (3 s … 300 s). The desktop answers with a restart
    (4.2); the outbox survives it and drains after a re-login to the same server and user. C++
    aborts here.
  - The query string carries `p` and `cursor`, no `ih`.
- **Thread anchor** (wire-compatible; C++ ignores extra fields):
  - Thread pushes carry `headerMessageId`, taken from the message whose id forms the thread id
    (`t:<message id>`, `crates/airpost-core/src/ids.rs:121`).
  - `apply_metadata` (`crates/airpost-sync/src/metadata_worker.rs:115`), given a `t:` id that does
    not exist locally and an `attributes.headerMessageId`, resolves it through the local message
    with that Message-ID to its `threadId`. With no such message yet, it parks the record under
    the `t:<hmid>…` pending key `import-legacy` already uses (`legacy_import.rs:80-95`), so the
    message's first save attaches it.
- **`modes.rs`:** parse the identity line as above and build the backend from it per O7,
  replacing the hard-coded `Arc::new(airpost_net::LocalOnly)`; `IDENTITY_SERVER` is not read.
  `WorkerContext.identity` is unused today and can go. Log the choice (server host, never the
  token) through `airpost_core::logmsg`.
- **Doc fixes** found in the sweep: `MetadataPush::model_class` is lowercase; the
  `MetadataWorker::cursor` doc says a reset clears the cursor, but it writes `"0"`.
- Unchanged: workers, tasks and the expiration worker.

### 4.2 Client (`apps/desktop`)

Needed for the backend (all B1-C):
- **Server URL.** Preferences gets a "Airpost server" section: URL field, Sign in, Sign out,
  status. The URL is validated as in 4.1 (`https`, or `http` on loopback) and probed with
  `GET /airpost/v1/server` before Sign in is enabled.
- **Login.** Sign in opens a `BrowserWindow` on `<url>/airpost/v1/login?device=<hostname>` with
  an in-memory session partition (no `persist:`), `sandbox`, no preload, no node integration.
  On each `did-finish-load` whose origin is the server's, the main process reads
  `#airpost-login-result` with `executeJavaScript`, decodes it, checks `type`/`version`, closes the
  window and clears the partition. Navigation outside the server's origin is blocked.
- **Storage.** The token goes in the keyset under `airpost-server-token`; config `airpost.server`
  holds `{url, userId, serverId, email}`, plus `trackingNS`, `publicURL` and `features` from
  `GET /airpost/v1/me` after login. Both stay out of logs and error reports (add the token to the
  redaction in `src/engine/process.ts`). Mailspring's carried-over `identity` key is never read.
- **Engines.** The bridge builds the 4.1 object from `airpost.server` and the keyset and writes it
  in place of `null` when the engine is `rust`; C++ always gets `null`. Sign in, sign out and a
  URL change restart every engine process (the line is only written at spawn).
- **Sign out** calls `DELETE /airpost/v1/tokens/current` (best effort), deletes the keyset entry
  and `airpost.server` except `url`, and restarts the engines.
- **Refresh and revocation.** On `ProcessIdentityRefreshNeeded`, and at launch, call
  `GET /airpost/v1/me`: on 200 update `airpost.server`; on 401 sign out locally (no DELETE) and show
  "signed out by the server" in the pane. Network errors change nothing.
- **Plugins.** Server-backed features are disabled with a tooltip until a server is connected
  (`docs/pro-features-backend.md`). The engine, not the desktop, builds the tracking URLs and
  `trackingId`, `rid`, `recipients` and the per-recipient bodies (3.6), from the `trackingNS`
  and `publicURL` of its own `/airpost/v1/me` answer. The
  open- and link-tracking toggles only write metadata, and they turn on when the engine
  reports `capabilities.server.readReceipts` / `linkTracking` available, which it does while a
  connected server's `/airpost/v1/me` has `features.tracking` (`not-on-server` when a server is
  connected without it). Sharing in B2-S.
- The engine's expiration executor claims an expiration (3.8) before it acts, when a server is
  connected; the desktop no longer acts on the built-in plugins' expirations (stage 4), so it
  claims nothing. A third-party plugin acting on its own expirations doesn't claim.
- Thread sharing stores and reads the returned `link` (3.7).
- Signature images get `width` / `height` attributes (3.7).

Client-only, no server: translation via a LibreTranslate URL and grammar via a LanguageTool URL
(section 1), Airpost OAuth client ids, Message-ID domain, branding strings. The updater, crash
reporter and Sentry are being handled outside this plan.

## 5. Security and privacy

- **Tracking data.** The server stores recipient addresses (the `rid → email` map) and
  timestamps inside metadata values: the sender's UI needs plaintext emails to match recipients.
  Tracking URLs carry only random ids, so a forwarded message reveals neither the recipient nor
  anything guessable about other messages. No IP addresses or user agents are stored, and access
  logs drop the path and query for `/t/`. Retention: `tracking_events` 90 days, and per-message
  event lists are capped (500 opens, 100 clicks per link, 500 pending events per tracking id).
  Open counts are unreliable anyway: Apple Mail Privacy Protection and Gmail's image proxy fetch
  pixels on delivery. Document it on the toggle.
- **Attacker-controlled input.** Everything in a tracking URL comes from whoever holds the mail.
  It is validated for length and format before anything is stored (3.6); nothing from the URL
  becomes a key outside the sender's own `ns`.
- **Delivered mail depends on the server.** Pixels and rewritten links are baked into sent
  mail. If the server goes away, tracked links break for recipients. The client should say so
  the first time link tracking is enabled.
- **Redirects.** Only to the stored URL for that `(trackingId, index)`, `http(s)` only;
  everything else gets the interstitial (3.6). Rows are found within the sender's `ns`, so
  another user on the same server cannot take over a sender's tracking rows.
- **Share links.** Random 128-bit slugs from the server. Public by design; revocable by
  unsharing, which deletes the content. Thread uploads are stripped of `bcc` and headers on
  ingest and the raw upload is never served (3.7). Responses carry
  `Content-Security-Policy: default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; sandbox`,
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`; non-image files are served
  with `Content-Disposition: attachment`. The viewer blocks remote images, so third-party
  pixels in a shared thread do not fire.
- **Auth.** Tokens are 256-bit random and stored as sha256; passwords argon2id, at most 4
  hashes at once. Login is limited to 5 attempts per minute per client IP, then per email; an
  IP over budget spends nothing from the email's, and emails over 254 bytes are rejected before
  either. The client IP is the peer, or for a peer in `trusted_proxies` (default loopback) the
  rightmost `X-Forwarded-For` hop that is not a trusted proxy; IPv6 is keyed per /64. Tokens
  are revocable per device. HTTPS is required
  for non-loopback public URLs (3.2). No cookies, so the uploaded HTML cannot ride a session.
  CORS allows only the app's `file://` / `null` origins (3.4). The login result page is
  `no-store`, and the token never appears in a URL or a redirect (3.4).
- **Rate limits and sizes.** Authenticated: 20 req/s per token, burst 100. Public (`/t`, `/a`,
  `/thread`, `/p`): 10 req/s per IP, applied to recording for `/t` (the pixel is always
  returned). Body limits: 64 KiB metadata, 25 MiB uploads, 2 MiB pages. Accounts: 50 per user.
  Streams: 4 per `(user, aid)`, one per `(token, aid)`, 50 per token, 200 per user, then 429.
- **Data removal.** `DELETE /airpost/v1/accounts/<aid>` and `airpost-backend user delete`.
- **Self-host defaults.** Bind loopback, signup closed, tracking and sharing enabled but off in
  the client until the user toggles them per message, no telemetry, `ih` / `p` dropped.

## 6. Tests and work breakdown

### 6.1 Test strategy

- **Backend unit tests:** the version, backfill and merge rules (3.5, 3.6) as table tests,
  tracking URL validation, redirect validation, cursor replay from `0`, tombstones in pages,
  rowid stability across updates, `v` clamping, `ns` scoping (another user's row with the same
  `trackingId` is never found), claims.
- **Backend integration tests** (`apps/backend/tests`): the login form and result page
  (decode `#airpost-login-result`, `no-store`, no token in any `Location`), `/airpost/v1/server`,
  `/airpost/v1/me`, `DELETE /airpost/v1/tokens/current`, and 404 for the dead paths in 3.4. Start the
  server on an ephemeral port
  with a temp data dir, drive it with a plain HTTP client; streaming tests use paused tokio time
  for the 5 s heartbeat, the 1 h close, revocation and the 429 stream limit; a CORS preflight
  test.
- **Engine against the real server** (B1-E, `apps/backend/tests`): `airpost-backend` exposes
  `test_server() -> (url, token)` from its lib, running its own tokio runtime on a thread,
  because the blocking reqwest client must not run inside a runtime. The backend dev-depends on
  airpost-net and drives `HttpMetadataBackend` directly: push, pull from `0`, outbox drain after a
  server restart, backfill, cursor discarded on a different `serverId`, the thread anchor
  round-trip. `apps/engine/tests/metadata_backend.rs` gains a case with the HTTP backend.
- **Harness** (`apps/engine/test`):
  - A `backend` fixture in `harness/servers/backend.py`. It runs `AIRPOST_BACKEND_BIN` (like
    `AIRPOST_ENGINE_BIN`), creates a user and token via the CLI, takes `userId` from `user add` and
    `serverId` from `GET /airpost/v1/server`, and checks `GET /airpost/v1/me` returns the same pair.
    `backend.backup` / `backend.restore` follow the restore procedure in `docs/self-hosting.md`.
  - A `backend:` scenario key. Scenario YAML has no identity key today: `scenario.py` builds
    `EngineProcess` without `identity=` (`engine.py:204`). The key makes the scenario pass
    `--identity`: the 4.1 Airpost server object for Rust; for C++ a Mailspring-shaped
    `{"id": userId, "emailAddress", "firstName": "", "lastName": "", "token", "createdAt": "2018-02-16T00:00:00.000Z"}`
    with `IDENTITY_SERVER` set to the fixture's URL (C++ throws without `createdAt`; a date
    after 2018-02-15 keeps it on id schema 1, `MailUtils::setBaseIDVersion`).
  - Scenario steps: `backend: push` (another device writes), `backend: get <url>` (a recipient
    hits a pixel or link), `backend: stop` / `start` (outage), `expect: backend_metadata`.
  - The engine only sends what the client gave it, so a scenario puts the 3.6 pixel and links
    into `perRecipientBodies` itself and extracts the URLs from the fake SMTP capture.
  - New scenarios: `backend-metadata-push`, `backend-metadata-from-other-device`,
    `backend-thread-metadata-anchor`, `backend-cursor-reset-replays`, `backend-stream-reconnect`,
    `backend-push-during-outage` (outbox), `backend-backfill-local-metadata`,
    `backend-token-revoked` (expects `ProcessIdentityRefreshNeeded`, no abort),
    `backend-open-tracking`, `backend-link-tracking`, `backend-tracking-before-push`,
    `backend-link-unknown-interstitial`. Written so far (B1-H): `backend-metadata-push`,
    `backend-push-during-outage`, `backend-token-revoked`, `backend-server-restored` (restore
    from an older backup: new `serverId`, cursor discarded, backfill) and
    `backend-identity-invalid` (plain `http` on a non-loopback host stays `LocalOnly`). Every
    `backend:` scenario also checks that no device token reaches the engine's log or stderr.
- **C++ oracle** (optional, not part of any gate: the desktop always gives C++ `null`; run with
  `AIRPOST_ENGINE_BIN` pointing at a C++ build): `backend-metadata-push`, `backend-metadata-from-other-device`,
  `backend-cursor-reset-replays`, `backend-stream-reconnect` and the tracking scenarios also run
  against the upstream C++ binary. It speaks the original protocol (head + pages + stream,
  `version: null`), so passing both engines is the wire-compatibility check. Not in the oracle
  set: `backend-token-revoked` (C++ `abort()`s), and the outbox, backfill and anchor scenarios
  (Airpost additions C++ lacks).
- **Desktop:** specs for URL validation, decoding the login result, the engine line per engine
  kind, and sign-out on a 401 from `/airpost/v1/me`; manual
  runs of sign-in, snooze across two machines, and a tracked send to a real mailbox.

### 6.2 Work breakdown

Packages within a phase are independent. A phase starts when the packages it depends on have
passed their gates.

**Scheduling (2026-10-16).** The engine workflow is done, so B1-E and B1-H can start now, in
parallel. `apps/desktop` is off limits until B1-C is scheduled (another workflow is editing it),
so nothing in B1-E or B1-H touches it. B1-E lands the 3.4 endpoints first, since B1-H's fixture
reads `serverId` from `GET /airpost/v1/server`.

**B0 Foundation (done).** Crate, config, schema, repo, users, tokens and CLI, auth extractor,
CORS, `/ping`, account removal, Dockerfile, `docs/self-hosting.md`. Its Mailspring-compatible
`/onboarding`, `/api/me` and stubs are replaced in B1-E (3.4).

**B1 (parallel, after B0).**
- **B1-M metadata server (done):** push with the version rule, backfill, head, pages with
  tombstones, stream with heartbeats, watch fan-out, the 429 limit, 1 h close, revocation
  close, the claim endpoint (3.8).
- **B1-E engine backend and Airpost auth** (done): in `apps/backend`, the 3.4 endpoints
  (`/airpost/v1/server`, `/airpost/v1/login`, `/airpost/v1/me`, `DELETE /airpost/v1/tokens/current`) and
  removal of the dead ones; then move the wire module to `crates/airpost-wire` (with `AirpostServer`)
  and add `apps/backend` to the root workspace in one change; `HttpMetadataBackend` with its
  stream client, the outbox, backfill, the `(serverId, userId)` cursor binding, the thread
  anchor, the identity-line parsing and O7 switch in `modes.rs`, 401/403 dormancy and 429
  handling, the doc fixes in 4.1. Gate: backend
  integration tests (6.1), the engine-against-server tests in `apps/backend/tests`, a CLI test
  for each identity-line case in 4.1; workspace fmt/clippy/test; the existing harness suite
  unchanged (still `LocalOnly`).
- **B1-H harness** (done): the `backend` fixture (`userId` from `user add`, `serverId` from
  `/airpost/v1/server`), the `backend:` scenario key with the per-engine identity in 6.1, and the
  step kinds. Gate: `backend-metadata-push`, where the Rust engine pushes a snooze and receives
  another device's snooze through the fixture (C++ dropped from the gate, see the oracle note
  in 6.1).
- **B1-C client** (after B1-E; when `apps/desktop` is free): everything in 4.2: the Airpost server
  section in Preferences, the login window, token in the keyset and `airpost.server` in config,
  the server object to the Rust engine and `null` to C++, engine restarts on sign-in, sign-out
  and URL change, refresh and 401 handling, feature gating from `airpost.server.features`,
  expiration claims, and the client-only items. Gate: the desktop specs in 6.1 and a manual
  sign-in against a local backend with the Rust engine.

**B2 (parallel, after B1-M; scenarios need B1-E and B1-H).**
- **B2-T tracking:** `/t/o`, `/t/l`, validation, interstitial, pending events, merge-on-push,
  dedupe and caps; client tracking ids and URLs from `publicURL` / `trackingNS` of
  `GET /airpost/v1/me` (client side after B1-C). Gate: the tracking scenarios on both engines.
- **B2-S sharing and assets:** upload, unshare-by-revoke, ingest stripping, static page, viewer
  with sanitizing, signature icons, quotas, the cleanup job; client `url` in thread-sharing
  metadata and signature dimensions. Gate: integration tests and a manual share from the
  desktop.

**B3 Ship (after B2).**
- Rate limits and body limits wired everywhere, retention jobs, access-log scrubbing.
- Docker image (distroless, non-root, `/data` volume, health check on `/ping`), Caddy and nginx
  examples, backup and restore docs.
- Security review against section 5.
- Deploy one instance and use it on two machines for a week.
- Gate: the full harness suite with the `backend` scenarios, on both engines (oracle set per
  6.1).

## 7. Risks and unknowns

- **Thread ids differ across devices.** A thread's id is `t:<id of the first message seen
  locally>` (`crates/airpost-core/src/ids.rs:121`, `Thread.cpp:23`), and imported threads use a
  `t:<hmid>…` pending form (`crates/airpost-store/src/legacy_import.rs:89`), so devices disagree for
  most threads and thread metadata parks in `DetatchedPluginMetadata`. The anchor in 4.1 fixes
  it for the Rust engine. A thread whose anchor message the other device never syncs (deleted,
  outside the sync window) still never attaches.
- **Duplicate expirations.** With thread metadata syncing, every device fires the same
  `metadata-expiration`; without the claim (3.8) send-reminders sends one email per device. A
  device that cannot reach the server acts anyway.
- **Backfill picks the server's value.** When a device first connects, existing server rows win
  over its local values (3.5). Local changes made under `LocalOnly` to objects another device
  already synced are lost, which is the conservative choice; it is not a merge.
- **Account ids** come from `sha256(email + imap/smtp user and host)[0:8]`, so they match
  across devices only when those settings match. Changing the IMAP host orphans an account's
  server metadata, as in Mailspring.
- **Stream through proxies.** Without `flush_interval -1` / `proxy_buffering off`, a proxy
  buffers the heartbeats and clients time out every 30 s. `X-Accel-Buffering: no` covers nginx
  defaults only.
- **Gaps in the Rust port the backend touches** (from the engine sweep): (1) no credentials on
  the trait and `LocalOnly` hard-coded, fixed by 4.1; (2) reset writes cursor `"0"`, handled by
  3.5; (3) `model_class` doc; (4) `version` null vs 0, handled by 3.5; (5) Rust skips unknown
  object types where C++ aborts, which the server avoids by rejecting them on push;
  (6) head persisted before paging, avoided by stream-from-0 in 4.1 (C++ keeps the bug, one
  reason it gets no server identity in the desktop); (7) failed metadata pushes are never
  retried, fixed by the outbox; (8) `import-legacy` carries Mailspring cursors, fixed by the
  cursor binding.
- **Not confirmed:** the original server's exact behaviour for cursor `0` and for
  `identity-refresh-needed` (inferred from client code); the deleted
  `docs/open-tracking-backend-changes.md`; the reqwest per-read timeout behaviour (4.1) until a
  test exercises it.
- **Signature GIFs** in `templates.tsx` are Mailspring's; Airpost needs its own images before B2-S
  ships them.

## Status

**Decisions for review (2026-10-16, no Mailspring ID):**
1. Login is a server-rendered page in a sandboxed Electron window with an in-memory session;
   the token comes back in the result page body, never in a URL (3.4, 4.2). Device-code login
   is left for a future CLI.
2. `GET /airpost/v1/server` (public: serverId, publicURL, signup, features) and `GET /airpost/v1/me`
   (auth: user, trackingNS, serverId, publicURL, features) replace `/api/me`; sign-out is
   `DELETE /airpost/v1/tokens/current`.
3. The engine's identity line becomes
   `{"type":"airpost-server","version":1,"url","token","userId","serverId"}` for Rust only. An
   invalid Airpost object logs and falls back to `LocalOnly`; it does not exit (4.1).
4. The cursor binds to the `(serverId, userId)` that `/airpost/v1/me` returns before each stream
   connect, not to the stdin copy, so a restored server is noticed without a desktop restart.
5. After a 401/403 the engine stops calling the server until restarted and keeps pushes in
   the outbox; the desktop checks `/airpost/v1/me` and signs out on 401.
6. Plain `http` is accepted only for loopback hosts, in the desktop and the engine.
7. All Mailspring-compatible endpoints except the metadata protocol and `/ping` go; B2 upload
   paths move to `/airpost/v1/assets` and `/airpost/v1/pages`.
8. The backend auth work is part of B1-E, not a new package. Token in the keyset as
   `airpost-server-token`; non-secret server info in config `airpost.server`.

- **B0 (done).** Crate, config, schema, repo, users, tokens and CLI, auth, CORS, `/ping`,
  `/api/me`, `/onboarding`, stubs, account removal, Dockerfile, `docs/self-hosting.md`. MIT
  headers applied (O6; relicensed GPL-3.0-only on 2026-10-16). `/api/me`, `/onboarding` and
  the stubs are now dead (3.4).
- **B1-M (done).** Push, backfill, head, pages, stream, fan-out, claims, with the integration
  tests from 6.1.
- **Review fixes (2026-10-15).** Stream wake-ups and account-removal closes run inside the
  blocking write, so a client that disconnects mid-request no longer loses the notify. Streams
  are keyed by token: a reconnecting device replaces its stale stream, and per-token and
  per-user caps bound streams of made-up aids; accepted sockets get TCP keepalive and
  `TCP_USER_TIMEOUT` (3.5). A cursor above the seq counter replays from `0`; `backup` gives the
  copy a new server id, and `rotate-server-id` exists for other restores (3.2). Backfill skips
  invalid records and returns them in `rejected`. Login limits use the forwarded client IP from
  trusted proxies, check the IP first, key IPv6 per /64, cap emails at 254 bytes and bound
  concurrent argon2. The data dir is 0700 and the database and backups 0600.
- **B1-E (2026-10-16, uncommitted).** `crates/airpost-wire` holds the wire types, with
  `AirpostServer`, `LoginResult`, `ServerInfo` and `Me`; `apps/backend` is in the root workspace.
  Backend: `/airpost/v1/server`, `/airpost/v1/login`, `/airpost/v1/me`, `DELETE /airpost/v1/tokens/current`;
  `/onboarding`, `/api/me`, `/api/feature_usage_event`, the stubs, `wire::Identity` and
  `STRIPE_PLAN` removed. Engine: `HttpMetadataBackend` with the outbox, backfill, binding,
  thread anchor (push and apply), 401/403 dormancy and retryable 429/5xx; the identity-line
  switch in `modes.rs`; `WorkerContext.identity` removed. Tests: `crates/airpost-net/tests/backend_server.rs`
  (push and stream from 0 with the anchor, outbox across a restart, backfill, binding after a
  rotation, revoked token, path prefix), airpost-sync binding and anchor tests, a CLI test per
  identity-line case. Not done: an HTTP-backend case in `apps/engine/tests/metadata_backend.rs`
  (covered by `crates/airpost-net/tests/backend_server.rs` instead), and a test of the 30 s per-read stream
  timeout (still unconfirmed, 7).
- **Next.** B1-H's Rust half (the fixture's Airpost server object), B2-T server side. B1-C waits for
  `apps/desktop` to be free.
- **Deferred: legacy Mailspring-compatible API.** For now Airpost assumes everything is local
  (`LocalOnly`), and server work waits. Later, consider a `/legacy` route set, off by default
  behind a server setting: the B0 identity login page, `/api/me` and the stubs (restore from
  `f9c75b055^:apps/backend/src/http/onboarding.rs` and `api.rs`, with their tests), and the
  metadata routes mounted a second time under the prefix. Stock Mailspring only reaches it
  through `env=development` (`http://localhost:5101`, no prefix) plus a local proxy; the C++
  engine takes `IDENTITY_SERVER=<server>/legacy` directly, which would bring back the C++
  oracle for the `backend-*` scenarios.
