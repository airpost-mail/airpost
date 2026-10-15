# Tern backend

This is the brief for `apps/backend`, the self-hostable server behind the engine's
`MetadataBackend` trait (plan D3) and the desktop's server features. It maps every Mailspring
server dependency to a treatment, lists the decisions still open, and gives the architecture,
the protocol the engine and client expect, and a work breakdown sized for a multi-agent
workflow. Read `docs/engine-port-plan.md` (D2, D3, D10–D12, sections 3 and 12) and
`docs/engine-port-status.md` first.

The Mailspring server is closed source. Every server behaviour here is reconstructed from the
client (`apps/desktop/app`), the C++ engine (`legacy/mailsync-cpp/MailSync/`) and the Rust
engine. Desktop paths are relative to `apps/desktop/app/`.

## 0. Inherited decisions, and the ones this plan changes

| # | Decision | Consequence for the backend |
|---|---|---|
| D2 | Every Pro feature stays, with no sign-in requirement | Snooze, send-later and reminders must work with no backend at all. The backend adds multi-device sync, tracking and sharing; it never gates a feature. |
| D3 | Metadata is local only; multi-device sync is out of scope; remote sync sits behind `MetadataBackend`; nothing calls `*.getmailspring.com` | **Changed.** Multi-device metadata sync is the core of this plan. `LocalOnly` stays the default when no server is configured. Update D3 in the engine plan when B1-E lands, together with the section 3 change in O7. |
| D10, D12 | One monorepo, one Cargo workspace; the backend reuses engine crates (ids, metadata models) | **Changed.** The backend reuses no engine crate: it has its own schema and never computes ids (3.1). It starts in its own workspace and joins the root one in B1-E (6.2). Update D12 in the engine plan at the same time. |
| D11 | Everything ships under GPL-3.0-only | The backend and the wire crate are written from scratch and are MIT (O6). The engine and desktop stay GPL-3.0-only. |

## 1. Scope

Every inventory item, with its treatment. **Backend**: the server implements it. **Client**:
replaced or removed in the desktop or engine, no server needed. **Dropped**: removed with no
replacement. **Later**: wanted, not in v1.

| Item | Treatment | Why |
|---|---|---|
| Plugin identity gate (`src/package-manager.ts:133`) | Client | Drop `isIdentityRequired` from thread-snooze, send-reminders and activity; keep it on open-tracking and link-tracking. The other gated plugins activate live at sign-in (`package-manager.ts:55`), so B1-C handles each of them first (rows below). |
| Mailspring ID sign-in webview (`onboarding/lib/page-authenticate.tsx`) | Backend + client | The backend serves a compatible `/onboarding` page (3.4). The client stops forcing the page on first run; it is reached from Preferences. |
| `/api/me` refresh and poll | Backend | How the client learns the identity and server features, and how it notices a revoked token (4.2). |
| Feature quotas, `/api/feature_usage_event` | Dropped | No plans or quotas. The backend still answers the POST with 204: `markUsed` queues `SendFeatureUsageEventTask` even when `featureUsage` is `{}` (`feature-usage-store.tsx:128-139`), and C++ runs it. |
| Pro checks and upsell (`hasProFeatures`, `page-initial-subscription.tsx`, please-subscribe notif) | Client | `hasProFeatures()` returns true; remove the subscription page and notification. |
| `/api/login-link`, `/payment`, `/dashboard` | Dropped | Billing only. |
| Login-link rewriting of links in emails (`src/components/evented-iframe.tsx:226-240`) | Client | Remove: it turns any link under the identity server into a single-sign-on call. |
| Open tracking | Backend, new Tern URL format | Needs a public host that writes events into metadata. Mailspring's `/o/<token>.png` pixels point at `link.getmailspring.com` and can never reach a Tern server, so the format is replaced (3.6). |
| Link tracking | Backend, new Tern URL format | Same, plus a safe redirect. |
| Metadata push, pull, delta stream | Backend | The core of the server. v1 carries message metadata (tracking state) and, with the thread anchor in B1-E, thread metadata (snooze, reminders). Send-later never syncs: it lives on drafts, which are local-only (O1). |
| `identity-refresh-needed` backend event | Backend | Sent when the identity or server feature set changes, so the client re-fetches `/api/me`. |
| Thread sharing (`/api/save-public-asset`, public viewer) | Backend, phase B2 | Needs upload, public storage and a viewer page. Until B2-S, B1-C gates the plugin on `tern.features.sharing`; it otherwise uploads to a missing endpoint and builds `shared.getmailspring.com` URLs. |
| Activity report sharing (`/api/share-static-page`) | Backend, phase B2 | Same storage as sharing. |
| Signature photo upload | Backend, phase B2 | Same storage. The URL is baked into delivered mail, so it must stay up. |
| Company logo (`logo.getmailspring.com`) | Dropped | No free logo service to proxy. The client removes the option; existing URL entry still works. |
| Signature social icons (`www.getmailspring.com/signature-assets/*.gif`) | Backend static files, phase B2; client fallback | Data URIs are blocked by many mail clients, so the icons need a host. Without a server the client renders text links. The GIFs need Tern-owned replacements. |
| Translation (`/api/translate`) | Client, in B1-C | Auto-translate is on once `hasProFeatures` is true, so with an identity it calls the backend's 404. Gate the plugin on a user-configured LibreTranslate URL (`{q, source:"auto", target, format:"html"}` → `{translatedText}`), with the adapter in `translation/lib/service.ts`. |
| Contact enrichment (`/api/info-for-email-v2`) | Dropped, in B1-C | No free data source. Disable the participant-profile plugin before sign-in ships. |
| Grammar check (`/api/grammar/check`) | Client, in B1-C | The response is already LanguageTool's `/v2/check`. Call a user-configured LanguageTool URL directly (form-encoded `text`, `language`); hide the toggle when none is set. Must land before sign-in is offered. |
| Newsletter opt-in | Dropped | |
| Gmail / O365 OAuth | Client | Tern's own Google and Azure app registrations; the loopback server shows its own "finished" page instead of redirecting to `id.getmailspring.com/oauth/finished`. |
| Auto-update (`updates.getmailspring.com`) | Client now; Tern feed later | Disable first: a Tern build can install Mailspring over itself today. A later feed can come from GitHub releases and is not this server's job. |
| Native crash reporter (`id.getmailspring.com/report-crash`) | Client | Disable. |
| Sentry DSN (Mailspring's project) | Client | Disable, or a Tern DSN configured at build time. |
| Engine identity input (`--identity`, stdin) | Engine | Selects the HTTP backend in the Rust engine (4.1). The desktop gives C++ `null` (O7). |
| `/api/resolve-dav-hosts` | Done in the engine | Rust resolves SRV locally. The backend answers 404, which the C++ engine reads as "no DAV, skip". |
| `/ping` | Backend | One line; also the container health check. |
| `@getmailspring.com` Message-IDs (`src/flux/stores/draft-factory.ts:66`) | Client | Use the sender's domain. |
| "Sent from Mailspring" signature, help links, `remove-tracking-pixels` host patterns | Client | Branding. |
| Snooze, reminders | Client (gate) + backend (mirror) | They fire locally from `metadata-expiration` deltas. The backend carries their metadata between devices and arbitrates which device acts on an expiration (3.8). |
| Send later | Client only | Fires locally from the draft's metadata. Not synced: `draft-store.ts:620-630` queues an undo-only task on purpose, and `SyncbackDraftTask` has no remote side. |

What v1 of the backend is, given the recommendations in section 2: accounts and device tokens,
the metadata protocol with the Tern additions (backfill, claims, the thread anchor echo), Tern
open and link tracking, `/api/me`, `/onboarding`, `/ping` and the compatibility stubs. Multi-device
in v1 means tracking state and, once B1-E's thread anchor lands, snooze and reminders; it never
covers send-later. Sharing and asset hosting follow in B2.

## 2. Open decisions

| # | Question | Recommendation | Reason |
|---|---|---|---|
| O1 | Features in v1 | Metadata sync and Tern open/link tracking in v1, with the thread anchor (4.1) in B1-E so snooze and reminders sync. Thread sharing, activity sharing and signature assets in B2, same release if they are ready. Translation, grammar and enrichment never in the backend. Send-later stays single-device. | Without the anchor, thread metadata parks in `DetatchedPluginMetadata` on the second device for most threads (7), and v1 multi-device would only carry tracking state, which only the sending device writes anyway. The anchor is wire-compatible and small. Sharing needs storage, a viewer and HTML sanitizing, which roughly doubles the surface. |
| O2 | Wire compatibility with the Mailspring ID API, or a new API | Compatible for the identity, `/api/me`, `/onboarding` and the metadata protocol: same paths, auth, bodies and stream framing. Tracking uses a new Tern URL format (3.6). Tern-only endpoints live under `/tern/v1/`. | The engine trait, the C++ engine and the client speak the metadata protocol, so compatibility there lets the C++ engine act as the backend's oracle, and lets the unmodified desktop in `env=development` (which maps the id host to `http://localhost:5101`) sign in against a local backend. Tracking URLs have no such constraint: every one already delivered points at `link.getmailspring.com`, and 4.2 rewrites `PLUGIN_URL` anyway. |
| O3 | Auth and identity | Multi-user data model; single-admin by default. Users are created with `tern-backend user add`; open signup is a config switch, off by default. Login is email + password (argon2id) on `/onboarding`, which mints one revocable token per device and returns it in `#identity-result`. The engine gets the token as it does today, in the identity JSON on stdin. | Matches the client's existing flow (webview → identity JSON → keychain → engine stdin), so the client needs no new sign-in UI. Per-device tokens make a lost laptop revocable. Pasting raw tokens is clumsier than a password and gives nothing back. |
| O4 | Storage | SQLite (rusqlite `bundled`, WAL) in v1, behind a `Repo` trait. Postgres only if someone runs a large shared instance. | One file to back up, same crate the engine uses. sqlx's SQLite driver would pull a second `libsqlite3-sys` and collide with rusqlite on `links = "sqlite3"`; a later Postgres backend should use `tokio-postgres`. |
| O5 | Deployment | One binary, `tern-backend serve`, plain HTTP on `127.0.0.1:5101` behind a TLS reverse proxy (Caddy or similar), plus a Docker image with `/data` as the only volume and `TERN_BIND=0.0.0.0:5101`. | No TLS stack to configure in-process, and no chance of pulling aws-lc-rs. Port 5101 is the client's development mapping (O2). |
| O6 | Licence | **Decided (2026-10-15): MIT** for `apps/backend` and `crates/ms-wire`. No dependency on GPL-derived crates; permissive deps only. No `/tern/v1/source` endpoint. | The user wants new code permissive. The backend is a separate program over HTTP and reimplements an interface, so it is not a derivative of the GPL code. |
| O7 | Engine switch for the HTTP backend | Rust: use the HTTP backend when the identity is non-null and `IDENTITY_SERVER` is an `http(s)` URL whose host is not `*.getmailspring.com`; otherwise `LocalOnly`. Desktop: pass the server identity only when `core.sync.engine` is `rust`, and `--identity null` to C++. C++ stays a test oracle, driven by the harness. | The desktop already passes `IDENTITY_SERVER = rootURLForServer('identity')`. The pending desktop diff adds `core.sync.engine` with `cpp` as the default (`config-schema.ts`, `mailsync-process.ts`). Given a Tern identity, C++ `abort()`s on a revoked token, and the crash handler then marks the mail account `SYNC_STATE_AUTH_FAILED`; C++ also keeps the head-before-pages bug (7) and has none of the Tern additions. The harness keeps `--identity null` by default, so existing scenarios stay local. This changes plan section 3 ("accepted and ignored"); update it when B1-E lands. |
| O8 | Public URL for tracking and shares | One `public_url` in server config, returned by `/api/me` as `tern.publicURL`. The client uses it for pixels, redirects and share links, falling back to the server URL. | Tracking and shares only work if recipients can reach the host; the API host may sit on a private network. |

## 3. Architecture

### 3.1 Crates and dependencies

| Crate | Role | Depends on |
|---|---|---|
| `apps/backend` (`tern-backend`, licence per O6) | Bin + lib: HTTP server, repo, auth, tracking, sharing, CLI. Until B1-E it has its own `[workspace]` table, `Cargo.lock` and target dir. | `wire` module (below); no engine crate |
| `tern_backend::wire` → `crates/ms-wire` (MIT) | Serde types for the wire protocol: metadata record, push body, stream line (`metadata` / `backend-event`), head response, backfill and claim bodies, identity JSON, the constants (heartbeat 5 s × 16 bytes, page size 500, low-speed 30 s, `v` max `u32::MAX`). Starts as a module of the backend; B1-E moves it to `crates/ms-wire` when the backend joins the root workspace. | serde, serde_json |
| `ms-net` | Gets `HttpMetadataBackend` (4.1) on `ms-wire`, in B1-E. | |

The backend does not compute message ids and has its own schema, so it depends on neither
`ms-store` nor `ms-core`; converting a wire record to `ms_core::Metadata` is one function in
ms-net. The point of `ms-wire` is that the engine and server cannot drift on the wire format,
as `packages/` does for client protocol types.

Why a separate workspace first: the root `members = ["apps/engine", "crates/*"]`
(`Cargo.toml:4`) would pick up a half-written `crates/ms-wire` the moment the directory exists
and break the engine workflow's cargo runs, and adding the backend to the root would touch
`Cargo.toml` and `Cargo.lock`, which that workflow owns. The cost is that the backend cannot use
`workspace = true` dependencies or the root `[profile.release]` until B1-E; it pins the same
versions by hand.

Backend dependencies (all MIT or MIT/Apache; confirm current versions and run `cargo deny` in
B0, as plan 4.5 did):

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

- `tern-backend serve [--config tern.toml]`; env overrides `TERN_DATA_DIR` (default `/data`),
  `TERN_BIND` (default `127.0.0.1:5101`; the Docker image sets `0.0.0.0:5101`, since loopback
  is unreachable from the proxy), `TERN_PUBLIC_URL`, `TERN_SIGNUP` (`closed`),
  `TERN_TRUSTED_PROXIES` (default loopback; section 5).
- `tern-backend user add|list|disable|enable|delete`, `token create|list|revoke`, `migrate`,
  `backup <path>` (`VACUUM INTO`), `rotate-server-id`. `user delete --yes` removes the user and
  every row and asset they own. `token create` prints a device token, for the harness fixture
  (6.1) and scripts.
- **Restores.** A restored database has an older seq counter than the cursors engines hold, so
  under the same server id they would skip every write until seq passed their cursor again.
  `backup` therefore gives the copy a new server id (engines treat it as a first pull, 4.1), and
  `rotate-server-id` does the same for a data dir restored any other way. The stream also
  replays from `0` for a cursor above the counter (3.5), which covers C++ and engines that
  restored without a rotation, as long as nothing was written since.
- Data dir: `tern.db` (+ WAL) and `assets/` (content-addressed files, B2). The dir is created
  0700 and `tern.db` (with its WAL and shm files) and backups 0600; an existing world-readable
  `tern.db` is tightened on open.
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
| `users` | `id` (random 24 chars), `email` unique, `first_name`, `last_name`, `password_hash`, `tracking_ns` (random 128-bit, base64url) unique, `created_at`, `disabled_at` | `id` is `identity.id`. `tracking_ns` scopes tracking URLs (3.6). |
| `tokens` | `id`, `user_id`, `hash` (sha256) unique, `name`, `created_at`, `last_used_at`, `revoked_at` | Format `tern_` + base64url(32 random bytes). Only the hash is stored. |
| `accounts` | `user_id`, `aid`, `created_at`; PK `(user_id, aid)` | Created on first use; at most 50 per user. `aid` is the client's 8-hex id (`idForAccount`), not unique across users. |
| `metadata` | `id` (INTEGER PRIMARY KEY), `user_id`, `aid`, `object_type`, `object_id`, `plugin_id`, `v`, `value` (JSON text), `header_message_id`, `tracking_id`, `seq`, `created_at`, `updated_at`; UNIQUE `(user_id, aid, object_type, object_id, plugin_id)`, the upsert target | `id` aliases the rowid: SQLite only keeps rowids stable across `VACUUM` and `VACUUM INTO` (backups) when an INTEGER PRIMARY KEY aliases them, and pages are ordered by it (3.5). One row per plugin value, including tombstones (`{}` / `[]`). `header_message_id` is the last non-null pushed (thread anchor, 4.1). `tracking_id` is copied from `value.trackingId` for `open-tracking` / `link-tracking`. Indexes `(user_id, aid, seq)` and `(user_id, plugin_id, tracking_id)`. |
| `seq` | single row counter | Global and monotonic; every metadata write takes the next value inside its transaction. Cursors are its decimal string. |
| `tracking_events` | `id`, `user_id`, `tracking_id`, `kind` (`open`/`click`), `link_index`, `rid`, `ts`, `applied` | Events whose metadata row has not been pushed yet wait here (`applied = 0`), at most 500 per `tracking_id`. Purged by retention. |
| `claims` | `user_id`, `aid`, `object_id`, `plugin_id`, `expiration`, `token_id`, `created_at`; PK of the first five | Expiration claims (3.8). Purged after 30 days. |
| `assets` (B2) | `id`, `user_id`, `slug` (random 128-bit), `kind` (`file`/`thread`/`page`/`signature`), `client_key`, `display_name`, `content_type`, `size`, `sha256`, `created_at`, `revoked_at` | Files on disk under `assets/<sha256[0:2]>/<sha256>`. |

All writes to `metadata` are `INSERT … ON CONFLICT DO UPDATE`, never `INSERT OR REPLACE`:
replace deletes and reinserts, giving the row a new rowid, which moves it across page offsets
(3.5).

### 3.4 Identity, auth and the compatibility endpoints

- **Auth on every API call:** `Authorization: Basic base64("<token>:")` (what `makeRequest` and
  the engine send); `Bearer <token>` also accepted. 401 for unknown, revoked or disabled.
- **`GET /onboarding?version=&skipSupported=true`:** a server-rendered login form. On success
  the response page contains `<div id="identity-result">base64(identity JSON)</div>`, which
  `page-authenticate.tsx` reads on `did-finish-load`, and carries `Cache-Control: no-store`
  because it holds the token. With `skipSupported` a Skip link renders `{"skipped":true}`. Each
  login creates a device token named from the User-Agent.
- **`GET /api/me`:** the identity without `token`:
  `{id, firstName, lastName, emailAddress, createdAt ("%Y-%m-%dT%H:%M:%S.000Z"), stripePlan:"Tern", stripePlanEffective:"Tern", featureUsage:{}, tern:{serverId, publicURL, trackingNS, features:{tracking, sharing}}}`.
  `createdAt` is the user's creation time and always present: the C++ engine throws at startup
  without it, and a date after 2018-02-15 keeps C++ on id schema 1 (`MailUtils::setBaseIDVersion`).
  `serverId` is random per instance, created at `migrate`; the engine binds cursors to it (4.1).
  A non-`Basic` plan makes `hasProFeatures()` true in an unmodified client; empty `featureUsage`
  makes every feature unlimited.
- **Stubs:** `POST /api/feature_usage_event` → 204 (still queued by `markUsed`, section 1).
  `GET /ping` → 200 `{}`. `/api/resolve-dav-hosts`, `/api/login-link`, `/api/newsletter/*`,
  `/api/translate`, `/api/grammar/check`, `/api/info-for-email-v2/*` → 404.
- **Account removal:** `DELETE /tern/v1/accounts/<aid>` deletes the account's metadata, events
  and claims, and closes its streams.
- **CORS:** the renderer calls with `fetch`, `credentials: 'include'` and an `Authorization`
  header; there is no `webSecurity` override and `main.js:419` only rewrites CSP, so Electron
  enforces CORS. B0 adds a `tower-http` CORS layer unconditionally: echo `Origin` only for
  `file://` and `null` (never `*`), allow credentials and the `Authorization` and `Content-Type`
  headers, and answer `OPTIONS` preflights.

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
  `TaskProcessor.cpp:996-1000`, Rust `fail()` in `crates/ms-tasks/src/lib.rs`), so the Rust
  engine keeps an outbox (4.1); without it, a lost push leaves the device's `v` above the
  server's and it ignores every later echo.
- The response is `200 {"v": new_v}`; both engines ignore the body.
- `value` larger than 64 KiB → 413. Tombstones (`{}`, `[]`) are stored like any value and never
  compacted, so a device that was offline for months still sees the delete.

**Backfill (Tern-only).** `POST /tern/v1/backfill/<aid>`, body: an array of up to 500
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

### 3.6 Tracking (Tern format)

The client generates, per tracked message, a random 128-bit `trackingId` (base64url, 22 chars),
and per recipient a random 64-bit `rid` (11 chars). Both live in the plugin value, which the
client pushes after the send as today:
- `open-tracking`: `{trackingId, recipients: {<rid>: <email>}, open_count, open_data}`.
- `link-tracking`: `{trackingId, recipients, links: [{url, click_count, click_data}]}`.

URLs, with `<ns>` the sender's `tern.trackingNS` from `/api/me`:
- pixel `<public_url>/t/o/<ns>/<trackingId>.<rid>.png`;
- link `<public_url>/t/l/<ns>/<trackingId>/<i>.<rid>?u=<url-encoded original>`.

The server looks the row up by `(user of <ns>, plugin_id, tracking_id)`. Scoping by `ns` is what
stops hijacking: a recipient on the same server sees the `trackingId` in their copy and could
push a row with it before the sender's push lands (minutes on Exchange, see
`remove_late_sent_copies` in `crates/ms-tasks/src/lib.rs`), but only under their own user, which
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

- `POST /api/save-public-asset` (multipart `filename`, `file`) → `{link}`. The client's
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
- `POST /api/share-static-page` `{key, html}` → `{link}`.
- Thread viewer: server-rendered from the stored fields, bodies through `ammonia`.
- Static signature icons at `/static/signature/<name>.gif`.
- Signature images: the client sets `width` / `height` on the `<img>` instead of Mailspring's
  `?msw=&msh=` resizer params; the server serves the file as uploaded.
- Limits: 25 MiB per file, 2 MiB per page, 1 GiB per user (config). A daily job deletes
  `kind = file` assets no longer referenced by any of the user's live shared threads (Mailspring
  never deleted them). It never touches `signature` (referenced from delivered mail) or `page`.
- The client builds the share URL itself (`thread-sharing/lib/main.tsx:41-49`,
  `shared.getmailspring.com/thread/<identity.id>/<key>`). Change it to store the returned
  `link` as `url` in the thread-sharing metadata and read it from there, so other devices get
  it too and the URL carries the server's random slug, not the guessable `<threadId>-<ts>` key.

### 3.8 Expiration claims (Tern-only)

Once thread metadata syncs, every running device receives the same `metadata-expiration`. For
send-reminders that means one reminder email per device (`send-reminders-store.ts`
`_sendReminderEmail`); for snooze, racing moves. `POST /tern/v1/claim/<aid>/<objectId>/<pluginId>/<expiration>`
inserts into `claims`: the first caller gets 200, every later one 409. The device holding the
claim gets 200 again if it repeats the call (a retry after a lost response). When a server is
connected, the client claims before acting and does nothing on 409. On a network error it acts
anyway (D2: no feature depends on the server), accepting a possible duplicate.

## 4. Engine and client changes

### 4.1 Engine (`crates/ms-net`, `crates/ms-sync`, `crates/ms-store`, `apps/engine/src/modes.rs`)

- **`HttpMetadataBackend { base_url, token, server_id, push_client, stream_client, outbox }`**
  in ms-net on `ms-wire`; the trait does not change, because credentials go into the
  constructor.
  - `push_client` is `HttpClient::new()`. `stream_client` is a separate blocking reqwest client
    with `http1_only()` (the workspace reqwest enables `http2`, which would negotiate h2 through
    Caddy; C++ forces HTTP/1.1 in `MetadataWorker.cpp:131`) and `timeout(30 s)`. In blocking
    reqwest 0.13 the client timeout bounds the wait for headers and each `Response::read` call
    separately (`blocking/client.rs:1497`, `blocking/response.rs:439`), so it acts as a 30 s
    per-read timeout matching curl's low-speed abort, not a total one. The stream must be read
    through `Read`, never `.text()`/`.bytes()`, and must not use `RequestBuilder::timeout`,
    which is total. If a test shows otherwise, add a watchdog that drops the response.
  - `push`: the POST in 3.5, Basic auth. 401/403 is non-retryable and fails the task, as in C++.
    On a retryable error (connect, timeout, 429, 5xx) it writes the record to the outbox and
    returns success, so the task semantics stay as they are.
  - **Outbox:** a `MetadataOutbox` table in the engine store (account, object type and id,
    plugin, version, value, header Message-ID; latest per key wins). The metadata thread drains
    it on every stream (re)connect, before opening the stream.
  - **Backfill:** on the first pull for an account against this server, queue every
    `ModelPluginMetadata` row of the account into the outbox flagged as backfill and send them
    through `/tern/v1/backfill/<aid>` (3.5). This covers metadata created under `LocalOnly` and
    metadata brought in by `import-legacy`. Records listed in `rejected` are dropped from the
    outbox, not retried.
  - **Cursor binding:** the cursor is stored with the identity's `tern.serverId` and
    `identity.id`. A missing or different pair means "first pull": discard the cursor,
    backfill, stream from `0`. The user matters because seq is global and only filtered per
    user: after signing in as another user on the same server (or `user delete` and re-adding
    the user, which gives a new id), the old cursor would skip the new user's rows below it.
    `import-legacy` carries the C++ `cursor-<aid>` (`legacy_import.rs` header), which is a
    Mailspring server position and would otherwise make the engine skip everything below it.
  - `pull(None)` / first pull: stream from cursor `0` instead of head + pages. That avoids the
    lost-pages bug (gap 6) without a protocol change.
  - `pull(Some(c))`: stream from `c`; for each line emit `Metadata` then `Cursor(c)`; ignore
    0–1 byte lines; log and skip bad JSON. Return the last cursor on a clean end. 429 and 5xx
    are retryable.
  - 401/403 on the stream: emit `IdentityRefreshNeeded` once, then return a retryable error so
    the worker backs off (3 s … 300 s). C++ aborts here.
  - The query string carries `p` and `cursor`, no `ih`.
  - `record_feature_usage` stays the no-op default.
- **Thread anchor** (wire-compatible; C++ ignores extra fields):
  - Thread pushes carry `headerMessageId`, taken from the message whose id forms the thread id
    (`t:<message id>`, `crates/ms-core/src/ids.rs:121`).
  - `apply_metadata` (`crates/ms-sync/src/metadata_worker.rs:115`), given a `t:` id that does
    not exist locally and an `attributes.headerMessageId`, resolves it through the local message
    with that Message-ID to its `threadId`. With no such message yet, it parks the record under
    the `t:<hmid>…` pending key `import-legacy` already uses (`legacy_import.rs:80-95`), so the
    message's first save attaches it.
- **`modes.rs`:** build the backend from `IDENTITY_SERVER` and `ctx.identity` per O7,
  replacing the hard-coded `Arc::new(ms_net::LocalOnly)`. Log the choice through `ms_core::logmsg`.
- **Doc fixes** found in the sweep: `MetadataPush::model_class` is lowercase; the
  `MetadataWorker::cursor` doc says a reset clears the cursor, but it writes `"0"`.
- Unchanged: workers, tasks and the expiration worker.

### 4.2 Client (`apps/desktop/app`)

Needed for the backend:
- `rootURLForServer('identity')` returns config `tern.serverURL`; empty means no server, and
  `makeRequest` already refuses to run without an identity.
- The identity is stored with the server URL it was issued by. On launch, and when
  `tern.serverURL` changes, an identity whose URL does not match is dropped, so a carried-over
  Mailspring `config.identity` is never polled against a Tern server.
- Preferences: a "Tern server" pane (URL, Sign in → the existing `/onboarding` webview,
  Sign out) replacing `preferences-identity.tsx`.
- Restart the mailsync processes when the identity changes. Today the bridge only reads
  `IdentityStore.identity()` when it spawns a client, and only logout relaunches the app.
- Pass the identity to the engine only when `core.sync.engine` is `rust`; C++ gets `null` (O7).
- Revocation: `fetchIdentity` swallows a 401 from `/api/me` (`identity-store.ts:242-267`), so
  after a revoke the engine backs off forever and the user sees nothing. On 401, mark the server
  signed out in the Tern server pane and restart the engines with identity `null`.
- Plugins that activate at sign-in (section 1), all before sign-in is offered: disable
  participant-profile; gate translation on a LibreTranslate URL; switch grammar to a LanguageTool
  URL; gate thread-sharing on `tern.features.sharing`. Drop `isIdentityRequired` from
  thread-snooze, send-reminders and activity; keep it on open-tracking and link-tracking.
- Tracking: the composer extensions generate `trackingId`, `rid` and the `recipients` map, and
  build the 3.6 URLs from `identity.tern.publicURL` and `identity.tern.trackingNS`; toggles
  appear only when a server is connected.
- Snooze and send-reminders claim an expiration (3.8) before acting when a server is connected.
- Thread sharing stores and reads the returned `link` (3.7).
- Signature images get `width` / `height` attributes (3.7).

Client-only, no server: the gate change and onboarding cleanup from section 1,
`hasProFeatures()` true, removing the login-link rewriting in `evented-iframe.tsx`, cutting the
updater, crash reporter and Sentry, Tern OAuth client ids, Message-ID domain, grammar via
LanguageTool URL, branding strings. These touch files the engine workflow is editing (6.2), so
they wait with the rest of B1-C.

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
  `no-store`.
- **Rate limits and sizes.** Authenticated: 20 req/s per token, burst 100. Public (`/t`, `/a`,
  `/thread`, `/p`): 10 req/s per IP, applied to recording for `/t` (the pixel is always
  returned). Body limits: 64 KiB metadata, 25 MiB uploads, 2 MiB pages. Accounts: 50 per user.
  Streams: 4 per `(user, aid)`, one per `(token, aid)`, 50 per token, 200 per user, then 429.
- **Data removal.** `DELETE /tern/v1/accounts/<aid>` and `tern-backend user delete`.
- **Self-host defaults.** Bind loopback, signup closed, tracking and sharing enabled but off in
  the client until the user toggles them per message, no telemetry, `ih` / `p` dropped.

## 6. Tests and work breakdown

### 6.1 Test strategy

- **Backend unit tests:** the version, backfill and merge rules (3.5, 3.6) as table tests,
  tracking URL validation, redirect validation, cursor replay from `0`, tombstones in pages,
  rowid stability across updates, `v` clamping, `ns` scoping (another user's row with the same
  `trackingId` is never found), claims.
- **Backend integration tests** (`apps/backend/tests`): start the server on an ephemeral port
  with a temp data dir, drive it with a plain HTTP client; streaming tests use paused tokio time
  for the 5 s heartbeat, the 1 h close, revocation and the 429 stream limit; a CORS preflight
  test.
- **Engine against the real server** (B1-E, `apps/backend/tests`): `tern-backend` exposes
  `test_server() -> (url, token)` from its lib, running its own tokio runtime on a thread,
  because the blocking reqwest client must not run inside a runtime. The backend dev-depends on
  ms-net and drives `HttpMetadataBackend` directly: push, pull from `0`, outbox drain after a
  server restart, backfill, cursor discarded on a different `serverId`, the thread anchor
  round-trip. `apps/engine/tests/metadata_backend.rs` gains a case with the HTTP backend.
- **Harness** (`apps/engine/test`):
  - A `backend` fixture in `harness/servers/backend.py`. It runs `TERN_BACKEND_BIN` (like
    `MAILSYNC_BIN`) and creates a user and token via the CLI.
  - A `backend:` scenario key. Scenario YAML has no identity key today: `scenario.py` builds
    `MailsyncProcess` without `identity=` (`mailsync.py:204`). The key makes the scenario pass
    the identity and set `IDENTITY_SERVER` to the fixture's URL.
  - Scenario steps: `backend: push` (another device writes), `backend: get <url>` (a recipient
    hits a pixel or link), `backend: stop` / `start` (outage), `expect: backend_metadata`.
  - The engine only sends what the client gave it, so a scenario puts the 3.6 pixel and links
    into `perRecipientBodies` itself and extracts the URLs from the fake SMTP capture.
  - New scenarios: `backend-metadata-push`, `backend-metadata-from-other-device`,
    `backend-thread-metadata-anchor`, `backend-cursor-reset-replays`, `backend-stream-reconnect`,
    `backend-push-during-outage` (outbox), `backend-backfill-local-metadata`,
    `backend-token-revoked` (expects `ProcessIdentityRefreshNeeded`, no abort),
    `backend-open-tracking`, `backend-link-tracking`, `backend-tracking-before-push`,
    `backend-link-unknown-interstitial`.
- **C++ oracle:** `backend-metadata-push`, `backend-metadata-from-other-device`,
  `backend-cursor-reset-replays`, `backend-stream-reconnect` and the tracking scenarios also run
  against the upstream C++ binary. It speaks the original protocol (head + pages + stream,
  `version: null`), so passing both engines is the wire-compatibility check. Not in the oracle
  set: `backend-token-revoked` (C++ `abort()`s), and the outbox, backfill and anchor scenarios
  (Tern additions C++ lacks).
- **Desktop:** a spec for `rootURLForServer` config and the identity/server-URL binding; manual
  runs of sign-in, snooze across two machines, and a tracked send to a real mailbox.

### 6.2 Work breakdown

Packages within a phase are independent. A phase starts when the packages it depends on have
passed their gates.

**Running next to the engine workflow.** B0, B1-M, the server side of B2-T, and B2-S live only
in `apps/backend`, which has its own `[workspace]`, `Cargo.lock` and target dir until B1-E, so
they touch neither the root manifest nor files the engine workflow edits. Everything else
waits:
- B1-E, B1-H and the client side of B2 touch `crates/ms-net`, `crates/ms-sync`,
  `crates/ms-store`, `apps/engine`, `apps/engine/test` or `apps/desktop`, all being edited now.
  Schedule them after the engine workflow, or hand them to it.
- B1-C is blocked: the engine workflow has uncommitted edits to `mailsync-process.ts`,
  `mailsync-bridge.ts`, `application.ts`, `mailspring-window.ts`, `config-schema.ts` and
  `build.js`. It starts when those land, or goes to that workflow.

**B0 Foundation (serial, one agent).**
- `apps/backend` crate with its own `[workspace]` table, MIT licence (O6), the `wire` module with the types and constants from 3.1, `cargo deny` passing. No
  `crates/ms-wire` directory and no change to the root `Cargo.toml` or `Cargo.lock`.
- Config, data dir, SQLite schema with migrations (upserts only), the `Repo` trait over rusqlite.
- Users, tokens and CLI (including `user delete`); the auth extractor; the CORS layer; `/ping`,
  `/api/me`, `/onboarding`, the stubs in 3.4, `DELETE /tern/v1/accounts/<aid>`.
- Dockerfile (`TERN_BIND=0.0.0.0:5101`) and a `docs/self-hosting.md` stub with the proxy
  settings from 3.2.
- Gate: unit and integration tests, including one that GETs `/onboarding`, POSTs the login,
  extracts `#identity-result`, base64-decodes it, and asserts `id`, `token`, `emailAddress` and
  `createdAt` (after 2018-02-15), plus `Cache-Control: no-store` on the page. A run of the
  unmodified desktop with `env=development` against `localhost:5101` is a manual,
  non-blocking check.

**B1 (parallel, after B0).**
- **B1-M metadata server:** push with the version rule, backfill, head, pages with tombstones,
  stream with heartbeats, watch fan-out, the 429 limit, 1 h close, revocation close, the claim
  endpoint (3.8). Gate: integration tests from 6.1.
- **B1-E engine backend** (after the engine workflow): move the wire module to `crates/ms-wire`
  and add `apps/backend` to the root workspace in one change; `HttpMetadataBackend` with its
  stream client, the outbox, backfill, cursor binding, the thread anchor, the O7 switch in
  `modes.rs`, 401 and 429 handling, the doc fixes in 4.1; plan sections 0 (D3, D12) and 3
  updated. Gate: the engine-against-server tests in `apps/backend/tests`; workspace
  fmt/clippy/test; the existing harness suite unchanged (still `LocalOnly`).
- **B1-H harness** (after the engine workflow): the `backend` fixture, the `backend:` scenario
  key and step kinds. Gate: a smoke scenario where the Rust and C++ engines both push and
  receive a snooze.
- **B1-C client** (blocked, see above): server URL config and the identity binding, the
  Preferences pane, restart on identity change, identity only to the Rust engine, 401 handling,
  the plugin gating, expiration claims, and the client-only items in 4.2.

**B2 (parallel, after B1-M; scenarios need B1-E and B1-H).**
- **B2-T tracking:** `/t/o`, `/t/l`, validation, interstitial, pending events, merge-on-push,
  dedupe and caps; client tracking ids and URLs from `tern.publicURL` / `tern.trackingNS`. Gate:
  the tracking scenarios on both engines.
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
  locally>` (`crates/ms-core/src/ids.rs:121`, `Thread.cpp:23`), and imported threads use a
  `t:<hmid>…` pending form (`crates/ms-store/src/legacy_import.rs:89`), so devices disagree for
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
- **Signature GIFs** in `templates.tsx` are Mailspring's; Tern needs its own images before B2-S
  ships them.

## Status

- **B0 (done).** Crate, config, schema, repo, users, tokens and CLI, auth, CORS, `/ping`,
  `/api/me`, `/onboarding`, stubs, account removal, Dockerfile, `docs/self-hosting.md`. Headers
  say GPL-3.0-only until O6 is applied to the code.
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
  concurrent argon2. The data dir is 0700 and the database and backups 0600. Cursor binding to
  `(serverId, identity.id)` is a B1-E change (4.1).
- **Next.** B1-E and B1-H wait for the engine workflow; B2-T can start.
