# Thin clients

Plan, 2026-10-23. Status 2026-10-25: all section 8 decisions accepted as recommended (#18: reader notes kept in `docs/thin-clients/`, deleted with this doc when the plan is done); stages 0-4 done. It moves domain logic out of the desktop and
web TypeScript into the engine, gives desktop and web one shared TS client, and drops the
desktop's direct SQLite reads. Paths are repo-relative; desktop paths are under
`apps/desktop/` unless they start with `apps/`, `crates/`, `docs/` or `packages/`.

The survey behind it is five reader notes in `docs/thin-clients/` (`desktop-data.md`,
`desktop-app.md`, `plugins.md`, `web.md`, `engine.md`, plus inventories `desktop-files.txt`,
`node-renderer.txt`). They hold one row per unit with its replacement. Section 6 summarises them.

## 1. Goal and the rule

Clients (desktop, web, CLI, MCP, later mobile) hold almost no state. The engine holds the
logic. Agreed refinements:

1. Clients hold no domain logic. They may hold UI state and caches: optimistic updates, undo
   position, scroll and selection, offline data on mobile. The engine gives thin clients live
   queries (query plus change feed), tasks it applies and reconciles itself, and server-side
   undo.
2. Anything the CLI, MCP, web or mobile would also want lives in the engine: models, queries,
   search, drafts, send, rules, signatures, templates, contacts, AI, integrations,
   account-level settings, export. Clients keep rendering (email HTML, the composer editor),
   layout, themes, keymaps, window state, OS integration (notifications, tray, file dialogs,
   default mail client) and their own preferences.
3. Plugins split the same way. Engine extensions hook tasks, messages and drafts and run for
   every client. UI extensions are slots, per client.
4. One shared TS client package in `packages/`, used by desktop and web: generated protocol
   types, a query/subscription client, models. The desktop drops `DatabaseStore`
   (`src/data/database/store.ts`, read-only better-sqlite3 on `tern.db`) and its query layer
   for it.

The rule, applied to each unit: if a second client would have to write the same code to
behave the same, it goes to the engine. If it only decides how something looks or where it
sits on this device, it stays in the client. If both, it splits.

Where I disagree or narrow the refinements:

- **Generate from `crates/tern-ipc`, not `crates/tern-wire`.** `tern-wire` (670 lines) is the
  engine-to-backend metadata wire (`MetadataPush`, `BackfillRecord`, `/tern/v1/...`). The
  socket protocol the clients speak is `tern-ipc` (98 names, 28 feature flags). The shared
  package needs `tern-wire` types only if the web ever talks to the backend directly.
- **Model types can't be generated today.** tern-store models are `data: Value` wrappers
  (`crates/tern-store/src/model.rs`) and many tern-ipc results carry `Value`. The only
  machine-readable model schema is `crates/tern-store/src/model_query_allowlist.json`, and it
  is generated from the desktop's TS models
  (`tests/integration/model-query-capture.test.ts`). The source of truth has to move into
  Rust first (section 3.3). Models in the shared package are typed JSON plus pure functions,
  not classes. The desktop keeps its classes as a layer over them for v1 plugins until 2.0.
- **Optimistic updates are mostly the engine's already.** `task.queue` answers after the
  local phase (`crates/tern-tasks/src/lib.rs` `perform_local`, `apps/engine/src/dispatch.rs`),
  and main-thread deltas flush within 5 ms, so on the local socket the engine's local phase
  is the optimistic apply. Clients should not re-implement local phases as web
  `src/engine/optimistic.ts` does. Over the gateway the round trip is longer, so intent RPCs
  return the post-local models (section 4.2) and the client swaps them in. A client keeps at
  most a generic "pending" overlay.
- **Display-time body transforms are domain logic.** Rendering stays client, but deciding
  which tracker pixels to strip and whether remote images load are rules every client needs,
  so they move to the engine (section 4.6).
- **Dropping `DatabaseStore` means dropping the SQLite connection and the SQL compiler, not
  the name.** About 100 exports of the v1 plugin API come from `data/`, `mail/` and `engine/`
  (58 `load`/`lazyLoad` entries, 13 models and 25 tasks registered with
  `lazyLoadAndRegister*`, 6 getters), among them `DatabaseStore`, `QuerySubscription`,
  `Matcher` and `Attributes` (`src/packages/api/mailspring-exports.js`). Removing them breaks v1 plugins, which
  `docs/desktop/plugin-api-v2.md` section 12 keeps until 2.0. A JSON-only facade over the
  engine stays until then.
- **`contextIsolation` is not reachable through this plan alone.** App windows run with
  `nodeIntegration: true, contextIsolation: false` (`src/main/windows/app-window.ts:121`)
  because plugins get Node (v1, and v2 per plugin-api-v2.md 2.3 and TD29). Moving the data
  layer removes Node from about 12 renderer files of 114 (109 in `src/` outside `src/main`, 5
  in `plugins/`), not from the window. Section 3.4 says what is reachable.
- **Some items in refinement 2 stay client.** Snooze and send-later presets and the
  natural-language date picker are UI choices (the engine only takes a time). The per-device
  reading delay (`core.reading.markAsReadDelay`) is a client preference. AI summary caches
  can stay per client; the engine already caches summaries 24 h and reply suggestions 1 h
  (`crates/tern-ai/src/lib.rs` `summary_ttl`, `suggestion_ttl`).

## 2. Where we are

### 2.1 Already in the engine

The engine already does more than the clients use:

- **Reads.** `query` (`crates/tern-store/src/model_query.rs`, 1388 lines) is a port of the
  desktop's `ModelQuery`. It answers all 65 desktop query shapes captured by
  `model-query-capture.test.ts`, checked against the TS compiler by
  `crates/tern-store/tests/model_query_parity.rs`, plus keyset `cursor` paging and the search
  grammar (`search_grammar.rs`, `search_dates.rs`). Also `threads.get`, `messages.get`,
  `search`, `counts.get` with `ThreadCounts` deltas, `tasks.list`, `metadata.list`.
- **The desktop route is built.** `core.engine.reads: "engine"` (default `"sqlite"`,
  `src/config/schema.ts`) sends every `DatabaseStore.run()` to `query`
  (`src/data/database/engine-reads.ts`). It never became the default; the gate was a
  real-profile latency measurement nobody ran. `structuredSearch()` (3 call sites) and the
  `ThreadCounts` raw SQL fallback stay on SQLite either way, because `Query.toJSON()` throws for
  structured search (`src/data/query/query.ts:419`).
- **Drafts and send.** `draft.create|update|delete|validate|send|cancelSend`
  (`apps/engine/src/drafts_api.rs`, `crates/tern-compose`: quoting, signatures, participants,
  mailto, tracking rewrite, tracker stripping in quotes, validation), pending sends with
  undo-send delay.
- **Automation.** The expiration executor (`crates/tern-sync/src/expiration_executor.rs`:
  send-later, snooze, reminders, resurfacing), mail rules, AI triage, phishing `warnings`
  (`crates/tern-store/src/derived.rs`), unsubscribe (`apps/engine/src/unsubscribe.rs`),
  `threads.snooze|unsnooze`, `templates.*`, `events.*`, `contacts.*`, `ai.*`,
  `integrations.*`, `account.*` with the engine's own `accounts.json` and secret store.
- **Undo data.** The local phase writes `ChangeFolderTask.undoPlacements`; an undo task's
  `restorePlacements` consumes it (`crates/tern-tasks/src/messages.rs`). `SyncbackEventTask
  {restore}` undoes event writes. The undo stack itself is desktop TS.

### 2.2 What still lives in desktop TS

Sizes are non-test lines.

| Area | Lines | What it holds that the engine has or should have |
|---|---|---|
| `src/data/database` | 835 | `DatabaseStore`, the background SQLite agent (`agent.js`, `src/main/ipc/database-agent.ts`) |
| `src/data/query` + `src/data/attributes` | 2.9k | SQL compiler, `QuerySubscription` with `Matcher.evaluate` in JS, range diffing |
| `src/data/models` | 3.4k | Thread/category/message semantics: `isHidden`, `displayName` with IMAP UTF-7, roles, `participantsForReply` |
| `src/data/tasks` | 2.2k | Task JSON builders for 24 task classes (26 files with `task.ts` and `factory.ts`), `TaskFactory` (archive/trash/spam rules), `createUndoTask` in 10 classes |
| `src/data/drafts` | 2.1k | JS `DraftFactory` fallback (570), `validateDraftForSending`, send prep |
| `src/data/stores` | 6.0k | `AccountStore` reading config.json, `ContactStore` search, `UndoRedoStore`, `TaskQueue` (whole Task table live), `ThreadCountsStore` SQL fallback |
| `src/data/mailbox-perspective.ts` | 654 | Which query a sidebar item means, what archive means from this view |
| `src/mail/search` | 1.4k | The search grammar parser and SQL backend (engine has it) |
| `src/mail/mbox.ts`, `features/account-sidebar/mbox-export-runner.ts` | ~400 | mbox assembly (the CLI has a second copy) |
| `src/engine/protocol.ts` | 954 | Hand-written tern-ipc types |
| `src/engine/supervisor/*`, `src/engine/process.ts` | 3.0k | Settings mirror config.json <-> engine (346), account seeding, crash/restart policy |
| `features/events/{header.tsx,rsvp-target.ts,invitation-calendar-refresh.ts}` | ~1.1k | ICS parse with ical.js, RSVP target choice, calendar refresh on invite |
| `features/list-unsubscribe/service.ts` | ~300 | RFC 2369/8058 parse and POST (engine fallback) |
| `features/message-autoload-images/store.ts` | ~150 | Remote-image policy and sender allowlist file |
| `features/remove-tracking-pixels` | 691 | Tracker rules for display (draft half is in tern-compose) |
| `features/draft-list/store.ts` | 299 | Pending/scheduled sends (the CLI computes the same) |
| `features/unread-notifications` | 376 | New-mail filter (engine `mail.new` decides it) |
| `features/thread-search` | ~1k | Grammar in TS, structured SQL |
| `plugins/activity` | ~2k of logic | Scans every message in range in the renderer to compute tracking stats |
| `plugins/{thread-snooze,send-later,send-reminders}` | ~1.8k | Fallback executors; snooze builds tasks itself and never calls `threads.snooze` |
| `plugins/phishing-detection` | 96 | Recomputes with the 2011 `tld` list, ignores engine `warnings` |
| `plugins/composer-templates/lib/store.ts` | 412 | Reads `<config>/templates` with `fs`; the engine's `templates.*` reads the same files |

Measured:

- 70 non-spec files reference `DatabaseStore` (60 in `src/`, 10 in `plugins/`), with 83 query
  call sites by grep (`findAll` 47, `find` 21, `findBy` 11, `modelify` 2, `run` 1, raw
  `_query` 1). By class: Message 27, Thread 20, Event 12, Contact 6, Calendar 6. Live:
  `Rx.Observable.fromQuery` 18 sites, `QuerySubscription` and subclasses 10, and about 20
  `DatabaseStore.listen`/`listenTo(DatabaseStore)` consumers (16 direct `listen` calls).
- The thread list opens one live message subscription per visible thread
  (`features/thread-list/data-source.ts`, up to ~150 with paging padding) because Thread rows
  carry no message summary.
- 19 non-test files hold 47 `Feature.*` references outside `protocol.ts` (Accounts 14,
  AiTriage 6, Integrations 5, ExpirationExecutor 4, DraftPipeline 3, Query 3, Rules 2,
  Counts 2, one each for 8 others; 13 are `supervisor/settings.ts` `KEY_FEATURES` and the
  feature list). Most are fallbacks for engines without a feature. The desktop runs its bundled
  engine in `serve` mode (a unit of another build is restarted, `supervisor/unit.ts`), so they
  are dead in a shipped build. The only engines without the `serve`-only features are
  `--mode sync` ones, which only the test harness's `stdio` transport starts
  (`apps/engine/test/harness/engine.py`).
- Neither `draft.validate`, `draft.update`, `draft.delete`, `files.get` nor `templates.*` is
  called by the desktop.
- 109 renderer files in `src/` (outside `src/main`) import fs/path/electron/child_process
  (`docs/thin-clients/node-renderer.txt`), plus 5 in `plugins/` (activity `csv-export.ts`,
  `dashboard/root.tsx`, `feed/table.tsx`; composer-templates `store.ts`,
  `preferences-templates.tsx`): 114. The brief's 113 most likely counted `plugins/` too; I
  didn't find which file changed.

### 2.3 What still lives in web TS

`apps/web/src` has no Node, Electron or database dependency. Its engine layer is 8.1k lines,
4.5k of them the in-browser mock engine.

- `src/engine/cache.ts`: a hand-written live-query evaluator for thread lists
  (`matchesThreadQuery`, `compareThreads`, sorted insert into pages). Every Thread delta also
  refetches `folders.list` and `labels.list`, although `counts.get` and `ThreadCounts`
  deltas exist.
- `src/engine/optimistic.ts`: a copy of the ChangeUnread/Starred/Folder/Labels local phases,
  Gmail rules included.
- `src/engine/tasks.ts`: task JSON for 11 classes. `features/mailbox/actions.ts`: a
  `TaskFactory` port.
- `features/composer/{draft,identities,signature,templates,send-later*}.ts` (~2.4k): reply and
  forward construction, signature insertion, templates in localStorage, a send-later runner.
  The web never calls `draft.create|send|validate`, all open to relayed devices.
- `src/engine/mock/*` (2.8k plus 1.75k fixtures) and `features/ai/mock.ts` (482): a second
  engine (ModelQuery compiler, search grammar, thread recompute, task local phases,
  expiration executor).
- 6 of 13 `TODO(engine)` markers are stale: `account.discover`, `account.reset`, engine
  account ids, `events.expand`, `templates.list|get` and integration secrets exist.
- It ignores `warnings` (phishing), Message `hidden` and `mail.new` (it does filter Contact
  `hidden` in `engine/hooks.ts` `useContacts`). `engine/rpc.ts` declares none of `draft.*`,
  `templates.*`, `counts.get`, `events.*` or `contacts.get|create|update`.

### 2.4 Duplicated logic

The main duplicates, all collapsing into one engine implementation or one shared TS module:

| Logic | Copies |
|---|---|
| Archive/trash/spam/move semantics | desktop `TaskFactory`, `apps/cli/src/organize.rs`, `apps/mcp/src/tools/write.rs`, `expiration_executor.rs archive_task`, web `features/mailbox/actions.ts` (5) |
| Task JSON builders | desktop `src/data/tasks`, web `src/engine/tasks.ts`, `crates/tern-client/src/tasks.rs` (3) |
| Protocol types | desktop `protocol.ts`, web `rpc.ts` + `types.ts`, `tern-ipc` (3) |
| "Does this thread belong in this list" | `Matcher.evaluate`, web `matchesThreadQuery`, engine SQL (3) |
| Mailbox perspective -> query | `mailbox-perspective.ts`, web `hooks.ts threadListQuery`, tern-store `threads()` (3) |
| Search grammar | `src/mail/search`, `search_grammar.rs`, web mock `search.ts`, MCP `search.rs` fallback (4) |
| Draft building | desktop `DraftFactory` JS path, web `composer/draft.ts`, `tern-compose`, MCP `compose.rs` fallback (4) |
| Send validation | `editing-session.ts`, templates `warningsForSending`, web `useComposerSession.ts`, `validate.rs` (4) |
| Templates store | plugin `fs`, `templates_api.rs`, web localStorage (3) |
| Signatures | config.json mirror, engine setting, web localStorage, desktop and web insertion code (4+) |
| Phishing check | plugin (`tld`), `derived.rs` (`psl`), and they disagree on unknown suffixes |
| mbox assembly | `src/mail/mbox.ts`, `apps/cli/src/mbox.rs` |
| Pending sends | `draft-list/store.ts`, `apps/cli/src/draft.rs scheduled_row` |
| Tracking events | `plugins/activity/lib/events.ts`, `apps/cli/src/followup.rs`, `apps/mcp/src/tools/followup.rs` |
| Triage chip / "answered" | desktop `plugins/ai/lib/triage.ts`, web `features/ai/triage.ts`, tern-store |
| IMAP UTF-7 and category names | `src/mail/imap-utf7.ts` + `Category.displayName`, web `engine/model.ts`, `tern-client/utf7.rs`, `tern-core/utf7.rs` |
| RSVP target, ICS parse | `features/events/rsvp-target.ts`, `pim_api.rs stored_copy`; ical.js in desktop, web `calendar/ics.ts`, tern-pim |
| Settings and accounts | config.json and engine `settings.json`/`accounts.json`, two-way mirror |
| Old-engine fallbacks | send-later, snooze, reminders and web `send-later-runner.ts` each re-implement the executor |
| Account setup rules | `features/onboarding/connection-security.ts` (port -> security, loopback hosts) and provider tiles vs web `settings/accounts/account-draft.ts` and `presets.ts`; `account.discover` has the servers |
| Mail rule schema | `src/data/mail-rules-templates.ts` (conditions, operators, actions for the editor) vs the engine's rule evaluator; no RPC publishes it |

Full lists: `desktop-data.md` section 4 (21 items), `desktop-app.md` section 11 (30),
`plugins.md` section 6 (19), `web.md` "Duplicated logic" (24), `engine.md` section 10 (19).

## 3. Target architecture

### 3.1 Engine responsibilities

The engine owns, for every client:

- **Data and its meaning.** Models and their derived fields (`hidden`, `warnings`, decoded
  `displayName` and sort order for categories, message summaries on threads, unsubscribe
  methods, remote-image decision, invitation summary), queries, search, counts, and live
  queries with engine-side membership and order.
- **Intents.** Mail actions as verbs with provider and perspective semantics, drafts and send,
  snooze, reminders, send-later, templates, signatures (including template rendering),
  contacts and groups, events and invitations, rules, AI, integrations, export (`.eml` bytes
  and mbox).
- **Undo.** An undo log built from its own data.
- **Settings shared across clients**, accounts, secrets, and the strings it writes into mail
  (quote attribution, forward header, resurfacing bodies) in the user's locale.
- **Engine extensions** (section 5): hooks it calls, settings namespaces, methods it routes.

### 3.2 The client contract

What a client may do, and what it calls:

| Concern | Contract |
|---|---|
| Reads | `query` for one-shot reads; `query.subscribe` for anything on screen (section 4.1). No client evaluates matchers or sort orders. |
| Raw change feed | `subscribe {accounts, classes}` stays for the CLI's `tern watch`, service plugins and the v1 `DatabaseStore.listen` facade. UI code uses live queries. |
| Writes | Intent RPCs (`threads.archive`, `draft.send`, ...). `task.queue` with raw task JSON stays for v1 plugins and the CLI's low-level commands. |
| Optimistic apply | Local socket: none; the local phase plus 5 ms deltas is fast enough. Gateway: intent RPCs return the post-local models, the client applies them, live queries then confirm. A remote failure reverts in the engine and notifies the caller. |
| Task completion | The caller gets `task.done {taskId, status, error}` on its own connection when it asked for it. Nobody watches the whole Task table. |
| Undo | Intent results carry an undo token; `undo.apply`, `undo.redo`, `undo.list` (section 4.3). The client keeps only which toast is showing. |
| Settings | Engine keys through `settings.get/set/changed`, held in a sync cache by the shared client. Client preferences stay local (config.json, localStorage). |
| Files | `files.get` (local path, local clients only), `files.read` (bytes, any client), `files.put {data}`; `messages.raw` for `.eml` bytes. No client computes the engine's file layout. |
| Export | The engine writes `.eml` and mbox to a path on its machine (`export.start`), or streams bytes to the client (`messages.raw`, `export.read`). The save dialog is the client's. |
| Capabilities | `hello.features` plus `capabilities` for what is usable; no client derives availability from raw settings. Old-engine fallbacks are removed: client and engine ship in one release. |

### 3.3 The shared TS package

`packages/client`, npm name `@tern/client`, linked with a `file:` dependency as
`packages/email-dark` is (`packages/README.md`). TypeScript source, no bundler, framework-free:
no React, no RxJS, no TanStack, no Node imports outside its `node` entry.

| Module | Contents | Comes from |
|---|---|---|
| `protocol/` (generated) | Method names, params and results, notifications, feature flags, error codes, settings keys, model JSON types, task JSON types | `crates/tern-ipc`, via JSON Schema (below) |
| `rpc` | JSON-RPC 2.0 core over a `Transport` (`send(line)`, `onLine`, `onClose`): pending map, timeouts, out-of-order replies, server-to-client requests (needed for extension hooks) | desktop `src/engine/client/connection.ts`, web `src/engine/ws.ts` |
| `transports/ws` | WebSocket transport (web, later mobile) | web `ws.ts` |
| `node` entry: `transports/unix` | NDJSON over a Unix socket with Node `net` | desktop `connection.ts` |
| `session` | hello, features, capabilities, connection state, reconnect with backoff, resubscribe | desktop `connection.ts`, `events.ts`, `engine-reads.ts`; web `ws.ts`, `connect.ts` |
| `live` | `LiveQuery` (subscribe, window, apply ops, ordered rows, dedup by query key, resubscribe after reconnect), `Counts`, `SettingsCache` | desktop `subscription-pool.ts`, `result-set.ts`; web `cache.ts`, `settings.ts` |
| `query` | JSON-only query builder and matcher JSON (no `sql()`, no `evaluate`) | desktop `data/query/query.ts` minus the SQL half |
| `api` | Typed wrappers: threads, drafts, undo, tasks with completion, contacts, events, files, ai streams, integrations | desktop `engine/client/{drafts,pim,ai,accounts}.ts`, web `features/ai/api.ts`, `composer/engine-calls.ts` |
| `models` | Pure functions on model JSON: participant display, contact display name and initials, comparators, role predicates | desktop `data/models/*` display helpers, web `engine/model.ts` |

Not in the package: React bindings (desktop React 17 with Rx, web React 19 with TanStack each
write a thin adapter), auth and pairing, gateway file URLs, the editor, the desktop's model
classes, the mock engine.

**Type generation.** Add `schemars` (MIT) to `tern-ipc`'s structs and derive JSON Schema for
every request, result and notification. A small Rust binary (`tern-ipc --bin schema`, or an
`xtask`) writes `packages/client/schema/tern-ipc.json`; `json-schema-to-typescript` (MIT)
turns it into `packages/client/src/protocol/*.ts`. Both outputs are committed, and CI fails
when a regeneration differs. JSON Schema over `ts-rs` because mobile (Swift, Kotlin) and the
MCP tool schemas can read the same file.

For models, add typed view structs in `tern-ipc` (`ThreadJson`, `MessageJson`, `FolderJson`,
... with `#[serde(flatten)] extra` so unknown keys pass through). tern-store keeps storing
`data: Value`; a test runs the fixture databases through tern-store and checks every emitted
row deserialises into its view struct. The query allowlist is then generated from the view
structs instead of from the TS models, which moves the schema's source of truth into Rust.

### 3.4 Desktop after

**Main process keeps:** windows, menus, tray, dock badge, the `tern-asset:` and `tern-file:`
schemes (backed by `files.get` paths), the OAuth loopback receiver, native notifications,
auto-update, profile reset, and engine lifecycle reduced to "ensure the engine unit runs, then
connect" (`src/engine/supervisor/unit.ts` plus a direct spawn on Windows, where `tern` installs
no units). It loses the settings mirror (`supervisor/settings.ts`), account seeding from
config.json, the crash/restart policy (`supervisor/host.ts`), most of `EngineServeProcess`
and the database agent: about 2.9k lines. The supervisor sends `account.test`,
`account.reset`, `engine.resetDatabase` and `engine.setLogLevel` over stdin today, but the
engine already takes the first two on the socket (`MethodClass::LocalOnly` in
`crates/tern-ipc/src/policy.rs`, `is_account_method` in `apps/engine/src/socket.rs`), and a
`serve --detached` engine takes the other two there too (`is_parent_method`; with a parent
channel they answer `WRONG_CHANNEL`). Joining the unit's detached engine is therefore enough
for them. `credentials.import` is the exception: stdin only, refused by a detached engine, so
the last KeyManager secrets have to reach the engine before the desktop stops spawning it
(stage 9), or the method needs a local-only socket path.

**Renderer keeps:** React UI, the Slate editor, the message frame and display sanitising
(DOMPurify), keymaps, themes, client prefs in config.json, the plugin and theme loader, and the
shared client over the Unix transport. On top of it, v1 compatibility: a `DatabaseStore`
facade (`find`, `findAll`, `findBy`, `count`, `modelify` over `query`; `listen` over the raw
feed), `QuerySubscription` and `Rx.Observable.fromQuery` over `query.subscribe`, model classes
over the generated JSON types, task classes as JSON builders, `TaskFactory` delegating to
intent RPCs.

**Node in the renderer after:** of the 109 files, about 12 lose Node with the data layer and
file features (`mbox-export-runner.ts`, `sidebar-item.ts`, `events/header.tsx`,
`message-autoload-images/store.ts`, `message-controls.tsx`, `thread-list/context-menu.ts`,
`attachment-items.tsx`, `evented-iframe.tsx`, `composer-editor.tsx`, the MCP policy pane x2,
`window/key-manager.ts`), and `data/database/store.ts` and `agent.js` go with `DatabaseStore`.
Two of these only lose `fs`/`path`: `evented-iframe.tsx` keeps `ipcRenderer` (clipboard) and
`shell` (show in folder, open external), `composer-editor.tsx` keeps `webUtils.getPathForFile`
for dropped files. `key-manager.ts` is the v1 `KeyManager` export, so it goes with 2.0, not
with this plan.
`data/stores/attachment.ts` keeps its dialogs but stops computing the engine's file layout. What stays: the engine transport (one file, `net`), plugin and theme
loading (`src/packages/`, ~10 files), OS integration (`window/platform/`, `platform/`), about
20 features that call `ipcRenderer`/`shell` for menus and links, and user-picked file I/O
(vCard, CSV, saving bytes from `messages.raw`). better-sqlite3 leaves the renderer; main still
uses it for the Mailspring import until that becomes `tern import mailspring`.

**`contextIsolation`:** not with plugins that get Node. What this plan makes possible is a
renderer whose core code uses Node only through `platform/` and the transport, which is the
precondition for a later decision to isolate core windows and host plugins elsewhere (section
8, decision 9).

### 3.5 Web after

The web keeps its UI, editor (`composer/html.ts`), auth and pairing, per-browser preferences
and routes. It drops `cache.ts`, `optimistic.ts`, `tasks.ts`, `keys.ts` perspective logic,
most of `model.ts`, `mailbox/actions.ts`, `composer/{draft,identities,signature,templates,
send-later-runner}.ts`, `settings/integrations/store.ts`, `settings/signatures/store.ts` and
the old-engine fallbacks (`use-unified-threads.ts`, the `threads.list` filter loop, client
date filtering). It uses `@tern/client` with the WS transport and a TanStack adapter over
`LiveQuery`. The mock engine stops growing; dev and tests move to a fixture-seeded real engine
(`e2e/stack.py` already starts one), and the mock is deleted once that runs in the dev server.

This needs relay policy changes (`crates/tern-ipc/src/policy.rs`): relayed writes of
`signatures` and `templates.save|delete` once the engine sanitises their HTML on write;
`integrations.*` open once the engine fills `config` from its own `core.integrations.<kind>`
setting (as `ai.*` does); event and contact tasks in `RELAYED_TASK_CLASSES`;
`account.test {accountId}` with stored secrets; a fixed OAuth redirect page for browsers.

### 3.6 CLI and MCP

They stay Rust on `tern-ipc` through `crates/tern-client`, and lose their copies of desktop
logic as the engine gains the RPCs: `organize.rs` resolve rules (intent RPCs), `mbox.rs`
(engine mbox), `followup.rs` event flattening (`tracking.events`), `draft.rs scheduled_row`
(`drafts.pending`), `mail_actions.rs parse_until` stays (CLI input parsing). `tern-client`
keeps engine discovery and routing and drops `tasks.rs` builders once nothing uses them. Both
gain undo (`tern undo`, an MCP `undo` tool). The `TODO(cli-parity)` block in
`apps/cli/src/cli.rs` lists three items: contact delete and groups (the engine already runs
`DestroyContactTask`, `SyncbackContactGroupTask`, `ChangeContactGroupMembershipTask` and
`DestroyContactGroupTask` through `task.queue`, so the CLI needs no new RPC for them),
`integrations.*` with provider key setup (all served today), and undo, the only one that needs
engine work (4.3). The MCP's `compose.rs` and `search.rs` fallbacks for engines without
`draft-pipeline`/`search-grammar` (only `--mode sync` engines today) are deleted.

`tern-mcp` also reads the desktop's `config.json` (`apps/mcp/src/config.rs` `DesktopConfig`):
`core.mcp.*` as a fallback when `mcp.json` is missing, and
`core.sending.defaultAccountIdForSend` when the engine keeps no settings. Both fallbacks go
with stage 9, so no Rust client reads `config.json`.

Mobile, later, uses `@tern/client` over the gateway (React Native) or types generated from the
same JSON Schema.

## 4. Engine API to add

### 4.1 Live queries

**Methods.**

```
query.subscribe   {query: QueryParams, window: {offset, limit}, include?: string[], key?: string}
                  -> {subId, rows, total, version}
query.setWindow   {subId, window: {offset, limit}} -> {rows, total, version}
query.unsubscribe {subId}
query.changed     notification {subId, version, total, ops: [
                     {op: "insert", index, row} | {op: "remove", id} |
                     {op: "update", row} | {op: "move", id, index}
                  ]} or {subId, version, total, reset: rows}
```

`QueryParams` is the existing `query` shape (`crates/tern-ipc/src/read.rs:253`), so every
desktop and web query works unchanged. `include` adds derived data per class:
`summary` on Thread, `body` and `warnings` on Message. `reset` replaces the window when a
diff would be larger than the window.

**Shapes it has to serve**, from the desktop-data survey (section 2 of `desktop-data.md`):

| Shape | Today | With live queries |
|---|---|---|
| Thread list per perspective (categories containsAny, inAllMail, unread/starred, order lmrt or lmst desc) | `MutableQuerySubscription` + one message subscription per visible thread | one subscription with `include: ["summary"]` |
| Unread view keeping just-read threads | client whitelist from queued tasks | same query `or id in [whitelist]`; whitelist stays UI state |
| Thread messages (`threadId =`), hidden filtered | `MessageStore` + delta patching | subscription on Message, `hidden` on rows |
| Draft list, pending sends first | `QueryResultSet` + `OutboxStore` | subscription plus `drafts.pending` |
| Thread search (grammar), subject suggestions | SQLite only | `query.search {grammar: true}` live; `now` fixed at subscribe |
| Folders, labels with counts | 2 live queries + counts | subscription on Folder/Label plus `ThreadCounts` |
| Tasks | whole Task table live (`TaskQueue`) | `tasks.list {status: pending}` subscription; `task.done` for callers |
| Contacts list, groups, books | 3 live | subscriptions; locale sort stays client |
| Focused contact, calendars | live | subscriptions |
| Events in a range | `events.expand` re-run on any Event delta | `events.subscribe {calendarIds, start, end}` -> `events.changed` with occurrences |
| Needs-reply (triage) | re-list on every Thread delta | `ai.triage.list` as a subscription |
| Activity | 2 live queries over metadata | `tracking.events` subscription |

**Thread summary.** `include: ["summary"]` adds to each Thread row: `messageCount` (visible
messages, hidden ones excluded), `senders` (ordered, with unread and draft flags), `hasDraft`,
`sending`, `hasAttachment`. The engine computes it in the same read from the Message table for
the rows in the window. This replaces the per-thread subscriptions in
`features/thread-list/data-source.ts` and the web's `senders.ts` TODO.

**How the change feed maps to subscriptions.** The `DeltaStream`
(`crates/tern-store/src/delta.rs`) already batches persists per transaction and knows class
and ids. A `LiveQueries` registry in the engine, on its own thread with its own read `Store`,
receives each flushed batch as `{accountId, class, ids}` from the `Hub`
(`apps/engine/src/socket.rs`). For each subscription:

1. Skip it unless the batch touches a class it depends on: its own class, plus Message for
   Thread subscriptions with `summary`, plus Folder/Label for category-scoped ones.
2. Re-run the subscription's window as `idsOnly` (the 1.3 ms median query in
   `test_query_latency.py`), diff against the previous id list, and fetch full rows only for
   inserted ids and changed ids still in the window.
3. Send one `query.changed` per subscription per flush, coalesced to at most one per 50 ms.

This re-runs SQL instead of evaluating matchers in memory, so membership and order are
decided by the same compiler as `query` and there is no fourth matcher. Incremental
evaluation can come later if measurement says so.

**Paging.** The window is `{offset, limit}`; scrolling calls `query.setWindow`. Internally the
engine uses the keyset cursor it already has for the window's first row. The desktop's
`page(start, end, 50, 100)` padding maps to the window size.

**Reconnect.** A client resubscribes its active subscriptions after reconnect (one query
each); no delta resume is needed for UI clients. Add a `seq` to `delta` notifications and
`subscribe {since}` with a bounded ring for raw-feed consumers (service plugins, `tern watch`)
that must not miss changes.

**Derived fields on every row.** `warnings`, `hidden`, category `displayName` and `sortOrder`,
label colour, `unsubscribe: {methods}`, `remoteImages`, and an invitation summary flag are
attached on `query` rows, live-query rows and deltas. Today `warnings` comes only from
`threads.get`/`messages.get` (`read_api.rs` `with_warnings`); `hidden` is already on Message
rows of `query` (`model_query.rs`, `with_hidden`) and of the read methods, but not on deltas.

### 4.2 Intent RPCs and tasks

- `threads.archive | trash | spam | notSpam | move {to} | label {add, remove} | markRead
  {unread} | star {starred} | removeFromView {perspective}` taking `threadIds` or
  `messageIds` across accounts. They resolve Gmail label vs folder semantics, source folders
  and the all-mail fallback once, in Rust, replacing `TaskFactory` and its four ports. Result:
  `{taskIds, undo, models}` where `models` are the post-local Thread rows.
- `categories.markRead {categoryId}` (query-scoped; today `sidebar-item.ts` ships every unread
  id), `categories.create {name}` with `/` separators (the engine already applies the server's
  delimiter), `categories.rename`, `categories.delete`, `categories.empty`.
- `draft.send {then: "archive"}` is already `after`; add extension send actions there (5.2).
  `draft.create {kind: "forward", asAttachment: true}`. `signatures.apply {body, signatureId}`
  and `signatures.render {template, data}`.
- `draft.schedule | unschedule`, `threads.remind | acknowledgeReminder`, `draft.update
  {tracking: {opens, links}}`: typed RPCs over the metadata shapes clients write by hand today.
- `task.queue {notify: true}` and the per-caller `task.done` notification. On a non-retryable
  remote failure the engine applies the task's undo locally and reports it, instead of
  leaving the local change until the 24 h hold lapses.
- `contacts.list {perspective: all | foundInMail | group, groupId, search}`, cross-account
  `contacts.search` with distinct-by-email and the `refs > 0`/`hidden` filters,
  `contacts.delete`, contact groups (typed wrappers: the delete and group tasks already exist in
  the engine).
- `events.invitation {messageId}` -> method, summary, organizer, times, our status, RSVP target
  or the reason there is none, conflicts. The engine refreshes the calendar itself when an
  invitation arrives (today `features/events/invitation-calendar-refresh.ts`, only while a
  desktop window is open). `events.search {query, range}` returning occurrences.
- `messages.raw {messageId, offset, length}` for `.eml` save, Show Original and forward as
  attachment; `export.start {scope, format: eml | mbox, path}` with progress and an engine
  mbox writer, replacing `src/mail/mbox.ts`, `mbox-export-runner.ts` and `apps/cli/src/mbox.rs`.
- `drafts.pending` (outbox, scheduled sends first), `tracking.events` and `tracking.stats
  {accounts, from, to, groupBy}`, `messages.translate {messageId, target}` with block
  extraction and cache, `ai.triage.correct`.
- Richer `mail.new` (subject, from, snippet; it carries `accountId`, `threadId` and
  `messageId` today), so notifiers and later mobile push need no read.
- Read limits: a reply-size budget with its own error code, a per-request timeout, and reads
  answered on a small pool per connection with out-of-order replies, so a slow FTS query no
  longer blocks the window's other reads. The out-of-order machinery exists (`answer_later`
  and the lanes in `apps/engine/src/workers.rs`, used for files, integrations, AI, unsubscribe,
  account setup); `query` and the other reads are answered inline on the connection thread
  (`socket.rs` `handle`). Relayed `query` is already capped (`policy::cap_relayed_query`).
- `rules.schema`: the conditions, operators and actions the engine's rules support, so the
  desktop's `data/mail-rules-templates.ts` and a web or CLI rule editor read one list.

### 4.3 Undo

The engine keeps an undo log: one entry per undoable intent or task, with the inverse built
from data it already writes (`undoPlacements`, previous flags and labels, metadata
`undoValue`, `SyncbackEventTask {restore}`, snooze's moves). The 10 `createUndoTask`
implementations in `src/data/tasks/*` are ported into `crates/tern-tasks`.

```
undo.apply {token?}         token omitted = the caller's last entry
undo.redo  {token?}
undo.list  {limit?}         -> [{token, client, label, at, canRedo}]
undo.changed  notification
```

Each entry records the `client` from `hello`. By default `undo.apply` without a token undoes
the caller's own last change, so Cmd-Z in the desktop doesn't undo what an agent did; `tern undo
--any` and `undo.list` see everything. The log is in memory, bounded (100 entries, 24 h).
Undo send stays `draft.cancelSend`, listed in the log too. `UndoRedoStore` becomes a view over
`undo.list` and keeps only the toast.

### 4.4 Settings ownership

The engine holds 20 keys today (`crates/tern-ipc/src/settings.rs` `SETTINGS_KEYS`).
`src/engine/supervisor/settings.ts` mirrors 16 of them two-way with config.json
(`ENGINE_SETTINGS_KEYS`), writes `resurfacing.snoozeBody|reminderBody` and
`composing.quoteTexts` one-way from the desktop's locale, and leaves `templates` alone. After
this plan config.json holds no engine key, and the desktop's `AppEnv.config` routes reads and
writes of engine keys to the shared client's `SettingsCache` (filled at hello, updated by
`settings.changed`), so UI code that reads `AppEnv.config.get('core.sending.undoSend')` keeps
working. Today config.json's copy is what windows read before the engine answers (and when it
doesn't start); the cache needs the same: last known values persisted per profile and served
read-only until hello, or the engine-unavailable state and first paint see defaults.

| Key | Today | Moves to |
|---|---|---|
| `accounts`, `accountsVersion`, `accountsToRemoveFromEngine` | config.json, seeded into the engine | engine only (`account.list`, `account.changed`); one migration release |
| `core.reading.autoloadImages` + `autoload-images-whitelist.txt` | config + file | `reading.remoteImages {mode, senders}` |
| `core.sending.defaultReplyType`, `core.sending.defaultSendType` | config | engine |
| `core.contacts.findInMailDisabled` | config | engine |
| `core.composing.grammarCheckLanguage`, `grammarCheckDisabledRules` | config | engine, next to `core.integrations.grammar` |
| `core.translation.automatic`, `core.translation.disabled` | config | engine, next to `core.integrations.translation` |
| `core.appearance.labelColors` | config | engine (label colour on Label JSON) |
| `core.calendar.disabledCalendars` | config | engine |
| `core.intl.language`, `core.workspace.use24HourClock` | config, rendered strings pushed as `composing.quoteTexts` and `resurfacing.*` | `intl.locale`, `intl.hourCycle`; the engine renders its own strings |
| `core.mcp` | already moved to `mcp.json` by the Preferences pane (`features/preferences/index.tsx`), which reads and writes it with `fs`; `tern-mcp` still falls back to config.json `core.mcp.*` | a `tern mcp policy get|set` command (doesn't exist yet) for the pane; the config.json fallback goes |
| `templates` | engine key, unused | removed |
| `plugins.<id>.*` (engine halves) | not possible: `setting_error` refuses unknown keys | engine namespace with a schema from the extension manifest |
| `core.reading.markAsReadDelay`, workspace, appearance, notifications, keymaps, themes | config | stay client |

For engine-written strings the engine embeds a small catalog (quote attribution, forward
header, resurfacing bodies, `draft.validate` messages) generated from the desktop's
localization files by a script, so translations keep one source.

### 4.5 Engine extension hooks

Extensions stay out of process, as service plugins are today (`examples/service-plugin`), so
an extension can't crash or block the engine beyond its deadline. A service connects to the
socket and registers:

```
hooks.register {hooks: [{name, filter?, timeoutMs, onTimeout: "skip" | "fail"}]}
hook.call      engine -> extension request {name, params} -> result
```

| Hook | Kind | Replaces |
|---|---|---|
| `draft.prepare` | sync, may edit the draft | `ComposerExtension.prepareNewDraft` |
| `draft.validate` | sync, adds warnings/errors | `warningsForSending` |
| `draft.transform` | sync, per-recipient bodies before send | `applyTransformsForSending`, `needsPerRecipientBodies`, and with them `clientFinalize` and `draft.send {bodies}` |
| `send.action` | sync, runs after send | plugin `sendActions` (the `after` list becomes open) |
| `task.before` / `task.after` | sync veto/modify / async | `onSendSuccess`, prototype patching by outgoing-undo, mute |
| `message.received` | ordered after rules, before `mail.new` | invitation refresh, package tracker, mute |
| `message.annotate` | at arrival, stored | extension fields on Message JSON (`ext.<id>`), extra `warnings` |
| `message.render` | at arrival or first read, cached | `MessageViewExtension.formatMessageBody` (tracker stripping, PGP) |
| `schedule` | timer | re-sent `metadata-expiration` deltas |

Plus `ext.call {extension, method, params}`, routed by the engine to the extension's
connection, so a web or mobile UI half can reach its engine half; extension features in
`hello`; and the `plugins.<id>.*` settings namespace. Bundled behaviour stays Rust built-ins
(tern-compose, executor, rules, triage); hooks are for third-party code. A JS SDK
(`packages/extension`, on `@tern/client`'s Node entry) lets plugin authors write the engine
half in JS, run by `tern service install` like any service.

### 4.6 Message display rules

`messages.get {body: "display"}` (and the `body` include on live queries) returns the body with
tracker pixels stripped by the rules in `crates/tern-compose/src/pixels.rs` and `desktop_data.json`
(today applied only to draft quotes), and a `remoteImages` decision from `reading.remoteImages`.
The client still sanitises for its own renderer and enforces CSP.

### 4.7 Performance: thread list over the socket vs direct SQLite

What we know: the only measurement is `apps/engine/test/test_query_latency.py`, 1.3 ms
median per page on 1,500 single-message threads. A scroll step costs 3 sequential round trips
(ids, missing threads, messages of threads). Each connection answers reads one at a time. Full
model deltas go to every window. There is no reply budget.

Measure first, before any engine work in section 4.1:

1. A real-profile fixture: an anonymised copy of a large profile (100k+ messages, long Gmail
   threads, several accounts), and the 65 captured shapes plus the grammar search queries.
2. For each shape, SQLite in-renderer vs `query` over the socket, end to end in a window
   (JSON encode, socket, parse, model inflation): p50/p95 for the first thread-list page, a
   scroll step, opening a 50-message thread, a grammar search.
3. Delta-to-render: star and archive with `core.engine.reads` on each value.
4. Idle CPU per window during a sync burst, with 1 and 3 windows open.

Targets: first page and scroll step p95 under 30 ms end to end, thread open under 50 ms,
delta-to-render under 50 ms. Measured 2026-10-25 on a synthetic profile
(`docs/thin-clients/perf.md`): met, and reads flipped; first pages of long threads miss. If the plain `query` route already meets them on the real profile,
flip `core.engine.reads` to `engine` right away (stage 2). Then the planned changes each remove
a known cost: summaries remove two of three round trips, live queries let windows stop taking
raw deltas, the per-connection read pool removes head-of-line blocking. Keep
`core.engine.reads` as a kill switch until `DatabaseStore`'s SQLite path is deleted.

## 5. Plugins

### 5.1 Engine extensions vs UI extensions

- **UI extensions** are per client: slots, sheets, settings tabs, commands, `bodyRendered`
  DOM decoration, `filterFiles`. plugin-api-v2.md describes the desktop's. The web has no slot
  system; when it gets one it is its own API with the same manifest.
- **Engine extensions** are services with hooks (4.5), installed once per engine and running
  for every client, including executor sends and agent actions.

One manifest describes both halves.

### 5.2 Changes to `docs/desktop/plugin-api-v2.md`

| Section | Holds | Changes |
|---|---|---|
| Inputs | "no plugin code runs inside the engine, background logic is a service plugin" | Still true for process isolation; service plugins become engine extensions with hooks |
| 2.2 manifest | `id`, `windowTypes`, `engineMethods`, `engineFeatures` | `service` becomes `engine: {bin, hooks, settings (schema), methods, features}`; the UI half keeps `main`/`windowTypes` |
| 3.2 slot methods | all 20 | none |
| 3.3 `addComposerExtension` | | moves to engine hooks `draft.prepare`, `draft.validate`, `draft.transform`, `task.after`; v2 has no window-side composer hooks |
| 3.3 `addSendAction` | menu entry in the UI | `perform` becomes an engine `send.action` |
| 3.3 `addMessageViewExtension` | `bodyRendered`, `filterFiles` | `formatBody` -> `message.render`; `allowsRemoteImages` -> `reading.remoteImages` policy and Message `remoteImages` |
| 3.4 events | `onAction`, `listenTo`, `this.engine.*` | `onDatabaseChange` -> `this.app.live(query)`; `onMetadataExpired` leaves the UI API (due work never runs in a window) |
| 4 `tern` module | | models are typed JSON from `@tern/client` plus helpers; `this.app.database` is the shared client's `query`/`live`; `tasks` exposes intent RPCs, not `TaskFactory` |
| 6 engine access | `this.engine`, `engineMethods`, the approval prompt for local-only methods | Unchanged for the UI half. The engine half connects as a service with its own `hello.client`; its hooks and `ext.call` methods are shown and approved with the plugin, not per call |
| 7 storage | `this.cache`, `this.dataDir` | `this.settings` splits: client prefs in config.json, account-level settings in the engine under `plugins.<id>.*`; `this.metadata` gains typed helpers for built-ins |
| 13 migration | | rows for send-later, thread-snooze, send-reminders, phishing-detection, composer-templates, ai, activity change (table below) |
| 14 stages | | stage 4 (`metadata`, events) waits for live queries; a new stage for the `engine` manifest block after 4.5 lands |

### 5.3 v1 plugins

They keep loading until 2.0 as plugin-api-v2.md section 5 promises. What changes under them:
`DatabaseStore`, `QuerySubscription`, `Matcher` and `Attributes` become facades over `query`,
`query.subscribe` and the raw feed (SQL compilation and `Matcher.evaluate` are gone). No bundled plugin calls
`Model.matches`/`Matcher.evaluate`; core does in `features/thread-list/store.ts:126` and
`components/list/selection.ts:130`, which move to live-query membership in stage 6. I haven't
checked the third-party plugins in plugin-api-v2.md's survey for it. `ComposerExtension` hooks keep running in the window for v1
plugins, so `clientFinalize` and `draft.send {bodies}` stay until 2.0, used only when a v1
plugin registers `applyTransformsForSending` or a send action. 2.0 removes them with the rest
of v1.

### 5.4 Where each bundled plugin ends up

| Plugin | Engine half | UI half (v2) |
|---|---|---|
| activity | `tracking.events`, `tracking.stats` replace the renderer scan (`dashboard/root.tsx`, `engagement/stats.ts`, `events.ts`) | sheet, feed, dashboard, CSV save dialog, notifications |
| ai | built in (`ai.*`, triage); `ai.triage.correct`, live `ai.triage.list` | thread header, reply suggestions, chip, settings |
| composer-grammar-check | `integrations.grammar`; language and disabled rules as engine settings | editor marks, toggle |
| composer-templates | `templates.*` (built in), `draft.validate` covers its warning; `store.ts` and `file.ts` go | picker, footer, prefs tab |
| link-tracking | built in (`tracking.rs`); `draft.update {tracking}` | composer toggle, click dots (`bodyRendered`) |
| open-tracking | built in; `draft.update {tracking}` | toggle, list icon, header status |
| participant-profile | `integrations.enrich` with a persistent cache on Contact | sidebar sections |
| phishing-detection | built in (`derived.rs`); plugin logic and `tld` go | renders `message.warnings` |
| send-later | built in (executor); `draft.schedule`; fallback in `main.ts` goes | composer button, draft status |
| send-reminders | built in; `threads.remind`, `acknowledgeReminder`; fallback goes | buttons, headers, list timestamp, sidebar item |
| thread-sharing | none (sharing doesn't exist in engine or backend) | disabled button |
| thread-snooze | built in; desktop calls `threads.snooze` once undo exists; `utils.ts` moves go | buttons, popover presets, sidebar item |
| translation | `messages.translate` with block extraction and cache; local language detection; settings | header banner, composer button |

Built-in features with an engine half: `composer-signature` (JS insertion goes;
`signatures.apply` and `signatures.render` for the dropdown and templates),
`remove-tracking-pixels` (both halves move: drafts already engine-side, display via 4.6),
`message-autoload-images` (policy moves, banner stays), `list-unsubscribe` (parse and POST go,
banner and `shell.openExternal` stay), `events` (parsing goes behind `events.invitation`),
`send-and-archive` (JS perform goes), `unread-notifications` (legacy filter goes, display
stays), `undo-redo` (toast over `undo.*`).

## 6. Classification

Readers' verdict counts, as the notes report them (not recounted: many rows carry mixed
verdicts such as "engine (already); fallback delete"). "engine" means the logic moves to or already is in the engine and the
TS goes; "split" means a UI half stays; "shared" means TS both clients need, in
`@tern/client`.

| Area (notes file) | Rows | engine | split | client | delete | shared |
|---|---|---|---|---|---|---|
| Desktop data layer (`desktop-data.md`) | 87 | 10 | 50 | 16 | 11 | (in split) |
| Desktop app outside data (`desktop-app.md`) | ~110 | 13 | 30 | 55 | 8 | 6 |
| Plugins and behaviour features (`plugins.md`) | 56 | 14 | 13 | 24 | 5 | |
| Web (`web.md`) | 68 | 17 | 25 | 25 | 1 | (in split) |
| Engine side and CLI/MCP (`engine.md`) | 57 | 14 | 19 | 9 | 15 | |

The units that carry most of the work, with their replacement:

| Unit | Verdict | Replacement |
|---|---|---|
| `src/data/database/store.ts`, `agent.js`, `main/ipc/database-agent.ts` | delete (facade stays for v1) | `query`, `query.subscribe` |
| `src/data/query/*` (1.9k) | split | JSON builder in `@tern/client/query`; `sql()`, result-set diffing go |
| `src/data/attributes/*` (1.1k) | split | JSON (de)serialisation only; SQL fragments and `evaluate` go |
| `src/data/models/*` (3.4k) | split | generated JSON types + `@tern/client/models`; derived fields from the engine |
| `src/data/tasks/factory.ts` | engine | `threads.*` intent RPCs |
| `src/data/tasks/*` other | split | JSON builders for v1; undo -> `undo.*` |
| `src/data/drafts/factory.ts` | delete JS path | `draft.create` |
| `src/data/drafts/{editing-session,change-set,store}.ts` | split | `draft.validate`, `draft.update`; session and popouts stay |
| `src/data/stores/account.ts` | split | `account.list` only |
| `src/data/stores/contact.ts` | engine | `contacts.search` cross-account, `contacts.list` |
| `src/data/stores/undo-redo.ts` | split | `undo.*`; toast state stays |
| `src/data/stores/task-queue.ts`, `outbox.ts` | split | pending-task subscription, `task.done`, `drafts.pending` |
| `src/data/stores/thread-counts.ts` | split | `counts.get` + deltas; SQL fallback goes |
| `src/data/mailbox-perspective.ts` | split | query building to `@tern/client`; removal semantics to `threads.removeFromView` |
| `src/mail/search/*` | engine (delete) | `search {grammar: true}` |
| `src/mail/mbox.ts`, `mbox-export-runner.ts` | engine | `export.start {format: mbox}` |
| `src/mail/imap-utf7.ts` | delete | `displayName` on Folder/Label |
| `src/engine/protocol.ts` | delete | `@tern/client/protocol` (generated) |
| `src/engine/client/{connection,events,bridge}.ts` | split | `@tern/client` rpc, session, live |
| `src/engine/client/oauth-revocation.ts` | delete | `account.remove {revoke}` |
| `src/engine/supervisor/{host,settings}.ts`, most of `supervisor.ts`, `process.ts` | delete / split | `tern` units, engine restart policy, settings in the engine |
| `features/thread-list/data-source.ts` | engine | thread `summary` include |
| `features/thread-search/query-subscription.ts` | engine | live grammar search |
| `features/events/{header,rsvp-target,invitation-calendar-refresh}` | split / engine | `events.invitation`, engine-side refresh |
| `features/list-unsubscribe/service.ts` | delete | `messages.unsubscribe`, `unsubscribe` field |
| `features/message-autoload-images/store.ts` | split | `reading.remoteImages` |
| `features/remove-tracking-pixels` | engine | `pixels.rs` for drafts and display |
| `features/draft-list/store.ts` | split | `drafts.pending` |
| `features/unread-notifications` | split | richer `mail.new` |
| `features/preferences/tabs/mcp/policy.ts` | split | `tern mcp policy get|set` (to add) |
| `features/main-calendar` (8.9k, 15 `DatabaseStore` sites) | split | `events.subscribe` for the range, Calendar subscription; grid, drag and recurrence UI stay |
| `features/contacts/store.ts` | split | `contacts.list {perspective}`; the three live queries become subscriptions |
| `src/data/stores/mail-rules.ts`, `src/data/mail-rules-templates.ts` | split | `mailRules` setting, `rules.*`, `rules.schema`; the editor stays |
| `features/onboarding/{connection-security,account-providers}` | split | `account.discover` for servers; the port/security rule into `@tern/client` (web has a copy) |
| `src/mail/eml.ts` | split | `messages.raw`; save dialog stays |
| `plugins/*` | see 5.4 | |
| web `engine/cache.ts`, `optimistic.ts`, `tasks.ts` | delete | `LiveQuery`, intent RPC results |
| web `features/composer/{draft,identities,signature,templates}.ts` | engine | `draft.create`, `templates.*`, `signatures` setting |
| web `features/mailbox/actions.ts` | engine | `threads.*` |
| web `engine/mock/*`, `features/ai/mock.ts` | client (dev), stop extending | fixture-seeded engine |
| `apps/cli/src/{organize,mbox,followup,draft}.rs` parts | split | intent RPCs, `export.start`, `tracking.events`, `drafts.pending` |
| `crates/tern-client/src/tasks.rs` | split | intent RPCs |

## 7. Migration

Each stage ships with the app working. Sizes: S up to 3 days, M 1-2 weeks, L 3+ weeks, for
one person or agent. Tracks: **E** engine (Rust), **P** shared package and web, **D** desktop.

| # | Stage | Track | Size | Depends on | Ships |
|---|---|---|---|---|---|
| 0 | Delete old-engine fallbacks and dead code | D, P | M | none | the `Feature.*` fallbacks in 19 desktop files (47 references, 2.2), 19 web files; JS `DraftFactory` path, JS send path, `oauth-revocation.ts`, send-later/snooze/reminder fallback executors (send-later's handler for `clientFinalize` rows stays: v1 transforms, until 2.0), web `send-later-runner.ts`, `use-unified-threads.ts`, `ThreadCounts` SQL, `EventRSVPTask.forReplying`, `scenario-editor` `evaluate`, MCP `compose.rs`/`search.rs` fallbacks; clients then require `serve`, and harness tests that drive the CLI or MCP against `--mode sync` engines move to `serve`. Roughly -5k lines. **Done 2026-10-25 (D, P):** desktop and web assume a current bundled engine; checks that only meant "connected" read `features` or `EngineEvents.whenConnected()`. Gone: the `Feature.*` fallbacks, the JS `DraftFactory` path (new, reply, forward and mailto drafts are `draft.create` only, and every `prepareNewDraft` hook runs on them), the built-in signature and tracking-pixel draft hooks, the supervisor's non-registry account path and reset straggler wait, the settings mirror's per-feature keys, `oauth-revocation.ts`, the snooze, reminder and send-later fallback executors, `ThreadCounts` SQL, Message-delta new-mail notifications, the JS unsubscribe, `scenario-editor` `evaluate`; web `send-later-runner.ts`, `use-unified-threads.ts`, non-grammar search and the other pre-feature branches. `DraftFactory.prepareBodyForQuoting`, `createDraftForResurfacing` and `EventRSVPTask.forReplying` are throwing stubs until 2.0 (plugin API 1.37.0). The draft corpus keeps its quoting, reply, forward, mailto and pixel cases as last generated. Deferred: the MCP `compose.rs`/`search.rs` fallbacks, CLI and MCP requiring `serve`, and the harness `--mode sync` tests (Rust, track E); DraftStore's send for plugin send actions stays with `clientFinalize` (v1, until 2.0); legacy keyset keys are still read until stage 9 moves them |
| 1 | Adopt what the engine already has | D, P | M | 0 | Desktop: `draft.validate`, `draft.update|delete`, `templates.*`, `files.get`, `warnings` in phishing-detection, search bar on `search {grammar}`; `structuredSearch()` keeps the query text so `Query.toJSON()` sends `search {text, grammar: true}` instead of throwing (3 call sites incl. participant-profile, and v1 plugins; `src/mail/search` has no AST-to-text printer). Web: `draft.create|send|validate`, `templates.list|get`, `counts.get` + `ThreadCounts`, `events.expand`, `warnings`, `hidden`; stale `TODO(engine)` markers fixed. **Done 2026-10-25 (D, P):** desktop validation is `draft.validate` (`isValidDraft` returns a Promise, findings localized by code, plugin API 1.38.0), `destroyDraft` is `draft.delete`, composer-templates is on `templates.*` (`file.ts` and its warning extension gone), attachment paths come from `files.get`, phishing-detection reads `warnings` from `messages.get` (`tld` dropped), and `structuredSearch()` keeps its text so `Query.toJSON()` sends `search {text, grammar: true}`, with the search bar on it and `backend-local.ts` gone. The web builds drafts with `draft.create`, sends with `draft.validate` + `draft.send` (undo is `draft.cancelSend`), lists and inserts `templates.list|get` (its per-browser templates are gone), reads counts from `counts.get` and `ThreadCounts`, the agenda from `events.expand`, shows `warnings` and drops `hidden` messages; the six stale markers are fixed. New for desktop users: the `template-variable` warning. Deferred: `draft.update` for the desktop session's saves (it has no `pluginMetadata`/`replyTo` and refuses scheduled drafts); synchronous `pathForFile` callers keep the local path, `files.get` covers open, save and preview; web template save/delete wait for `templates.save|delete` on relayed devices; web scheduling keeps writing send-later itself (`draft.schedule`, 8a); the mock got minimal `draft.*`, `templates.*`, `counts.get` and `events.expand` handlers (the removed web builders moved there) so dev and tests keep working |
| 2 | Measure, then flip reads | D, E | S-M | 1 (search) | Real-profile harness (4.7); `core.engine.reads` default `engine` if targets hold; database agent unused. **Done 2026-10-25 (D, E):** a synthetic profile instead of a real one (`crates/tern-sync/examples/perf_profile.rs`: 100k messages through `MailProcessor`, 33k threads, a Gmail-style and two IMAP accounts, contacts, calendars), measured in the desktop's spec window against a release `serve` engine (`tests/integration/thin-perf.test.ts`, driven by `apps/engine/test/tools/thin_perf.py`); numbers in `docs/thin-clients/perf.md`. The targets hold (first page p95 at most 24 ms, scroll 16, thread open 3, delta to render 7), so `core.engine.reads` defaults to `engine` and `"sqlite"` is the kill switch; specs keep `"sqlite"` with `_query` stubbed unless they install `MockEngine`/`FixtureReads`. With reads on the engine the database agent only serves the kill switch. Known miss: first pages whose 200 threads carry more than about 8 messages each (the `--busy-top` stress profile: 33-113 ms) pay about 11 ms per MB of message JSON for the list's messages join, which stage 5's thread `summary` removes; the 30k-contact list and 1,000-thread searches pay the same (no target). Deferred: idle CPU per window during a sync burst (4.7 item 4, the profile engine syncs nothing), the 65 captured shapes one by one, multi-window and gateway runs |
| 3 | Types | E, P | M | none | `schemars` on `tern-ipc`, model view structs and their round-trip test, allowlist generated from Rust, `packages/client/src/protocol` generated with a CI check; `protocol.ts`, `rpc.ts`, `types.ts` become re-exports (TD26). **Done 2026-10-25:** view structs in `tern-ipc` `models.rs`/`tasks.rs` checked by `crates/tern-sync/tests/view_structs.rs`, allowlist from `query_attrs.rs`, `tern-ipc-schema` + `generate.mjs` with the `protocol` CI job; the web's `account.test` check (`ok`, which the engine never sends) fixed on the way. Deferred: typed `integrations.*` results and `ContactDetails.data`; the envelope types stay in the apps until stage 4 |
| 4 | Shared client core | P, D | M | 3 | `@tern/client` rpc, session, transports, settings cache, `task.done` handling; web switches first, then desktop `connection.ts`/`events.ts`. **Done 2026-10-25 (P, D):** `packages/client` has `rpc` (envelope, `RpcError`, `EngineUnavailableError` with `code: null`, the `Transport` interface, `RpcPeer`: timeouts, answers in any order, handlers for requests from the engine), `session` (hello and subscribe on each connection, features and capabilities, backoff, calls held for the handshake or until open, `resubscribe`, an `onEnd` step per client: retry, stop, or wait for a new endpoint), `transports/ws`, `transports/unix` behind `@tern/client/node`, `SettingsCache` and `waitForTask` (`task.done` when the engine sends it, `Task` deltas until then), with vitest specs run by the `protocol` CI job. The web's `WsEngineClient` is the session plus the gateway's session probe, close codes and file routes; its envelope, `EngineUnavailableError`, backoff, settings store and `waitForTask` come from the package. The desktop's `EngineConnection` is the session over `unixTransport`, keyed by engine generation, keeping its account routing, offline queue, raw delta batches and `engine.status` ProcessStates; `EngineConnectionError` is gone (a refusal is `RpcError`, no answer `EngineUnavailableError`, as `EngineEvents.request` now rejects too), and the desktop's envelope types are the package's. Small changes: the desktop answers requests from the engine with `MethodNotFound` instead of ignoring them, and a status for the same engine during a reconnect backoff no longer cuts the backoff short. Deferred: the desktop's use of `SettingsCache` (stage 9, with the `AppEnv.config` facade and the persisted cache), `task.done` and `task.queue {notify}` in the engine (stage 5), and `EngineEvents`, which stays the windows' facade over the connection (features, `whenConnected`, engine state). Packaging fix from the QA (2026-10-26): the packaged desktop carried all of `packages/client` with its `node_modules` (about 82 MB of dev tooling); `build/build.js` now copies a workspace package's `package.json` and its `files` only, requires its runtime dependencies to be desktop dependencies, and fails the build if the asar carries anything else of `node_modules/@tern/*` (`packages/README.md`) |
| 5 | Engine read surface | E | L | 2 (numbers), 3 | `query.subscribe|setWindow|unsubscribe`, thread `summary`, derived fields on rows and deltas, delta `seq`, reply budget, timeouts, read pool, `task.done`, `events.subscribe`, live `ai.triage.list`. **Part a done 2026-10-26 (E):** `query.subscribe|setWindow|unsubscribe` with `query.changed` (`tern-ipc` `live.rs`, engine `live.rs`): a subscription is a `query` without paging plus an `{offset, limit}` window, run on its own `liveQueries` thread and read store; a flush that touched the query's class re-runs the window as ids plus a count, at most once per 50 ms, and sends `remove`/`insert`/`move`/`update` steps (the longest run of rows that kept their order stays) or `reset` when the steps would outnumber the rows, with `version` one up per change; `key` replaces a subscription on the same connection, and a relayed window stops at `READ_MAX_LIMIT`. `events.subscribe|unsubscribe` sends `events.changed` with the whole occurrence list when an `Event` or `Calendar` flush changed it. `delta` carries a per-process `seq`; `subscribe` returns `{feed, seq}` and takes `since {feed, seq}`, replaying from an 8 MiB ring (`resumed: false` when it no longer holds them or the feed is another process's). `task.queue {notify: true}` answers as before, then sends `task.done {accountId, taskId, status, error, task}` once to that connection (also when the task finished before the wait began). Reads stop after 30 s (`READ_TIMEOUT`, SQLite progress handler) and read answers over 32 MiB are refused (`REPLY_TOO_LARGE`). Features `live-queries`, `events-subscribe`, `delta-seq`, `task-done`, `read-limits`; `@tern/client` types regenerated. Fixed on the way: the rules and AI-triage delta taps dropped the item and the flush, which the socket's sink now needs. Harness: `test_live_queries.py` (a star through `task.done` and `query.changed` on `serve`). Deferred to part b: thread `summary`, derived fields, the read pool, live `ai.triage.list`, and re-runs for other classes a query depends on (a subscription re-runs only on deltas of its own class, so a `Folder`/`Label` rename does not reach thread rows that embed it). Also deferred: keyset windows (offsets now), the reply budget on notifications, re-running event subscriptions only for their own calendars, remote-failure revert (stage 7), and clients using `notify` and live queries (stage 6). **Part b done 2026-10-26 (E):** `include: ["summary"]` on Thread `query` and live-query rows (`tern-store` `summary.rs`, `ThreadSummaryJson`): `messageCount`, `senders` (with `unread`, and `me` from every account's address and aliases), `hasDraft`, `sending`, `hasAttachment`, `snippet` and `lastReply`, read from the window's messages in the same read with hidden ones left out; a Thread subscription with `summary` re-runs when a message of one of its threads changes. Derived fields (`tern-store` `derived.rs`, where `warnings.rs` moved): `hidden` and `warnings` on Message, `displayName` and `sortOrder` on Folder and Label, on the rows of `query`, live queries and the read methods and on the socket's deltas (stdout's stay as stored). The socket answers a connection's reads out of order on up to 4 read connections on the `reads` lane (`read_pool.rs`). `ai.triage.subscribe|unsubscribe` with `ai.triage.changed` (the whole list, after a Thread flush of its accounts or a `core.ai.*` change). Features `thread-summary`, `derived-fields`, `read-pool`, `ai-triage-subscribe`; `@tern/client` types regenerated. Perf (`docs/thin-clients/perf.md`): summary first pages p95 5.6-11.7 ms, 6.0-25.1 on the stress profile; the messages-join slowdown in the first run was noise, not the pool or the derived fields. Deferred: a cheap `total` for live queries (the count costs 6-12 ms on 5-10k-thread lists, so the live first windows of unified sent and all mail miss 30 ms on the stress profile), re-runs on Folder/Label changes for the Thread rows that embed them, the other derived fields of 4.1 (label colour, `unsubscribe`, `remoteImages`, the invitation flag; 8c, 8e), and `ai.triage.changed` as steps |
| 6 | Live queries in the clients | P, D | L | 4, 5 | `@tern/client/live`; desktop `QuerySubscription`, `fromQuery`, `MessageStore`, `TaskQueue` over it with the same APIs, so UI code doesn't change; thread list on `summary`; web `cache.ts` replaced by a TanStack adapter; core UI code stops depending on raw deltas, but a window keeps the raw feed while the v1 `DatabaseStore.listen` facade or a plugin listener needs it (until 2.0) |
| 7 | Intents and undo | E, then D, P, CLI | L | 5 (`task.done`) | `threads.*`, `categories.*`, undo log and `undo.*`, remote-failure revert; `TaskFactory` delegates, `UndoRedoStore` over `undo.list`, desktop snooze on `threads.snooze`; web undo; `tern undo`, MCP undo; CLI/MCP organize logic and `tern-client` task builders go; web `optimistic.ts` goes **Engine part done 2026-10-26 (E):** `threads.archive|trash|spam|notSpam|move|label|markRead|star|removeFromView` take `threadIds` or `messageIds` of any accounts and an optional `perspective {categoryIds, starred, unread}`; each account's tasks are built on its `main` thread before any is queued (`tern-tasks` `intents.rs`, a port of `TaskFactory`, `tasksForRemovingItems` and the Gmail label/folder rules), so an unknown folder or label fails the call with nothing done, then queued in chunks of 500 ids. The answer is `{taskIds, undo, models, unchanged}`: the Thread rows (Message rows first for `messageIds`) the local phases changed, up to 500, and the ids nothing changed. `categories.markRead|create|rename|delete|empty` the same way (only `markRead` is undoable). The undo log (engine `undo.rs`, in memory, 100 entries, 24 h) keeps one entry per undoable intent, `threads.snooze`, `draft.send` inside its undo window (undone with `draft.cancelSend`) and `undo.record` (tasks a client queued itself); an entry belongs to the `hello.client` up to its `/` (`@device` when relayed). `undo.apply|redo {token?, any, tz, notify}` take the caller's own latest entry without a token; `undo.list {mine, limit}` shows everyone's, newest first; `undo.changed` goes to the subscribers of the entry's accounts. The inverses are built in Rust (`tern-tasks` `undo.rs`): the flag and label tasks now record each message they changed (`undoFlags`, `undoLabels`), so an undo puts back exactly those; moves use `undoPlacements`; snooze metadata carries `undoValue`; events, contact groups and their memberships as the desktop's `createUndoTask`. Remote failure: a flag, label or move task that fails for good puts back the change on the messages the server didn't take (moves already did), marks the task `reverted`, `task.done` carries `reverted`, and the entry leaves the log as `failed`. Features `intents`, `undo`, `remote-revert`; `@tern/client` types regenerated; harness `intent-archive-undo-redo` and `intent-remote-failure-reverted` (serve), and `client.rpc` `expect` takes dotted paths. Deferred to 7c: every client change (`TaskFactory` delegating, `UndoRedoStore` over `undo.list`, desktop snooze on `threads.snooze`, web undo and `optimistic.ts`, `tern undo`, MCP undo, the CLI/MCP organize logic); also an intent whose second account's queue fails keeps the first account's tasks queued without an entry, `undo.changed` for an expired entry is only sent when the log is next read or written, and the labels are English only (clients word them from `action` and `count`). |
| 8a | Drafts and compose | E, D, P | M | 7 | `draft.create {asAttachment}`, `signatures.apply|render`, `draft.schedule`, tracking toggles; web composer on the engine |
| 8b | Contacts | E, D | M | 6 | `contacts.list`, cross-account search, delete, groups; `ContactStore` search goes |
| 8c | Calendar and invitations | E, D, P | M | 6 | `events.invitation`, engine refresh on invite, `events.search`; ical.js leaves `features/events` and web |
| 8d | Export and files | E, D | M | none | `messages.raw`, `export.start` with mbox; `mbox.ts`, runner, CLI `mbox.rs` go; attachments via `files.get|read` |
| 8e | Message display rules | E, D, P | M | 5 | display tracker stripping, `reading.remoteImages`, `unsubscribe` field, body-link unsubscribe |
| 8f | Tracking and activity | E, D, CLI | M | 6 | `tracking.events|stats`; activity renderer scan, CLI/MCP followup flattening go |
| 8g | Notifications and outbox | E, D | S | 6 | richer `mail.new`, `drafts.pending` |
| 9 | Settings and accounts | E, D, P | M | 4 | keys in 4.4 move, `AppEnv.config` facade, engine string catalog, settings mirror and config.json `accounts` go (one migration release), the parent-channel methods over the detached engine's socket (3.4), KeyManager's remaining secrets moved before `credentials.import` (stdin only) becomes unreachable, a persisted read-only `SettingsCache` for before hello, supervisor down to `unit.ts` + Windows spawn, relayed write policy for signatures/templates/integrations, `tern mcp policy get|set`, `tern-mcp`'s config.json fallbacks removed |
| 10 | Engine extensions | E, D | L | 7, 9 | `hooks.register`, `hook.call`, `ext.call`, `plugins.<id>.*` settings, extension features; plugin-api-v2.md updated (5.2); `packages/extension` JS SDK |
| 11 | Plugin split | D | M | 8*, 10 | bundled plugins to v2 as UI-only (5.4), merged with plugin-api-v2.md stage 8 |
| 12 | Remove `DatabaseStore`'s SQLite path | D | M | 6, 7, 8* | SQL compiler, `Matcher.evaluate`, attribute SQL, the database agent, better-sqlite3 from the renderer; v1 facades stay; `core.engine.reads` removed |
| 13 | Renderer Node cleanup | D | S-M | 8d, 9, 12 | the ~12 files in 3.4 lose Node; core renderer code reaches Node only through `platform/` and the transport |

**Parallel work.** Stages 0, 1 and 3 can start together (0 and 1 desktop and web, 3 engine and
package). Stage 2 can run beside 3. Once 5 lands, 8b, 8c, 8e, 8f and 8g are independent of each
other and of 7; 8d needs nothing and can go any time. 9 needs only 4 and can run beside 5-8.
10 waits for 7 and 9. 12 and 13 are last.

**Behind the same APIs.** Stages 6 to 8 keep the desktop's store and model APIs
(`QuerySubscription`, `Rx.Observable.fromQuery`, `MessageStore`, `TaskFactory`, `AppEnv.config`,
`DatabaseStore.find*`) and swap what is under them, so feature code changes only where a unit's
verdict says so. Each domain stage removes its TS logic in the same release it switches.

## 8. Risks and open decisions

| # | Question | Recommendation |
|---|---|---|
| 1 | Generate from `tern-wire` as asked, or `tern-ipc`? | `tern-ipc`. It is the socket protocol; `tern-wire` is the backend wire. |
| 2 | `schemars` + JSON Schema or `ts-rs`? | `schemars`: one schema for TS, mobile and MCP. `ts-rs` is simpler if mobile is far off. |
| 3 | Where model types come from | Rust view structs in `tern-ipc` checked against tern-store output; allowlist generated from them. Not typed structs in tern-store (would break the "unknown keys survive" rule). |
| 4 | Live-query evaluation: re-run window and diff, or incremental in memory | Re-run and diff. One compiler, no fourth matcher; revisit only if stage 2 numbers say so. |
| 5 | Thread list slower over the socket than in-process SQLite | Measure first (stage 2); summaries, live queries and the read pool remove the known costs; keep `core.engine.reads` as a kill switch until stage 12. |
| 6 | Undo scope: per client or per engine | Per client by default, with `--any` / `undo.list` for everything. An agent's change shouldn't be undone by Cmd-Z in the app. |
| 7 | Engine extension runtime: out of process or embedded JS/WASM | Out of process with deadlines, plus a JS SDK. Keeps plugin-api-v2.md's "no plugin code in the engine". |
| 8 | v1 plugin compatibility | Keep `DatabaseStore`/`QuerySubscription` facades and window-side `ComposerExtension` for v1 until 2.0; drop SQL and `Matcher.evaluate` earlier. |
| 9 | `contextIsolation` for app windows | Not a goal of this plan. Revisit with plugin API 3.0 or a separate plugin host; this plan only keeps core code's Node use behind `platform/` and the transport. |
| 10 | Desktop transport: `net` in the renderer or proxied through main/a utility process | Keep `net` in the renderer behind `@tern/client/node`. A proxy adds a hop and buys nothing while windows have Node anyway. |
| 11 | Optimistic layer in clients | None on the local socket; intent RPC results carry post-local models for the gateway; web `optimistic.ts` goes with stage 7. |
| 12 | Engine-written strings | The engine embeds a catalog generated from the desktop's localization files and takes `intl.locale`/`intl.hourCycle`. |
| 13 | Which borderline keys move (`disabledCalendars`, `labelColors`, `markAsReadDelay`) | Move the first two (data the web and mobile show the same way); keep the reading delay per client. |
| 14 | Templates and signatures from relayed devices | The engine sanitises their HTML on write (`tern-compose/sanitize.rs`), then relayed writes open. One template store: the engine's files. |
| 15 | Windows has no `tern` units | Main keeps a direct spawn path on Windows; everything else in the supervisor goes. |
| 16 | The web mock engine | Stop extending it now; replace with a fixture-seeded real engine for dev and tests, then delete it. |
| 17 | Remote-failure rollback changes behaviour | Apply the undo locally and notify the caller; covered by an engine harness scenario before release. |
| 18 | The reader notes live in `/tmp` | Copy them to `docs/thin-clients/` if the long-tail tables should survive; otherwise this doc's section 6 is what stays. |
| 19 | `--mode sync` engines as client targets (they lack every `serve`-only feature) | Drop them: after stage 0 the CLI, MCP and desktop require `serve`; the harness keeps `--mode sync` for engine tests only. |
