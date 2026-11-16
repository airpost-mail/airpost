# Engine surface for thin clients (2026-10-23)

Read-only survey of `apps/engine`, `crates/airpost-ipc`, `crates/airpost-wire`, `crates/airpost-client`,
`apps/cli`, `apps/mcp`, `apps/gateway`, and the client layers that talk to them
(`apps/desktop/src/engine`, `src/data/{database,query,attributes,models,tasks,drafts}`,
`apps/web/src/engine`). Paths are repo-relative.

## 0. Facts that change the plan's wording

- **The engine protocol is `crates/airpost-ipc`, not `crates/airpost-wire`.** `airpost-wire` (670 lines) is
  the engine<->backend metadata/tracking wire (`MetadataPush`, `BackfillRecord`, stream lines,
  `/airpost/v1/...`). A shared TS client package generates from `airpost-ipc` (request/response/notification
  types) and needs `airpost-wire` only if web talks to the backend directly.
- **The desktop already has an engine read path.** `core.engine.reads: "engine"` (default `"sqlite"`,
  `src/config/schema.ts:32`) routes every `DatabaseStore.run()` through `query` as `Query.toJSON()`
  (`src/data/database/engine-reads.ts`, `store.ts:_inflated`). The flip to default was gated on a
  real-profile latency measurement, the stage 9 desktop search half and a trial day
  (commit 189b89cac, docs/desktop-client-plan.md, since deleted). Exceptions that stay on SQLite
  regardless: `structuredSearch()` queries (search bar), `ThreadCountsStore` raw
  `DatabaseStore._query('SELECT * FROM ThreadCounts')` fallback, and the `database-agent`
  background process (`src/main/ipc/database-agent.ts`, `src/data/database/agent.js`).
- **The engine's `query` is a port of the desktop's `ModelQuery`.** `crates/airpost-store/src/model_query.rs`
  (1388 lines) compiles `Query.toJSON()` to the same SQL as `Query.sql()`; the class/attr allowlist
  (`model_query_allowlist.json`: Thread 12 attrs, Message 11, Folder 4, Label 4, Contact 7,
  ContactGroup 2, ContactBook 2, Event 8, Calendar 2, Task 3, File 3) is *generated from the TS
  models* by `apps/desktop/tests/integration/model-query-capture.test.ts`. The direction of truth
  is TS -> Rust today; removing the TS models means moving that source of truth into Rust.
- **No plugin code runs in the engine.** The only extension mechanism is out-of-process "service
  plugins" (`examples/service-plugin`, `docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md` "Service Plugins",
  `apps/engine/test/test_service_plugin.py`): a program on the socket reacting to deltas and its own
  `metadata-expiration` rows. No synchronous hooks.

## 1. RPC inventory (airpost-ipc v1, `PROTOCOL_VERSION = 1`)

98 names in `airpost_ipc::method`: 87 requests + 11 notifications. 28 `feature` flags gate them
(`hello.features`). JSON-RPC 2.0, NDJSON over a Unix socket, `MAX_LINE_BYTES` 64 MiB,
per-connection reader thread answers in order (reads on that thread; account work goes to the
account's main thread; slow calls "answered out of order" on worker pools).

| Group | Methods | Notes |
|---|---|---|
| session | `hello`, `subscribe`, `engine.status`, `engine.logs.tail`, `engine.setLogLevel`*, `engine.resetDatabase`*, `engine.shutdown`*, `test.crash`/`test.panic` (debug) | *parent stdin or `--detached` socket |
| reads | `folders.list`, `labels.list`, `threads.list`, `threads.get`, `messages.get`, `search`, `contacts.search`, `tasks.get`, `tasks.list`, `counts.get`, `metadata.list`, `query` | `READ_DEFAULT_LIMIT` 50, `READ_MAX_LIMIT` 500 (not applied to `query` without `limit`) |
| tasks | `task.queue`, `task.cancel`, `sync.wake`, `bodies.fetch`, `calendar.sync` | 21 task classes |
| mail actions | `threads.snooze`, `threads.unsnooze`, `messages.unsubscribe` | |
| drafts | `draft.create`, `draft.update`, `draft.delete`, `draft.validate`, `draft.send`, `draft.cancelSend` | serve only |
| templates | `templates.list|get|save|delete` | files in `<config>/templates` |
| files | `files.get`, `files.read`, `files.put` | `files.get` local-only (returns a path); gateway serves HTTP |
| PIM | `events.expand|create|update|rsvp`, `contacts.get|create|update|import|export` | |
| accounts | `account.list|add|remove|test|reset|discover|update|reorder|setSecrets`, `account.oauth.begin|complete`, `credentials.import`, `secrets.set|delete|has`, `server.set|clear` | |
| settings | `settings.get`, `settings.set` | 23 keys |
| rules | `rules.reprocess`, `rules.cancel` | |
| integrations | `integrations.translate|detect|languages|grammar|enrich|test` | |
| AI | `ai.models|test|summarize|suggestReplies|draftReply|cancel|usage`, `ai.triage.run|list` | streamed via `ai.delta` |
| notifications | `delta`, `account.state`, `account.changed`, `mail.new`, `settings.changed`, `capabilities.changed`, `disconnect`, `ai.delta`, `ai.triage.progress`, `rules.progress`, `rules.disabled` | |

Usage by client (distinct `method::` refs): CLI ~65, MCP 29, web ~41 strings in `rpc.ts`,
desktop ~55. Web doesn't call `draft.*`, `templates.*`, `events.*` writes, `contacts.get|create|update`,
`metadata.list`, `rules.*`; desktop doesn't call `threads.list|get`, `messages.get`, `search`,
`contacts.search`, `templates.*`, `draft.update|delete|validate`.

Relayed (gateway) policy: `crates/airpost-ipc/src/policy.rs` `method_class` (Open / LocalOnly /
ParentOnly / Debug), `SCOPE_ADMIN`, `RELAYED_SETTINGS_KEYS` (5 keys: undoSend,
defaultAccountIdForSend, unsnoozeToTop, composing.html, defaultSignatures), `LOCAL_ONLY_SETTINGS_PREFIX`
`core.ai.`. Relayed devices can't write `signatures`, `templates`, `mailRules` (HTML the desktop
renders is treated as trusted).

## 2. Task model

- `task.queue {accountId, task}` -> account main thread -> `TaskProcessor::perform_local`
  (`crates/airpost-tasks/src/lib.rs:187`): saves the Task row, runs the local phase, sets `remote`,
  saves; answers with the task JSON *after* the local phase, so the local DB change and its deltas
  (main-thread `STREAM_DELAY_MAIN_MS` = 5 ms) are the optimistic apply. Local failure: task ends
  `complete` with `error` (RPC error `TASK_REJECTED` -32002). Foreground IDLE is interrupted after a
  debounce for the remote phase.
- Reconciliation: `perform_local_change_on_messages` (`messages.rs:495`) bumps `_suc` and sets
  `_sa = now + 24h` so sync scans can't revert the change before the remote phase; remote success
  decrements `_suc`. Remote failure: retryable -> `retry_later`, else `fail` -> task `complete` +
  `error`. I found no explicit rollback of the local change on a non-retryable remote failure; the
  hold lapses and a later scan restores server state.
- Undo data written by the engine: `ChangeFolderTask.undoPlacements` (local phase), consumed by an
  undo ChangeFolderTask's `restorePlacements` (`messages.rs:242`); `SyncbackEventTask {restore:
  true}` for event undo/redo; `canBeUndone` on the move data rules write. The undo *stack* and the
  construction of undo tasks are client-side (`createUndoTask` in 10 desktop task classes,
  `src/data/stores/undo-redo.ts` waits up to 10 s for the engine's echo of `undoPlacements`).
- 21 task classes in the engine: ChangeUnread, ChangeStarred, ChangeFolder, ChangeLabels,
  ChangeRoleMapping, SyncbackCategory, DestroyCategory, ExpungeAllInFolder, SyncbackDraft,
  DestroyDraft, SendDraft, SyncbackMetadata, GetMessageRFC2822, GetManyRFC2822, EventRSVP,
  SyncbackEvent, DestroyEvent, SyncbackContact, DestroyContact, SyncbackContactGroup,
  DestroyContactGroup, ChangeContactGroupMembership. Task JSON is built by clients in three places
  (desktop `src/data/tasks` 2241 lines, web `src/engine/tasks.ts`, `crates/airpost-client/src/tasks.rs`).
  Higher-level RPCs exist only for snooze/unsnooze, unsubscribe, drafts and PIM.

## 3. Change stream

- `subscribe {accounts?, classes?}` (socket only). Then `delta` notifications
  `{accountId, modelClass, type: persist|unpersist|metadata-expiration, modelJSONs}` with the
  **full model JSON** (`DeltaParams`, `methods.rs:429`). Coalesced per class in `DeltaStream`
  (`crates/airpost-store/src/delta.rs`): worker threads 500 ms, main thread 5 ms.
- No sequence number, cursor or resume: a client that drops misses deltas; the desktop's
  `EngineReads` emits `resync` and every `QuerySubscription` re-runs. Only the first unfiltered
  subscriber gets a held backlog (`SOCKET_BACKLOG_BYTES` 16 MiB). Slow subscribers past
  `SOCKET_QUEUE_BYTES` (64 MiB) get `disconnect`.
- Filters are account and class only; every window/connection decodes every Thread/Message delta.
- `ThreadCounts` deltas + `counts.get` (feature `counts`) give a resumable pattern for counts only.

## 4. Read/query API

- `query` (`QueryParams`): class, accounts (null = all), where (matcher tree
  `{attr,op,value}`/`and|or|not`), search `{text, grammar, tz, now}` (grammar on Thread only),
  order, limit/offset, keyset `cursor` -> `nextCursor`, distinct, include (`body` on Message),
  idsOnly, count. Rows are stored JSON plus `hidden`. No reply-size budget, no per-request timeout.
- `threads.list` (folder|label|role, `before` cursor), `threads.get` (thread + messages, no
  bodies, with `warnings`), `messages.get` (`body`, files, `warnings`), `search` (plain or grammar,
  multi-account), `contacts.search` (one account), `metadata.list` (plugin id, expiry filters,
  offset), `tasks.list`, `counts.get`.
- `warnings` (phishing, `apps/engine/src/warnings.rs`) only on `threads.get`/`messages.get`, not on
  `query` rows nor deltas, so a desktop on `query` doesn't get them.
- Thread JSON has no message count or per-message senders (web `TODO(engine)` in
  `src/features/mailbox/list/senders.ts`); the desktop thread list does 3 round trips per 25-row
  scroll step (ids, missing threads, messages of threads), per `test_query_latency.py`.

## 5. Settings, accounts, secrets

- Engine settings store (`apps/engine/src/settings.rs`, keys in `crates/airpost-ipc/src/settings.rs`
  `SETTINGS_KEYS`, 23 keys): `core.sending.undoSend`, `core.sending.defaultAccountIdForSend`,
  `core.notifications.unsnoozeToTop`, `core.composing.html`, `signatures`, `defaultSignatures`,
  `templates` (accepted, unused: templates are files), `mailRules`, `mailRulesAutoSince`,
  `resurfacing.snoozeBody|reminderBody`, `composing.quoteTexts`,
  `core.integrations.translation|grammar|enrichment`, `core.ai.providers|features|accounts|limits|includeAddresses`.
  `settings.changed` broadcast, `storeId` for re-copy.
- The desktop keeps all of them in config.json too and mirrors both ways
  (`src/engine/supervisor/settings.ts`, 346 lines, KEY_FEATURES/COPIED_KEY/KEPT_KEY rules).
- Accounts: engine registry `accounts.json` + secret store (`crates/airpost-secrets`), feature
  `accounts`/`account-setup`. Desktop `AccountStore` still loads `accounts` from config.json and
  merges `account.list`/`account.changed` (`src/data/stores/account.ts:38`).
- Client-owned settings other clients would want (not in the engine store):
  `core.sending.defaultReplyType`, `core.reading.autoloadImages` + the sender allowlist file
  `autoload-images-whitelist.txt` (`src/features/message-autoload-images/store.ts:13`),
  `core.appearance.labelColors`, `core.calendar.disabledCalendars`,
  `core.contacts.findInMailDisabled`, `core.composing.grammarCheck*`, `core.translation.automatic|disabled`,
  `core.workspace.showImportant`, `core.intl.language` (engine gets only the rendered
  resurfacing/quote texts), `core.mcp`. Web keeps signatures, templates and integration keys in
  localStorage (`apps/web/src/features/settings/signatures/store.ts:3`,
  `composer/templates.ts:3`, `settings/integrations/store.ts:61`).

## 6. Extension mechanism

- None in-process. Service plugins (separate binaries, launchd/systemd units, socket clients).
- Draft hooks are bridged, not run: `draft.create {save:false}` returns an unsaved draft so a
  client runs its own `prepareNewDraft` (desktop skips `ENGINE_PREPARED_EXTENSIONS` =
  Signature, TrackingPixels, `src/engine/client/drafts.ts:31`); `draft.send {bodies}` carries the
  client's `applyTransformsForSending` output, kept with a pending send
  (`crates/airpost-sync/src/pending_sends.rs`). A CLI/MCP/web send or a send-later scheduled from
  them gets no plugin transforms. `plugin-api-v2.md:313` says so for `addComposerExtension`.
- Built-in engine automation that would be "engine extensions" if plugins could add their own:
  mail rules (`apps/engine/src/rules.rs`, `crates/airpost-tasks/src/mail_rules.rs`), expiration executor
  (`crates/airpost-sync/src/expiration_executor.rs`: send-later, thread-snooze, send-reminders,
  tracking claims), AI triage (`apps/engine/src/ai_triage.rs`), phishing `warnings.rs`,
  unsubscribe (`unsubscribe.rs`), tracking rewrite (`crates/airpost-compose/src/tracking.rs`).

## 7. Types into TS

Hand-written: desktop `src/engine/protocol.ts` (954 lines, "Hand-written from crates/airpost-ipc"),
web `src/engine/rpc.ts` (679) + `types.ts` (443). No `schemars`/`ts-rs`/`specta` in any
Cargo.toml (TD26 in docs/tech-debt.md). Rust models are `data: Value` wrappers with accessors
(`crates/airpost-store/src/model.rs`), so there is no Rust struct to derive model types from; the
only machine-readable model schema is the allowlist JSON generated from TS. `packages/` holds
`email-dark` only.

## 8. Performance

- Only measurement: `apps/engine/test/test_query_latency.py` (1,500 single-message threads on the
  fake dovecot, captured thread-list queries): median 1.3 ms per page on a release build, reported in
  the stage 8 status; no `latency.json` kept in `apps/engine/test/runs`. A real-profile
  measurement with long threads was the flip precondition and wasn't done.
- Obvious costs vs in-renderer better-sqlite3: JSON encode in Rust + socket + `JSON.parse` +
  model inflation in the renderer per row (SQLite path also parses the `data` column, so the delta
  is the socket hop and a second serialization); 3 sequential round trips per scroll step;
  head-of-line blocking per connection (a slow FTS `query` holds every later read in that window);
  no reply budget (an unbounded `findAll(Contact).where(refs>0)` reply can exceed line limits and
  is treated as a connection drop, then retried 3x with resync); full-model deltas fanned out to
  every window.
- The desktop SQLite path has its own background agent for `.background()` queries; the engine
  path has none (one reader thread per connection).

## 9. Unit table

Verdicts: engine = stays/extends in the engine; client = stays in the client; split = part moves;
delete = removable once its replacement is the only path (most are feature-gated fallbacks for
engines the desktop no longer ships with).

### 9.1 Protocol and transport

| Path | Logic / state | Node/Electron/DB deps | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `crates/airpost-ipc` | protocol v1, 98 names, policy, blocking `Client` | serde | engine | all of it | schema export for TS; delta seq/resume; `query.subscribe` |
| `apps/engine/src/socket.rs` (2000) | acceptor, per-conn reader/writer, `Hub` fan-out, subscribe filters, backlog | Unix socket | engine | account/class filters, bounded queues | per-query subscriptions, reply budget, per-conn read concurrency |
| `apps/engine/src/dispatch.rs`, `serve.rs`, `registry.rs`, `accounts_api.rs` | main-thread dispatch, account registry, OAuth, restart policy | fs | engine | `accounts`, `account-setup` | none for thin clients |
| `crates/airpost-wire` | backend metadata/tracking wire | serde | engine (backend) | — | not the engine protocol; only web-to-backend would need TS types |
| `apps/gateway` | WS relay `/gateway/v1/ws`, pairing, device scopes, file up/download, health | axum | engine side (server) | transparent relay + policy recheck | nothing protocol-specific; inherits stream gaps (no resume over flaky networks matters more here) |
| `crates/airpost-client` | engine discovery (`run/*.json`), multi-engine router `Engines`, task builders, metadata shapes, grammar search params, utf7, safe file names | fs | split | used by CLI and MCP | task builders and metadata shapes are domain logic that should be RPCs; discovery/routing stays client |
| desktop `src/engine/protocol.ts` (954) | hand-written TS types/consts | os, crypto, path | delete (generate) | — | generated package |
| desktop `src/engine/client/connection.ts`, `events.ts`, `api.ts`, `bridge.ts` (~1.4k) | socket client, hello/feature gating, delta inflation to models, plugin `Engine` API | net, ipcRenderer | split | — | moves into shared TS client (transport adapters: unix socket in desktop, WS in web) |
| desktop `src/engine/supervisor/*` (1939), `process.ts` (1063) | spawn/adopt engine, units, settings mirror | child_process, fs | client (main process) | `serve --detached`, units via CLI | settings mirror becomes unnecessary once config.json drops engine keys |
| web `src/engine/rpc.ts`, `ws.ts`, `connect.ts`, `types.ts` (~2.1k) | hand-written envelope/types, WS transport, reconnect | browser | delete types / client transport | — | shared TS client |

### 9.2 Reads, queries, live data

| Path | Logic / state | Node/Electron/DB deps | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `crates/airpost-store/src/model_query.rs`, `read.rs`, `search_grammar.rs`, `search_dates.rs` | ModelQuery compiler, keyset paging, grammar search | rusqlite | engine | `query`, `search` | source of truth for schema (now TS-generated allowlist); joined page read; derived fields (`warnings`, counts, senders) in rows |
| `apps/engine/src/read_api.rs` | read dispatch | ReadStore | engine | 12 read methods | reply budget, timeout, `warnings` on `query` |
| desktop `src/data/database/store.ts` (614) | DatabaseStore: read-only better-sqlite3 on airpost.db, LRU statement cache, trigger/listen of change records | better-sqlite3, ipcRenderer, path | delete | `query` path already in it | real-profile latency, structuredSearch over `query`, ThreadCounts raw SQL removal |
| desktop `src/data/database/agent.js`, `src/main/ipc/database-agent.ts` | background SQLite child process | better-sqlite3, MessagePort | delete | — | — |
| desktop `src/data/database/engine-reads.ts` (131) | `query` routing, reconnect contract, `resync` | — | split (into shared client) | `query` | resume token would remove `resync` |
| desktop `src/data/query/*` (1860) | ModelQuery builder, `sql()`, QuerySubscription(+Pool, Mutable, Unread), range diffing, idsOnly refetch | DatabaseStore | split | `query` JSON form | live queries in the engine (query + change feed, membership decided engine-side) |
| desktop `src/data/attributes/*` (1059), `matcher.ts` (428) | attribute types, matcher eval in JS (`item.matches`) and SQL | — | delete (after live queries) | Rust matcher in `model_query.rs` | engine-side membership |
| desktop `src/data/models/*` (3438) | Model classes, attribute metadata, `isHidden`, Thread/Category semantics, `queryable` source for the allowlist | — | split | JSON stored as is, `hidden` | generated model types; allowlist owned by Rust |
| desktop `src/mail/search/{parser,ast,backend-local,backend-imap}.ts` (1355) | search grammar parser and SQL/IMAP backends | — | delete | `search_grammar.rs` (`search-grammar` feature) | desktop to send `search {grammar:true}` text (engine side done) |
| desktop `src/data/stores/thread-counts.ts` | counts from `counts.get` + raw SQL fallback | DatabaseStore._query | split | `counts.get`, ThreadCounts deltas | drop SQL fallback |
| desktop `src/data/mailbox-perspective.ts`, `stores/focused-perspective.ts` | which query a sidebar item means (roles, starred, unread, unified, drafts) | DatabaseStore | split | `query` covers the filters | named perspective queries (same in web `keys.ts`) |
| desktop `src/data/stores/message.ts` (550) | thread messages, body reads, expand state | DatabaseStore, EngineReads | split | `query` include body, `bodies.fetch` | `threads.get` with bodies on demand is already there; warnings |
| web `src/engine/cache.ts`, `hooks.ts`, `keys.ts`, `model.ts` (~1.3k) | TanStack cache patched from deltas, `matchesThreadQuery`, sort compare, category semantics ported from desktop | browser | delete (after live queries) | `query` | live queries |
| `apps/cli/src/read.rs`, `watch.rs` | multi-engine merge, raw delta print | — | client | reads, subscribe | — |
| `apps/mcp/src/model.rs`, `search.rs` | LLM output shapes; grammar fallback for old engines | — | client / delete (`search.rs`) | `search {grammar}` | — |

### 9.3 Tasks, optimistic apply, undo

| Path | Logic / state | Node/Electron/DB deps | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `crates/airpost-tasks`, `crates/airpost-sync/src/tasks.rs` | 21 task classes, local/remote phases, 24 h hold, retry | rusqlite, IMAP | engine | all | rollback on remote failure; per-row pending state for clients |
| desktop `src/data/tasks/*` (2241, 26 files) | task classes, `createUndoTask`, JSON serialisation, descriptions | — | split | the JSON formats | higher-level RPCs (`threads.move|label|star|markRead`, `categories.*`) so clients stop building task JSON |
| desktop `src/data/stores/task-queue.ts`, `outbox.ts` | Task model view, waits | DatabaseStore | client (view) | `tasks.list`, Task deltas | — |
| desktop `src/data/stores/undo-redo.ts` (193) | undo/redo stack, waits for engine `undoPlacements` | — | split | `undoPlacements`/`restorePlacements`, event `restore` | server-side undo: `undo.list`/`undo.apply` keyed by task id, built from the engine's own data |
| web `src/engine/tasks.ts`, `optimistic.ts` | task builders, thread-level guesses before the engine answers | — | delete builders; client optimistic stays optional | local phase is the optimistic apply (5 ms deltas) | higher-level RPCs; optimistic patches unnecessary on local socket, useful over gateway |
| `crates/airpost-client/src/tasks.rs`, `apps/cli/src/organize.rs`, `apps/mcp/src/tasks.rs` | task JSON builders, batching `THREADS_PER_TASK`, name lookups | — | split | — | same higher-level RPCs |

### 9.4 Drafts, send, compose

| Path | Logic / state | Node/Electron/DB deps | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `crates/airpost-compose` (draft, quote, signature, templates, tracking, validate, mailto, identity, sanitize, plaintext) and `apps/engine/src/drafts_api.rs` (2196) | draft building, validation, tracking rewrite, pending sends | — | engine | `draft.*`, `draft-pipeline`, `draft-editing`, `templates`, `tracking` | plugin send hooks in-engine |
| desktop `src/data/drafts/factory.ts` (570) | JS draft builders as fallback to `draft.create` | — | delete fallbacks | `draft.create` | — |
| desktop `src/data/drafts/editing-session.ts`, `store.ts`, `change-set.ts` | editor session, autosave, `warningsForSending` checks, send flow | — | split | `draft.update`, `draft.validate` | desktop still validates in TS and saves via SyncbackDraftTask, not `draft.update` |
| desktop `src/engine/client/drafts.ts` | `buildDraft`, `sendDraft`, `cancelSend` | — | client (thin wrapper) | yes | — |
| `plugins/composer-templates/lib/store.ts` | reads/writes `<config>/templates` with `fs`, parse/stringify, watcher | fs | delete storage, keep UI | `templates.*` on the same files | relayed write policy for templates |
| web `src/features/composer/{draft,signature,html,recipients,templates,send-later*}.ts` (~2.4k) | reply/forward construction, quoting, signature placement, templates in localStorage, send-later runner | browser | delete | `draft.create|update|send|validate`, `templates.*`, executor | relayed policy for `signatures`/`templates` HTML |
| `apps/mcp/src/compose.rs`, `drafts.rs` | `airpost_compose` fallback for engines without the pipeline | — | delete fallback | `draft.create` | — |
| `plugins/send-later`, `thread-snooze`, `send-reminders` (517/544/734) | UI + metadata writes; client-side executor when no `expiration-executor` | DatabaseStore | split | executor, `threads.snooze` | send-later/reminder RPCs (they still write metadata via SyncbackMetadataTask, shape in 3 places) |
| `plugins/phishing-detection` (96) | Reply-To vs From domain check with `tld` | tld | delete | `warnings.rs` (psl) | `warnings` on `query` rows |
| `plugins/open-tracking`, `link-tracking` | metadata toggles + UI | — | client (UI) | engine rewrites on send | — |

### 9.5 Settings, accounts

| Path | Logic / state | Node/Electron/DB deps | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `apps/engine/src/settings.rs` + `SETTINGS_KEYS` | 23 keys, validation, `settings.changed` | fs | engine | yes | keys listed in 5 (reply type, image allowlist, label colours, disabled calendars, grammar/translation prefs); drop unused `templates` key |
| desktop `src/engine/supervisor/settings.ts` (346) | two-way mirror config.json <-> engine | — | delete (after config.json drops the keys) | store | windows reading settings async |
| desktop `src/data/stores/account.ts` (508) | accounts from config.json + engine merge | config | split | `account.list`, `account.changed` | engine as the only account list |
| desktop `src/data/stores/signature.ts`, `mail-rules.ts` | editors over settings | config | client (editor) | `signatures`, `mailRules` | — |
| desktop `src/engine/client/integrations.ts`, `ai.ts` | settings shape, key routing, keyset fallback | KeyManager | split | `secrets.*`, `ai-keys` | drop keyset fallback |
| web `src/features/settings/{signatures,integrations}/store.ts` | localStorage | browser | delete | settings + secrets | relayed write policy |
| `apps/cli/src/{settings,signature,rules,template}.rs` | editors; rule shape validation | — | client | settings, `rules.*` | rule schema validation is in CLI; engine accepts any array |

### 9.6 PIM, AI, integrations, export

| Path | Logic / state | Node/Electron/DB deps | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `apps/engine/src/pim_api.rs`, `crates/airpost-pim`, `crates/airpost-dav` | ICS/vCard build, recurrence expansion | — | engine | `events.*`, `contacts.*` | contact delete/groups RPCs (tasks exist) |
| desktop `src/data/calendar/ics.ts`, `features/main-calendar/data-source.ts`, `features/events/*` | ical.js parsing for display/RSVP target | ical.js | split | `events.expand {first}` | — |
| `apps/engine/src/ai_api.rs`, `ai_triage.rs`, `crates/airpost-ai` | summarize/reply/draft/triage, usage | HTTP | engine | `ai.*` | — |
| `apps/engine/src/integrations_api.rs`, `crates/airpost-integrations` | translate/grammar/enrich | HTTP | engine | `integrations.*` | — |
| `crates/airpost-tasks/src/export.rs` + desktop `src/mail/mbox.ts` + `apps/cli/src/mbox.rs` | `.eml` export in engine, mbox assembly in two clients | fs | split | GetMessage/GetManyRFC2822Task | engine-side mbox output |

### 9.7 Extensions

| Path | Logic / state | Node/Electron/DB deps | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `examples/service-plugin` | out-of-process plugin on the socket | — | engine extension (today's form) | deltas, metadata-expiration, task.queue | sync hook points (before send, new draft, message arrived, task veto), in-engine runtime or a hook RPC with a deadline |
| `src/packages/composer-extension.ts`, ExtensionRegistry.Composer | `prepareNewDraft`, `warningsForSending`, `applyTransformsForSending`, `onSendSuccess` | — | split | `draft.create {save:false}`, `draft.send {bodies}` | engine-run equivalents so CLI/MCP/web/executor sends get them |
| `docs/desktop/plugin-api-v2.md` sections 3, 6 | desktop-only plugin API; `this.engine`, `this.metadata`, `engineMethods` prompt | — | split | — | an engine-extension half of the manifest |

## 10. Duplicated logic

1. ModelQuery compile/eval: desktop `src/data/query/query.ts` `sql()` + `src/data/attributes/matcher.ts`
   (JS `matches`) vs `crates/airpost-store/src/model_query.rs` (kept equal by
   `tests/model_query_parity.rs`) vs web `src/engine/cache.ts` `matchesThreadQuery`/`compareThreads`.
2. Search grammar: desktop `src/mail/search/*` (1355 lines) vs `search_grammar.rs` + `search_dates.rs`
   (1333) vs web `src/engine/mock/search.ts` (mock) vs MCP `src/search.rs` fallback.
3. Task JSON builders: desktop `src/data/tasks/*`, web `src/engine/tasks.ts`, `crates/airpost-client/src/tasks.rs`
   (CLI, MCP; checked against fixtures).
4. Undo construction: `createUndoTask` in 10 desktop task classes; engine writes `undoPlacements`; web and
   CLI have no undo for mail actions (CLI `TODO(cli-parity)`: "Undo of the last change").
5. Draft building: desktop `DraftFactory` JS paths, web `composer/draft.ts`/`signature.ts`/`html.ts`,
   `crates/airpost-compose` (engine), MCP `compose.rs` fallback.
6. Send validation: desktop `editing-session.ts` + templates extension vs `airpost_compose::validate`.
7. Phishing check: `plugins/phishing-detection` (`tld`, 2011 PSL) vs `apps/engine/src/warnings.rs` (`psl`); they
   disagree on unknown suffixes.
8. Plugin metadata shapes (send-later, send-reminders, open/link-tracking): desktop plugins, web
   `composer/send-later.ts`, `crates/airpost-client/src/metadata.rs`.
9. Send-later execution: engine executor vs desktop plugin fallback vs web `send-later-runner.ts`.
10. Templates: plugin `fs` store + parse/stringify vs `airpost_compose::templates` + `templates_api.rs`;
    web localStorage copy; unused `templates` settings key.
11. Signatures: desktop config.json (mirrored), engine `signatures`, web localStorage, CLI editor.
12. Settings and accounts: config.json and the engine stores, two-way mirror; AccountStore merge.
13. Thread/category semantics (roles, `inAllMail`, unread/starred counts as refs): desktop models, web
    `src/engine/model.ts` ("keep the semantics in step"), Rust store.
14. `Message.isHidden()` (desktop) vs `with_hidden` (`crates/airpost-store/src/read.rs`).
15. mbox framing: desktop `src/mail/mbox.ts` vs `apps/cli/src/mbox.rs`.
16. Modified UTF-7: desktop `src/mail/imap-utf7.ts`, `crates/airpost-client/src/utf7.rs`, `crates/airpost-core/src/utf7.rs`.
17. Optimistic thread patches: web `optimistic.ts` vs the engine's local phase.
18. Integration/AI key handling: engine secret store vs desktop KeyManager keyset fallback.
19. Mail-rule definitions: desktop `src/data/mail-rules-templates.ts` vs `apps/cli/src/rules.rs` vs
    `crates/airpost-tasks/src/mail_rules.rs`.

## 11. Gaps for thin clients (engine side)

1. Live queries: `query.subscribe {query}` -> initial rows + `query.changed {subId, added, removed,
   updated, moved, count}` with engine-side membership and order, plus a delta `seq` and
   `subscribe {since}` resume so reconnects don't re-run everything.
2. Query shapes: a thread-list page read (threads + message summaries/senders/count in one call);
   `warnings` and other derived fields on `query` rows and deltas; contacts across accounts in
   `contacts.search`; named perspectives (inbox/unified/starred/unread/drafts/snoozed) so desktop
   `MailboxPerspective` and web `keys.ts` stop deriving them.
3. Read limits: reply-size budget with its own error code, per-request timeout, and either
   concurrent reads per connection or a background pool for heavy queries.
4. Server-side undo: undo log keyed by task (the engine already has the data for moves and
   events), `undo.apply`/`redo.apply`, exposed to CLI/MCP/web.
5. Higher-level mail RPCs (`threads.move|archive|trash|label|star|markRead`, `categories.create|rename|delete`)
   so clients stop building task JSON in three languages.
6. Remote-failure reconciliation: rollback or a "failed" state per affected row, not only on the Task.
7. Engine extension hooks: synchronous points the engine calls (draft prepared, before send body
   transform, message arrived, task queued) for every client and for executor sends.
8. Settings: move the client-owned keys in section 5 that every client needs; relayed write policy
   for `signatures`/`templates` with HTML sanitising in the engine; drop the config.json mirror.
9. Types: schema export from `airpost-ipc` (schemars or ts-rs) and typed model schemas owned by Rust
   (replacing the TS-generated allowlist) into a `packages/` client.
10. Desktop-side, not engine: send `search {grammar:true}` from the search bar; use `draft.update`/
    `draft.validate`/`templates.*`; flip `core.engine.reads` after a real-profile measurement.
