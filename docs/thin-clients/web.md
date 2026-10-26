# apps/web data layer: thin-client audit (2026-10-23)

Scope: `apps/web/src` (~29.2k non-generated lines incl. tests, 45 test files; 22.6k non-test
excluding `components/ui`). Non-test LOC by area: engine/ 3.5k, engine/mock/ 4.5k (2.8k without
fixtures), mailbox 3.4k, composer 3.8k, search 2.3k, settings 2.7k, ai 2.2k (482 of it the AI mock),
auth 0.9k, calendar 0.3k, components 0.7k, lib 0.4k, routes 0.3k.

No Node/Electron/DB deps anywhere in the web: browser APIs only (fetch, WebSocket, localStorage,
DOMParser, crypto.subtle, iframe sandbox). Framework deps that matter for sharing: React 19,
TanStack Query 5, TanStack Router, date-fns. The desktop is React 17 + RxJS/QuerySubscription, so a
shared package must be framework-free with per-app adapters.

## How the web talks to the engine

- Transport: `src/engine/ws.ts` `WsEngineClient` -> `wss://<origin>/gateway/v1/ws` (cookie-auth
  upgrade) -> `apps/gateway` -> engine Unix socket `<profile>/run/engine.sock`. Interface
  `src/engine/client.ts` `EngineClient {call, onNotification, onStateChange, connect, fileUrl,
  uploadFile, prepareFile}`; `MockEngine` (`src/engine/mock/engine.ts`) implements the same.
- Handshake: `hello {protocol:1, client}` then `subscribe {accounts:null, classes:RENDERED_CLASSES}`
  (`cache.ts`: Thread, Message, Folder, Label, Account, Task, ProcessState, Contact, Calendar, Event,
  ProcessIdentityRefreshNeeded). Not subscribed: ThreadCounts, ContactGroup, ContactBook, File.
  Reconnect re-reads every query. Before every connect `GET /gateway/v1/session`.
- Reads (TanStack Query, `src/engine/hooks.ts`, keys in `keys.ts`): `account.list`, `engine.status`,
  `folders.list`/`labels.list` (with counts), thread lists via `query {class:'Thread', where,
  order, cursor}` (fallback `threads.list` + client filter), `threads.get`, `messages.get {body}`,
  `bodies.fetch`, `search {grammar, tz, now}` (fallback plain + client date filter), contacts via
  `query {class:'Contact'}` (fallback `contacts.search` per account), `query {class:'Calendar'}`,
  `query {class:'Event', where rs/re overlap}`, `settings.get`, `ai.*`.
- Writes: `task.queue` with client-built task JSON (`src/engine/tasks.ts`: ChangeUnread/Starred/
  Folder/Labels, SyncbackDraft, SendDraft, DestroyDraft, SyncbackMetadata, ExpungeAllInFolder,
  SyncbackCategory, DestroyCategory), `task.cancel`, `threads.snooze/unsnooze`,
  `messages.unsubscribe`, `settings.set`, `account.add/test/reset/remove` (admin only),
  `integrations.test` (admin only), `ai.*`. Files over HTTP: `GET /gateway/v1/files/:aid/:fileId
  [?inline=1]`, `HEAD` same (forces download = `prepareFile`), `POST /gateway/v1/files/:aid?name=`.
- Live updates: no live queries. `cache.ts` `applyDelta` patches the TanStack cache from `delta`
  notifications by re-running list membership (`matchesThreadQuery`) and sort (`compareThreads`)
  client-side, i.e. a hand-written client-side live-query evaluator for one shape (thread lists).
  Category counts: every Thread/Folder/Label delta invalidates and refetches `folders.list` +
  `labels.list` for the account (`invalidateCounts`), although the engine has `counts.get` and
  ThreadCounts deltas (`feature::COUNTS`).
- Optimistic: `useQueueTask.onMutate` runs `optimistic.ts` `optimisticThreads` (a TS copy of the
  ChangeUnread/Starred/Folder/Labels local phases, including Gmail folder/label semantics) and
  applies it as a fake Thread delta; on error refetches. The engine's `task.queue` already replies
  after `perform_local` (apps/engine/src/dispatch.rs:281), so this only hides the round trip.
  (Stage 7c removed it: mail actions are `threads.*` intents and the web applies their `models`.)
- Feature gates: 19 non-test files check `hello.features` (Query 4, SearchGrammar 6, Settings 5,
  Snooze 5, Unsubscribe 3, Files 3, ExpirationExecutor 3, Capabilities 3, Ai 11, AiTriage 6) and
  keep old-engine fallbacks (see "delete" rows). The engine ships in the same release as the web
  bundle (`apps/desktop/tern-web`, gateway serves it), so the fallbacks guard engines that no
  longer exist.
- Protocol types: hand-written. `src/engine/rpc.ts` (679 lines: envelope, error codes, method map
  for ~50 methods, notifications) and `src/engine/types.ts` (443 lines: model JSON for Thread,
  Message, Folder, Label, Contact, Account, File, Event, Calendar, 18 task classes, Delta). Desktop
  has a third copy, `apps/desktop/src/engine/protocol.ts` (954 lines, 85 exported types). The Rust
  source of truth is `crates/tern-ipc` (not `crates/tern-wire`, which is the backend metadata
  protocol). tern-ipc depends on serde only; no ts-rs/specta/schemars anywhere in the workspace.
  Model JSON is untyped `serde_json::Value` in tern-store (`models/thread.rs` builds it with
  `json!`), and many tern-ipc results carry `Value` (`EventsExpandResult.occurrences`,
  `AccountDiscoverResult.account`, model rows), so model types can't be generated from Rust
  today; the desktop model classes and `crates/tern-store/src/model_query_allowlist.json` are the
  de facto schema.

## Engine methods the web doesn't use yet (all exist in tern-ipc; relay class from policy.rs)

Open to any paired device: `draft.create|update|send|cancelSend|validate|delete`,
`templates.list|get`, `events.expand`, `contacts.get|export`, `counts.get`, `tasks.list`,
`tasks.get`, `metadata.list`, `account.discover`, `account.update` (display fields),
`account.reorder`. Also unused: `warnings` on Message JSON (`feature::MESSAGE_WARNINGS`, phishing),
`hidden` (`feature::MESSAGE_HIDDEN`), `mail.new`, `rules.disabled`, `tracking` (engine-side).
Admin only (LocalOnly): `templates.save|delete`, `events.create|update|rsvp` (and their tasks are
outside `RELAYED_TASK_CLASSES`), `contacts.create|update|import`, `secrets.*`, `server.*`,
`account.oauth.*`, `account.setSecrets`, `rules.reprocess|cancel`, `integrations.*`.

## Units

Verdicts: engine = logic should live in (or already lives in) the engine and the web should call
it; client = stays in the web (UI state, rendering, OS/browser integration, own prefs); split =
pure-client part stays, the rest moves or goes to the shared TS package (`packages/`, "shared");
delete = dead once the matching engine feature is assumed (same-release engine) or replaced.

### Engine client core (`src/engine/`)

| path | logic / state | Node/Electron/DB deps | verdict | engine already has | missing |
|---|---|---|---|---|---|
| engine/rpc.ts | JSON-RPC envelope, `RpcErrorCode`, `EngineFeature`, typed `RpcMethods` (~50 methods), `RpcNotifications` | none | shared (generated) | tern-ipc `method`, `feature`, `code`, param/result structs | TS codegen from tern-ipc; ~25 newer methods untyped here (draft.*, templates.*, events.*, contacts.get…, counts.get, tasks.list, metadata.list, account.discover/oauth/update/reorder, secrets.*, rules.*) |
| engine/types.ts | model JSON shapes, 18 task JSON classes, `Delta`, `PlacementFlags`, `STANDARD_ROLES` | none | shared (generated) | tern-store models (untyped `Value`), tern-tasks task structs, model_query_allowlist.json | typed model structs or a JSON schema in Rust to generate from; one copy instead of web types.ts + desktop protocol.ts |
| engine/client.ts | `EngineClient` interface, connection state/problem enums | none | shared | — | transport abstraction so desktop (Unix socket) and web (WS) share it |
| engine/ws.ts | WS transport, request ids/timeouts, hello+subscribe, reconnect backoff, gateway close codes, session probe, file URL/upload/HEAD | none (fetch, WebSocket) | split: framing/reconnect -> shared, gateway HTTP bits (session, files, close codes) -> web | gateway `/gateway/v1/{ws,session,files}`, tern-ipc framing | — |
| engine/connect.ts | `ensureConnected`, `createEngineClient` (mock vs gateway), `waitForTask` (follows Task deltas) | none | split: `waitForTask` -> shared; mode choice -> web | Task deltas, `tasks.get` | a "wait for task" call or task-completion notification to the caller (today every client filters all Task deltas) |
| engine/mode.ts | mock/gateway selection, gateway base URL | none | client | — | — |
| engine/provider.tsx, context.ts | React provider: connect, seed process state, account.state handling, delta fan-out to cache | none | client (React adapter over shared store) | — | — |
| engine/keys.ts | TanStack query keys; `ThreadQuery` (account/folder/label/role/categoryIds/starred/unread) | none | split: `ThreadQuery` = a mailbox perspective -> engine; keys -> client | `query` with matchers | named perspectives (`inbox`, `unified:inbox`, `starred`, Sent sort) resolved by the engine |
| engine/hooks.ts `threadListQuery`, `threadSortFor`, `fetchThreadPage` | builds the `query` for a list the way desktop mailbox perspectives do (categories containsAny, inAllMail outside spam/trash, lmrt vs lmst for Sent); fallback pages `threads.list` and filters starred/unread client-side up to FILTER_ROUNDS | none | engine (perspective -> query); fallback delete | `query`, `threads.list` | perspective resolution in the engine (same rules exist in desktop `data/mailbox-perspective.ts`, web `keys.ts`/`hooks.ts`; `threads.list` has its own in tern-store) |
| engine/hooks.ts `useThread/useMessages/useBodies` | thread detail cache, keeps cached bodies across refetch, `messages.get body:true` then `bodies.fetch` for missing, version bookkeeping | none | split: body-loading policy -> shared client; caching -> client | `threads.get`, `messages.get {body}`, `bodies.fetch` | `threads.get {bodies:true}` (one call; engine fetches missing bodies and answers or streams) |
| engine/hooks.ts `useSearch`, `searchClock` | grammar search with tz/now; fallback per-account plain search + client date filter | none | engine (already); fallback delete | `search {grammar, tz, now, accounts}`, `query.search` | — |
| engine/hooks.ts `useContacts` | autocomplete via `query Contact`; fallback `contacts.search` per account, merge | none | engine (already); fallback delete | `query`, `contacts.search` | ranking across accounts in one call (today `query` sorts by refs only) |
| engine/hooks.ts `useCalendars/useEvents` | reads Calendar/Event rows with an overlap `where` on rs/re | none | engine | `query`, `events.expand` (Open) | web should use `events.expand` (occurrences with summary/location/all-day, recurrence) instead of raw rows + ics.ts |
| engine/hooks.ts `useQueueTask` + engine/optimistic.ts | optimistic thread mutation per task class, incl. Gmail folder semantics, LOCKED_ROLES, `_refs/_u` counters | none | split: optimistic overlay stays client; the per-class rules -> engine | `task.queue` replies after local phase (dispatch.rs `perform_local`) | `task.queue` result carrying the post-local-phase models (or a `dryRun`/`preview` mode) so clients don't re-implement local phases |
| engine/cache.ts `applyDelta` | client-side live-query: list membership (`matchesThreadQuery`), sorted insert into infinite pages, message list patching, body preservation, category upsert, counts invalidation, task ring (100) | none | split: shared live-query client; matching -> engine | `subscribe {accounts, classes}` deltas, `counts.get` + ThreadCounts deltas | live queries: `query.subscribe` returning a subscription id and add/remove/move events (or "your query changed, re-read page N"); subscribe to ThreadCounts instead of refetching folders/labels lists |
| engine/model.ts | role names/order, IMAP UTF-7 decode, `categoryDisplayName` (strips INBOX/[Gmail]/Tern/[Mailspring]), `sortCategories`, `threadInCategory/HasRole`, `compareThreads/Messages`, `sortsAfter`, contact display/initials, `isFromAccount`, id generators | none | split: display name/sort/UTF-7 -> engine fields; comparators -> shared; initials -> client | tern-client `utf7.rs`, CLI `organize.rs display_name`, desktop `Category.displayName` | `displayName` and `sortOrder` on Folder/Label JSON; `isMe`/`fromMe` per message |
| engine/tasks.ts | task JSON builders for 11 classes | none | split: shared until replaced by intent RPCs | tern-client `tasks.rs` (same builders in Rust for CLI/MCP), desktop `data/tasks/*` | intent RPCs (`threads.archive/trash/spam/move/label/markRead/star`) so no client builds task JSON |
| engine/settings.ts | tab copy of `settings.get` values and `hello.capabilities`, `settings.changed`/`capabilities.changed` | none | shared (store) | `settings.get/set/changed`, capabilities | — |
| engine/mock/* (engine.ts 838, store.ts 414, apply-task.ts 385, query.ts 367, search.ts 467, mail-actions.ts 251, changes.ts 61; fixtures 1.75k) | in-browser engine: SQLite stand-in with thread recompute, task local phases, ModelQuery compiler, search grammar, snooze/unsubscribe/expiration executor, settings policy, account methods | none | client (dev/test only); stop extending | the real engine; `e2e/stack.py` already runs real engine + fake IMAP/SMTP | a fixture-seeded engine profile (or recorded replay) so dev/tests don't need a TS engine; until then every engine feature is written twice |

### Mailbox (`src/features/mailbox/`)

| path | logic / state | Node/Electron/DB deps | verdict | engine already has | missing |
|---|---|---|---|---|---|
| mailbox/actions.ts | TaskFactory port: archive (Gmail = drop Inbox label, else archive/all folder), trash, spam/not-spam, move (Gmail label move dropping viewed label), toggle label, per-account split, snooze/unsnooze calls | none | engine | `threads.snooze/unsnooze`; tasks; CLI `organize.rs` has the same resolve logic in Rust, desktop `data/tasks/factory.ts` `tasksForArchiving/MovingToTrash/MarkingAsSpam` | `threads.archive|trash|spam|move|label` (by thread ids, any accounts) returning task ids + undo token |
| mailbox/mailbox.ts | URL -> `Mailbox` (category / per-account starred / unified role), `roleQuery`, titles, `viewCategoryFor`, `ownAddresses` | none | split: perspective definitions -> engine; routing/titles -> client | `query` | perspective ids the engine understands (see keys.ts) |
| mailbox/use-mailbox.ts, use-unified-threads.ts, use-account-data.ts | resolves mailbox and loads lists; unified merge across accounts for engines without `query` | none | client (hook); use-unified-threads delete | `query {accounts:null}` | — |
| mailbox/list/senders.ts (TODO) | message count from max `_refs`, sender label from participants | none | engine | Thread JSON `participants`, `folders[]._refs` | `messageCount` and ordered `senders` (with unread/draft flags) on Thread JSON |
| mailbox/labels.ts | `labelHue` = desktop `Category.hue()` | none | engine (or shared) | — | label colour on Label JSON (also needed by CLI/mobile) |
| mailbox/snooze.ts, composer/send-later.ts | snooze and send-later presets (date math) | none | client (UI choice), or shared | engine computes due times only | optional `snooze.presets`; fine as client |
| mailbox/default-mailbox.ts, last-account.ts, sidebar/* (nav-model, collapsed, AccountNav, UnifiedNav, AccountSwitcher) | first inbox redirect, sidebar account choice in localStorage, nav tree | none | client | — | — |
| mailbox/list/selection.ts, use-list-shortcuts.ts, list-bus.ts, picker-store.ts, ListHeader, ThreadRow, ThreadListView | selection, cursor, keyboard, list->reader bus, picker dialog | none | client | — | — |
| mailbox/reader/use-mark-read.ts | mark-read after `markReadDelayMs` (browser pref) | none | client (pref ideally shared) | — | `core.reading.markAsReadDelay` is not an engine setting key; desktop and web keep separate values |
| mailbox/reader/expansion.ts | which messages start expanded | none | client | — | — |
| mailbox/reader/unsubscribe.ts | parses List-Unsubscribe to decide whether to show the button; calls `messages.unsubscribe`, opens web link | none | split: header check -> engine flag | `messages.unsubscribe`, `hListUnsub` on Message JSON | `unsubscribe: {methods:[...]}` on Message JSON so clients don't parse RFC 2369 |
| mailbox/reader/MessageItem, MessageList, AttachmentList, ReaderToolbar, SnoozeMenu, compose.ts | rendering, toolbar, reply/forward launch | none | client | — | phishing `warnings` and `hidden` not shown (engine has both) |
| mailbox/ThreadReaderView.tsx | reader layout, inline composer slot | none | client | — | — |

### Composer (`src/features/composer/`)

| path | logic / state | Node/Electron/DB deps | verdict | engine already has | missing |
|---|---|---|---|---|---|
| composer/draft.ts | DraftFactory port: new/reply/reply-all/forward, participants for reply, Re:/Fwd: subjects, reply attribution, quoting, forward header, autoaddress cc/bcc, change identity, `draftToMessage`, header message id | none | engine | `draft.create {kind: new/reply/reply-all/forward/mailto, behavior, template, tz, save}` (Open; desktop `data/drafts/factory.ts` already calls it) | nothing for building; web just doesn't call it |
| composer/identities.ts | From choices (account + aliases), reply identity (`_fromContactForReply`) | none | engine (reply identity) / client (picker) | `draft.create` picks From for replies | an `identities` list per account in `account.list` (aliases parsed) |
| composer/recipients.ts | address parsing, splitting, dedupe, validation | none | split: parse-as-you-type client; validation engine | `draft.validate` (`invalid-recipient`, `no-recipients`) | — |
| composer/html.ts | editor sanitising, quoted split, text<->html, blank check, URL normalise | none (DOMParser) | client (editor) | — | — |
| composer/signature.ts + settings/signatures/store.ts (TODO) | default signature inserted under the body; signatures + defaults in localStorage | none | engine | `signatures`/`defaultSignatures` settings; `draft.create` inserts the From address's default signature | `signatures` not in `RELAYED_SETTINGS_KEYS` (desktop renders it with Node); web reads could use `settings.get` today |
| composer/templates.ts + TemplatesMenu (TODO) | templates in localStorage | none | engine | `templates.list/get` (Open), `draft.create {template}`, `templates.save/delete` (LocalOnly) | relayed `templates.save/delete` (needs desktop to treat template HTML as untrusted); the TODO's "desktop keeps files" point is solved: the engine reads/writes the desktop's files |
| composer/useComposerSession.ts | send checks (recipients, uploads, blank subject), undo-send (engine path via send-later metadata or tab timer), schedule, discard, attachment prep, task following | none | split: checks/send/undo -> engine; window state -> client | `draft.send {delayMs, after}`, `draft.cancelSend`, `draft.validate`, `draft.delete`, expiration executor | nothing new; the web hand-rolls `SyncbackMetadataTask {send-later: {expiration, isUndoSend}}` and watches deltas instead |
| composer/engine-calls.ts `watchEngineSend` | follows SendDraftTask/metadata error for the toast | none | shared | Task deltas | send outcome notification to the caller of `draft.send` |
| composer/draft-saver.ts, autosave.ts | debounced SyncbackDraftTask, destroy previous on account change | none | split: debounce client; save -> `draft.update` | `draft.update {fields, detach}`, `draft.delete` | `draft.update` refuses drafts with a pending send (fine) |
| composer/send-later-runner.ts | sends due drafts on `metadata-expiration` for engines without the executor | none | delete | `feature::EXPIRATION_EXECUTOR` | — |
| composer/useDraftSource.ts | loads the starting draft (blank/reply/forward/edit) and its source message | none | split | `draft.create`, `messages.get` | — |
| composer/store.ts, window-actions.ts, route-account.ts, local-store.ts, ComposerHost/Window/Form/Inline, BodyEditor, EditorToolbar, RecipientField, FromSelect, SendControls, WindowHeader, AttachmentList, QuotedText | composer windows (docked/fullscreen/minimised), editor, UI | none | client | — | — |

### Search and palette (`src/features/search/`)

| path | logic / state | Node/Electron/DB deps | verdict | engine already has | missing |
|---|---|---|---|---|---|
| search/query.ts | chip tokenizer over `field:value`, edit/toggle tokens, date value parse, `toEngineQuery`, `dateRange/inDateRange` (fallback filter) | none | split: chips client; date fallback delete | tern-store grammar via `search {grammar}` | a `search.parse` returning tokens/ranges would let clients drop the tokenizer (optional) |
| search/suggest.ts, operators.ts | operator/value suggestions (folders, contacts, is:/has:/dates) | none | client (uses engine data) | contacts `query`, categories | — |
| search/SearchView, SearchBox, SearchFilters, DateFilter, SearchResultRow, Highlight, thread-sender.ts, useAllCategories.ts | results UI, highlighting, sender label | none | client | — | match snippets/highlights from the engine (not returned; I didn't check whether the FTS index could produce them) |
| search/recent.ts, store.ts, params.ts, palette.ts, CommandPalette, usePaletteCommands | recent searches (localStorage), palette commands and fuzzy match | none | client | — | — |

### Settings (`src/features/settings/`)

| path | logic / state | Node/Electron/DB deps | verdict | engine already has | missing |
|---|---|---|---|---|---|
| settings/accounts/presets.ts (2 TODOs) | provider presets copied from desktop's provider table, `guessServers` imap./smtp. guess | none | engine | `account.discover` (Open, provider table + MX via tern-providers), `account.oauth.begin|complete` (LocalOnly, loopback redirect) | OAuth from a browser: fixed redirect page + `account.oauth.*` relayed; web TODO marks `account.discover` as future, it isn't |
| settings/accounts/account-draft.ts (TODO) | builds Account JSON from the form, cleartext consent, `accountIdFor` (SHA-256 of email+settings, first 8 hex) | none (crypto.subtle) | engine | `account.add` derives ids (tern-core `ids.rs account_id`, `feature::ACCOUNT_SETUP`), `account.add {secrets, test:true}` | — (web just doesn't use the engine's id) |
| settings/accounts/actions.ts (2 TODOs) | add (test then add), test, reset, remove; cache invalidation | none | engine (already) | `account.add/test/reset/remove` (LocalOnly: admin devices) | `account.test {accountId}` with stored secrets (still absent: `AccountTestParams` needs full `account`) |
| settings/accounts/AccountForm, AddAccountView, ServerFields, ProviderPicker, AccountRow, AccountsSettings, status.ts | forms, status mapping | none | client | `account.state`, `account.list` | — |
| settings/integrations/store.ts + providers.ts (TODO) | provider configs in localStorage, keys in tab memory, sent per request | none | engine | `core.integrations.*` settings, secret `integration:<provider>`, `integrations.*` read the stored key (`feature::ACCOUNT_SETUP`), `capabilities.integrations` | relayed write of `core.integrations.*`/`secrets.set` (LocalOnly by design); web UI could be read-only from settings + capabilities today |
| settings/capabilities.ts, CapabilitiesSettings | "is feature X available and from where" from local integrations store + engine capabilities | none | split: should read only engine `capabilities` | `hello.capabilities`, `capabilities.changed` | local features (snooze, send-later, reminders, templates) as capability entries too |
| settings/general/*, shortcuts/*, SettingsLayout/Index, components/* | density, reading pane, mark-read delay, undo delay (falls back to engine setting), shortcut list, session section | none | client (undo delay already engine) | `core.sending.undoSend` | — |
| settings/engine-call.ts | `callEngine` wrapper | none | shared | — | — |

### AI (`src/features/ai/`)

| path | logic / state | Node/Electron/DB deps | verdict | engine already has | missing |
|---|---|---|---|---|---|
| ai/api.ts | streamed calls with `streamId` + `ai.delta`, error key mapping, cancel | none | shared | `ai.*`, `ai.delta`, `AiFailed` data | — |
| ai/config.ts | parses `core.ai.*` read-only, provider rules "mirror tern-ai ProviderConfig and desktop engine/client/ai.ts" | none | split: parse in shared; rules engine | `core.ai.*` settings, `capabilities.ai` | an `ai.status` summary (which features usable for which account) so clients don't re-derive from raw settings |
| ai/triage.ts | reads `tern-ai-triage` metadata, CATEGORIES mirror tern-ai `Category::ALL`, user corrections via SyncbackMetadataTask | none | split | triage worker, `ai.triage.list` | `ai.triage.correct` RPC (today clients write the plugin's metadata shape) |
| ai/needs-reply.ts, NeedsReplyView | `ai.triage.list` per account then threads by `query id in` | none | client (could be one call) | `ai.triage.list`, `query` | `ai.triage.list {include:'threads'}` |
| ai/summary-memory.ts | per-tab summary cache | none | client | engine caches 24 h | — |
| ai/hooks.ts, ThreadSummary, SuggestReply, TriageChip, AiSettings, reply-insert.ts | UI, editor insertion | none | client | — | — |
| ai/mock.ts (482) | mock `ai.*` incl. engine's checks | none | client (dev only), stop extending | tern-ai | see engine/mock |

### Calendar, auth, shell

| path | logic / state | Node/Electron/DB deps | verdict | engine already has | missing |
|---|---|---|---|---|---|
| calendar/ics.ts, agenda.ts, CalendarView (TODO) | parses VEVENT lines (title, location, all-day, recurrence flag) from stored ICS; agenda days; read-only | none | engine (parse/expand), client (agenda layout) | `events.expand` (Open: occurrences, recurrence, tz), tern-pim | relayed `events.create/update/rsvp` and their tasks in `RELAYED_TASK_CLASSES`; the TODO's "recurrence expansion" is done in the engine |
| auth/* (session, transport, connection, pairing-code, redirect, SessionGuard, PairView, OfflineBanner, ConnectionStatus, SessionMenu, use-sign-out) | gateway pairing/session/unpair, offline/engine-down states | none | client (web/mobile-only, gateway-specific) | gateway `/gateway/v1/{pair,unpair,session}` | — |
| components/email (sanitize.ts, EmailFrame, MessageBody) | sandboxed iframe, CSP, cid: rewrite to gateway route, remote-content detection | none | client (rendering) | gateway inline file route | remote-content and tracking-pixel detection is also in desktop (`features/remove-tracking-pixels`); an engine `hasRemoteContent` flag would share it (optional) |
| components/AppShell, MailPanes, Theme*, lib/shortcuts.ts, lib/format.ts, lib/preferences.ts, routes/* | layout, shortcuts registry, formatting, per-browser prefs, routes | none | client | — | — |
| lib/undo-send.ts | engine `core.sending.undoSend` with local fallback | none | engine (already), drop fallback | settings | — |

There is no plugin system in the web: no slots, no extension registry. `docs/desktop/plugin-api-v2.md`
UI extensions would need a web slot story; engine extensions would reach the web for free.

## TODO(engine) markers (13 in src/; TODO-engine.md's line numbers for engine.ts are stale: 684/701, not 620/637)

| marker | what it needs | status in engine today |
|---|---|---|
| features/mailbox/list/senders.ts:3 | message count and per-message senders on Thread JSON | missing (no DCP stage) |
| features/calendar/CalendarView.tsx:3 | create/edit/RSVP events, recurrence expansion | `events.expand` exists and is Open (use it now); `events.create/update/rsvp` exist but LocalOnly, and event tasks are outside `RELAYED_TASK_CLASSES` |
| engine/mock/apply-task.ts:147 | mock behaviour for EventRSVPTask and contact tasks | mock-only; engine has them |
| engine/mock/engine.ts:684 | `account.add` from the web (admin) | exists (LocalOnly = admin device), engine derives ids |
| engine/mock/engine.ts:701 | `account.test` on the socket (admin) | exists, LocalOnly |
| features/settings/accounts/actions.ts:30 | `account.test {accountId}` with stored credentials | missing |
| features/settings/accounts/actions.ts:47 | `account.reset` on the socket (admin) | exists, LocalOnly; marker is stale |
| features/settings/accounts/account-draft.ts:138 | engine assigns account id | exists (`account.add` without `id`, `feature::ACCOUNT_SETUP`); marker is stale |
| features/settings/accounts/presets.ts:25 | OAuth from the web | `account.oauth.begin/complete` exist (LocalOnly, loopback `redirectPort`); missing a fixed redirect page/hosted callback and relay policy |
| features/settings/accounts/presets.ts:111 | `account.discover` | exists and is Open; marker is stale |
| features/settings/integrations/store.ts:61 | keys stored by the engine (`secrets.set`) | exists, LocalOnly; `integrations.*` already read `integration:<provider>` when the key is omitted |
| features/settings/signatures/store.ts:3 | signatures from the engine | `signatures`/`defaultSignatures` settings exist; relayed write of `signatures` refused (`RELAYED_SETTINGS_KEYS`); reads possible now |
| features/composer/templates.ts:3 | templates from the engine | `templates.list/get` Open, `draft.create {template}` Open; save/delete LocalOnly; the "desktop keeps files" part is solved (engine reads the desktop's files) |

Six of 13 are stale or partly stale: the engine method exists and is open, or exists for admin
devices, and only the web call is missing.

## The gateway (`apps/gateway`, Rust, axum)

- Routes (`src/http/mod.rs:511-521`): `GET /gateway/v1/session`, `POST /gateway/v1/pair`,
  `POST /gateway/v1/unpair`, `GET /gateway/v1/ws` (upgrade), `GET|HEAD /gateway/v1/files/{aid}/{id}`
  (via engine `files.get`), `POST /gateway/v1/files/{aid}` (25 MiB, via `files.put`), `/healthz`,
  static `--web-root`.
- WS relay (`src/http/ws.rs`): one engine socket connection per WebSocket; first frame must be
  `hello`, forwarded as a relayed hello with the device's scopes; every later frame is parsed,
  checked with `tern_ipc::policy` (`relayed_method_allowed`, `relayed_task_allowed`,
  `relayed_settings_allowed`, `relayed_request_allowed`) and re-encoded; engine lines go to the
  browser unchanged (no notification filtering). Rate limit 50 frames/s, 8 MiB frames, heartbeat
  30 s/45 s, bounded queue with backpressure onto the engine's slow-subscriber drop.
- Devices (`src/devices.rs`): pairing codes, httpOnly cookie, hashed tokens, `admin` scope at
  pairing; admin devices may call LocalOnly methods and any task class.
- So the gateway exposes the engine's JSON-RPC surface 1:1, filtered by policy. It adds no API of
  its own beyond auth and files.
- Can it serve the desktop? The protocol is the same, so one TS client works over both. The
  desktop shouldn't route through it: it would be a relayed connection (policy-limited unless
  admin), needs cookie pairing, adds a hop, and the desktop also needs ParentOnly methods (stdin)
  and local file paths (`files.get` returns paths, `draft.update {attach: [paths]}`). Recommended:
  shared client with a `Transport` interface; web = WS to gateway, desktop = Unix socket (moved out
  of the renderer into main/preload/utility process, which also removes `net` from renderer code),
  mobile = WS to gateway. File access stays behind `EngineClient.fileUrl/uploadFile/prepareFile`
  (gateway HTTP for web/mobile, `files.get` path -> `tern-asset://`/`file://` for desktop).

## What could become the shared `packages/` TS client

1. Protocol types generated from `crates/tern-ipc` (method names, features, error codes, params,
   results, notifications). Needs a codegen step in Rust (ts-rs or schemars -> json-schema-to-ts);
   none exists. Model JSON (Thread, Message, ...) needs typed structs or a schema first, since
   tern-store keeps `Value`.
2. `EngineClient` + JSON-RPC core from `engine/client.ts`, `engine/ws.ts` (pending map, timeouts,
   reconnect/backoff, hello/subscribe, re-read on reconnect), with transports per app.
3. A framework-free normalized store + query/subscription layer: today's `cache.ts`/`keys.ts`/
   `hooks.ts` logic minus TanStack; React 19 adapter (web) and React 17 adapter (desktop). With
   engine live queries this shrinks to "hold rows, apply add/remove/move".
4. Task/intent helpers (`engine/tasks.ts`, `connect.ts waitForTask`, `composer/engine-calls.ts`
   `watchEngineSend`, `ai/api.ts`) until the intent RPCs replace task JSON.
5. Small pure helpers both clients need for display: comparators, contact display name/initials.
   Display name, sort order and label hue should come from the engine instead.
Not shared: auth/pairing, mock engine, components, editor/html.ts, preferences.

## Duplicated logic

Same thing computed in TS and Rust, or in desktop and web (D = desktop TS, W = web TS, R = Rust).

| logic | where | note |
|---|---|---|
| Protocol types | W `engine/rpc.ts`+`types.ts` (1.1k lines), D `src/engine/protocol.ts` (954), R `crates/tern-ipc` | three hand-maintained copies |
| Task JSON builders | W `engine/tasks.ts`, D `src/data/tasks/*.ts`, R `crates/tern-client/src/tasks.rs` | three copies of the desktop's Task serialisation |
| Archive/trash/spam/move resolution (TaskFactory) | W `features/mailbox/actions.ts`, D `src/data/tasks/factory.ts`, R `apps/cli/src/organize.rs` (`resolve`, `by_role`, `spam`) | Gmail label vs folder rules in three places |
| Task local phases (optimistic) | W `engine/optimistic.ts`, W mock `apply-task.ts`, R `crates/tern-tasks` | engine already runs them before replying |
| Thread recompute from messages (`_refs`, `_u`, flags) | W mock `store.ts recomputeThread`, R tern-store `Thread::apply_message_attribute_changes` | |
| ModelQuery compiler/matcher | W mock `query.ts`, D `src/data/attributes/matcher.ts` + `data/query/*`, R `tern-store/src/model_query.rs` | |
| Search grammar | W mock `search.ts` (467), R tern-store search, D `features/thread-search` | web's own `query.ts` is a chip tokenizer, not a full parser |
| Live list membership/sort | W `engine/cache.ts matchesThreadQuery/insertThread`, D `data/query/mutable-result-set.ts`/`subscription.ts` | both re-evaluate queries client-side on deltas |
| Mailbox perspectives -> query | W `engine/hooks.ts threadListQuery`, `features/mailbox/mailbox.ts`, D `src/data/mailbox-perspective.ts`; R `tern-store threads()` for `threads.list` (CLI uses that) | inAllMail/spam/trash and Sent sort rules |
| Category display name, IMAP UTF-7 | W `engine/model.ts`, D `data/models/category.ts`, R `tern-client/src/utf7.rs`, CLI `organize.rs display_name` | |
| Label hue | W `features/mailbox/labels.ts`, D `Category.hue()` | |
| Draft building (reply/forward/quote/attribution/participants/identity) | W `features/composer/draft.ts`, `identities.ts`, D `data/drafts/factory.ts` (fallback), R `crates/tern-compose` behind `draft.create` | desktop already prefers `draft.create`; web never calls it |
| Undo-send / send-later | W `useComposerSession.ts` + `send-later-runner.ts`, D `plugins/send-later`, R expiration executor + `draft.send {delayMs}` | |
| Signature insertion | W `composer/signature.ts` + localStorage store, D `features/composer-signature`, R `draft.create` | |
| Templates | W localStorage `composer/templates.ts`, D `plugins/composer-templates` files, R `templates.*` | |
| Account id derivation | W `account-draft.ts accountIdFor`, D account model, R `tern-core/src/ids.rs account_id` | |
| Provider presets / discovery | W `accounts/presets.ts`, D onboarding provider table, R `tern-providers` + `account.discover` | |
| Snooze presets | W `mailbox/snooze.ts`, D `plugins/thread-snooze` popover, R CLI `mail_actions.rs parse_until` | UI-level; fine to keep per client |
| List-Unsubscribe header parse | W `reader/unsubscribe.ts`, D `features/list-unsubscribe`, R `messages.unsubscribe` | clients only need "offered methods" |
| AI provider rules / triage categories | W `features/ai/config.ts`, `triage.ts`, D `src/engine/client/ai.ts`, R `tern-ai` | |
| Integrations config + capability availability | W `settings/integrations/store.ts`, `settings/capabilities.ts`, D `src/engine/client/integrations.ts`, R `capabilities` | |
| ICS field parsing | W `calendar/ics.ts`, D `data/models/event.ts`/main-calendar, R tern-pim + `events.expand` | |
| The whole mock engine | W `engine/mock/*` 2.8k + `ai/mock.ts` 0.5k, D `tests/support/fake-serve-engine.ts`, `mock-engine.ts` | second implementation of the engine in each client |

## Engine gaps for thin clients (from this area)

1. Live queries: `query.subscribe` (id, window, add/remove/move/update events) to replace
   `cache.ts` and desktop QuerySubscription. Partial: deltas + `query` exist.
2. Counts: web ignores `counts.get`/ThreadCounts and refetches category lists per Thread delta.
   Exists; client should subscribe.
3. Intent RPCs for mail actions (`threads.archive|trash|spam|move|label|read|star`), any accounts,
   returning task ids + undo token. Missing (only snooze/unsnooze/unsubscribe exist).
4. Server-side undo: `undo.last`/`undo {token}` (CLI TODO lists undo too). Missing; web has no undo
   for mail actions at all.
5. `task.queue` (or intent RPCs) returning post-local-phase models for optimistic UIs. Missing.
6. Thread JSON: `messageCount`, ordered senders, display names/sort order/colour on categories,
   `unsubscribe` methods on Message JSON. Missing.
7. Named perspectives (unified inbox, starred, Sent-by-last-sent) resolved by the engine. Missing.
8. Typed model structs / schema in Rust + TS codegen from tern-ipc. Missing.
9. Relay policy for web writes: `signatures`, `templates.save/delete`, `core.integrations.*`,
   event and contact tasks, `account.test {accountId}`, browser OAuth redirect. Policy decisions
   plus one missing param.
10. `threads.get {bodies:true}` / one call that fetches missing bodies. Missing (two-step today).
11. Task completion to the caller (`draft.send` outcome, `waitForTask`) without filtering every
    Task delta. Missing.
12. Fixture/replay engine for dev and tests, so the TS mock can stop growing. `e2e/stack.py` is the
    start.
