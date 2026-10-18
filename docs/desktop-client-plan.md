# Desktop as a client of the engine

Draft, 2026-10-16, not yet confirmed by the user. Roadmap step 3 (memory:
tern-roadmap-after-parity): turn `apps/desktop` into a client of the engine protocol, the way
`apps/cli` and `apps/mcp` already are, with plugins kept first-class. Written against branch
`roadmap/desktop-client` (main + engine phase C `serve` + rebrand + integrations UI +
tern-mcp, head fa9b69a79). Engine paths are relative to the repo root, desktop paths to
`apps/desktop/app/`.

The goal set by the user: the engine holds the logic, Electron holds the UI and the plugins'
UI. No new logic goes into Electron; when a feature needs logic that a desktop store runs
today, the logic moves to the engine and the store becomes a cache or a caller. Background
is in `docs/architecture.md` (engine is the only process touching mail and credentials),
`docs/engine-protocol-plan.md` (protocol, read API, phase C), `docs/plugins-inventory.md`,
`docs/integrations-plan.md`, `docs/mcp.md` and `docs/backend-plan.md` section 4.

Marks: **V** = read or grepped in this tree; **I** = inferred, to check when the stage runs.

## 0. Decisions

| # | Decision |
|---|---|
| T1 | Electron keeps windows, menus, tray, OS integration, native notifications, sounds, the badge, the composer editor, plugin loading and plugin UI, the OAuth browser hand-off and redirect receiver, the backend login window, and starting the local engine from the main process (T7). Everything else that touches mail, credentials or settings shared with other clients runs in the engine. |
| T2 | Reads go through one structured engine method, `query`, taking the JSON form of a `ModelQuery`. The engine compiles it to SQL in `ms-store` against an allowlist generated from the desktop's `queryable` attributes. No raw SQL over the socket, not even locally. |
| T3 | `DatabaseStore` keeps its public API (`find`, `findBy`, `findAll`, `count`, `modelify`, `run`, `listen`) and changes transport only. `QuerySubscription` and its pool keep doing their diffing on deltas in the renderer. Engine-side live queries (`query.subscribe`) wait until the gateway and web need them. |
| T4 | The renderer's read-only SQLite stays as the transport until `query` passes the parity tests (T2), and whenever the engine lacks `query`. A flag switches it per window; the default flips once the search grammar (stage 9) ships; the SQLite path is deleted only after the C++ engine and the per-account and stdio fallbacks are gone (stage 10). |
| T5 | `query` supports `offset`/`limit` as well as cursors. The desktop list stays offset-based; `threads.list` keeps its keyset cursor for the CLI. |
| T6 | The Gmail-style search grammar moves to the engine (`ms-store`), and `query`'s search matcher takes the query text, plus the client's time zone and `now` for date terms (`before:`, `after:`). One parser for desktop, CLI, tern-mcp and web. `SearchQueryParser` stays exported for UI use. |
| T7 | Engine supervision moves from the main window's renderer (`MailsyncBridge`) to the Electron main process (`src/browser/engine-supervisor.ts`). The supervisor is the one owner of `serve`'s stdin (the parent channel): `KeyManager` moves to main with it (safeStorage is a main-process API that the renderer reaches through `@electron/remote` today), it reads the account list from `config.json` in main, takes `ProcessAccountSecretsUpdated` from `serve`'s stdout, and runs stdin-only calls (`account.add`, `account.test`, `credentials.import`) for any window over IPC. Until stage 11 it always spawns its own engine; reusing an engine found at `run/engine.json` waits for `--detached` and engine-owned credentials. Every window opens its own socket connection for reads and deltas; side effects that must run once (Task completion handlers, `OnlineStatusStore`, account state) stay in the main window, and `Actions.queueTask` keeps going through `ActionBridge` to the main window. The IPC rebroadcast of deltas goes. |
| T8 | The engine executes due expirations for the built-in plugin ids `send-later`, `thread-snooze` and `send-reminders` (expiration executor), and announces it as `hello.features: ["expiration-executor"]`. Clients that see the feature stop acting on those rows, except send-later rows marked `clientFinalize` (T9), which the desktop's send-later handler still sends. `metadata-expiration` stays for every other plugin id and for the declined rows. |
| T9 | A scheduled send is rebuilt from the current draft when it comes due, not prebuilt. After-send actions are a fixed vocabulary (`send`, `send-and-archive`). The desktop marks a draft `clientFinalize: true` in its send-later metadata when its action key is outside that vocabulary or a third-party `ComposerExtension` defines `applyTransformsForSending`; the executor leaves those rows to the client, as today. |
| T10 | The send pipeline moves to the engine as `draft.send` / `draft.cancelSend` (finalisation, undo delay, built-in tracking transforms) and `draft.create` (DraftFactory). Undo becomes a protocol call. `draft.send` with a delay writes the same send-later `{expiration, isUndoSend, actionKey}` metadata the desktop writes today and the executor fires it, so a pending send is persisted, fires at the next start if the engine quit inside the window, and stays visible to tern-mcp's `ensure_not_sending`. Third-party send hooks run in the desktop for sends it starts and pass their output to `draft.send`; they don't run for CLI, MCP or engine-executed sends, and the plugin docs say so. |
| T11 | The engine owns a settings store for keys it or other clients need: `core.sending.*`, `core.notifications.unsnoozeToTop`, `core.composing.html`, signatures and default signatures, templates, mail rules, the localized resurfacing texts. UI-only preferences stay in the desktop's `config.json`. The main process's `ConfigPersistenceManager` mirrors the engine-owned keys over its connection: it reads `settings.get` before it pushes config to windows, applies `settings.changed`, forwards writes to `settings.set`, and keeps writing the keys to `config.json` so an engine without `settings` still has them. Renderers keep today's synchronous `AppEnv.config` and its IPC. |
| T12 | Mail rules are evaluated by the engine on the `rulesReady` save it already computes. The desktop keeps the editor. |
| T13 | The engine owns the account registry (`<config>/accounts.json`, non-secret) and the secrets (OS keychain, encrypted-file fallback for headless). Secrets go client to engine only, through write-only methods, and are never read back. This replaces phase C's "secrets never go over the socket". Methods that write accounts or secrets or touch the engine itself form a **local-only class** (`account.add`, `account.test`, `account.setSecrets`, `account.update` of host or username, `account.remove`, `account.reset`, `secrets.*`, `credentials.*`, `server.*`, `engine.logs.tail`, `engine.setLogLevel`, `engine.resetDatabase`, `engine.shutdown`). `apps/gateway` declares its connection as relayed in `hello`; the engine refuses local-only methods on a relayed connection and the gateway refuses to relay them, unless the device token carries an admin scope granted at pairing (0.1 item 23). |
| T14 | OAuth: the client receives the browser redirect (Electron main on `127.0.0.1`, or the CLI); the engine holds the PKCE verifier, state and client secret and does the exchange. Provider discovery and the account test run in the engine. |
| T15 | The C++ engine and the `TERN_ENGINE_PROCESS=per-account` / `TERN_ENGINE_TRANSPORT=stdio` fallbacks are deleted before credentials move, so there is one credential path. |
| T16 | Integrations run on the `serve` engine (`integrations.*`). Keys go per request until T13 lands, then the engine reads them from its store. Availability comes from the engine (`capabilities` in `hello` and `engine.status`). |
| T17 | `internal_packages/mcp-server` is deleted. Its preferences pane becomes an editor of `<config>/mcp.json`, shows `mcp-audit.log` and runs `tern-mcp install`. |
| T18 | Plugin exports keep their shape. A new `Engine` export (`request`, `subscribe`, `features`) is the supported way to reach engine methods. `PLUGIN_API_VERSION` is decoupled from the app version, starting at 1.26.0, matched with semver against `engines.mailspring`; Tern-only manifest fields go in an optional `tern` block. |
| T19 | Third-party background logic runs as a service plugin: a separate program on the engine socket, the way tern-mcp is a separate program (tern-mcp itself is a stdio server registered in MCP client configs, not a service). Service plugins need a user unit (launchd LaunchAgent, systemd `--user`) installed by `tern service install`, and an engine that outlives the desktop, so they come after stage 11 (stage 19). No plugin code runs inside the engine for now. |
| T20 | `crates/tern-client` holds discovery, profile resolution, pid liveness, hello and reconnect, the multi-engine router, typed method wrappers, a task-wait helper and the task builders, shared by the CLI, tern-mcp and later the gateway. |
| T21 | The engine replies out of order for slow methods (`integrations.*`, `account.test`, `account.oauth.complete`, `account.discover`, `files.read`), run on a worker pool. Reads and account commands keep their per-connection order. Clients already match replies by id (JSON-RPC). |
| T22 | Tracking URLs, `trackingId`/`rid` and expiration claims are built by the engine, not the desktop. `docs/backend-plan.md` 4.2 is amended to say so. |
| T23 | Calendar and contact editing (ICS building and edits, recurrence exceptions and expansion, RSVP and counter-proposals, vCard building, VCF import/export, about 4.1k TS lines) stays in the desktop until stage 18, which moves it to the engine as `events.*` and `contacts.update`. Until then the CLI, tern-mcp and web can read events and contacts but not create, edit or RSVP. |

### 0.1 To confirm with the user

Deduplicated from the five readers. Recommended answer first.

1. **Reads (T2, T3, T4).** Structured `query` with diffing kept in the renderer, live queries
   later, renderer SQLite as the transport until parity? *Yes.* It removes SQLite from the
   renderer with about 300 changed TS lines and no change to the 120 or so call sites.
2. **Third-party queries.** Must plugins keep the arbitrary `ModelQuery` API? *Yes*: the
   allowlist is every attribute marked `queryable` in the desktop models, checked by a parity
   test, so anything a plugin can query today keeps working.
3. **Search grammar (T6).** Engine parses the query text, with the client's time zone and
   `now` for date terms, supporting a documented subset of chrono's natural-language dates?
   *Yes*, one grammar for every client; resolving dates in each client would repeat the
   logic in TS and Rust.
4. **Paging (T5).** Keep offset paging for the desktop list? *Yes*; changing `ListTabular` to
   cursors is UI work with no benefit for a local engine.
5. **Windows (T7).** Per-window engine connections instead of the main window proxying, with
   the main process owning `serve`'s stdin and `KeyManager` moving to main in stage 6?
   *Per-window, KeyManager to main.* The other choice (main asks the main window for
   account JSON and secrets over IPC) keeps secrets in the renderer and needs the main window
   open before the engine can start.
6. **Scheduled sends (T9).** Rebuild from the draft when due, or prebuild the task when
   scheduling? *Rebuild.* A scheduled draft can be edited, metadata syncs across devices,
   and a prebuilt `SendDraftTask` carries the body, which the backend may never hold.
7. **Third-party send hooks (T10).** Desktop-only and documented, with an engine callback to a
   connected client left for later? *Yes.* `clientFinalize` (T9) keeps them working for
   desktop-scheduled sends while the desktop is open.
8. **New draft methods (T10).** `draft.create` / `draft.send`, or a `finalize: true` flag on
   `SendDraftTask`? *New methods.*
9. **Send validation.** Move the warnings (attachment mentioned, salutation mismatch, empty
   template) to the engine as `draft.validate`? *Yes, in the draft stage*; the recipient
   blacklist stays in the UI.
10. **Settings (T11).** Engine-owned settings store now for the listed keys, UI prefs left in
    `config.json`? *Yes*, it is a prerequisite for the executor and rules.
11. **Rules (T12).** Same pass as the executor? *No, its own stage after settings*; it is
    larger and independent.
12. **Backend plan (T22).** Amend 4.2 so the engine builds tracking URLs and claims? *Yes.*
13. **Secrets over the socket (T13).** Write-only, never read back, peer-uid check as the
    boundary for local clients? *Yes*; otherwise the CLI can never add or reconnect an
    account. Until this is confirmed and stage 12 lands, `account.test` and `account.add`
    stay on `serve`'s stdin.
14. **Registry location.** `accounts.json` beside the database, not a table in `edgehill.db`?
    *accounts.json*, so Reset and Rebuild keep the accounts.
15. **Headless Linux without Secret Service.** AES-GCM file keyed from systemd-creds or a 0600
    key file, documented as weaker than a keyring, plaintext refused? *Yes.*
16. **Keychain layout.** One item holding the whole keyset (one prompt, as KeyManager does)?
    *Yes.*
17. **Fallbacks (T15).** Delete the C++ engine and the per-account/stdio paths before
    credentials move? *Yes.*
18. **OAuth (T14).** Client receives the redirect, engine does the exchange; register Tern's
    own Google and Microsoft client ids before the move? *Yes to both*; old refresh tokens
    stay bound to the old ids, so existing accounts keep the old id until they reconnect.
19. **Restart policy.** Engine retries a stopped account with backoff (today's CrashTracker
    rule), and stops retrying on auth failure? *Yes*; a headless engine has nobody else to
    do it. This changes phase C's "the engine never restarts an account".
20. **Service plugins (T19).** Out-of-process service plugins after stage 11 (stage 19),
    in-engine plugin code (WASM) out of scope? *Yes.*
21. **Versioning (T18).** `PLUGIN_API_VERSION` from 1.26.0, `engines.mailspring` the only
    version key, a `tern` block for new fields, no `engines.tern`? *Yes.*
22. **Gateway auth (later).** Device tokens from a local pairing flow, not backend sign-in?
    *Local pairing*; the backend stays optional.
23. **Local-only methods through the gateway (T13).** Refuse account, secret, server and
    engine-admin methods on relayed connections, with an admin scope a device can be granted
    at pairing so a headless server can still be set up from the web? *Yes.* Without the
    scope a headless engine needs the CLI over SSH for every account change.
24. **Calendar and contact editing (T23).** Keep it in the desktop, documented as Tier D,
    and move it in stage 18 after the draft pipeline? *Yes.* It is about 4.1k TS lines with
    its own tests; moving it before reads and credentials delays the parts every client needs.
25. **Resurfacing texts (stage 4).** The desktop writes the localized snooze and reminder
    bodies into `settings` on launch and on language change, and the engine falls back to
    English? *Yes*; the engine has no UI locale.

## 1. Today (summary of the readers' maps)

- **Supervision.** `MailsyncBridge` in the main window's renderer spawns and supervises
  `serve` (`src/flux/mailsync-bridge.ts:66-95`, `src/mailsync-process.ts:1011-1240`,
  `src/engine-client.ts`), so a window reload restarts the engine (V). It builds the
  account JSON with secrets in the renderer (`mailsync-bridge.ts:305-317`, from
  `KeyManager._getKeyHash` and `insertAccountSecrets`; `key-manager.ts:18` reaches
  safeStorage through `@electron/remote`) and decides which accounts to add by diffing the
  renderer's `AccountStore` (`engine-client.ts:196-209`). `account.add` with secrets is
  stdin-only and the socket refuses it (`apps/engine/src/serve.rs:8-13`) (V).
- **Reads.** Every window opens `edgehill.db` read-only with better-sqlite3
  (`src/flux/stores/database-store.ts:51-73`) and compiles queries in JS (`query.ts` 564
  lines, `matcher.ts` 365). 66 non-spec files reference `DatabaseStore` (V); about 120
  call-site lines (106 single-line matches plus multi-line chains), about 23 of them in
  `mcp-server`. One raw SQL read: `thread-counts-store.ts:31` (V). Structured search
  compiles in JS too (`matcher.ts:300-327` calls `LocalSearchQueryBackend.compile`); its
  callers are `search-query-subscription.ts:43-47`, `search-bar-util.ts:30`,
  `sidebar-related-threads.tsx:60` and mcp-server, and every one has the source string.
  Plain `.search(text)` (an FTS prefix `MATCH`, `matcher.ts:361-364`) backs contact search
  (`contact-store.ts:56-61`) (V).
- **Live updates.** Already engine deltas (`subscribe`), applied by `QuerySubscription` in
  JS; the main window rebroadcasts them to other windows over IPC (`mailsync-bridge.ts:480`)
  (V). Only the main window runs the once-only side effects: Task `onSuccess`/`onError`
  (`_onIncomingChangeRecord`, `mailsync-bridge.ts:497-516`), `ProcessState` into
  `OnlineStatusStore` and `ProcessAccountSecretsUpdated` into KeyManager (`:470-477`, both
  skipped by the rebroadcast); other windows only `trigger` (`:518-531`) (V).
- **Writes.** Only through `Actions.queueTask` to `task.queue`; the renderer writes no rows.
  `queueTask`/`queueTasks` are main-window actions (`actions.ts:112,120`), so `ActionBridge`
  forwards a composer's tasks to the main window, where `TaskQueue` and `UndoRedoStore` see
  them (V). The main process still deletes `edgehill.db*` (`src/browser/application.ts:344-371`)
  and opens it read-write for the C++ switch (`mailsync-process.ts:206-268`) (V).
- **Logic in the desktop.** Send later (`send-later/lib/main.ts:26-63`), undo send
  (`draft-store.ts:512-646` via send-later metadata), snooze (`snooze-store.ts:72-102`,
  `snooze-utils.ts:28-90`), send reminders (`send-reminders-store.ts:30-109`), the draft
  pipeline (`draft-factory.ts` 473 lines, `send-draft-task.ts:15-153`), mail rules
  (`mail-rules-processor.ts` 297, rules in localStorage), unsubscribe
  (`list-unsubscribe/lib/unsubscribe-service.ts:71-160`), new-mail detection
  (`unread-notifications/lib/main.ts`), the OAuth code exchange and provider discovery
  (`onboarding-helpers.ts:65-575`), the credential keyset (`key-manager.ts`, 346 lines) and
  the account registry (`account-store.ts`, `config.json`). Calendar and contact editing:
  `src/ics-event-helpers.ts` (1,845 lines: `createICSString`, `updateEventTimes`,
  `createRecurrenceException`, `createCounterProposal`), `src/calendar-utils.ts` (578),
  `main-calendar/lib/core/calendar-data-source.ts` (580, recurrence expansion),
  `calendar-rsvp.ts` (263), `events/lib/rsvp-target.ts` (261),
  `contacts/lib/ContactInfoMapping.ts` (314, vCard) and `VCFImportExport.ts` (261). The
  resurfacing message for snooze and reminders is built by
  `DraftFactory.createDraftForResurfacing` (`draft-factory.ts:318-341`) and sent with
  `SendDraftTask.forSending(…, {silent: true})` (`snooze-utils.ts:62-75`, localized body;
  `send-reminders-store.ts:29-37`, English body). All V.
- **The engine** only announces due expirations; its module doc says "the client acts on the
  delta" (`crates/ms-sync/src/metadata_expiration.rs:4-6`, V). Its read API takes one
  `accountId` per call, orders threads by an unindexed `lastMessageTimestamp`, and has no
  reads for events, calendars, contact groups, tasks lists or plugin metadata (V).
  `crates/ms-integrations` has no consumers (V).

Bugs found while mapping. The snooze and OAuth-bind fixes go in stage 2; the others in the
stage that touches the code:
- `snooze-store.ts:84-94` clears the expiration on every thread in the change record
  (`threads.map`), not only the due ones (`unsnooze`). A batch holding a due and a not-yet-due
  snooze drops the second one's expiration and it stays in Snoozed (V by reading, not run).
- `Package.isEngineSet()` (`src/package.ts:104`) reads `this.json.engines.mailspring` and
  throws when `engines` is missing; the call is outside the try in `package-manager.ts:112`,
  so one bad third-party `package.json` can stop activation of the packages after it (code V,
  loop effect I).
- `oauth-signin-page.tsx:139` calls `listen(LOCAL_SERVER_PORT)` with no host, so the
  redirect receiver binds every interface; only the `state` check protects it (code V).
- KeyManager keys secrets by email, not account id (`key-manager.ts:66-97`), so an account
  re-added with the same email and another host gets the old password (V by reading).

Stale docs: `docs/mcp.md` "What the desktop still has to do" lists the desktop reply-to-draft
bug, fixed in 3cbbbbbbe, and says mcp-server's install code writes the `mailspring` entries
(`docs/mcp.md:152`); it writes a `tern` entry pointing at the in-app `mcp-stdio-bridge.js`
(`claude-desktop-config.ts:17-18,47-51`). `docs/plugins-inventory.md` overcounts (Actions in 32 built-ins, not
52; DatabaseStore in 17, not 38), says phishing-detection uses `GetMessageRFC2822Task`
(message-list and thread-list do), and says plugins make no bridge calls (onboarding imports
`MailsyncProcess`). Fix both when their stages land.

## 2. End state

**The desktop owns:**
- UI: windows, menus, tray, SNI, taskbar, theme, the composer (Slate, `DraftEditingSession`,
  `DraftChangeSet`), lists and their selection, popovers, preferences panes.
- Plugin loading, `ComponentRegistry`, the four `ExtensionRegistry` registries, and the
  third-party hooks that run in a UI (T10).
- Read caches: `AccountStore`, `CategoryStore`, `ThreadCountsStore`, `MessageStore`,
  `ContactStore`, `TaskQueue`, `QuerySubscription` and friends, all fed by `query` and deltas.
- OS integration: native notifications (from `mail.new`), sounds, the dock/taskbar badge
  (summed from engine counts), `tern:` and `mailto:` handlers, default-client registration,
  autoupdate, crash reporting.
- Hand-offs that need the user's machine: opening the OAuth browser and receiving the
  redirect, the backend login window, Mailspring profile and keychain import (one-time).
- Starting a local engine from the main process (T7), and its stdin lifeline when the
  desktop spawned it; reusing a detached engine after stage 11.
- Until stage 18 (T23): calendar and contact editing (ICS, recurrence, RSVP, vCard), as
  Tier D.

**Moved to the engine:** structured reads and search; counts; send later, undo send, snooze,
send reminders; draft creation and send finalisation, signatures, built-in tracking
transforms and claims; mail rules; settings shared between clients; unsubscribe; new-mail
detection; integrations and their keys; capabilities; accounts, secrets, OAuth exchange,
discovery, test, reset, revoke, restart policy; database reset and log access; later
(stage 18) event and contact editing.

**Goes away:**
- `internal_packages/mcp-server` (4,101 lines) except a pane of about 300 lines.
- better-sqlite3 in the renderer, `database-agent.js`, the retry and query-plan code in
  `database-store.ts:216-408`, and `Query.sql()` as a transport. `sql()` is also the
  subscription key and equality check (`query-subscription-pool.ts:106`,
  `mutable-query-subscription.ts:7,14`, `contacts/lib/Store.ts:104`); those switch to
  `JSON.stringify(query.toJSON())` before `sql()` is retired, and it stays for specs until
  they move to a mock engine.
- `src/services/search/search-query-backend-local.ts` (292 lines).
- The expiration handlers in thread-snooze and send-reminders, and all of send-later's but
  the `clientFinalize` branch (about 220 lines),
  `_onSendDraft`'s delay and finalisation, the logic half of `draft-factory.ts`,
  `mail-rules-processor.ts`, `unsubscribe-service.ts`, the detection half of
  `unread-notifications`.
- `key-manager.ts`, `oauth-revocation.ts`, the token exchange and discovery in onboarding,
  secrets in account JSON and in `windowProps`, `ProcessAccountSecretsUpdated`,
  `GMAIL_CLIENT_*` in the environment, the `identity` param of `account.add`.
- The IPC delta rebroadcast, the main process's `_deleteDatabase`, `moveAsideRustDatabase`
  and the C++ engine with its fallbacks.
- Duplicates in other clients: `apps/web/src/features/composer/send-later-runner.ts`, the
  web `draft.ts`/`signature.ts`, most of `apps/mcp/src/drafts.rs`, `apps/mcp/src/search.rs`
  post-filters, and the two copies of discovery in `apps/cli` and `apps/mcp` (T20).

## 3. Engine work

Grouped by area. Each entry: shape, then the clients it unblocks (D desktop, C cli, M mcp,
W web/gateway). Params and results are sketches; names follow the existing `noun.verb` style
in `crates/ms-ipc/src/lib.rs`. All methods are additive (no `protocol` bump) and announced in
`hello.features`.

### 3.1 Protocol groundwork

- **`hello.features`** and the same list in `engine.status`: `["query", "search-grammar",
  "expiration-executor", "settings", "draft-pipeline", "rules", "integrations", "accounts",
  "files", ...]`. Clients gate behaviour on features, never on versions. D C M W.
- **`capabilities`** in `hello` and `engine.status`, plus a `capabilities.changed`
  notification: `{translate: {provider, available}, grammar: …, enrich: …, tracking:
  {available, reason}, sharing: …}`. Replaces the desktop-only decisions in
  `src/flux/stores/capability-store.ts`. D M W.
- **Out-of-order replies (T21).** A small worker pool per engine; methods tagged slow are
  answered when done. ~150 Rust. D W.
- **`crates/tern-client` (T20).** From `apps/cli/src/discovery.rs` (411), `apps/mcp/src/
  discovery.rs` (302), the two `pid_alive` and `default_config_dirs` copies and the task
  builders (`apps/mcp/src/tasks.rs`, `apps/cli/src/tasks.rs:48-80`). ~600 lines, removes
  ~800. C M W.
- **Headless `serve`.** `--detached` (no stdin lifeline, no exit 141), used by
  `tern engine start` and the launchd/systemd units. Needs 3.6 first. `serve` runs the
  `migrate` step itself before binding the socket, so a long-running engine and the desktop
  never migrate the same database; `hello` and `engine.status` carry `build` and `protocol`
  so a client can tell an older engine from the bundled one. C M W.
- **Notification routing.** Every new notification, with its filter key, backlog and dedup
  rule. Filters and the first-subscriber backlog are defined today only for deltas and
  `account.state` (engine-protocol-plan 1.1 step 3, 4.1); a connection that never calls
  `subscribe` gets none of these.

  | Notification | Filter | Backlog | Dedup |
  |---|---|---|---|
  | `settings.changed` | none (passes any `accounts` filter) | no; call `settings.get` after `subscribe` | values are absolute |
  | `capabilities.changed` | none | no; current value in `hello` / `engine.status` | absolute |
  | `account.changed` | `accountId` | no; `account.list` after `subscribe` | absolute |
  | `mail.new` | `accountId` | no: mail that arrives with no client subscribed is not notified, as with the desktop closed today (I) | carries `messageId`; a client drops repeats. Each client posts its own notification (desktop native, web only when its tab allows) |
  | `rules.progress` | the connection that called `rules.reprocess` | no | `jobId` |
  | `rules.disabled` | none | no; the rule's `disabled` and `error` are saved in `settings` | `ruleId` |
  | `query.changed` | the connection that owns `subId` | no; re-run after reconnect | `subId` |


### 3.2 Reads

- **`query`** (T2). Params:
  ```jsonc
  {"class": "Thread", "accounts": ["a1"] | null,
   "where": {"and": [{"attr": "categories", "op": "containsAny", "value": ["c1","c2"]},
                     {"attr": "inAllMail", "op": "=", "value": true}]},
   "search": {"text": "from:ann has:attachment", "grammar": true,
              "tz": "Europe/Lisbon", "now": 1792108800} | null,
   "order": [{"attr": "lastMessageReceivedTimestamp", "dir": "desc"}],
   "limit": 100, "offset": 0 | null, "cursor": "…" | null,
   "distinct": true, "include": ["body"], "idsOnly": false, "count": false}
  ```
  Result `{rows: [model JSON], count?, ids?, nextCursor?}`. Operators: `= != < > <= >= in
  "not in" contains containsAny startsWith like`, `and`/`or`/`not` (the set in
  `src/flux/attributes/matcher.ts`). Joins the desktop already uses: `ThreadCategory` (with
  `inAllMail`/`unread` on the join, and the `ThreadListCategoryIndex` /
  `...SentIndex` subselect from `query.ts:403-450`), `ModelPluginMetadata`,
  `ContactContactGroup`, `MessageBody`. Classes: Thread, Message, Folder, Label, Contact,
  ContactGroup, ContactBook, Event, Calendar, Task, File by id. No 500-row clamp on `id in`
  lists. `search` with `grammar: false` is the plain FTS prefix match of `.search(text)`
  (`matcher.ts:361-364`) and ships with `query` (stage 7), so contact search works on its
  own; `grammar: true` arrives with the search grammar (stage 9) and is refused with a clear
  error before that. New file `crates/ms-store/src/model_query.rs` on the existing
  `ReadStore`. ~1.5-2k Rust with tests. D M W.
  - Allowlist: generated from the desktop's `queryable` attributes (`src/flux/attributes/
    attribute.ts:16-34`) into a checked-in table; a test fails when the two differ.
  - Parity test: a fixture database, a list of real `ModelQuery`s captured from the desktop
    (every call site in section 1's inventory), compared between `Query.sql()` on
    better-sqlite3 and `query` on the engine.
- **Search grammar** (T6). Port `src/services/search/search-query-parser.ts`,
  `search-query-ast.ts` and the local backend (1,125 TS lines) to `ms-store`, using the
  `ThreadSearch` columns. Used by `query.search`, by `search` (which gains the grammar and
  `accounts`), the CLI and tern-mcp. `IMAPSearchQueryBackend` follows later. Date terms are
  parsed today by chrono-node's casual parser in the client's locale and time zone
  (`search-query-backend-local.ts:257-259`, `date-utils.ts:92-110`); the engine resolves
  them against the request's `tz` and `now`, never its own clock and zone, and supports a
  documented subset (ISO dates, month and weekday names, `today`/`yesterday`, `N
  days|weeks|months ago`, `last week|month|year`), grown from the desktop spec cases. A term
  outside the subset is an error, not a silent empty match. ~1.1-1.5k Rust with the dates.
  D C M W.
- **Counts.** `counts.get {accounts?}` returning every `ThreadCounts` row, plus `ThreadCounts`
  as a delta class emitted when `Thread::after_save` changes a row. Replaces the full-table
  read on every Thread delta (`thread-counts-store.ts:21-41`); `badge-store.ts` keeps the
  summing. `folders.list` and `labels.list` already compute counts (engine-protocol-plan
  1.2), so this is independent of `query` and ships in stage 1. ~200 Rust. D C W.
- **`tasks.list {accountId?, status?: ["local","remote"], class?, headerMessageId?}`** over
  `Task` (index `TaskByStatus`). Replaces TaskQueue's own load, the scan in
  `draft-store.ts:475-478` and mcp's `ensure_not_sending` (`apps/mcp/src/tools/write.rs:
  452-480`). Optional **`task.wait {accountId, taskId, timeoutMs}`** for the CLI and mcp.
  ~150-300 Rust. D C M.
- **`metadata.list {accounts?, pluginId, objectType?: "Thread"|"Message", draft?,
  expiresBefore?, hasExpiration?, after?, before?, limit}`** returning models. Unblocks
  tern-mcp `list_scheduled_sends`, `list_tracked_messages`, `get_top_links` and the
  activity plugin's scans. A special case of `query`, but cheap to ship first. ~200 Rust.
  D M W.
- **`hidden`** on Message JSON in reads (the `Message.isHidden` rule, `src/flux/models/
  message.ts:452-463`), so tern-mcp and web stop re-implementing it. ~30 Rust. M W.
- **Files.** `files.get {accountId, fileId}` → `{path?, size, contentType, downloaded}`,
  fetching the body when missing and replying when the file exists (replaces mcp's 15 s
  poll); `files.read {accountId, fileId, offset, len}` → base64 chunk; `files.put
  {accountId, name, contentType, data | path}` → `{fileId}` for composer attachments.
  `GetMessageRFC2822Task` gains a mode that returns a file id instead of writing a
  client-chosen path. ~400 Rust. M W (the local desktop keeps reading paths).
- **Live queries** (deferred, T3). `query.subscribe {query}` → `{subId}`; `query.changed
  {subId, ids, models, count}` after each flush that touches the class. ~1k Rust. W.

### 3.3 Executors and the send pipeline

- **Expiration executor** (T8, T9) in `crates/ms-sync/src/metadata_expiration.rs`. For a due
  row:
  - `send-later` on a draft: clear the metadata, finalise (below) and queue `SendDraftTask`,
    then the after-send action (`send-and-archive` queues the archive tasks from
    `send-and-archive-extension.ts:5-27`). Covers undo send as well, since undo send writes
    send-later metadata with `isUndoSend` today. Rows with `clientFinalize: true` are
    emitted as `metadata-expiration` and left to the client. On a non-draft, clear the row.
  - **Incomplete drafts.** A draft synced from another device (or not yet downloaded) can
    lack its body, its plaintext flag or a non-inline file. Before finalising, the executor
    checks for all three; if any is missing it queues the body fetch and the file downloads
    and reschedules the row (1, 2, 4, 8 minutes, then it stops and sets `error` on the
    metadata so every client can show it), instead of sending an incomplete message. This is
    the wait tern-mcp does today for `send_draft` (`docs/mcp.md:128-131`).
  - `thread-snooze`: move back by provider role (Gmail `ChangeLabelsTask`, otherwise
    `ChangeFolderTask` with `sourceFolderIds`, from `snooze-utils.ts:28-60`) when the
    thread is still in Snoozed, then mark unread or send the resurfacing message per
    `core.notifications.unsnoozeToTop` (default true, `config-schema.ts:369-372`), and clear
    the expiration of that thread only.
  - `send-reminders`: a full port of the handler, which is not expiration-only
    (`send-reminders-store.ts:39-90`). On every Thread save that carries the metadata
    (`Thread::after_save`): record the `lastReplyTimestamp` baseline when it is missing, and
    clear the metadata when `lastMessageReceivedTimestamp` differs from it (a reply arrived),
    so every client sees the reminder go when the reply lands, not only at due time. When
    due: set `shouldNotify`, clear `expiration`, send the resurfacing message. The engine also
    sets the baseline when it promotes `thread:send-reminders` at send time
    (`crates/ms-tasks/src/send.rs:610-650`, which today attaches only `{expiration,
    sentHeaderMessageId}`). Clearing `shouldNotify` when the user leaves the thread stays in
    the desktop (UI state, a metadata task).
  - **Resurfacing message** for snooze and reminders, a minimal builder in the engine (not
    `draft.create`, which is stage 13): From `{email: account.emailAddress, name:
    "<account.name> via Tern"}` so `Message.isHidden` (`message.ts:452-463`) still matches,
    To the account's own address, the thread's subject, In-Reply-To the reminder's
    `sentHeaderMessageId` or else the newest non-draft message of the thread, body from the
    `resurfacing.snoozeBody` / `resurfacing.reminderBody` settings (written localized by the
    desktop, English fallback in the engine). Queued as a silent `SendDraftTask`, as
    `draft-factory.ts:318-341` and `snooze-utils.ts:62-75` do. ~150 Rust.
  - Claims (backend-plan 3.8) are made here once a server is connected (T22).
  ~1-1.4k Rust. D M W.
- **Send finalisation** shared by the executor and `draft.send`: the From address must belong
  to the draft's account (the engine fails the task with a clear error rather than
  re-creating the draft; the composer runs `ensureCorrectAccount` on every From change,
  `account-contact-field.tsx:42`, and at send, `draft-store.ts:549`, while the send-later
  button only writes metadata, `send-later-button.tsx:77,88`, so the error case is a draft
  whose From never went through the composer, e.g. one tern-mcp created), prune inline files no longer referenced, strip zero-width spaces from
  plaintext (`send-draft-task.ts:51-79`, `draft-store.ts:584-590`), pass per-recipient
  bodies through (`send.rs:192-217` already supports them). ~300 Rust.
- **`draft.send {accountId, headerMessageId, delayMs?, after?: "send"|"send-and-archive",
  bodies?: {self, perRecipient: {email: html}}}`** → `{taskId?, scheduledAt?}`. With
  `delayMs`, the engine writes send-later `{expiration, isUndoSend: true, actionKey}` on the
  draft, as `draft-store.ts:526-530,593-594` does today, and the executor fires it: the pending send
  is in the database, survives a quit inside the undo window (the desktop sends
  `engine.shutdown` on unload and kills after 8 s; `serve` exits 141 without its parent) and
  fires at the next start, late rather than lost, and tern-mcp's `ensure_not_sending`
  (`apps/mcp/src/tools/write.rs:452-470`) keeps refusing edits during the window. `bodies`
  carries the output of third-party send hooks (T10); it is stored with the pending send
  (I: in the metadata value or a local side table, to decide in stage 13).
  **`draft.cancelSend {accountId, headerMessageId}`** clears the metadata → `{cancelled}`.
  D C M W.
- **`draft.create {accountId?, kind: "new"|"reply"|"reply-all"|"forward"|"mailto"|"resurface",
  threadId?, messageId?, mailto?, fields?}`** → Message JSON, saved as a draft. Ports
  `draft-factory.ts` (from and alias, `account.autoaddress`, `core.composing.html`,
  Message-ID, quoting with sanitize and inline styles, cid stripping, reply-draft reuse,
  default send account) and the built-in `prepareNewDraft` extensions (signature insertion,
  tracking-pixel removal from quotes). Replaces `apps/mcp/src/drafts.rs` (620) and the web
  `composer/draft.ts` (298). ~1.2k Rust. D C M W.
- **`draft.validate {accountId, headerMessageId}`** → `{errors: [...], warnings: [...]}`
  (blank subject, attachment mentioned, invalid recipients, salutation mismatch, empty
  template from `draft-editing-session.ts:277-389` and `composer-templates`). ~300 Rust.
  D C M W.
- **Tracking transforms** (T22): pixel before `.gmail_quote`, link rewrite outside it,
  `trackingId`/`rid`, per-recipient bodies, using `trackingNS`/`publicURL` from the backend.
  Inert today (`PLUGIN_URL = null`, server-only capability), so it ships with the backend
  work, not before. HTML rewriter in Rust (lol_html or html5ever, both MIT/Apache). ~500 Rust.
- **`threads.snooze {accountId, threadIds, until}`** and **`threads.unsnooze`**: the task
  pair plus metadata in one call, for the CLI and mcp. ~150 Rust. C M W.
- **`messages.unsubscribe {accountId, messageId, method?}`** → `{method: "one-click"|"mailto"|
  "web", ok, url?}`. The engine POSTs RFC 8058 one-click itself or sends the mailto message;
  the client only opens web links. Uses `listUnsubscribe`/`listUnsubscribePost` already
  parsed (`crates/ms-mime/src/parse.rs:117-119`). ~300 Rust. D C M W.
- **`threads.share {accountId, threadId, shared}`** with B2. W D.

### 3.4 Settings and rules

- **`settings.get {keys?}`** → `{values, storeId?}`, **`settings.set {values}`**, notification
  **`settings.changed {values}`**. Stored by the engine at `<config>/settings.json`, written
  atomically. `storeId` is new whenever the engine starts the store from nothing (no file, or
  a corrupt one set aside), so a client that copied its values in once copies them again. Keys: `core.sending.undoSend`, `core.sending.defaultAccountIdForSend`,
  `core.notifications.unsnoozeToTop`, `core.composing.html`, `signatures`,
  `defaultSignatures`, `templates`, `mailRules`, `mailRulesAutoSince`,
  `resurfacing.snoozeBody`, `resurfacing.reminderBody`. ~400 Rust. D C M W. On the desktop
  the main process is the one settings client (T11): its `ConfigPersistenceManager` holds
  the values, so a window's synchronous `AppEnv.config.get` never sees defaults while a
  `settings.get` is in flight.
- **Rules** (T12). `rules.list` / `rules.set` are `settings` keys; **`rules.reprocess
  {accountId}`** → `{jobId}`; notifications **`rules.disabled {ruleId, error}`** and
  **`rules.progress {jobId, done, total}`**. Evaluation on the `rulesReady` save
  (`crates/ms-store/src/models/message.rs:536-541`), ported from `mail-rules-processor.ts`
  and `mail-rules-templates.ts`; the forward action uses `draft.create` + `draft.send`.
  ~1k Rust. D C M.

### 3.5 Notifications

- **`mail.new {accountId, threadId, messageId}`**, once per message that lands unread in the
  inbox after the first headers sync (the rule in `unread-notifications/lib/main.ts:36-53,
  255-300`). Ships in stage 1; filter, backlog and dedup in 3.1. Optional `reminder.due` and
  `thread.unsnoozed`. ~200 Rust. D C W.

### 3.6 Accounts, secrets and engine lifecycle

- **SecretStore** (new `crates/ms-secrets`): macOS Keychain generic password, service "Tern
  Engine" (`security-framework` is already in `Cargo.lock`); Linux Secret Service (the
  `secret-service` crate, MIT/Apache); fallback AES-256-GCM file `secrets.enc` (`ring`)
  keyed from `$CREDENTIALS_DIRECTORY` or `TERN_ENGINE_KEY_FILE` (0600). Refuses plaintext.
  Windows DPAPI later. One item holding the keyset. Names `account:<id>:imap|smtp|
  refresh-token`, `integration:<provider>`, `tern-server-token`. ~600 Rust.
- **Registry**: `<config>/accounts.json` (non-secret Account JSON, 0600, atomic). `serve`
  starts every registered account itself. Rotated OAuth tokens are persisted to the store
  (no more `ProcessAccountSecretsUpdated`). ~400 Rust.
- **Methods** (socket, owner = same uid, all in the local-only class of T13 except
  `account.list` and `account.discover`):
  - `account.list` → full non-secret Account JSON plus state; `account.changed {account}`.
  - `account.discover {email, provider?}` → IMAP/SMTP template (MX, ISPDB and mailcore
    tables, Thunderbird autoconfig; tables in `crates/ms-providers`). ~500 Rust + data.
  - `account.oauth.begin {provider, sharedMailbox?}` → `{sessionId, authUrl, redirectUri}`;
    `account.oauth.complete {sessionId, code, state}` → `{account}` (exchange, userinfo or
    Graph `/me`, discovery, test, store). ~400 Rust.
  - `account.test {account, secrets}` → the result `--mode test` prints today
    (`apps/engine/src/modes/test_auth.rs:44`, refactored to return it). On `serve`'s stdin
    from stage 10, called through the main-process supervisor; on the socket only from
    stage 12, once T13 is confirmed.
  - `account.add {account, secrets, test?}`, `account.setSecrets {accountId, secrets}`
    (reconnect), `account.update {accountId, patch}` (changing an IMAP/SMTP host or username
    clears the stored secrets), `account.reorder {ids}`, `account.remove {accountId, revoke?,
    deleteData?}` (Google revoke from `src/oauth-revocation.ts`), `account.reset {accountId}`.
  - `secrets.set {name, value}`, `secrets.delete {name}`, `secrets.has {names}` → `{name:
    bool}`. No `get`.
  - `server.set {url, userId, serverId}` / `server.clear`; the engine builds its own
    `TernServer`, so `account.add`'s `identity` goes.
  - `credentials.import {accounts, secrets}` on serve's **stdin** only (migration, section 5).
  - Account id derived in Rust byte for byte as `idForAccount`
    (`onboarding-helpers.ts:31-44`), with a test against vectors produced by the JS.
  ~2-2.5k Rust in total with the store and registry. D C M W.
- **Restart policy**: a stopped account is retried with backoff (5 crashes in 5 minutes stops
  it, as `engine-client.ts:60-120` does); `account.state` gains `retryAt` and `authFailed`;
  auth failures are classified in the engine, not by string matching in the desktop. ~200
  Rust.
- **Lifecycle**: `engine.resetDatabase` (stop accounts, delete `edgehill.db*`, restart),
  `engine.logs.tail {accountId?, bytes}`, `engine.setLogLevel {verboseUntil}`. ~200 Rust.

### 3.7 Integrations

- `integrations.translate | detect | languages | grammar | enrich | test` per
  `docs/integrations-plan.md` section 4, error code `-32010`, on the out-of-order pool
  (3.1). `apps/engine/Cargo.toml` gains `ms-integrations`; `apps/engine/src/
  integrations_api.rs` routes. Keys come in `params.config` until 3.6, then from the store,
  configured with `secrets.set {name: "integration:<provider>"}` and the non-secret provider
  config in `settings`. ~400-550 Rust. D M W.

### 3.8 Calendar and contacts (stage 18, T23)

- **`events.expand {calendarIds, start, end, tz}`** → occurrences, replacing the expansion in
  `calendar-data-source.ts` and `calendar-utils.ts`. **`events.create`**, **`events.update
  {eventId, patch, scope: "this"|"following"|"all"}`** (sequence bumps, recurrence exceptions,
  exclusion dates, attendee edits, from `ics-event-helpers.ts`), **`events.rsvp {eventId,
  status, comment?, counter?}`** (from `calendar-rsvp.ts` and `rsvp-target.ts`), each
  building the ICS and queueing the existing `SyncbackEventTask` / `EventRSVPTask`.
  **`contacts.update {contactId, patch}`** building the vCard (`ContactInfoMapping.ts`),
  **`contacts.import {vcf}`** / **`contacts.export {ids}`** (`VCFImportExport.ts`). Rust
  `icalendar` and `rrule` crates (MIT/Apache, I to check). ~2.5-3k Rust (I). D C M W.

## 4. Desktop work

### 4.1 Reads off SQLite

1. **Engine connection per window** (T7, stage 6). `EngineSupervisor` in `src/browser/`
   takes over `EngineServeProcess` and the respawn half of `EngineClient`, and owns `serve`'s
   stdin:
   - `KeyManager` moves to the main process (it already calls safeStorage through
     `@electron/remote`); the renderer keeps a thin IPC facade with the same methods until
     KeyManager goes (section 5 step 6). `ProcessAccountSecretsUpdated` from `serve`'s stdout goes straight
     to it.
   - The account list comes from `config.json` `accounts`, which the main process already
     persists; the `ensure()` diff (`engine-client.ts:196-209`) runs on config changes in
     main. `account.add` builds its JSON there (today's `mailsync-bridge.ts:305-317`).
   - Account-state callbacks (`onAccountLaunched`, `onAccountExited`, `onTooManyFailures`)
     go to the main window over IPC, which updates `AccountStore` and `OnlineStatusStore` as
     now.
   - stdin-only calls from other windows (onboarding's `account.test` from stage 10, the
     migration's `credentials.import`) go over IPC to the supervisor.
   - It always spawns its own engine; a live engine at `run/engine.json` from an earlier
     instance is waited out (exit-3 retries, as today), and so is the startup `migrate`.
     Reuse waits for stage 11.
   Each renderer opens its own socket connection through a small `EngineConnection` (the
   client half of today's `engine-client.ts`), calls `hello` and `subscribe`, and only
   triggers `DatabaseStore`. Task `onSuccess`/`onError` (send sound, error dialogs,
   `draftDeliverySucceeded`, `onSendSuccess` hooks) and `ProcessState` handling stay in the
   main window, so they fire once. `Actions.queueTask(s)` keeps going through `ActionBridge`
   to the main window; windows don't call `task.queue` themselves. The main process opens a
   socket connection of its own for `settings` (T11). `_onIncomingRebroadcastMessage` and
   the IPC relay go for `serve`. The C++ engine and the per-account and stdio modes have no
   socket for other windows, so they stay in the main window's `MailsyncBridge` with the
   rebroadcast until stage 10 deletes them.
2. **Compat transport** (stage 8). `DatabaseStore._query` / `run` serialise the `ModelQuery`
   (`Query.toJSON()`, new, next to `sql()`) and call `query`. `inflateResult` builds models
   from the returned JSON as now. `_query` is already async, so the call sites, the pool,
   `QuerySubscription`, `MutableQuerySubscription`, `UnreadQuerySubscription`,
   `ObservableListDataSource` and every store built on them are unchanged. Behind
   `core.engine.reads: "sqlite" | "engine"`. The SQLite transport is used whenever `hello`
   is missing or lacks `query` (the C++ engine, per-account and stdio modes) whatever the
   flag says. The default flips to `engine` once the parity tests pass on the trial profile
   and stage 9 has shipped, since search needs the grammar.
   **Reconnect contract.** Today reads keep working while the engine respawns or has given
   up, because each window opens `edgehill.db` itself; over the socket they don't. So: a read
   issued while the connection is down waits for it, up to 10 s, then rejects with an
   `EngineUnavailable` error; on reconnect `EngineConnection` re-sends `hello` and
   `subscribe`, `QuerySubscriptionPool` re-runs every live subscription and
   `ThreadCountsStore` re-reads `counts.get`, since deltas were missed (engine-protocol-plan
   1.4 already says the desktop re-reads what it shows). While down, and after
   `engineGaveUp`, the windows show the engine state with a Try Again action instead of
   empty lists. Respawn backoff (0-8 s), the exit-3 loop (up to 30 x 1 s) and give-up are in
   `engine-client.ts:463-499`.
3. **Counts** (stage 1). `ThreadCountsStore` reads `counts.get` once and applies
   `ThreadCounts` deltas.
4. **Search** (stage 9). Built-in call sites already hold the source string; they call a new
   `Query.searchText(text)` (`query.search {text, grammar: true, tz, now}`). Third-party
   `structuredSearch(ast)` keeps working through a printer back to the grammar, a visitor
   on the existing `SearchQueryExpressionVisitor` (`search-query-ast.ts:1`, ~80 lines, I),
   and is deprecated. Plain `.search(text)` maps to `grammar: false` from stage 8.
5. **Delete** the SQLite path, `database-agent.js`, the retry code and better-sqlite3 from
   the renderer bundle once `engine` has been the default for a release and stage 10 has
   removed the C++ engine and the per-account and stdio fallbacks (none of them answer
   `query`, and per-account processes can't page a unified inbox across accounts). Specs
   move to a mock engine that answers `query` from fixtures (`database-store.ts:75-81` opens
   `edgehill.test.db` today). Subscription keys move to `JSON.stringify(query.toJSON())`
   first (section 2).
6. **Call-site moves** are optional after that: `thread-list-data-source.ts`'s per-thread
   subscriptions and the bulk message read are the hot path and the first candidates for a
   single `query` with `include`.

### 4.2 Plugin logic

- **Send later, undo send, snooze, reminders.** When `hello.features` has
  `expiration-executor`:
  - send-later's handler (`send-later/lib/main.ts:26-63`, registered at `:69-71`) stays
    registered and acts only on rows with `clientFinalize: true`; the executor sends the
    rest. Without the handler those rows would never fire.
  - thread-snooze's `_onMetadataExpired` (`snooze-store.ts:72-102`) and send-reminders'
    `_onDatabaseChanged` (`send-reminders-store.ts:39-90`) are not registered; the engine
    does the whole job, including the reminder baseline and the clear on reply (3.3).
    send-reminders' focus handler (`:94-111`) stays.
  - Buttons and popovers stay and still write metadata. `_onSendDraft` and the send-later
    button set `clientFinalize: true` per T9. The web's `send-later-runner.ts` checks the
    same feature.
  Without the feature (older engine) all three handlers run as today; the snooze batch bug
  is fixed in stage 2, before this.
- **Send pipeline.** With `draft-pipeline`: the composer's Send calls `draft.validate`, shows
  the warnings, runs third-party `applyTransformsForSending` if any are registered and passes
  the result as `bodies`, then calls `draft.send {delayMs: core.sending.undoSend}`. Undo
  calls `draft.cancelSend`. `SendDraftTask.forSending` and the delay half of `_onSendDraft`
  go; `onSendSuccess` hooks run from the Task delta as they do now.
- **Draft creation.** `DraftFactory` methods call `draft.create` and keep their signatures;
  third-party `prepareNewDraft` hooks run on the result before the session opens. The
  signature and pixel-removal extensions are removed (the engine does both).
- **Tracking and activity.** The composer toggles keep writing metadata; the engine
  transforms (3.3). The activity plugin's scans become `metadata.list`.
- **Rules.** `MailRulesStore` becomes an editor over `settings` (`mailRules`) and a
  `rules.reprocess` caller; `mail-rules-processor.ts` goes. One-time move of
  `MailRules-V2` and `MailRules-Auto-Since` from localStorage to `settings.set`.
- **Settings** (T11). The engine-owned key list (3.4) lives in the main process's
  `ConfigPersistenceManager`. At startup it calls `settings.get` before it sends config to
  windows, merges the values into the settings it pushes (`on-config-reloaded`,
  `config.ts:343-347`), applies `settings.changed`, and forwards renderer writes of those
  keys to `settings.set`. It keeps writing them to `config.json` too, so a C++ or
  per-account engine, or a downgrade, still finds them. Renderers and plugins see no change:
  `AppEnv.config.get` stays synchronous. One-time push of the current values on the first
  launch with `settings`; after that the engine wins on conflict. The desktop also writes
  the localized `resurfacing.*` texts on launch and on language change.
- **Unsubscribe.** `list-unsubscribe` calls `messages.unsubscribe` and opens a web link when
  told to. Detection of the link in the body stays a UI concern only if the engine can't do
  it from headers (I: the body fallback in `main.tsx:76-108` should move too).
- **Notifications.** `unread-notifications` listens for `mail.new`; throttling, summaries
  and the native notification stay.

### 4.3 MCP pane

- Delete `internal_packages/mcp-server` (T17). Its HTTP server on port 2587 and
  `claude-desktop-config.ts` go with it. That file writes a `tern` entry (`:17-18`) pointing
  at `mcp-stdio-bridge.js` in `app.asar.unpacked` (`:47-51`), so after the deletion users'
  Claude Desktop configs point at a missing file.
- `tern-mcp install` treats a `tern` entry whose args end in `mcp-stdio-bridge.js` as its
  own stale entry and replaces it without `--force` (today it refuses any `tern` entry that
  doesn't run tern-mcp, `apps/mcp/src/install.rs:7-9`). The desktop runs it once after the
  upgrade when such an entry exists, so the detection stays in tern-mcp.
- tern-mcp is bundled and signed with the desktop: `build/build.js:249-250` ships only
  `mailsync-rs` today, so the build gains `tern-mcp` in the same places (asar unpack,
  signing, notarisation, Linux packages).
- New pane in `preferences` (~300 lines): edits `<config>/mcp.json` (per-account access,
  the policy tern-mcp reads), shows the tail of `mcp-audit.log`, and runs the bundled
  `tern-mcp install` for Claude Desktop and Claude Code, showing its output. `core.mcp.*`
  config keys are migrated into `mcp.json` once and then ignored.

### 4.4 Integrations on serve

- `Integrations.setTransport` (`src/integrations.ts:407`) is wired to `Engine.request`.
  Composer and thread-popout windows call it on their own connection (stage 6).
- `CapabilityStore` reads `capabilities` from `hello` and `capabilities.changed` instead of
  decrypting the keyset; until 3.6 it keeps sending the key per request, read from
  KeyManager (integrations-plan I3), then sends none.
- Preferences > Integrations saves keys with `secrets.set` once `accounts` is in the
  features.

### 4.5 Onboarding and accounts

- `AccountStore` becomes a cache of `account.list` and `account.changed`; `Actions.
  updateAccount` calls `account.update`; `syncState`/`syncError` come from `account.state`.
- Stage 10: `serve` starts at app launch with zero accounts (today the desktop spawns it
  only once `AccountStore` has an account, `engine-client.ts:196-209`), so first-run
  onboarding has an engine. Onboarding's test (`onboarding-helpers.ts:429`, a
  `MailsyncProcess` in the onboarding window) becomes `account.test` on `serve`'s stdin,
  sent by the main-process supervisor over IPC.
- Stage 12: onboarding calls `account.discover`, `account.oauth.begin`/`complete`,
  `account.test`, `account.add` on its own socket connection. The redirect receiver moves
  to the main process, still bound to `127.0.0.1` (the bind itself is fixed in stage 2),
  forwarding `code` and `state`.
- Reconnect (`preferences-account-details.tsx:161-165`, `account-error-notif.tsx:52-56`)
  passes only `{accountId}`; new secrets go to `account.setSecrets`.
- Remove calls `account.remove {revoke: true, deleteData: true}`; Reset calls
  `account.reset`; "Rebuild database" calls `engine.resetDatabase`; logs come from
  `engine.logs.tail`; verbose logging calls `engine.setLogLevel` instead of relaunching.

## 5. Credentials and headless

**End state.** The engine owns the account registry (`accounts.json`) and every secret
(SecretStore). Clients send secrets write-only and never get them back; `account.list`,
deltas, `hello`, logs and crash reports never carry them. A headless engine (`serve
--detached` under launchd/systemd) starts its registered accounts itself, refreshes and
persists tokens, retries stopped accounts, and runs the executors. The desktop holds nothing
secret after migration.

**Why write-only over the socket is acceptable.** The peer-uid check (S6) is the same
boundary as stdin: a process that can reach the socket can already queue sends. Read-back is
what would break macOS's per-app keychain protection, and there is none. Changing a host or
username clears the stored secrets, so a same-uid process can't redirect a stored password
to another server. The argument holds for local clients only: `apps/gateway` is a same-uid
client that relays remote callers (`docs/architecture.md`), so the local-only class (T13)
is refused on relayed connections unless the device holds the admin scope.

**Migration from KeyManager.**
1. Prerequisite: T15 (no C++ engine, no per-account or stdio paths), so only `serve` needs
   credentials.
2. `hello` / `engine.status` report `credentials: {store: "keychain"|"secret-service"|"file",
   accounts: n}`. On launch the supervisor compares with `config.json` `accounts`.
3. If the engine lacks accounts, the supervisor reads the keyset through `KeyManager.
   _getKeyHash()` (in the main process since stage 6; it still runs the 1.23 legacy path)
   and sends `credentials.import
   {accounts: [Account JSON with secrets], secrets: {integration-*, tern-server-token}}` on
   serve's stdin.
4. The engine maps `<email>-*` keys to every account with that email by account id, writes
   `accounts.json` and the keychain item, and answers `{imported: [ids], failed: [...]}`.
5. The desktop sets `credentialsMovedToEngine`, renames `credentials` to
   `credentialsBeforeEngine`, keeps `accounts` read-only for one release (downgrade), and
   switches `AccountStore` to the engine.
6. One release later: delete `credentialsBeforeEngine`, `key-manager.ts`,
   `oauth-revocation.ts`, the `KeyManager` export (`src/global/mailspring-exports.js:200`,
   `.d.ts:236-237`), and the secret handling in onboarding, preferences and notifications.
7. CLI-only and headless installs have nothing to migrate; they use `tern account add`.

**Mailspring keychain migration.** `src/mailspring-keychain-migration.ts` (191 lines) reads
Mailspring's "Mailspring Safe Storage" secret and decrypts its keyset. It stays in the
desktop: only a desktop install has a Mailspring profile, and it is read-only towards
Mailspring's files and keychain item. After the migration stage it feeds
`credentials.import` directly (Mailspring profile import → decrypted keyset →
`credentials.import`) instead of writing the Tern safeStorage keyset first. It is deleted
when Mailspring import is dropped.

**Signing.** Keychain items created by the bundled engine stay readable across updates
signed with the same Developer ID. Unsigned dev builds and the unsigned dist/Homebrew
binaries prompt on every rebuild, and a launchd engine can't answer a prompt; dev builds use
the file store via `TERN_ENGINE_KEY_FILE`.

## 6. Plugin API

**Guarantees.**
- Tier A, pure UI, unchanged: `ComponentRegistry` and all roles and locations, the
  MessageView, ThreadList and AccountSidebar extensions, the component kit (77 components),
  `WorkspaceStore`, `FocusedPerspectiveStore`, `FocusedContentStore`, `PopoverStore`,
  `ModalStore`, `PreferencesUIStore`, themes, keymaps, menus, `localized`, utils, `Actions`
  UI actions.
- Tier B, same API on an engine transport: `DatabaseStore` reads and `listen`,
  `DatabaseChangeRecord`, the query classes, every model, `Actions.queueTask(s)`,
  `TaskQueue`, `TaskFactory` and the 26 Task classes, the stores built on reads. A query that
  works today keeps returning the same rows (parity test, 3.2).
- Tier C, logic moved, UI kept: send later, snooze, reminders, rules, search, built-in send
  transforms, integrations. Their exports (`MailRulesProcessor`, `IMAPSearchQueryBackend`)
  are deprecated.
- Tier D, desktop-only and documented as such: `ComposerExtension` editor hooks,
  `DraftEditingSession`, third-party `prepareNewDraft`, `applyTransformsForSending`,
  `needsPerRecipientBodies`, `warningsForSending`, send actions, `UndoRedoStore`,
  `GrammarCheckPluginAPI`. Until stage 18 also calendar and contact editing (T23): events
  and contacts created or edited from the CLI, tern-mcp or web need stage 18.

**New.** `Engine` export: `Engine.request(method, params)`, `Engine.subscribe(filter, cb)`,
`Engine.features`. Plugins reach the read API, `integrations.*`, `metadata.list` and anything
added later through it, instead of importing `MailsyncProcess`.

**Versioning.**
- `PLUGIN_API_VERSION` (semver) exported as `apiVersion`, starting at 1.26.0; minor bumps for
  additions (1.27.0 adds `Engine`, `apiVersion`), major only when a deprecated export is
  removed.
- `Package.isEngineSet()` becomes `semver.satisfies(PLUGIN_API_VERSION, engines?.mailspring)`
  inside the try (fixes the crash in section 1). `semver` (ISC) becomes a direct dependency.
- Optional manifest block `"tern": {"engineFeatures": [...], "capabilities": [...],
  "service": {...}}`. A plugin whose features or capabilities are missing is listed but not
  activated, with the reason in Preferences > Plugins.
- `deprecate(name, {since, replacement, removeIn})` for exports in
  `mailspring-exports.js`. The lazy getters there (`:13-28`) replace themselves with a plain
  value on first access, so a deprecated export instead gets a getter that stays a getter:
  it caches the value, walks the stack to find the calling package's directory, and warns
  once per package, recorded in Preferences > Plugins. At least one minor release or six
  months between deprecation and removal.

**What breaks for third-party plugins, and when.**
| Stage | Change | Effect |
|---|---|---|
| 2 | semver check | plugins with an unsatisfiable range stop loading; with `*` or `>=1.x` nothing changes |
| 4 | engine executes expirations | none, as long as their drafts are marked `clientFinalize` (T9); plugins that listened for `metadata-expiration` of the built-in ids stop getting them |
| 8 | reads over the engine | private `DatabaseStore._query` with raw SQL fails; queries on non-`queryable` attributes fail with an error instead of a SQL error; reads during an engine restart wait up to 10 s, then reject (4.1 step 2). No change to tasks: `Actions.queueTask.listen` in the main window still sees every desktop window's tasks through `ActionBridge`, and never CLI or MCP tasks (use Task deltas for those) |
| 9 | search in the engine | `structuredSearch(ast)` deprecated in favour of `Query.searchText`; date terms outside the documented subset fail |
| 13 | draft pipeline in the engine | send hooks don't run for CLI, MCP or engine-executed sends (documented) |
| 2.0.0 | removals | `MailsyncProcess`, `KeyManager`, `MailRulesProcessor`, `IMAPSearchQueryBackend`, `QuerySubscriptionPool` internals, `MailspringTestUtils` |

Plugins can't define new Task classes (the engine rejects unknown `__cls`), which is already
true today.

**Background logic.** Service plugins (T19, stage 19): a package may declare `tern.service`
(a binary speaking the engine protocol); Preferences offers "install service", which runs
`tern service install <package>` to write a user unit (launchd LaunchAgent, systemd
`--user`) started after the detached engine. Their due work comes from
`metadata-expiration` for their plugin id, new mail from `mail.new`, and their lists from
`metadata.list`. Until stage 19, third-party background logic runs in the desktop as
today, and only while it is open.

## 7. Stages

Each stage is one implementation workflow: engine part, desktop part, tests, trial check on
`~/tern-trial`. Stages ship independently; a feature flag in `hello.features` lets the
desktop run against an engine without it. Sizes are rough.

| # | Stage | Depends on | Unblocks | Size |
|---|---|---|---|---|
| 1 | Client crate, `hello.features`, `tasks.list`, `metadata.list`, `hidden`, `counts.get` + `ThreadCounts` delta, `mail.new` | — | D C M W | ~1.4k Rust |
| 2 | Desktop cleanups: MCP pane, mcp-server removed, tern-mcp bundled and signed, stale-entry replacement in `tern-mcp install`, plugin semver check, snooze batch fix, OAuth receiver on `127.0.0.1` | — | D M | ~450 TS, −3.8k, ~50 Rust, build and signing |
| 6 | Engine supervisor and `KeyManager` in main, per-window connections | — | D | ~600 TS moved |
| 3 | Engine settings store, mirrored by the main process | 1, 6 | D C M W | ~400 Rust, ~200 TS |
| 4 | Expiration executor, send finalisation, resurfacing message | 1, 3 | D M W | ~1.4k Rust, −220 TS |
| 5 | Integrations on serve + `Engine` export + capabilities | 1, 6 | D M W | ~600 Rust, ~250 TS |
| 7 | `query` compiler (with plain FTS search) + parity tests | 1 | D M W | ~2k Rust |
| 8 | DatabaseStore on `query`: flag and reconnect contract; default flip; SQLite removed | flag 6, 7; flip 9; removal 10 | D | ~300 TS, −600 |
| 9 | Search grammar in the engine, with `tz`/`now` dates | 7 | D C M W | ~1.3k Rust, −300 TS |
| 10 | Delete C++ engine and fallbacks; lifecycle methods; `account.test`/`reset` on stdin through the supervisor; `serve` starts with zero accounts | 6 | D C | ~300 Rust, −1k+ |
| 11 | Account registry, SecretStore, import, restart policy, `--detached`, migrate inside `serve`, engine reuse | 10 | D C M W | ~1.6k Rust, ~300 TS |
| 12 | Onboarding through the engine (discover, OAuth, add, reconnect, remove) | 11, 0.1 item 13 | D C W | ~1k Rust, −500 TS |
| 13 | Draft pipeline: `draft.create`, `draft.send`, `draft.validate` | 3, 4, 6 | D C M W | ~1.8k Rust, −700 TS |
| 14 | Mail rules in the engine | 3, 13 | D C M | ~1k Rust, −300 TS |
| 15 | `messages.unsubscribe`, `threads.snooze` | 1 | D C M W | ~450 Rust |
| 16 | Files methods | 1 | M W | ~400 Rust |
| 17 | Tracking transforms + claims (with the backend) | 13 | D W | ~500 Rust |
| 18 | Calendar and contact editing in the engine (3.8) | 7, 13 | D C M W | ~2.5-3k Rust, −4k TS |
| 19 | Service plugins: `tern.service`, `tern service install`, user units | 11 | D | ~400 Rust, ~150 TS |
| later | `query.subscribe`, `apps/gateway` with the local-only class (T13) | 7, 11, 16 | W | ~1k + 2k Rust |

Stage numbers are names, not order; 6 is listed before 3 because 3, 5 and 13 need engine
calls from composer and thread-popout windows (signatures, `core.sending.undoSend`,
`core.composing.html` in the composer; grammar check, translation and the `Engine` export
in composer and popout windows, per their `windowTypes`; the composer's Send), and before
stage 6 only the main window has a connection. Stage 6 has no engine work.

Order: 1, 2 and 6 first (no dependencies; they fix real gaps for CLI and MCP and give every
window a connection). 3 and 4 next: the executor fixes scheduled sends needing the window
and the double-fire risk between clients, which grows with every new client. 5 can run
beside 4, 15 and 16 any time after 1. 7 to 9 move reads; 8 flips its default after 9 and
deletes SQLite after 10. 10 to 12 move credentials, which is the prerequisite for headless.
13, 14, 17 and 18 finish the logic; 19 after 11.

**1. Client crate, small reads, counts, new mail.**
- Engine: `crates/tern-client`; `hello.features`; `tasks.list`; `metadata.list`; `hidden`;
  `counts.get` and the `ThreadCounts` delta (3.2); `mail.new` (3.5) with the routing rules
  of 3.1.
- Desktop: `ThreadCountsStore` on `counts.get` and the delta (removes the only raw SQL read,
  `thread-counts-store.ts:31`); `unread-notifications` on `mail.new` behind the feature.
- CLI and mcp move to `tern-client`; tern-mcp ports `list_scheduled_sends`,
  `list_tracked_messages`, `get_top_links`, and `ensure_not_sending` uses `tasks.list`;
  `tern watch` on `mail.new`.
- Tests: `ms-store` unit tests per filter; `tests/socket.rs` for each method; a counts delta
  after a flag change; `mail.new` once per message and not during the first headers sync;
  the CLI and mcp suites unchanged.
- Trial: `tern tasks list`, the three MCP tools against the trial profile; mark a thread
  read and watch the badge; a new-mail notification.

**2. Desktop cleanups.**
- Delete `internal_packages/mcp-server`; new MCP pane (4.3); `core.mcp.*` migrated to
  `mcp.json` once.
- Bundle and sign `tern-mcp` with the app (`build/build.js:249-250`, signing and
  notarisation, Linux packages). `tern-mcp install` replaces a stale `tern` entry that points
  at `mcp-stdio-bridge.js`; the desktop runs it once after the upgrade when one exists.
- `Package.isEngineSet()` semver check inside the try; `apiVersion` export.
- Snooze batch fix: `snooze-store.ts:84-94` clears the expiration of `unsnooze`, not
  `threads`.
- `oauth-signin-page.tsx:139` binds the redirect receiver to `127.0.0.1`.
- Update `docs/mcp.md` (reply bug fixed; `:152` says `mailspring` entries) and
  `docs/plugins-inventory.md` (stale items, section 1).
- Tests: package-manager spec with a package lacking `engines`; pane spec writing
  `mcp.json`; snooze spec with a due and a not-due thread in one batch; tern-mcp install
  test replacing a bridge entry without `--force` and still refusing a foreign one.
- Trial: on a copy of a profile that had the old plugin's entry, upgrade and check Claude
  Desktop's `tern` entry runs tern-mcp; install from the pane; run one tool from Claude Code.

**6. Supervisor and connections.**
- Desktop only, as 4.1 step 1: `EngineSupervisor` in main as the one owner of `serve`'s
  stdin; `KeyManager` in main with an IPC facade in the renderer;
  `ProcessAccountSecretsUpdated` handled in main; account list from `config.json`; account
  state to the main window over IPC; one connection per window; Task handlers,
  `OnlineStatusStore` and `queueTask` routing unchanged in the main window; IPC rebroadcast
  removed. No engine reuse. The exit-3 retry loop stays for a reload during shutdown. The
  startup `migrate` (`application.ts:97-101`) waits for a live engine's pid from
  `run/engine.json` to exit first.
- Tests: spec on respawn and on a main-window reload; a composer window gets deltas on its
  own connection; one send sound and one error dialog per task with two windows open;
  a token rotation reaches KeyManager in main.
- Trial: reload the main window, check the engine pid doesn't change; open a composer
  window and see deltas arrive; send from the composer and hear one sound.

**3. Settings store.**
- Engine: `settings.get/set/changed`, `<config>/settings.json`.
- Desktop: the main process mirrors the engine-owned keys (4.2 Settings); one-time push;
  signatures and default signatures included; config.json kept in step for engines without
  `settings`.
- tern-mcp reads the default sender and aliases from `settings` and `account.list` instead of
  `config.json` (`apps/mcp/src/config.rs:52-76`) where the keys are engine-owned.
- Tests: engine round-trip and notification; main-process spec with a mock engine (a
  composer window reads a signature synchronously at boot); a C++ engine run still sees the
  keys in `config.json`.
- Trial: change undo-send delay in Preferences, see it in `tern settings get`; change it with
  `tern settings set` and see the composer pick it up.

**4. Expiration executor.**
- Engine: executor branches (3.3), the incomplete-draft wait, send finalisation, the
  resurfacing message builder, the reminder baseline and reply clear on Thread save;
  feature `expiration-executor`.
- Desktop: send-later's handler filtered to `clientFinalize` rows; snooze and reminder
  handlers off with the feature; `clientFinalize` marking; localized `resurfacing.*` texts
  written to `settings`. Web: `send-later-runner.ts` gated. tern-mcp: `schedule_send` text no
  longer says the desktop must run.
- Tests: harness scenarios for a due send-later draft, undo-send with the window closed
  before it fires, an undo-send whose engine quit inside the window and fires at the next
  start, a header-only scheduled draft with an attachment (fetched, then sent once), snooze
  back to inbox (Gmail and IMAP) with `unsnoozeToTop` on and off, a reminder with and without
  a reply (the reply clears it before due), the resurfacing message hidden by
  `Message.isHidden`, a `clientFinalize` row emitted and sent by the desktop handler, two
  snoozes in one batch with one not due.
- Trial: schedule a send two minutes out, quit the window but keep the engine
  (`tern engine status`), check it arrives once; snooze and reminder by hand, in a non-English
  UI language once.

**5. Integrations on serve.**
- Engine: `integrations.*`, out-of-order pool, `capabilities` in `hello`.
- Desktop: `Engine` export; `Integrations.setTransport` in every window; `CapabilityStore`
  on capabilities; key still per request, read through the KeyManager facade.
- Tests: engine against `ms-integrations`' mock server; desktop spec on the transport.
- Trial: translate a message with LibreTranslate in a popout, grammar check a draft in a
  composer window.

**7. Query compiler.**
- Engine: `query`, the allowlist table, `search` with `grammar: false`.
- Desktop: `Query.toJSON()`; a capture of the queries from every call site (section 1) for
  the parity fixtures.
- Tests: parity suite (desktop SQL vs engine on one fixture db), including the
  `ThreadListCategoryIndex` plan (`EXPLAIN QUERY PLAN` uses the index) and contact search.
- Trial: none in the UI; run the parity suite against a copy of the trial database.

**8. DatabaseStore on the engine.**
- Desktop: transport switch and reconnect contract (4.1 step 2), SQLite whenever `hello`
  lacks `query`, mock engine for specs, subscription keys on `toJSON`. The default flips
  after stage 9; the SQLite path is deleted a release later and not before stage 10.
- Tests: the existing store and subscription specs on the mock engine; a timing check on the
  thread list scroll (`thread-list-data-source.ts`); kill the engine with the thread list
  open and check reads wait, the list re-runs after respawn and shows a state after
  give-up.
- Trial: a day on `core.engine.reads: "engine"` (after stage 9): inbox, unified inbox,
  Starred, Unread, the draft list, calendar, contacts, search, activity.

**9. Search grammar.**
- Engine: parser and backend in `ms-store`, dates against `tz`/`now`; `search` and
  `query.search` on it.
- Desktop: `Query.searchText(text)` at the built-in call sites; `structuredSearch(ast)`
  printed back to text and deprecated; local backend deleted. tern-mcp drops its
  post-filters and its refusal of `OR`/`NOT`/`from:`.
- Tests: the desktop parser's spec cases replayed against the Rust parser; date terms with
  two time zones; the printer round-trips every spec AST.
- Trial: the search bar with `from:`, `in:`, `has:attachment`, `OR`, dates; subject
  suggestions; participant-profile related threads.

**10. Fallbacks out, lifecycle in.**
- Engine: `account.test` and `account.reset` as stdin methods; `engine.resetDatabase`,
  `engine.logs.tail`, `engine.setLogLevel`.
- Desktop: delete the C++ engine paths, `per-account` and `stdio` modes,
  `moveAsideRustDatabase`, `_deleteDatabase`; the supervisor starts `serve` at launch with
  zero accounts and runs onboarding's test over IPC; call the new methods.
- Tests: harness for test and reset over stdin; first-run onboarding with no account; A/B
  harness retired or pinned to an old binary.
- Trial: Reset an account, Rebuild the database, toggle verbose logging; add a test account
  on a fresh profile.

**11. Accounts and secrets in the engine.**
- Engine: SecretStore, registry, autostart, token persistence, restart policy,
  `credentials.import`, `credentials`, `build` and `protocol` in `hello`, `--detached`,
  `migrate` at `serve` startup.
- Desktop: migration (section 5); `AccountStore` on `account.list`; the supervisor reuses
  an engine found at `run/engine.json` once credentials are in the engine. Before
  migrating or reusing, it compares the running engine's `build` and `protocol` with the
  bundled binary; when they differ it asks the engine to stop (`engine.shutdown`, on the
  socket for a detached engine as a local-only method, or the launchd/systemd unit's
  restart) and starts the bundled one.
- Tests: import of a keyset with duplicate emails; downgrade with `credentialsBeforeEngine`;
  restart after a token rotation keeps the account working; file store on Linux CI; an
  older detached engine running when an updated desktop starts.
- Trial: migrate the trial profile, restart the app, check both accounts sync; run
  `serve --detached` with the desktop closed and check mail still syncs; update the app
  with the detached engine running.

**12. Onboarding through the engine.**
- Engine: `account.discover`, `account.oauth.*`, `account.add` and `account.test` on the
  socket (after 0.1 item 13), `account.setSecrets`, `account.update`, `account.remove
  {revoke}`, `secrets.*`, `server.*`; the local-only class (T13); Tern's own OAuth client ids.
- Desktop: onboarding and reconnect as in 4.5; redirect receiver in main on `127.0.0.1`;
  integrations keys through `secrets.set`.
- CLI: `tern account add` (IMAP and OAuth with a loopback receiver).
- Connection security (TD4, keep in the rewrite): the engine accepts exactly `"SSL / TLS"`,
  `"STARTTLS"` and `"none"` for `imap_security`/`smtp_security` and refuses anything else,
  empty or missing at `account.add`/`account.test` (`Account::validate`). `account.discover`
  never returns `none`: ports 143 and 25 mean STARTTLS, an autoconfig `plain` server is tried
  with STARTTLS, unknown ports default to STARTTLS. Saving `none` needs an explicit "Send
  password unencrypted" checkbox, stored as the host it was given for
  (`settings.<imap|smtp>_allow_cleartext: "<host>"`), except for `127.0.0.1`, `localhost`
  and `::1` (Proton Bridge); the engine refuses `none` for any other host unless that key
  matches the host, so every client (desktop, web, CLI) has to ask. `ms-providers` ports the
  cleaned `mailspring-provider-settings.json`, which has no `none` entry; the desktop rules
  are in `internal_packages/onboarding/lib/connection-security.ts` (copied in
  `apps/web/src/features/settings/accounts/account-draft.ts`).
- Tests: id derivation vectors; OAuth against a mock provider; discovery tables (no `none`
  anywhere); a relayed connection refused on every local-only method.
- Trial: add an IMAP account and one OAuth account (not the user's production accounts),
  reconnect one, remove one.

**13. Draft pipeline.**
- Engine: `draft.create`, `draft.send` (undo through send-later metadata), `draft.cancelSend`,
  `draft.validate`, signatures from settings; feature `draft-pipeline`. The resurfacing
  builder from stage 4 becomes `draft.create {kind: "resurface"}`.
- Desktop: 4.2 send pipeline and draft creation, from main and composer windows on their
  own connections. tern-mcp: `drafts.rs` replaced by `draft.create` (MCP drafts get
  signatures). Web: `draft.ts` and `signature.ts` replaced.
- Tests: an HTML corpus comparing desktop and engine quoting and signatures; undo within the
  delay; quit inside the delay and fire at the next start; `bodies` from a test extension.
- Trial: new, reply, reply-all, forward, undo send, send with attachment.

**14. Mail rules.** Engine evaluator and reprocess; desktop editor on `settings`; localStorage
move. Tests: the rules spec cases replayed against the engine. Trial: a rule that labels and
one that forwards.

**15. Unsubscribe, snooze method.** Engine `messages.unsubscribe`, `threads.snooze`;
desktop `list-unsubscribe` on it; mcp tools. Trial: one-click unsubscribe on a test list.

**16. Files.** Engine `files.*`; tern-mcp attachments on it. Trial: `get_attachment` for a
file not yet downloaded.

**17. Tracking and claims.** With the backend work (paused): engine transforms and claims,
`docs/backend-plan.md` 4.2 amended. Trial: a tracked send against a local backend.

**18. Calendar and contacts.** Engine 3.8; desktop calendar, events and contacts packages
call it and keep their UI; `ics-event-helpers.ts`, `calendar-utils.ts` (logic half),
`ContactInfoMapping.ts` and `VCFImportExport.ts` go. CLI and tern-mcp gain event and contact
writes. Tests: the existing ICS and vCard specs replayed against the engine; recurrence
expansion compared on a fixture calendar. Trial: create, move and RSVP an event, edit a
CardDAV contact, import a VCF.

**19. Service plugins.** `tern.service` manifest block; `tern service install|uninstall
<package>` writing a launchd LaunchAgent or systemd `--user` unit ordered after the detached
engine; Preferences > Plugins "install service". Tests: unit file generation per platform;
a sample service receiving `metadata-expiration` for its plugin id. Trial: install the
sample service, quit the desktop, see it act.

## 8. Risks and open items

- **Double execution.** Desktop plugin, web runner and executor all react to the same
  expiration until the feature gate ships; with metadata sync every device's engine fires.
  Gate on `hello.features`; claims in the engine (T22); D2 accepts a duplicate on a network
  error.
- **Query drift.** `QuerySubscription` re-checks matchers in JS (`query-subscription.ts:137`);
  if the engine's SQL disagrees (inAllMail, DISTINCT, collection joins, FTS), rows flicker in
  and out. The parity suite (stage 7) is the guard; keep it in CI.
- **Latency.** Reads go from in-process better-sqlite3 to a socket round trip with JSON, and
  the thread list issues several queries per scroll. Not measured; stage 8 measures before
  flipping the default. Per-window connections and out-of-order slow methods keep reads from
  queuing behind long calls. Reads also stop working while the engine restarts, which they
  don't today; the reconnect contract (4.1 step 2) bounds the wait and re-runs subscriptions.
- **Plugin allowlist.** A query on an attribute missing from the allowlist breaks a plugin.
  Generating the allowlist from `queryable` attributes covers built-in models; third-party
  plugins can't add models, so that is the whole surface.
- **HTML fidelity.** Quoting, sanitising, inline styles and tracking transforms use DOM APIs
  in the desktop; a Rust port produces different HTML. Corpus test in stage 13.
- **Third-party send hooks.** Lost for CLI, MCP and engine-executed sends. `clientFinalize`
  keeps desktop-scheduled sends working while the desktop is open; a send due while it is
  closed waits for it, as today. An engine callback to a connected client is an open item.
- **Credentials.** Data loss on migration (keep `credentialsBeforeEngine` one release);
  account id derivation must match JS byte for byte; email-keyed secrets map to several
  accounts; keychain prompts on unsigned builds; a Linux file store keyed on the same disk is
  obfuscation and is documented as such.
- **Old OAuth client ids.** The Gmail and Microsoft ids in `onboarding-constants.ts` look like
  Mailspring's registrations (I). Moving the exchange into the engine is the point to switch
  to Tern's; existing refresh tokens stay bound to the old ids.
- **Lifetime.** Moving supervision to the main process changes window-reload behaviour and
  crash reporting; `serve` still exits 141 when its spawning desktop goes away
  (`apps/engine/src/serve.rs:13,627`), so "send while the desktop is closed" needs
  `--detached` (stage 11), not only the executor. Until then a pending undo send or
  scheduled send fires at the next start (T10).
- **Version skew.** Today the main process runs the one-shot `migrate` at every launch
  (`src/browser/application.ts:97-101`) while an engine from the previous run may still hold
  `edgehill.db` for up to 30 s after its stdin closed; with a detached engine the gap is
  unbounded. Stage 6 waits for a live engine's pid to exit before migrating; stage 11 moves
  `migrate` into `serve` startup and compares `build`/`protocol` before reusing.
- **Gateway exposure.** `apps/gateway` is a same-uid socket client for remote callers; without
  the local-only class (T13) it would relay secret writes, host changes, logs and database
  resets. Lands with stage 12, before the gateway exists.
- **Calendar and contacts.** Until stage 18 other clients can't write events or contacts
  (T23); a web or MCP feature that needs it waits for that stage.
- **Settings migration.** Signatures and preferences in `config.json`, rules and the recipient
  blacklist in localStorage, web signatures in browser storage: one store, no data lost.
- **Scheduled sends across devices.** Rebuilding from the synced draft (T9) means any engine
  holding the draft and metadata can send it; claims decide which.
- **Specs.** The desktop specs read `edgehill.test.db`; they need a mock engine before the
  SQLite path is deleted.

Open items:
- Drafts whose From never went through the composer (created by tern-mcp, CLI or web) and
  whose address matches no account: the engine fails them with an error; whether the
  executor should instead move them to the matching account (stage 4: it fails them, with
  `error` on the send-later metadata; to confirm).
- The supported subset of natural-language date terms (stage 9), from the desktop's spec
  cases.
- Where `draft.send`'s `bodies` are stored during the undo window (stage 13).
- Whether the unsubscribe body-link fallback moves to the engine (stage 15: it stays in the
  clients, the engine reads headers only; to confirm).
- An engine-side hook for third-party send transforms, if users ask for it.
- `IMAPSearchQueryBackend` (server-side search) in the engine, after stage 9.
- Windows: named pipe, DPAPI store.

## Status

2026-10-16: draft written from five readers' maps of `roadmap/desktop-client` (fa9b69a79).
Revised the same day after review: stage 6 owns the parent channel and moves `KeyManager`
to main, with no engine reuse before stage 11; 3, 5, 13 depend on 6; stage 8's flip waits
for 9 and its SQLite removal for 10; send-later keeps a `clientFinalize` handler; the
reminder port covers baseline and reply clear; stage 4 builds the resurfacing message and
waits for incomplete drafts; undo send stays on synced metadata; reconnect contract; settings
mirrored in main; local-only method class; calendar and contacts (stage 18) and service
plugins (stage 19) added; cheap fixes moved to stages 1 and 2. Nothing implemented. Waiting
for the user to confirm section 0.1.

2026-10-16, stage 1 delivered (`roadmap/client-s1`): `crates/tern-client` (engine.json-first
discovery, hello, multi-engine router, typed calls, task builders) under `tern` and
`tern-mcp`; `hello.features` (`counts`, `tasks-list`, `metadata-list`, `message-hidden`,
`mail-new`, also in `engine.status`); `counts.get` and the `ThreadCounts` delta; `tasks.list`
(pending tasks first, oldest queued first, then finished ones newest first, so the hundred
retained finished tasks can't crowd out pending ones); `metadata.list`; `hidden` on Message
reads; `mail.new` from `MailProcessor::insert_message` (unread, inbox, not a draft, not from
the account or an alias, UID at or above the stored UIDNEXT), socket only, live, filtered by
account. tern-mcp ports `list_scheduled_sends`, `list_tracked_messages`, `get_top_links`, and
its send check uses `tasks.list`; `tern task list` and `tern watch` on `mail.new`
(`--new-mail`). Desktop: `ThreadCountsStore` on `counts.get` plus deltas (deltas that arrive
while `counts.get` is out are replayed on its answer), `unread-notifications` on `mail.new`,
both behind `hello.features` and fed by the serve engine through `src/engine-events.ts`.
Deviations: socket tests live in `apps/engine/tests/read_api.rs` and `socket.rs` unit tests
rather than `tests/socket.rs`. Open: per-account engines and the C++ path still read the
`ThreadCounts` table and notify from message deltas; no `mail.new` for folders whose UIDNEXT is
0 or missing; mail that arrived while no client was connected fires `mail.new` at the next
catch-up (the desktop's launch-time check drops it, `tern watch --new-mail` prints it);
`tasks.list` has no offset; the CLI's `targets()` doesn't dedupe an account held by two
engines; the three MCP tools page `metadata.list` up to 10,000 messages per account and filter
tracking in Rust.

2026-10-16, stage 2 (desktop cleanups), branch `roadmap/client-s2`: delivered. The
`mcp-server` plugin, its HTTP server and the 2587 listener are gone, and so is
`@modelcontextprotocol/sdk`. tern-mcp is bundled next to `mailsync-rs` (asar unpacked,
`build-release.sh`, Linux arm64 workflow, deb/rpm from the app directory; the existing
osxSign and notarisation config already covers it, no build run). `tern-mcp install` replaces
the old bridge entry without `--force`, and `install --if-stale` replaces it only when it is
pinned to the same profile; the desktop runs that once, 5 s after start, from the main window
of an installed copy (not dev mode, and on macOS only from /Applications). Plugins without a
satisfied `engines.mailspring` are skipped by `semver.satisfies`; `apiVersion` is exported
(1.26.0). The snooze batch clears only due threads. The OAuth receiver binds 127.0.0.1.
Deviation: the MCP pane is the "AI assistants" section of Preferences > Integrations, not a
tab (user's call). It edits `mcp.json` with tern-mcp's validation (re-read before each
change, unknown keys kept at every level), migrates `core.mcp.*` once with HTTP off, shows a
`core.mcp` tern-mcp refuses as an error instead of offering defaults, runs tern-mcp for status
and client install/uninstall, and shows `mcp-audit.log`. Open: `lang/en.json` not updated;
the desktop doesn't start `tern-mcp http`; old HTTP registrations in Codex and Claude Code
are documented, not removed; an entry pointing at a moved tern-mcp isn't repaired; Microsoft's
`localhost` redirect against the 127.0.0.1 bind needs the trial check; no `/usr/bin/tern-mcp`
in the Linux packages; stage 5 decides whether `apiVersion` becomes 1.27.0.

2026-10-16, stage 6 (supervisor and connections), branch `roadmap/client-s6`: delivered.
`EngineSupervisor` (`src/browser/engine-supervisor.ts`) runs `serve` from main and is the one
writer to its stdin; accounts come from `config.json`, secrets from `KeyManager`, now a
main-process class (`src/browser/key-manager.ts`) with a renderer facade over IPC that keeps
the old API; the Mailspring keychain migration moved with it unchanged.
`ProcessAccountSecretsUpdated` is stored in main. Each window has its own socket connection
(`src/engine-connection.ts`); the main window keeps Task handlers, `OnlineStatusStore` and
`queueTask` routing and gets account events over IPC with ack and replay across reloads. The
IPC rebroadcast is gone for `serve` only. No engine reuse; the exit-3 loop is unchanged;
startup `migrate` waits up to 40 s for a live pid in `run/engine.json`. Deviations: only the
main window subscribes to every account (so it alone takes the startup backlog), other
windows subscribe to the accounts the engine runs; the main window seeds `OnlineStatusStore`
from `engine.status` on each connect; commands sent while a window's socket is down are kept
(bounded) and sent after the reconnect; will-quit waits for `engine.shutdown`, and launches
during that wait relaunch the app afterwards. Per-account, stdio and the C++ engine still run
in the main window's `MailsyncBridge`. Open items: no IPC path yet for stdin-only calls from
other windows (`account.test`, stage 10; `credentials.import`, stage 11); deltas sent while
a window's socket is down or the main window is reloading are lost to it, so those Task
handlers don't run; a non-main window misses an account's first deltas until its subscribe
follows the status; a stale `run/engine.json` with a reused pid costs the full 40 s; Squirrel
install after the engine stop is unchecked on a signed build; `test-crash` has no socket
method; KeyManager still keys by email (stage 11). Rebased on stages 1 and 2: each window's
connection feeds its `EngineEvents` (hello features as the answer is read, `request()` for
`counts.get`, `gone` on a drop, `mail.new`) and `applyDeltas` hands `ThreadCounts` deltas to it
in every window, never rebroadcast; `ThreadCountsStore` and `unread-notifications` still run in
the main window only. The supervisor's `EngineClient` drops stage 1's hello, gone and
`mail.new` hooks and its `features()`/`request()`; its own connection still receives `mail.new`
(not class-filtered) and ignores it.

2026-10-16, stage 10 (remove the C++ engine and its fallbacks), branch `roadmap/client-s10`:
delivered. Engine: on `serve`'s stdin `account.test {account}` (`--mode test`'s object as the
result, from `test_auth::result`; a rotated refresh token is in its `account` only),
`account.reset {accountId}` (forgets a held account, waits 10 s for stragglers, deletes its
rows), `engine.resetDatabase` (forgets every account, deletes `edgehill.db*`, migrates a new
one in-process; socket read sessions reopen it through a database generation) and
`engine.setLogLevel {verboseUntil}` (restarts running accounts with their current secrets);
`engine.logs.tail {accountId?, bytes?}` on the socket (64 KiB default, 1 MiB cap). These four
stdin calls run on named threads and answer out of order; a panic there fails that call only.
Desktop: `core.sync.engine`, `MAILSYNC_ENGINE`, `TERN_ENGINE_PROCESS`, `TERN_ENGINE_TRANSPORT`,
`MailsyncProcess.sync`/`sendMessage`/`test`/`resetCache`/`account`, the main window's
per-account clients, the IPC rebroadcast, the move-aside of a Rust database for C++ and Attach
to Xcode are gone. The supervisor starts `serve` at launch with zero accounts (outside spec
mode) and respawns it without accounts; onboarding's test is `account.test` through the
supervisor (`SupervisorChannel.TestAccount`), and a rotated token from it is kept; Reset Cache
is `account.reset` while EngineClient holds the account; Rebuild database calls
`engine.resetDatabase` and falls back to deleting the files after the engine stopped; the
failed-`migrate` rebuild at launch still deletes the files (no engine runs then); verbose
logging is a config change the supervisor passes on as `engine.setLogLevel`, without the
relaunch; the account log in Preferences comes from `engine.logs.tail`. `serveSocketPath`
replaces `engineSocketPath`. `legacy/mailsync-cpp` stays for the harness oracles and
`import-legacy` for Mailspring profiles. Deviations: `MailsyncProcess.test()`, `resetCache()`
and `sync()` were plugin-visible through `mailspring-exports`, so this is a plugin API break;
Windows still has no engine (no named pipe yet), as before. Open: no harness scenario drives
the four stdin calls (the Rust tests in `tests/serve.rs` and `tests/test_mode.rs` do); unused
"Attach Mailsync to Xcode" strings remain in `lang/*.json`; the SQLite read path is still in
place (T4).

2026-10-18, tern-client (branch `roadmap/client-next`) takes stages 3, 4, 5, 8, 11, 12, 13, 14,
17, 18, 19 and their tech-debt items. Run one stage at a time in one worktree (the disk has
about 20 GB free, not enough for a target directory per stage), each merged to main after its
gate. Order and rough sizes:

| Order | Stage | Folded in | Size |
|---|---|---|---|
| 1 | 3 settings store | TD35 (Config writes off `@electron/remote`) | ~400 Rust, ~300 TS |
| 2 | 4 expiration executor | TD32 before it, TD33 queue | ~1.8k Rust, −220 TS |
| 3 | 5 integrations on serve, `Engine` export | TD34 before it, TD29 | ~600 Rust, ~1k TS |
| 4 | 11 accounts and secrets | TD43, TD44, TD45, TD46, TD12 engine half | ~2.5k Rust, ~300 TS |
| 5 | 12 onboarding through the engine | TD47, TD4's no-cleartext rule kept | ~1k Rust, −500 TS |
| 6 | 13 draft pipeline | TD48, TD49 (send), TD33 queue | ~1.8k Rust, −700 TS |
| 7 | TD39 folder identity | before 14 | ~1 week |
| 8 | 14 mail rules | | ~1k Rust, −300 TS |
| 9 | 19 service plugins | | ~400 Rust, ~150 TS |
| 10 | 17 tracking and claims | needs backend tracking endpoints | ~500 Rust |
| 11 | 8 DatabaseStore on `query` (flag and reconnect only) | TD40, TD41; waits for stage 7 | ~300 TS |
| 12 | 18 calendar and contacts | TD50; waits for stage 7 | ~2.5-3k Rust, −4k TS |

Trial checks on `~/tern-trial` are left to the user; each stage lists its trial steps as
pending.

2026-10-18, stage 3 (settings store), branch `roadmap/client-next`: delivered and merged.
Engine: `settings.get {keys?}` → `{values}`, `settings.set {values}` (null clears a key) and
`settings.changed {values}` on `serve` only (`apps/engine/src/settings.rs`), feature `settings`;
`<config>/settings.json` is one flat object keyed by config key path, rewritten through a
temporary file and rename. Keys and value kinds are in `ms_ipc::SETTINGS_KEYS` (the 3.4 list);
an unknown key or a wrong kind is refused with nothing stored. `settings.changed` goes to every
subscribed connection whatever its filter, except the one that made the change; the parent's
changes reach everyone; nothing is held for a later subscriber. Each connection gets its
`settings.*` answers and other connections' notifications in the order the store applied them
(the answer is queued under the store's lock). `settings.get` also returns `storeId`, a random
id kept in the file as `$storeId` and made anew when the store starts without one (no file, or a
corrupt one moved to `settings.json.bad`); a file that exists but can't be read (or can't be
moved aside) makes `settings.get` as well as `settings.set` fail with INTERNAL_ERROR. Relayed
connections may call both, but without the admin scope `settings.set` takes only the keys in
`ms_ipc::policy::RELAYED_SETTINGS_KEYS` (the undo-send delay, default sender, unsnooze-to-top,
HTML composing, default signatures), checked by the engine with `relayed_request_allowed`.
`tern settings get [key…]` and `tern settings set <key> <json|text>`. tern-mcp takes the default
sending account from `settings.get` when an engine keeps settings (unset there means none), else
from `config.json`. Desktop: `src/browser/engine-settings.ts` mirrors the six keys the desktop
holds today (`core.sending.undoSend`, `core.sending.defaultAccountIdForSend`,
`core.notifications.unsnoozeToTop`, `core.composing.html`, `signatures`, `defaultSignatures`)
over the supervisor's connection: `settings.get` first; a store whose `storeId` differs from
`engineSettingsStore` in config.json gets config.json's values copied in (one set per key), and
the id is saved once every copy is answered; a key whose copy the engine refused is listed in
`engineSettingsKept` and kept in config.json until the engine has a value; then pending local
writes, and the engine wins for the rest; a failing `settings.get` leaves config.json alone; local writes go to `settings.set` one key at a time; `settings.changed` is
applied to config.json and every window; config.json keeps every key. Launch waits up to 3 s for
the first sync before opening windows. TD35's stage-3 part: Config's initial read and writes use
`ConfigChannel` (`sendSync`) instead of `@electron/remote`; `.eslintrc` bans `@electron/remote`
imports and requires outside a 56-file allowlist. Tests: `settings.rs` unit tests, the serve
round trip and notification test, the CLI harness test (serve: get/set/clear, socket: refused),
tern-mcp's default-account test, `engine-settings-spec.ts` against the fake serve engine
(push once then engine wins, a new empty store with a new id gets config.json again, a
failing `settings.get` keeps config.json, a refused copy kept across launches, a window's
synchronous boot read, forwarding and notifications, a
local write kept over an earlier notification, an engine without `settings` left on
config.json), `preferences-signatures-spec` (the preview is sanitized), the serve test of a
relayed device refused `signatures` but allowed `core.sending.undoSend`, and the policy unit
test; full desktop suite passes. Review fixes: the signature preview in Preferences renders
through `SanitizeTransformer` (the composer already sanitized uneditable blocks); `tern
settings get --json` prints `{values}` only. The review fixes were checked with the affected
specs and harness tests; the full desktop suite reruns at the gate.
Decisions taken, to confirm: `templates` is an array and the desktop doesn't mirror it (its
templates are files in `<config>/templates`); `mailRules`/`mailRulesAutoSince` wait for stage 14
and `resurfacing.*` for stage 4; settings live in `serve` only (no store in per-account `sync`);
config.json's values win when they are copied into a store, after that the engine wins; a lost
or corrupt `settings.json` gives a new store id, so the desktop copies its values again instead
of clearing them (other clients' values in the lost file stay in `settings.json.bad`); relayed
devices without the admin scope can't set `signatures`, `templates`, `mailRules`,
`mailRulesAutoSince` or `resurfacing.*` (HTML the desktop renders with Node, or rules acting on
mail), while `settings.get` stays open. Deviation: the C++ engine is gone (stage 10), so
"a C++ engine run still sees the keys" is covered by the spec of an engine without `settings`
and by config.json being written in every case. Open: aliases and auto-cc in tern-mcp still come
from config.json (accounts are engine-owned from stage 11); writes made while the engine is down
are pushed on its next start only within the same app run; tern-web: the web client's
signatures in browser storage should move to `settings` (not ours), which for a device without
the admin scope needs the web's signature HTML treated as untrusted first and `signatures`
added to `RELAYED_SETTINGS_KEYS`; `apps/gateway/src/http/ws.rs` should switch to
`policy::relayed_request_allowed` so it refuses those `settings.set` calls itself (the engine
refuses them today). Trial pending: change the
undo-send delay in Preferences and see it in `tern settings get`; change it with
`tern settings set core.sending.undoSend 15000` and see the composer pick it up.

Gate (2026-10-18, rebased on 1c81e4dd2): fmt, clippy, `cargo test --workspace`, `cargo deny`,
release builds of mailsync, tern, tern-mcp and tern-gateway, the serve harness quick tier on the
release binaries (131 passed, 1 skipped), desktop lint and typecheck, full desktop suite (2962
passing). Open from the review (low): tern-mcp's `Engines::settings` turns a failed
`settings.get` into an empty map (`unwrap_or_default` in `apps/mcp/src/discovery.rs`), and
`account_for_new_draft` (`write.rs`) reads an unset key as "no default" and skips config.json, so
a transient engine error, or a store that hasn't had the desktop's first copy yet, puts a new
draft on the first account instead of config.json's default sender; `settings.set` has no size
limit on a value below the 64 MiB frame limit and is open to relayed devices without the admin
scope for the plain keys, so a remote device can store large values that are rewritten to
`settings.json`, sent to every subscriber, copied into config.json and pushed to every window;
serve tests are missing for several 3.1 routing rules of `settings.changed` (an `accounts`
filter, a connection that never subscribed gets nothing, no backlog for a later subscriber, a
relayed connection calling `settings.get`/`set`), for writes refused when `settings.json` is
unreadable, and for `sync` mode having neither the feature nor the methods (METHOD_NOT_FOUND);
`tern settings get|set` checks keys and kinds against the CLI's own `ms_ipc::SETTINGS_KEYS`, so
a CLI older than the engine refuses keys a newer engine takes (clients should gate on features,
not versions); `resetSettingsForSchemaChange` in `config.ts` reads with `GetRaw` and writes with
`SetAll` in two `sendSync` calls, so a `settings.changed` applied in between is overwritten and
`ConfigPersistenceManager.setSettings` then sends the stale value to `settings.set`, reverting
another client's change; the `@electron/remote` ban only covers what `lint:check` globs
(`app/src/**/*.{ts,tsx}`, `app/internal_packages/**/*.{ts,tsx}`), so `.js` files in `app/src`
and `app/spec` aren't checked. Trial steps above still pending.

2026-10-18, stage 7 (`query` with plain search), branch `roadmap/web-ready`: delivered, not
merged. Engine: `crates/ms-store/src/model_query.rs` compiles the DCP 3.2 params on
`ReadStore` against a checked-in allowlist (`model_query_allowlist.json`, every `queryable`
and joined-data attribute of the eleven classes, with natural orders), including the
`ThreadCategory` subselect, `ModelPluginMetadata` and `ContactContactGroup` joins, `include:
["body"]`, `idsOnly`, `count`, `distinct`, offset paging and a keyset `cursor` (`""` first,
id tiebreak); `search.grammar: false` is `.search(text)`'s FTS prefix match and `true` is
refused with `INVALID_PARAMS`, `data: {"unsupported": "search-grammar"}`. `query` is a read
method on the socket, in the relayed Open class, announced as `hello.features: "query"`;
`tern-client` has `Engine::query`. Desktop: `Query.toJSON()` and `Matcher.toJSON()`, plus
`spec/model-query-capture-spec.ts`, which regenerates and checks the allowlist and about 60
queries from the call sites (`toJSON()` and `sql()` each). `tests/model_query_parity.rs`
runs both against one fixture database and compares rows and order, and checks the
allowlist against the schema. Deviations: the desktop SQL runs on rusqlite, not
better-sqlite3 (same SQL text, keeps the suite in `cargo test`); SQLite reads the thread list
subselect through `ThreadCategory_val_id`, not `ThreadListCategoryIndex`, for the desktop's
own SQL as much as for ours, with or without large-mailbox statistics, so the plan test
asserts the desktop's plan and no `Thread` scan instead; `accounts: null` adds no account
clause, as the desktop reads; `Event.status` is queryable on the desktop but has no column,
so the engine refuses it; `structuredSearch` has no JSON form until stage 9, so those call
sites aren't captured; no CLI command (`tern rpc query '<json>'` works) and no tern-mcp
change, whose search post-filters go with stage 9's grammar. Open: the parity suite has not
run against a copy of the trial database; the capture covers the call sites with fixture ids,
not every runtime variation.

2026-10-18, stage 16 (files), branch `roadmap/web-ready`: delivered, not merged. Engine
(`apps/engine/src/files.rs`, socket only, feature `files`, all three `LocalOnly`):
`files.get {accountId, fileId}` → `{path?, size, contentType, downloaded, filename}`; for a
File not on disk it sends `bodies.fetch` for its message and answers from a `files` thread
once the file appears or after 30 s with `downloaded: false` (at most 64 waiting, engine-wide),
out of order with the connection's later requests (T21). `files.read {accountId, fileId,
offset, len}` → `{data, offset, size, eof}`, base64, at most 4 MiB a call, `NOT_FOUND` for a
file not downloaded. `files.put {accountId, name, contentType?, data | path}` → `{fileId,
file}`: a new 40-character lowercase id, the bytes copied into the files directory, and a File
row saved on the account's main thread (so `files.get` and the gateway serve uploads and the
attachments of a reopened draft); refused with `path` on relayed connections. Body fetches,
uploads and exports write each file under `files/.tmp` and rename it into place, so a waiting
`files.get` never sees half a file. GetMessageRFC2822Task without `filepath` is file-id mode:
the local phase sets `fileId` (a client's value is replaced, so an export can't overwrite
another file), the remote phase saves the message as File `<subject>.eml`
(`message/rfc822`). `tern-client` has `files_get`, `files_read`, `files_put`; tern-mcp's
`get_attachment` and `resources/read` use `files.get` when the engine has `files` (its
poll stays for older engines). Deviations: `files.put` returns the File JSON as well as
`fileId`, and `files.get` the `filename`; `files.read` and `files.put` are answered in order
(local disk only); the engine waits by looking at the files directory every 100 ms rather
than on a body-fetch signal; no `tern files` command (`tern rpc` reaches the methods). Open:
uploads never attached to a sent message stay, File row and bytes; leftover `files/.tmp`
parts after a crash aren't swept; the desktop still reads paths itself. Trial pending:
`get_attachment` for a file not yet downloaded (covered on the fake server by `test_mcp.py`).
2026-10-18, stage 4 (expiration executor), branch `roadmap/client-next`: delivered, not merged.
TD32 first: `--mode reset`, `account.reset` and `engine.resetDatabase` keep every message's and
thread's plugin metadata in `DetatchedPluginMetadata` under the Message-ID keys `import-legacy`
uses (`crates/ms-store/src/metadata_stash.rs`; reset no longer deletes that table), and the
resync attaches it again. Engine: `crates/ms-sync/src/expiration_executor.rs`, run by the
`metadataExpiration` thread in `serve` only (the context carries the settings store there);
feature `expiration-executor`. Due `send-later` on a draft: unless `clientFinalize` or an
`actionKey` other than `send`/`send-and-archive`, the expiration is cleared, the draft finalised
(`finalise_draft`: From must be the account's address or an alias, inline files the body no
longer references are dropped, zero-width spaces stripped from plaintext, no per-recipient
bodies) and a `SendDraftTask` queued, then for `send-and-archive` the archive task of
`tasksForArchiving`; on a message that is no longer a draft the row is cleared. A draft missing
its body, plaintext flag or a file on disk gets a body fetch and another look after 1, 2, 4 and
8 minutes, then `{expiration: null, error}`; a From that isn't the account's gets the same
error. `thread-snooze`: expiration cleared; if the thread is still in Snoozed, Gmail swaps the
Snoozed label for the inbox's, other providers move it to the inbox with `sourceFolderIds` of
Snoozed; then the resurfacing message, or with `core.notifications.unsnoozeToTop` false a
`ChangeUnreadTask` when nothing in the thread is unread. `send-reminders`: `Thread::before_save`
records the `lastReplyTimestamp` baseline (with `shouldNotify: false`) on any reminder with an
expiration and none, and clears the reminder (`{}`) once `lastMessageReceivedTimestamp` moves, so
the promotion of `thread:send-reminders` at send gets its baseline from the same hook; when due,
`shouldNotify: true`, expiration cleared, resurfacing message in reply to `sentHeaderMessageId`.
Resurfacing message: from `{account.name or email} via Tern` to the account itself, the
thread's subject, In-Reply-To the given Message-ID or the thread's newest non-draft message,
body from `resurfacing.snoozeBody` / `resurfacing.reminderBody` or the English fallback, sent as
a silent `SendDraftTask`. Metadata changes go through `SyncbackMetadataTask`'s local phase; every
other plugin's rows (and declined ones) are emitted as before. TD33: sends the engine queues carry
`retryWhenOnline`; a retryable SMTP error before the server accepted the message puts the task
back in `remote` (for up to 24 h after the first, `_offlineSince`), the foreground round goes on
with the other tasks and the worker sleeps on that error (offline shown); SMTP connection errors
on other sends are now marked retryable in their error JSON. Desktop: with `expiration-executor`
the send-later handler acts on `clientFinalize` rows only and the snooze and reminder
expiration/database handlers are off (the reminder focus handler stays);
`SendActionsStore.needsClientFinalize` marks undo-send and send-later-button rows (an action
outside `send`/`send-and-archive`, a composer extension with its own `applyTransformsForSending`,
or the built-in tracking extensions when they would transform the draft); the draft list shows a
send-later `error`; `SendDraftTask.onError` only logs for silent tasks; `src/resurfacing.ts`
builds both bodies (the snooze one's broken markup fixed) for the window and for the settings
mirror, which writes them in the configured language (`intl.localizerFor`) to an engine with the
executor after each sync and when `core.intl.language` changes. tern-mcp's `schedule_send` says
the engine sends when the engine has the executor, the desktop otherwise. Harness:
`client.settings`, `server.smtp_stop`/`smtp_start`, `transports:` (the two `metadata-expiration-*`
scenarios and `reset-keeps-plugin-metadata` now run on stdio and socket only); serve scenarios
`expiration-send-later` (two sent once, a `clientFinalize` one emitted), `expiration-undo-send`
(window closed with the engine running; engine quit inside the window, sent after the next
start), `expiration-send-later-header-only` (slow; empty body row, fetched, sent once with its
attachment), `expiration-snooze` (IMAP, two snoozes one due, resurfaced and hidden),
`expiration-snooze-gmail` (labels, unsnooze-to-top off, marked unread), `expiration-send-reminders`
(reply clears one, the other reminds), `expiration-send-offline`, `expiration-reset-keeps-snooze`.
Rust tests: executor cases in `crates/ms-tasks/tests/tasks.rs` (send, decline, From, waits, body
arrives, non-draft, IMAP and Gmail unsnooze, unread, left Snoozed, reminders, offline deferral,
scan), `reminder_update`, `finalise_draft`, the stash. Specs: send-later, snooze, reminders,
`needsClientFinalize`, resurfacing texts and silent `onError`, the mirror's resurfacing writes.
Decisions taken, to confirm: the executor runs in `serve` only (per-account `sync` keeps emitting
deltas); a From matching no account fails the send with `error` on the metadata instead of moving
the draft; the incomplete-draft waits are kept in memory, not written to the row, so an engine
restart starts them again; the reminder baseline and clear run in `Thread::before_save` rather than
`after_save` (one save instead of two), which also covers promotion in `send.rs`; the built-in
tracking extensions count as transforms only when they would change the draft
(`needsPerRecipientBodies`); the resurfacing texts are written only to engines with
`expiration-executor`, not mirrored into config.json, and the reminder string is new in
`lang/en.json` (other languages fall back to English until translated); an engine send stays
queued for 24 h of failures, then fails like any send; Send and Archive on a draft with no thread
archives nothing, as the desktop does. Open: tern-web: `apps/web` send-later-runner should stop
sending when the engine announces `expiration-executor` (not ours); the desktop's file-deleting
Rebuild fallback and failed-`migrate` rebuild still lose local-only metadata (TD32); stage 13:
"Queued, sends when online" on the draft and the retry for composer sends (TD33), `draft.send`
reusing `finalise_draft`; stage 17: claims (T22) and engine-side tracking transforms, until then
tracked scheduled drafts stay `clientFinalize`; metadata the engine writes in `before_save`
(baselines, reply clears) isn't pushed to a metadata backend, only the executor's own
`SyncbackMetadataTask`s are; the harness doesn't cover IMAP with unsnooze-to-top off, Gmail with it
on, or Send and Archive (the ms-tasks tests do); a missing attachment file can't be staged in the
harness, so `expiration-send-later-header-only` covers the empty body only. Trial pending: schedule
a send two minutes out, close the window and keep the engine, see it arrive once; snooze a thread
and set a reminder by hand and see both come back with no window open; once with the UI in a
language other than English, check the resurfacing text.

2026-10-18, stage 4 review fixes, branch `roadmap/client-next`. Account removal forgets the
plugin metadata: `account.reset` takes `keepMetadata` (default true, Reset Cache) and
`engine.resetDatabase` too; the desktop's removal passes false on both paths
(`resetCacheForAccount(account, {keepMetadata: false})`, `application:reset-database
{keepMetadata: false}` for the last account), so an account added back, whose id is the same,
gets no old schedules (`remove-account-drops-plugin-metadata`, ms-store and ms-ipc tests, a
supervisor spec). A deferred engine send no longer stalls the foreground: the task is skipped
for 2 minutes (`DEFERRED_RETRY`) or until `wake-workers`, the inbox is still fetched and idled on
with the IDLE (or the non-IDLE wait) bounded by the retry time (`IdleInterrupt::interrupt_at`),
and the account isn't reported offline for an SMTP-only failure (`expiration-send-offline` now
appends inbox mail while SMTP is down and expects it within 30 s and no connection error). The
desktop send-later handler mirrors the engine's decline rule (acts on `clientFinalize` or on a
draft whose `actionKey` the engine doesn't perform, via `SendActionsStore.enginePerformsSendAction`);
tern-mcp's `schedule_send` marks a non-engine `actionKey` `clientFinalize` and says the app sends
it. `list_scheduled_sends` now lists `scheduled`, `due` (past `sendAt`, not sent yet) and `failed`
(with `error`) drafts, and a draft read through `get_message`/`get_thread` carries
`scheduledSend`. Decisions taken, to confirm: `keepMetadata` as an optional param on the
existing methods rather than waiting for stage 12's `account.remove {deleteData}`; deferred sends
are retried every 2 minutes and on `wake-workers`; the MCP list puts failed sends first, and a
stale `error` next to a new expiration is not shown. Trial pending: remove and re-add a test
account that had a snoozed thread and see the snooze gone.


2026-10-18, stage 4 gate and merge, branch `roadmap/client-next`: merged to main. Rebased on
main after stages 7 and 16 (tern-web) landed; the executor and SendDraftTask now use stage 16's
`File::find_in` instead of this branch's `File::find_on_disk` (same lookup), and the
`expiration-executor` feature sits next to `files`. Gate green: fmt, clippy, `cargo test
--workspace`, `cargo deny`, release build of mailsync, tern, tern-mcp and tern-gateway, the
harness quick tier on serve with the release binaries, desktop lint, typecheck and the full spec
suite. Open, from the review (low): the send-later clear and the queued `SendDraftTask` are two
`perform_local` calls, not one transaction, so a crash or database error between them leaves a
draft with no expiration and no send, with nothing saying why; an engine-queued send that fails
for good (rejected recipient, auth, the 24 h offline window) writes no `error` to the send-later
metadata, so only an open desktop window sees it and a headless or web client can't; SMTP
Connection/Parse errors are now retryable and engine sends retry them for 24 h, including a
connection dropped after the DATA terminator but before the 250 reply, so a message the server
already accepted can be delivered again on each retry; the executor's in-memory `waiting`
entries aren't reset when a waiting draft is rescheduled or its send-later cleared, so a
rescheduled draft resumes from the old count (fewer than the 1/2/4/8 min looks) and cancelled
drafts' entries stay for the life of the thread; `Thread::before_save` writes the reminder
baseline and reply clear through `upsert_metadata(.., None)`, which bumps the version without a
`SyncbackMetadataTask`, so with `HttpMetadataBackend` the backend never sees it and another
device's update at the same version is then ignored (`current >= v`), losing that reminder
change; the harness covers snooze on IMAP with unsnooze-to-top on and Gmail with it off only,
and no Send and Archive (ms-tasks unit tests only); `needsClientFinalize` only counts an
`applyTransformsForSending` that is an own property of the registered extension, so a
third-party extension inheriting it from its own base class isn't marked `clientFinalize` and
the engine sends without the transform (T9); TD32's Rebuild after a failed `migrate`
(`application.ts`) and the `_resetDatabaseAndRelaunch` fallbacks with no supervisor or a failed
`resetDatabase` still call `_deleteDatabase` and lose local-only snoozes and scheduled sends; no
spec covers `_onSendDraft` writing `clientFinalize` on undo-send rows, `SendLaterButton` writing
it, or `SendLaterStatus` rendering `error` (only `needsClientFinalize` is specced);
`PLUGIN_API_VERSION` stays 1.26.0 though `metadata-expiration` is no longer delivered for the
built-in ids and `SendActionsStore.needsClientFinalize` is new (section 6 wants a minor bump),
and nothing tells third-party plugins writing send-later metadata to set `clientFinalize` when
they finalise themselves; the reminder sentence and `Not sent: %@` are only in `lang/en.json`,
so a non-English UI sends an English reminder body until translated (0.1 item 25) and the draft
list shows the engine's English error reason. tern-web: `apps/web`'s send-later runner should
stop when the engine announces `expiration-executor`. Trial pending (not run): schedule a send
two minutes out, close the window with the engine running, see it arrive once; undo-send with
the window closed inside the window; snooze a thread and set a reminder and see both come back
with no window open; with the UI in another language, check the resurfacing text; remove and
re-add a test account that had a snoozed thread and see the snooze gone.

2026-10-18, stage 9 (search grammar), engine, CLI and tern-mcp halves, branch
`roadmap/web-ready`: delivered, not merged. `crates/ms-store/src/search_grammar.rs` ports
`SearchQueryParser`, the AST and `LocalSearchQueryBackend` (match-compatible subtrees become one
`ThreadSearch MATCH`, bound rather than inlined; `is:`, `has:attachment` and dates go on
`Thread` columns as the desktop wrote them); `search_dates.rs` resolves date terms to the start
of the named day in the request's `tz` (IANA, UTC when absent), counting back from its `now`:
ISO dates and months, month names with or without day and year, weekday names, `today`,
`yesterday`, `N days|weeks|months|years ago`, `Nd`/`Nw`/`Nm`/`Ny` (the web's presets) and
`last week|month|year`; names without a year and weekdays are the latest on or before today. A
term outside that, or a relative one without `now`, is `INVALID_PARAMS`. `query.search
{grammar: true}` works on Thread (other classes refuse it); `search` takes `grammar`, `tz`,
`now` and `accounts` (feature `search-grammar`). `tern search` sends the query as typed with the
local zone and clock, one call per engine across its accounts, and exits 2 on a query the engine
refuses; tern-mcp's `search_mail` passes the query through and keeps only the access policy
filter (its own parser stays for engines without the feature). Tests: the parser spec cases as
Rust tests; `apps/desktop/app/spec/search-query-capture-spec.ts` writes
`crates/ms-store/tests/fixtures/search_queries.json` (AST and desktop WHERE for 68 queries),
and `tests/search_grammar_parity.rs` checks the engine builds the same AST (or fails where the
desktop fails) and finds the same threads for each date-free query on a fixture database;
dates in two zones there, in the unit tests (Lisbon, Tokyo, Auckland, Honolulu, Santiago's DST
gap) and in `tests/read_api.rs` on the socket; `test_cli.py` and `test_mcp.py` on the fake IMAP
server. Deviations from the desktop: a field keyword is a field only when a `:` follows (`trip
to paris` is three words; the desktop threw and fell back to a plain search); quoted tokens are
never keywords; a word glued to `:` (`http://x.test`, `subject:re:hello`) stays one word; a term with no
letters or digits (`-`, a stray `:`, `""`) is an error rather than an empty match; unknown `is:`/`has:`
values, a stray `)`, empty parentheses and a trailing `AND`/`OR`/`NOT` are errors; `after:`
includes its day (`>=` the day's start) and `before:` excludes it, where the desktop compared
with chrono-node's noon and a strict `>`; a query has at most 200 tokens (the parser
recurses per token on a socket reader's stack), and its FTS part is written with runs of
AND/OR flat and nests at most 30 deep (FTS5's parser stack overflows at 34); `search.tz` is
checked only when a date term needs it; English month and weekday names only. `QueryError`
lost its `Unsupported` variant, so `query` no longer answers `data.unsupported`. Open: the
desktop half (`Query.searchText`, the printer for `structuredSearch`, deleting the local
backend) is tern-client's; `search` keeps its `lastMessageTimestamp` order and has no cursor;
`IMAPSearchQueryBackend` is still later.

2026-10-18, stage 15 (unsubscribe, snooze method), branch `roadmap/web-ready`: delivered, not
merged. Engine (socket, all Open for relayed devices): `threads.snooze {accountId, threadIds,
until}` and `threads.unsnooze` (feature `snooze`) queue what `snooze-utils.ts` does, built from
the store in `ms_tasks::mail_actions`: the move (Gmail swaps the Inbox label for Snoozed's;
elsewhere `ChangeFolderTask` with `sourceFolderIds: [inbox]`), then a `thread-snooze`
`{expiration}` metadata task per thread, answering `{taskIds}`; the stage 4 executor brings
the threads back. Unsnooze moves back only threads still in Snoozed and clears the metadata,
without a resurfacing message. `until` must be in the future (`-32602`); an unknown thread or
an account without a Snoozed folder is `NOT_FOUND`. `messages.unsubscribe {accountId,
messageId, method?}` (feature `unsubscribe`, `ms_net::unsubscribe`) picks one-click, else
mailto, else web, as the desktop does; one-click is an RFC 8058 POST from an `unsubscribe`
thread (T21) only to an https host whose every address is public, pinned to the checked
addresses, no redirects, 10 s connect and 30 s total; mailto (at most 5 recipients, subject
control characters dropped, body up to 16 KiB, "unsubscribe" when empty) becomes a plain-text
`SendDraftTask` from the address or alias the list wrote to; web returns `url`. A message
whose body (and so header) isn't fetched yet is fetched and waited for as `files.get` does.
URIs are never logged. `tern-client`, `tern snooze/unsnooze/unsubscribe` (`--until 2h`,
`tomorrow`, a date, a date and time) and tern-mcp `snooze_threads`, `unsnooze_threads` and
`unsubscribe` (Send category) use them; the desktop's `list-unsubscribe` calls the engine for
header methods when it has `unsubscribe` and opens web links itself; desktop snooze stays on
its tasks. Deviations: mailto is sent without the composer review the desktop gave it; the
body-link fallback stays in the clients (the web doesn't look for one). Tests: ms-net unit
tests with a local HTTP server, `apps/engine/tests/mail_actions.rs`, the harness scenarios
`snooze-method` and `unsubscribe-mailto` (new `client.rpc` step), `test_cli.py`, tern-mcp and
web tests, the desktop spec. Open: the snooze move is built twice (here and in the
executor); one-click on a release build has only the unit and debug-binary tests (no public
https test server); trial pending: one-click unsubscribe on a test list.
