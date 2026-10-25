# apps/desktop/src (outside data/ and mail/): thin-client audit

Scope: `features/`, `components/`, `window/`, `platform/`, `packages/`, `config/`, `intl/`,
`utils/`, `engine/` (process, protocol, client, supervisor) and `main/`. `data/` and `mail/` are
someone else's area; they show up here only where a unit in scope leans on them
(TaskFactory, perspectives, DatabaseStore, mail/mbox, mail/search).

Verdicts: **engine** = the logic belongs in the engine (the TS goes away or becomes a call);
**client** = stays in the desktop (UI, layout, OS integration, plugin loading); **split** = UI
stays, a domain part moves; **delete** = dead code or a fallback for engines without a feature
(the engine is bundled, so these never run in a shipped build); **shared** = client code that
the web needs too and belongs in the shared TS package (packages/), not the engine.

Inventory used for the counts: `docs/thin-clients/desktop-files.txt` (537 files, lines,
Node imports, DatabaseStore/queueTask/engine refs per file) and
`docs/thin-clients/node-renderer.txt` (109 non-main files importing fs/path/electron/
child_process; 18 of them are in data/ or mail/, 91 in scope).

Engine surface checked against: `crates/tern-ipc/src/lib.rs` `mod method` (95 methods and
notifications), `crates/tern-tasks/src/lib.rs` task classes, `crates/tern-ipc/src/settings.rs`
`SETTINGS_KEYS`, `crates/tern-compose`, `apps/engine/src/*.rs`.

---

## 1. engine/ (client side, process, protocol)

| path | logic / state | Node/Electron/DB | verdict | engine has | missing |
|---|---|---|---|---|---|
| `engine/protocol.ts` (954 l) | Hand-written TS copy of the socket protocol: RPC envelope, `Method`, `Feature`, params/results, socket path rules | `path`, `os`, `crypto` (socket path hash) | shared (generated) | `crates/tern-ipc` (methods.rs, read.rs, drafts.rs, pim.rs, settings.rs, ...) | Codegen. The header says "generating these from it waits until the surface grows". tern-ipc has only serde, no ts-rs/schemars/typeshare. Note: the socket types live in **tern-ipc**, not tern-wire (tern-wire is the backend metadata-sync wire). The web has its own copy (`apps/web/src/engine/types.ts` 443 l, `rpc.ts` 679 l) |
| `engine/client/connection.ts` (433 l) | JSON-RPC over NDJSON on the Unix socket, reconnect, hello | `net`, `events` | shared (transport pluggable) | socket + `hello` | Transport abstraction: desktop renderer uses Node `net`, web uses WS via gateway (`apps/web/src/engine/ws.ts`). Shared package needs `Transport` interface; desktop keeps a `net` (or main-proxied) transport |
| `engine/client/api.ts` (345 l) | `Engine` export for plugins, per-plugin method approval (`core.plugins.engineMethodApprovals`), stack-walking caller detection | `fs`, `path` (stack paths) | client | `tern-ipc/policy.rs` (local-only methods) | Nothing; stays as the plugin-facing wrapper |
| `engine/client/bridge.ts` (488 l) | `EngineBridge`: validates/sends `task.queue`, inflates deltas into models, `DatabaseStore.trigger`, ThreadCounts deltas, account log tail | `electron` (ipcRenderer, shell), `path`; DatabaseStore | split | `task.queue`, `delta`, `subscribe {classes, accounts}` | Becomes the shared package's change feed + task client. The model inflation (`Utils.convertToModel`) goes with the models into the shared package |
| `engine/client/events.ts` (111 l) | EngineEvents emitter: features, capabilities, mail-new, connected | `events` | shared | `hello.features`, notifications | — |
| `engine/client/drafts.ts` (113 l) | `draft.create` (save:false), `draft.send` with window-computed `bodies`; `ENGINE_PREPARED_EXTENSIONS` skip list; JS fallback without `draft-pipeline` | none | split | `draft.create/send/cancelSend/update/delete/validate` | Delete the fallback. `bodies` exist because `applyTransformsForSending` runs in the window (plugin hooks) — needs engine-side send hooks (section 9) |
| `engine/client/accounts.ts` (223 l) | discover, OAuth begin/complete (redirect via main's loopback receiver), add/test/reconnect, secrets | `electron` ipcRenderer (OAuth redirect) | client | `account.discover/oauth.begin/oauth.complete/add/update/remove/test/setSecrets`, `secrets.*` | — (the OAuth loopback receiver in `main/ipc/oauth-receiver.ts` is OS integration) |
| `engine/client/pim.ts` (211 l) | `events.expand/create/update/rsvp`, `contacts.get/create/update/import/export`; queues the event writes itself so UndoRedoStore sees them | DatabaseStore.find (1) | split | all of those | Server-side undo; the "queue it here so undo works" dance goes once the engine has undo |
| `engine/client/integrations.ts` (551 l) | Provider table (`PROVIDERS`), request param building, error mapping, KeyManager fallback for keys | none | split | `integrations.*`, `secrets.set`, `capabilities` | Delete the KeyManager key fallback (engines without `ai-keys`/account setup). Provider table is duplicated in web settings |
| `engine/client/ai.ts` (285 l) | `core.ai.*` settings shape, keyName fallback | none | split | `ai.*`, settings `core.ai.*` | Delete key fallback; the settings types belong in the shared package |
| `engine/client/oauth-revocation.ts` (59 l) | Revokes the Google grant on account removal, secrets from KeyManager | — | delete | `apps/engine/src/accounts_api.rs` `revoke` (ported from this file) on `account.remove` | Nothing; only runs for engines without account setup |
| `engine/resurfacing.ts` (25 l) | Localized unsnooze/reminder bodies pushed to engine settings | — | engine | settings `resurfacing.*` | Engine-owned strings by locale (section 3) |
| `engine/process.ts` (1063 l) | Binary paths, `EngineServeProcess` (spawn serve, stdin JSON-RPC, rotated secrets on stdout, status window, crash error mapping, `LocalizedErrorStrings`) | `child_process`, `net`, `fs`, `crypto`, `electron` | split → CLI | `tern engine start|restart|install` (apps/cli/src/builtin.rs, units.rs) | See section 2 |
| `engine/channels.ts` (119 l) | Supervisor IPC channel names | — | client (shrinks) | — | — |

## 2. Supervisor and main process

| path | logic / state | deps | verdict | engine/CLI has | missing |
|---|---|---|---|---|---|
| `engine/supervisor/supervisor.ts` (826 l) | Owns serve's stdin; seeds the engine from config.json `accounts` + KeyManager (`credentials.import`), drops KeyManager copies, keeps `accountsToRemoveFromEngine`, joins `--detached` engines, restarts the unit on build mismatch | `fs`, `net`, `path` | split → CLI | `tern engine *` units (launchd/systemd), engine `Feature.Accounts` keeps accounts+secrets and restarts stopped accounts itself, `account.*` on the socket | Windows: `units.rs` says "None where tern installs no units (Windows)", so main must keep a spawn path there. One-time migration (credentials.import, accountsToRemove) can become a `tern migrate` step |
| `engine/supervisor/host.ts` (701 l) | `EngineClient`: crash windows (5 in 5 min), respawn backoff, per-account relaunch, reset straggler wait | none | delete (mostly) | engine restarts stopped accounts with `retryAt` (Feature.Accounts); unit restart policy | Crash-loop UI signal: engine/unit health in `tern --json status` exists; desktop only needs to show it |
| `engine/supervisor/settings.ts` (346 l) | Two-way mirror of 16 engine-owned keys between config.json and `settings.*`, store-id/COPIED/KEPT bookkeeping, pushes `resurfacing.*` and `composing.quoteTexts` | none | delete | `settings.get/set/changed` | Windows read engine settings straight from the engine (sync cache in the shared client). Needs engine-side localized texts (section 3) |
| `engine/supervisor/unit.ts` (66 l) | Reads `services.json`, runs `tern engine start|restart` | `child_process`, `fs`, `path` | client (keep, grows) | `tern engine *` | This becomes the whole supervisor on macOS/Linux |
| `main/profile/key-manager.ts` (441 l) + `window/key-manager.ts` (86 l) | safeStorage keychain blob `credentials`: account secrets (legacy, before `credentialsMovedToEngine`) and integration keys for old engines | `electron`, `fs` | delete (after a migration release) | `tern-secrets` (`secrets.enc`), `secrets.set/has/delete` | — |
| `main/profile/mailspring-import*.ts`, `mailspring-keychain.ts` | Mailspring profile import, reads Mailspring's sqlite and keychain | `better-sqlite3`, `child_process`, `fs` | engine/CLI (later) | `credentials.import`, `tern-store/legacy_import.rs` | `tern import mailspring` would let headless and CLI users do it; low priority |
| `main/ipc/database-agent.ts` + `data/database/agent.js` | utilityProcess for DatabaseStore `.background()` queries | `electron`, better-sqlite3 | delete | `query` | Goes with DatabaseStore |
| `main/ipc/oauth-receiver.ts` | Loopback HTTP server for the OAuth redirect | `http` | client | `account.oauth.*` | — |
| `main/ipc/external-fetch.ts` | Credential-less fetch from main (unsubscribe fallback, iframe image copy, CSP) | `electron` net | client (shrinks) | `messages.unsubscribe` does the POST | Drop the unsubscribe use (fallback) |
| `main/ipc/quick-preview.ts`, `main/windows/quick-preview.ts` | Attachment previews from `<config>/files` | `fs`, `crypto` | client | `files.get/read` | Read via `files.read` when the engine is remote |
| `main/ipc/app-api.ts`, `notification.ts`, `cli-link.ts`, `attachment-safety.ts` | Dialogs, clipboard, native notifications, `tern` symlink, attachment open checks | `electron` | client | — | — |
| `main/windows/*`, `frame/*`, `protocols/*`, `platform/*`, `autoupdate/*`, `application.ts`, `main.js`, `permission-policy.ts`, `profile/{marker,reset}.ts` | Windows, message frame scheme, `tern-asset:`/`tern-file:`, menu, tray, taskbar, updates, profile reset | `electron`, `fs` | client | — | `profile/reset.ts` deletes tern.db itself; `engine.resetDatabase` exists, so reset could be engine-side except for the "wipe config" part |

Supervisor recommendation: lifecycle goes to `tern` (units on macOS/Linux, socket activation
where it can), main keeps only "ensure the engine unit is running, then connect" plus a direct
spawn on Windows. Everything stdin-only that windows reach over IPC (`account.test`,
`account.reset`, `engine.resetDatabase`, `engine.setLogLevel`) should be socket methods under
the local-only policy, so neither main nor the supervisor needs the stdin channel. That
removes `host.ts`, `settings.ts`, most of `supervisor.ts` and `EngineServeProcess` (about
2.9k lines of main-process TS).

## 3. config/ and settings: account-level vs client-level

`config/config.ts` (1021 l, electron IPC to main), `schema.ts` (430 l), `persistence-manager.ts`
(289 l, fs), `migrator.ts`, `keys.ts`, `utils.ts`: **client** (the store stays for client
preferences). The engine already owns 16 mirrored keys plus `templates`, `resurfacing.*`,
`composing.quoteTexts` (`SETTINGS_KEYS`).

Already engine-owned (mirrored by `supervisor/settings.ts`): `core.sending.undoSend`,
`core.sending.defaultAccountIdForSend`, `core.notifications.unsnoozeToTop`,
`core.composing.html`, `signatures`, `defaultSignatures`, `core.integrations.{translation,
grammar,enrichment}`, `core.ai.{providers,features,accounts,includeAddresses,limits}`,
`mailRules`, `mailRulesAutoSince`.

Should move to the engine (CLI/MCP/web/mobile want the same answer):

| key | where read | why engine |
|---|---|---|
| `accounts`, `accountsVersion`, `accountsToRemoveFromEngine` | AccountStore, supervisor, onboarding, application.ts, Mailspring import | config.json is still the desktop's list of accounts; the engine has `accounts.json` + `account.list`. Two sources of truth. AccountStore should read `account.list` + `account.changed` |
| `core.reading.autoloadImages` + `<config>/autoload-images-whitelist.txt` | message-autoload-images store | Remote-image policy and per-sender allowlist are per user; the web has its own `loadRemoteImages` in localStorage |
| `core.contacts.findInMailDisabled` | contacts | Per-account: whether "found in mail" contacts show. The engine extracts them; nothing in Rust reads the key |
| `core.sending.defaultReplyType`, `core.sending.defaultSendType` | composer | The CLI/MCP reply default should match |
| `core.reading.markAsReadDelay` | message-list | Web copies the default by hand (`apps/web/src/lib/preferences.ts`); borderline, could stay per client |
| `core.calendar.disabledCalendars` | calendar sidebar/search | Which calendars show; borderline (view pref), but `events.expand` callers on every client want it |
| `core.composing.grammarCheckDisabledRules`, `grammarCheckLanguage` | composer-grammar-check | Integration settings next to `core.integrations.grammar` |
| `core.translation.automatic`, `core.translation.disabled` | translation plugin | Same, for translation |
| `core.appearance.labelColors`, `labelColorScheme` | LabelColorStore | Label colours are per label; web would show the same. Borderline |
| `core.intl.language`, `core.workspace.use24HourClock` | intl | The engine needs a locale and hour cycle for the texts it writes (quote attribution, forward header, resurfacing bodies, rule forwards). Today the desktop renders those strings and pushes them (`QUOTE_TEXTS_KEY`, `resurfacing.*`). Engine should own the strings and take a `locale`/`hourCycle` setting |
| `core.mcp` | Preferences > Agents | Stays local, but belongs to `tern`/`tern-mcp` (mcp.json), not config.json |
| `core.plugins.installs`, `core.disabledPackages` | packages | Client for UI plugins; engine extensions need their own registry (section 9) |

Client-level (stay in config.json): `core.workspace.*` (mode, showNavRail, systemTray,
trayIconStyle, traySystemTheme, showImportant, showUnreadForAllCategories, menubarStyle,
interfaceZoom, hiddenLocations), `core.themes`, `core.theme`, `core.appearance.{useSystemAccent,
emailRenderMode, lightThemeName, darkThemeName}`, `core.keymapTemplate`,
`core.attachments.{openFolderAfterDownload,displayFilePreview}`, `core.reading.{loadGravatar,
detailedHeaders, detailedNames, restrictMaxWidth, backspaceDelete, swipeDisabled,
descendingOrderMessageList}`, `core.composing.{spellcheck, spellcheckDefaultLanguage,
grammarCheck}`, `core.notifications.{enabled, sounds, sound, countBadge, countBadgeAllAccounts,
enabledForRepeatedTrackingEvents}`, `core.sending.sounds`, `core.tutorial.seen`,
`core.sync.verboseUntil`, `core.engine.reads` (delete with DatabaseStore), `addedToDock`,
`askedAboutAppMove`, `mailto.prompted-about-default`, `credentials*` (delete).

## 4. features/

Pure-UI parts waved through as **client**: `attachments` (80 l), `custom-fonts` (8), `custom-sounds`
(42), `mode-switch` (109), `theme-picker` (316, path for theme dirs), `system-tray` (163,
BadgeStore + ipc), `print` (278, hidden print window), `notifications/items/{default-client,
dev-mode,update}`, `notifications/wrapper`, `preferences/{index,root,sidebar,icon}`,
`preferences/tabs/{appearance,general,reading,notifications,advanced,language-row,
startup-rows,keymaps/*,server}`, `onboarding/pages/*` (except where noted), `main-calendar/views/*`,
`drag-*`, `thread-list/{columns,participants,scroll-tooltip,selected-items-stack,toolbar,
vertical,aria-utils,injects-toolbar-buttons,message-list-toolbar}`, `message-list/{email-frame,
email-frame-styles-store,email-render-mode,find-in-thread,hidden-messages-toggle,message-item*,
message-participants,message-timestamp,scroll-tooltip,subject-line-icons,sidebar-*}`,
`draft-list/{columns,draft-list,preview,selected-items-stack,send-status,sending-progress-bar,
toolbar*}`, `contacts/{svg-icons,detail-read,detail-edit,yymmdd-input,typeahead-free-input,
list-editor,list-context-menu,add-contact-toolbar}`.

### account-sidebar (1418 l)

| path | logic / state | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `sidebar-store.ts`, `sidebar-section.ts`, `account-switcher.tsx`, `account-commands.ts` | Section building from AccountStore/CategoryStore/ThreadCounts/Outbox, collapsed state in `AppEnv.savedState` | electron ipc (switcher) | client | `folders.list`, `labels.list`, `counts.get` | — |
| `sidebar-item.ts` `onMarkAllAsRead` | Loads every unread thread of the folder (`DatabaseStore.findAll(Thread)` with categories/unread/inAllMail matchers, spam/trash rule) then queues `ChangeUnreadTask` with all ids | DatabaseStore | engine | `ChangeUnreadTask` | A query-scoped task ("mark every thread in folder X read") so big folders don't ship all ids, and CLI/web get it |
| `sidebar-item.ts` `createCategory`/`detectFolderSeparator` | Guesses the IMAP delimiter from existing paths, UTF-7 encodes, builds the full path | — | engine | `SyncbackCategoryTask` (tern-client/tasks.rs says the engine writes the server's delimiter and namespace prefix from a `/` name) | Desktop should send `/`-separated names and drop the guess |
| `sidebar-item.ts` export folder (eml / mbox), `mkdirSync` staging | Queues `GetManyRFC2822Task {outputDir}` or `{format:'mbox', mboxPath, outputDir: <mbox>.<token>.partial}` | `fs` | split | `GetManyRFC2822Task` (writes `NNNNN - subject - day.eml` into a dir, crates/tern-tasks/src/export.rs) | Engine-side mbox output |
| `mbox-export-runner.ts` (177 l) | Watches TaskQueue, incrementally appends staged .eml into `<dest>.incomplete`, atomic rename, cleanup on cancel/failure, resumes after restart | `fs`, TaskQueue | engine | engine stages .eml only | The engine (or the export task) should write the mbox itself. The CLI re-implements the same assembly (`apps/cli/src/mbox.rs` 242 l "as the desktop's mail/mbox.ts writes it", `files.rs export_mbox`) |

### category-mapper, category-picker (865 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `category-mapper/preferences-category-mapper.tsx` | Role mapping UI, queues `ChangeRoleMappingTask` | — | client | `ChangeRoleMappingTask` | — |
| `category-picker/{label,move}-picker-popover.tsx`, `toolbar-category-picker.tsx` | Category lists via Rx observables, builds `ChangeLabelsTask`/`ChangeFolderTask`, "create and apply" | data/query/observables | client (calls) | the tasks | High-level "move/label these threads" verbs would let the popover stop choosing between folder and label semantics per account (shared with web's `features/mailbox/actions.ts`) |

### composer (1690 l) and composer-signature (1464 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `composer/view.tsx` (650 l) | Editor host, attachments drop/paste, send flow, runs `session.validateDraftRecipients()` / `validateDraftForSending()` | `electron` webUtils, `path` (tern-file URLs) | split | `draft.validate` (tern-compose/validate.rs, ports DraftEditingSession + composer-templates checks) | Desktop never calls `draft.validate`; the TS checks in `data/drafts/editing-session.ts` are a second copy |
| `composer/{header,account-contact-field,fields,header-actions,action-bar-plugins,attachments-area,quoted-text-control,send-action-button,compose-button}` | UI; from-account switching recreates the draft (DraftEditingSession) | — | client | `draft.update` | — |
| `composer-signature/composer-extension.ts` | `prepareNewDraft` inserts the signature (`SignatureStore.signatureForEmail`) | — | delete | `draft.create` already applies it (tern-compose/signature.rs); `ENGINE_PREPARED_EXTENSIONS` skips this | Delete with the JS draft fallback |
| `composer-signature/utils.ts` `applySignature`, `currentSignatureId` | Signature insertion/replacement in HTML, used again when the user switches signature in the composer dropdown | — | split | tern-compose/signature.rs | An engine call to re-apply a signature to a body (or the shared TS package), so the two insertion rules can't drift |
| `composer-signature/{preferences-signatures,templates,template-picker,photo-picker,account-default-picker,composer-dropdown,constants}` | Signature editor and HTML templates, settings `signatures`/`defaultSignatures` | `crypto` (ids) | client | settings | — |
| `send-and-archive/extension.ts` | Send action: `SendDraftTask.forSending` then `TaskFactory.tasksForArchiving` on the thread | DatabaseStore.modelify | engine | `draft.send` | `draft.send {then: 'archive'}` (or a post-send action field); CLI/MCP/web can't do it otherwise, and a scheduled send can't |

### contacts (2813 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `store.ts` (166 l) | Live `MutableQuerySubscription` on Contact (refs>0, !hidden, source filters, group membership), ContactGroup and ContactBook queries, perspective filtering, substring search, `localeCompare` sort in JS | DatabaseStore, QuerySubscription | split | `query` on Contact, `contacts.search` (autocomplete only: accountId + query + limit) | `contacts.list {accountId?, perspective: all|found-in-mail|group, groupId?, search?}` as a live query; locale-aware sort can stay client |
| `files.ts` (108 l) | .vcf import/export through dialogs, temp file for drag-out | `fs` | split | `contacts.import {vcf}`, `contacts.export` | Fine on desktop; `fs` stays for the user-picked file (OS integration) |
| `detail.tsx`, `detail-toolbar.tsx`, `perspectives-list.tsx` | `contacts.get/update`, queues `DestroyContactTask`, `ChangeContactGroupMembershipTask`, `SyncbackContactGroupTask`, `DestroyContactGroupTask` | — | client | all those tasks | (CLI lacks delete/groups, per TODO(cli-parity); engine has them) |
| `google-support.ts`, `found-in-mail-enabled-bar.tsx` | Google People read-only notice, `core.contacts.findInMailDisabled` | — | split | — | Setting to engine (section 3) |
| `list.tsx` | List UI, drag-out vcf | `electron` webUtils, `path` | client | — | — |

### draft-list (792 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `store.ts` (299 l) | Paged drafts query; `pendingSends`: drafts with `send-later` metadata or a running `SendDraftTask`, soonest first, pinned above the rest; marks `uploadTaskId`; follows the focused draft by `headerMessageId` | DatabaseStore, QueryResultSet, OutboxStore | split | `query`, `metadata.list`, `tasks.list` | An outbox/scheduled view in the engine (`drafts.pending` or a `query` flag). The CLI computes the same thing (`apps/cli/src/draft.rs` `scheduled_row`/`scheduled`) |

### events (1156 l, invitation header)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `header.tsx` (834 l) | Reads the .ics attachment from disk, parses it with ical.js (`CalendarUtils.parseICSString`), method (request/reply/cancel/counter), times with tz mapping (`findOneIana`), finds our stored copies (`DatabaseStore.findAll(Event).where({icsuid})`, calendars), conflicts, RSVP | `fs`, `electron`; DatabaseStore listen/findAll | split | `events.expand {ics, focus, conflicts}`, `events.rsvp {ics, messageId, ...}` (picks the stored copy itself: `pim_api.rs` `stored_copy`) | `events.invitation {messageId}` → method, summary, organizer, times, our partstat, target/problem, conflicts. Then the header is rendering only and the web/mobile get invitations |
| `rsvp-target.ts` (211 l) `resolveRSVPTarget` | Picks the copy we may answer: writable, ours by ownership or name, ambiguous/read-only/not-ours problems | ical.js | delete (after the above) | `pim_api.rs` `stored_copy`, `is_own_calendar`, `default_calendar` | Engine reports *why* (problem enum) instead of only `NotOurs` |
| `invitation-calendar-refresh.ts` | On an arriving message with an ICS (`rulesReady`), throttled `calendar.sync` for the account | — | engine | `calendar.sync` | Engine should refresh calendars itself when an invitation lands (it already sees `rulesReady`) |
| `ics-attachment.ts` | Best ICS file of a message | — | engine (with the above) | — | — |

### list-unsubscribe (690 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `service.ts` | RFC 2369 header parse, RFC 8058 one-click POST via ExternalFetch, mailto via `App.openUrl`, best-option choice; `performEngineUnsubscribe` | `electron` shell | delete (fallback) / client (open web link) | `messages.unsubscribe` (POST, mailto send, web link back) | Delete the TS parse/POST path; only `shell.openExternal` for `method: 'web'` stays |
| `index.tsx`, `header.tsx` | Banner UI; body-link scan ("unsubscribe links found only in the body are left to the client") | — | split | header-only | Body-link detection in the engine so web/CLI get it |

### main-calendar (8863 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `data-source.ts`, `main-calendar.tsx`, `event/*`, `helpers.tsx`, `quick-event-popover.tsx`, `views/*` | Grid, drag/resize, recurrence UI; reads go to `events.expand`, writes to `events.create/update`; calendars from DatabaseStore | DatabaseStore (calendars, Event listen), `electron` (index) | client | `events.expand/create/update/rsvp`, `calendar.sync` | Live query for Calendar rows and an Event change feed scoped to a range |
| `search-bar.tsx` | `DatabaseStore.findAll(Event).search(q).limit(10)` then expands | DatabaseStore | engine | `query` (Event is in model_query_allowlist.json) | `events.search {query, range}` returning occurrences, so clients don't expand themselves |
| `quick-event-popover.tsx` | Natural-language date via chrono (`DateUtils.parseDateString`) | — | shared | — | — |

### message-autoload-images (225 l), remove-tracking-pixels (691 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `message-autoload-images/store.ts` | Remote-image policy: `core.reading.autoloadImages`, not in spam, per-sender allowlist in `autoload-images-whitelist.txt`, per-message allow; regexes to decide whether to show the banner | `fs`, `path` | split | — | Policy + allowlist as engine settings (e.g. `reading.remoteImages {auto, senders[]}`); the banner and CSP stay client |
| `remove-tracking-pixels/index.ts` + `mail-tracker-blocker-rules.ts` (501 l) | MessageView `formatMessageBody` strips tracker images on display; Composer `prepareNewDraft` strips them from quotes | — | split | tern-compose/pixels.rs + `desktop_data.json` (draft rules copied from this file by `draft-corpus-capture.test.ts`) | Display-side stripping isn't in the engine or the web. Either `messages.get {body:'display'}` returns the stripped body, or the rules move to the shared TS package from one source. The composer half is already engine-side (delete with the JS fallback) |

### message-list (3141 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `message-controls.tsx` Show Original | `GetMessageRFC2822Task {filepath: <tmp>/<id>.eml}`, `waitForPerformRemote`, reads it back, escapes into an HTML page | `fs`, `path` | split | `GetMessageRFC2822Task` (engine writes to a path on *its* filesystem) | `messages.raw {messageId}` returning bytes (chunked like `files.read`). Path-based export can't work for a remote engine |
| `message-controls.tsx`, `message-list.tsx` Download/Save as .eml | Same task with a user-picked path | — | split | same | Same; desktop keeps the save dialog and writes the bytes |
| `message-list.tsx` Forward as attachment | Stages .eml, `DraftFactory.createDraft` in JS, `SyncbackDraftTask`, `Actions.addAttachment` | — | engine | `draft.create kind=forward` (inline only) | `draft.create {kind:'forward', asAttachment:true}` |
| `index.tsx` | Thread lookup by id | DatabaseStore.find | client | `threads.get` | — |

### notifications (914 l), unread-notifications (376 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `unread-notifications/index.ts` `Notifier` | Legacy path: picks new mail from Message deltas with `headersSyncComplete`; both paths re-filter unread, after launch, not loopback, in inbox (Thread lookup), batch 100 ms, dedupe 2000 ids, sound quiet period, mark-read/archive from the notification (queueTask) | DatabaseStore listen/findAll, EngineEvents | split | `mail.new` (tern-sync `is_new_mail`: UIDNEXT, unread, not draft, not own address, in inbox) | Delete the legacy branch and the duplicate filter. `mail.new` should carry subject/from/snippet/threadId so no DB read is needed (mobile push wants the same). Sound/notification display stays client |
| `notifications/items/{account-error,offline,disabled-mail-rules}` | Account state, offline, `rules.disabled` banners | `electron` ipc | client | `account.state`, `rules.disabled` | — |
| `notifications/sidebar/{sync,syncback,export}-activity` | Progress from TaskQueue and folder sync state | TaskQueue | client | `tasks.list`, `account.state` | — |

### onboarding (2481 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `helpers.ts`, `store.ts`, `pages/oauth-signin.tsx`, `pages/account-settings*.tsx` | Discovery, add/reconnect, OAuth, `container_folder` default, field tidy | `electron` ipc | client | `account.discover/add/test/oauth.*` | — |
| `connection-security.ts` | Port → TLS mode, loopback exemption, `repairStoredSecurity` | — | shared | tern-providers decides discovered security | Web copies it (`apps/web/src/features/settings/accounts/account-draft.ts` "Same rule as the desktop's ...connection-security.ts") |
| `account-providers.tsx` | Provider tiles | — | client | — | Web has its own preset table (`presets.ts`, "from the desktop's onboarding table") |
| `pages/initial-preferences.tsx` | Keymap template list from disk | `fs`, `path` | client | — | — |

### preferences (6884 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `tabs/accounts/{accounts,details,list}.tsx` | Account editing, reorder, aliases, autoaddress, logs (`engine.logs.tail`), temp file of the log | `electron` | client | `account.update/reorder/remove`, `engine.logs.tail` | — |
| `tabs/mail-rules.tsx` + `components/scenario-editor/*` | Rule editor over `MailRulesStore` (settings `mailRules`), `rules.reprocess` | — | client | `mailRules` setting, `rules.reprocess/cancel/progress/disabled` | `scenario-editor/models.ts` `Comparator.evaluate`/`Template.evaluate` look unused in src/ and plugins/ (the engine evaluates rules) → delete |
| `tabs/integrations.tsx` | Provider forms, test, keys (KeyManager read for old engines) | — | client | `integrations.test`, `secrets.*`, `capabilities` | Delete KeyManager fallback |
| `tabs/composing.tsx` | Composing prefs, default account | — | client | settings | — |
| `tabs/mcp/{policy,section}.tsx` (1184 l) | Reads/validates/writes `<config>/mcp.json` ("Validation follows tern-mcp's"), audit log tail, token file | `fs`, `path` | split → CLI | apps/mcp `policy.rs` | `tern mcp policy get/set --json` so the pane stops re-implementing tern-mcp's validation |
| `tabs/mcp/tern-mcp.ts`, `tabs/plugins/tern-cli.ts`, `tabs/local-services.ts`, `tabs/services-rows.tsx`, `tabs/command-line-row.tsx` | Run bundled `tern`/`tern-mcp` for installs, services, CLI link | `child_process` | client | `tern service|engine|gateway|mcp-http *`, `tern --json status` | — (already the agreed model) |
| `tabs/plugins/plugins.tsx` | Plugin list, approvals, service plugin install via `tern service install` | — | client | — | Engine-extension listing (section 9) |

### thread-list (2654 l) and thread-search (1072 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `thread-list/data-source.ts` (111 l) | Thread page subscription, then one `findAll(Message, {threadId: missing})` and a QuerySubscription *per visible thread* for `__messages` (senders, drafts, hidden filter) | DatabaseStore, QuerySubscription ×N | engine | `query` on Thread (no message join) | Live thread-list query whose rows carry message count, per-message senders, has-draft, attachment flags (web TODO in `apps/web/src/features/mailbox/list/senders.ts` asks the same) |
| `thread-list/store.ts` | Perspective → data source, perspective key | MutableQuerySubscription | client (on new live query) | — | — |
| `thread-list/{toolbar-buttons,quick-actions,thread-list,context-menu,icon,empty-folder-bar}` | Archive/trash/spam/not-spam/star/unread/swipe via `TaskFactory.tasksForArchiving/MovingToTrash/MarkingAsSpam`, `perspective.canArchiveThreads/canMoveThreadsTo/tasksForRemovingItems` | DatabaseStore (context-menu modelify/findBy) | engine (semantics) | low-level `Change{Folder,Labels,Unread,Starred}Task`, `ExpungeAllInFolderTask` | Verb tasks/RPCs (`threads.archive/trash/spam/notSpam/remove {perspective}`) so "what archive means on Gmail vs IMAP, from this folder" lives once. Today: TaskFactory + perspectives (data/), CLI, MCP, web `features/mailbox/actions.ts` each decide |
| `thread-list/context-menu.ts` export .eml | `GetMessageRFC2822Task {filepath: path.join(dir, name)}` per newest message (`EmlUtils.newestExportableMessagesForThreadIds`) | `path` | split | the task | `messages.raw` (above) |
| `thread-list/permalink-handler.ts` | `tern://` thread links, lookup by headerMessageId | `electron` ipc, DatabaseStore.findBy | client | `query` | — |
| `thread-search/query-subscription.ts` | Parses the grammar in TS (`mail/search/parser`), `structuredSearch` SQL, `.background()`, limit 1000, plus `SearchBarResults` plugin ids | DatabaseStore | engine | `search {grammar:true, tz, now}`, `query.search {grammar}`, tern-store `search_grammar.rs`/`search_dates.rs` | DatabaseStore's engine path throws on `structuredSearch` (`data/query/query.ts:419`), so search can't leave SQLite through DatabaseStore yet; send the text with `grammar:true` instead. Live search results (change feed) missing |
| `thread-search/bar-util.ts`, `bar.tsx`, `store.ts`, `mailbox-perspective.tsx`, `tokenizing-contenteditable.tsx` | Suggestions (subject matches via structuredSearch, ContactStore), chips | DatabaseStore | split | `search`, `contacts.search` | `search.suggest {prefix}` would cover both; the chip editor stays client and is duplicated in web `features/search/query.ts`, `suggest.ts` |

### undo-redo (267 l)

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `undo-redo/toast.tsx` + `data/stores/undo-redo.ts` | Toast, undo-send extension of the window (re-saves `send-later` expiration), `core:undo` | DatabaseStore.find | split | undo send via `draft.cancelSend` / send-later metadata | Server-side undo (`undo.last`, `undo.list`): task pairs are already defined per class (`createUndoTask`), so the engine can keep the stack. CLI parity lists "Undo of the last change" as missing |

## 5. components/

| path | logic | deps | verdict | engine has | missing |
|---|---|---|---|---|---|
| `kit/*` (20 files, 1407 l), `sheets/*`, `table/*`, `multiselect/*`, `decorators/*`, `injected/*`, `searchable/*`, `menu`, `modal`, `flexbox`, `scroll-region`, `resizable-region`, `tokenizing-text-field`, pickers, `outline-view*`, `editable-*`, `spinner`, `switch`, etc. | UI | — | client | — | — |
| `composer-editor/*` (21 files, 6549 l) | Slate editor, conversion, emoji, grammar marks, templates, plaintext helpers (`plaintext.ts` 124 l) | `electron` webUtils, `fs`/`os` temp file for pasted blobs | client (editor); `plaintext.ts` duplicated | tern-compose/plaintext.rs (quote deepening, 72-col wrap) | Pasted files: `files.put {data}` instead of a temp file + path |
| `list/{data-source,observable-data-source,selection,lazy-rendered-list,tabular*}` | List windowing; `selection.ts` listens to DatabaseStore to refresh selected models | DatabaseStore.listen | client (on the shared live-query API) | — | Change feed in the shared client |
| `participants-text-field.tsx` | Expands contact-group tokens with `findAll(Contact).contactGroups.containsAny` | DatabaseStore, electron ipc | client | `query` on Contact | — |
| `mail-important-icon.tsx`, `mail-label-set.tsx` | Queue `ChangeLabelsTask` (important/remove label) | — | client | the task | — |
| `attachment-items.tsx` | Drag-out of an attachment by its path under `<config>/files`, `fs.existsSync` | `fs`, `path` | split | `files.get/read` | With a remote engine, drag-out needs `files.read` to a temp file |
| `evented-iframe.tsx` | Context menu on email images: copy/save reads `tern-file:` from `<config>/files`, external images via ExternalFetch | `fs`, `path`, `electron` | split | `files.read` | Same |
| `contact-profile-photo.tsx` | Gravatar md5 | `crypto` | client | — | — |
| `engine-unavailable-state.tsx`, `syncing-list-state.tsx`, `empty-list-state.tsx` | Engine-reads state | engine-reads | client | — | — |
| `swipe-container.tsx` | macOS swipe direction (`defaults read`) | `child_process` | client | — | — |
| `nav-rail.tsx`, `webview.tsx`, `mailspring-import.tsx` | UI/OS | `electron` | client | — | — |
| `scenario-editor/models.ts` | Rule comparators with `evaluate()` that nothing calls | — | split (keep templates, delete evaluate) | rules.rs | — |

## 6. window/, platform/, intl/, utils/

| path | logic | deps | verdict | notes |
|---|---|---|---|---|
| `window/app-env.ts` (1040 l) | AppEnv globals, `engineBridge`, savedState, window dims, column widths | `electron`, `path` | client | `engineBridge` moves to the shared client |
| `window/{keymap-manager,menu-manager,command-registry,event-handler,spellchecker,menu-helpers,bootstrap,secondary-bootstrap,promise-extensions,key-manager}` | Keymaps from disk, menus, commands, spellchecker dictionaries, `tern://` handling | `electron`, `fs` | client | `key-manager.ts` (facade) deletes with KeyManager |
| `window/platform/{native-notifications,do-not-disturb-*,linux-theme,system-start-service}` | OS notifications, DND, GTK theme, login items | `electron`, `child_process`, `fs` | client | — |
| `window/external-fetch.ts` | Credential-less fetch types | `electron` | client | — |
| `platform/*` (app-api, compile caches, error logger, devtools, default-client-helper, linux-desktop, safe-shell, system-sounds, cli-link) | Electron/OS runtime | `electron`, `fs`, `child_process` | client | — |
| `intl/localize.ts` (309 l) | Localization tables, RTL | `fs`, `electron` | client + engine gap | The engine needs its own copy of the strings it writes (quote attribution, forward header, resurfacing, rule forwards); today `intl/date-format.ts` `quoteTexts()` and `engine/resurfacing.ts` render them in TS and push them as settings |
| `intl/date-format.ts` (218 l) | Locale/hour-cycle formats, `quoteTexts` | — | split | see above |
| `utils/date.ts` (543 l) | Snooze presets (later today, tonight, tomorrow, weekend, next week), chrono natural-language parse, time strings | — | shared | CLI re-implements a subset (`apps/cli/src/mail_actions.rs` `parse_until`, `MORNING_HOUR` "the desktop's morning (date-utils.ts)"); engine has `search_dates.rs` for search |
| `utils/{regexp,dom,backoff-schedulers,shared-listener,aiff,file-url,fs,private-temp}` | Helpers | `fs`, `path` | client | `regexp.ts` signature/quote regexes are also in tern-compose |

## 7. packages/ (plugin and theme loading)

| path | logic | deps | verdict | notes |
|---|---|---|---|---|
| `manager.ts`, `package.ts`, `builtin-feature.ts`, `bundled-packages.ts`, `install-record.ts`, `resources.ts` | Discovery under `<config>/packages` and bundled roots, activation, install hashes | `fs`, `path`, `crypto`, `electron` | client | UI-extension host. Engine extensions need a separate registry the engine reads (section 9) |
| `theme-manager.ts`, `theme-preview.ts`, `style-manager.ts`, `compile-cache-less.ts`, `sound-registry.ts` | Themes, CSS, LESS for v1, sounds | `fs`, `electron` | client | — |
| `component-registry.ts`, `extension-registry.ts`, `service-registry.ts`, `composer-extension.ts`, `message-view-extension.ts` | Slots and behaviour hooks | — | split | `ComposerExtension.prepareNewDraft/warningsForSending/applyTransformsForSending/onSendSuccess` and `MessageViewExtension.formatMessageBody` are mail behaviour that only the desktop runs (plugin-api-v2.md line 313 already says "they don't run for CLI, MCP or engine-executed sends") |
| `api/mailspring-exports.{js,d.ts}` | v1 API: exposes DatabaseStore, QuerySubscription(Pool), MutableQuerySubscription, Rx `fromQuery` | DatabaseStore | client (compat) | v1 plugins need a DatabaseStore-shaped shim over `query` + change feed until 2.0; plugin-api-v2.md §4 already plans `this.app.database` "over the engine" |
| `api-version.ts` | API version | — | client | — |

## 8. File features

| feature | today | engine has | gap |
|---|---|---|---|
| Save message as .eml (message menu, thread menu, context menu) | `GetMessageRFC2822Task {filepath}`: the engine writes to a path the client names | `GetMessageRFC2822Task` | Path is the engine's filesystem: breaks for web/gateway/remote engines. `messages.raw {messageId, offset, len}` (bytes, like `files.read`) |
| Show Original | Same task into a private temp dir, read back | same | same |
| Export folder as .eml files | `GetManyRFC2822Task {outputDir}` | the task | Path-based; fine local, not remote |
| Export folder as .mbox | Engine stages .eml, desktop `mbox-export-runner.ts` + `mail/mbox.ts` assemble; CLI `mbox.rs` does the same | staging only | Engine-side mbox writer (or a streamed export) with progress |
| Forward as attachment | Stage .eml, JS DraftFactory, addAttachment | `draft.create forward` inline | `asAttachment` |
| Attachments open/drag/save/preview | Direct reads of `<config>/files/...` (AttachmentStore in data/, attachment-items, evented-iframe, quick-preview) | `files.get/read/put` | Desktop could switch to `files.read` behind a cache when the engine is remote; `files.put {path}` is refused on relayed connections |
| Pasted/dropped attachments | Temp file + path | `files.put {data|path}` | Use `files.put {data}` for blobs |
| vCard import/export | Desktop reads/writes the user-picked file, engine converts | `contacts.import/export` | None |
| CSV | I found no CSV import/export in scope (SheetJS is only in quick-preview vendor) | — | — |
| ICS invitations | Desktop reads the attachment file and parses ical.js | `events.expand {ics}`, `events.rsvp {ics}` | `events.invitation {messageId}` |
| Print | Hidden print window renders the thread HTML | — | Client (OS) |
| Templates | `composer-templates` plugin reads `<config>/templates/*.html` itself | `templates.list/get/save/delete` (tern-compose/templates.rs reads the same files) | Desktop doesn't call `templates.*`; two readers of one directory |

## 9. Plugins: UI extensions vs engine extensions

Desktop hooks that are mail behaviour and should become engine extensions (run for every
client and for engine-executed sends):

- `ComposerExtension.prepareNewDraft` (composer-templates, signature, tracking pixels: the last
  two are already ported into `draft.create`).
- `ComposerExtension.warningsForSending` (composer-templates' unfilled variables: ported into
  `draft.validate`, which the desktop doesn't call).
- `ComposerExtension.applyTransformsForSending` / `needsPerRecipientBodies` (link/open tracking:
  ported into tern-compose/tracking.rs; third-party PGP plugins would still need a hook).
  This is why `draft.send` takes window-computed `bodies`.
- `ComposerExtension.onSendSuccess`, send actions like send-and-archive.
- `MessageViewExtension.formatMessageBody` for tracker removal (and translation/link-tracking
  plugins).
- `DatabaseStore.listen` uses in send-later, send-reminders, thread-snooze, ai, mute,
  spoof-guard (plugin-api-v2.md §1.4 table): these are reactions to data changes that belong to
  the engine (the expiration executor already covers send-later/snooze/reminders).

What the engine has: built-in ports of the bundled ones (tern-compose), rules, AI triage,
expiration executor, service plugins (`tern service install`, a process next to the engine that
talks over the socket). What it lacks: a hook API (e.g. `hooks.register {event:
'draft.beforeSend'|'message.arrived'|'task.beforeQueue', ...}` served by a service plugin over
the socket, or an embedded WASM/JS runtime). plugin-api-v2.md is desktop-only today: §3.3
`addComposerExtension`, §6 engine access, §7 metadata; it needs a section that splits the
manifest into `ui` (per client, slots) and `engine` (hooks run by the engine or a service
plugin), and says which v1 hooks map to which side.

## 10. Node usage that disappears

91 non-main files in scope import fs/path/electron/child_process (18 more in data/ and mail/,
not counted here). Most are OS integration and stay. The ones that go away, or lose their Node
use, once the data layer and the file features move:

- `fs` for mail data (8): `features/account-sidebar/mbox-export-runner.ts`,
  `features/account-sidebar/sidebar-item.ts` (staging dir), `features/events/header.tsx` (ICS
  read), `features/message-autoload-images/store.ts` (allowlist file),
  `features/message-list/message-controls.tsx` (raw .eml), `features/thread-list/context-menu.ts`
  (`path.join` for export), `components/attachment-items.tsx`, `components/evented-iframe.tsx`
  (tern-file reads; the image-save half stays).
- Temp files for uploads (1): `components/composer-editor/composer-editor.tsx` (`files.put {data}`).
- MCP policy file (2): `features/preferences/tabs/mcp/{policy.ts,section.tsx}` (via `tern mcp policy`).
- Supervisor (main-only, 4 files: `engine/process.ts`, `engine/supervisor/{supervisor,unit,host}.ts`):
  shrink to `unit.ts` + a Windows spawn.
- KeyManager facade: `window/key-manager.ts`; main `profile/key-manager.ts`.
- Background DB agent: `main/ipc/database-agent.ts` (+ `data/database/agent.js`).

So about 12 renderer files in scope lose Node, plus about 5 main-process files. The rest are
UI that only needs `ipcRenderer.send('command')` / `shell.openExternal` (about 20 feature files;
these could go through `platform/app-api.ts` but stay Electron either way), plugin and theme
loading (10 in packages/), OS integration (window/platform, platform/), and the engine socket
(`engine/client/connection.ts` uses `net`; a WebSocket or main-proxied transport would drop it
too).

Fallback branches to delete: 17 files check `Feature.*` for engines without a feature (63
checks: Accounts 14, ExpirationExecutor 10, AiTriage 6, Integrations 5, DraftPipeline 4, Query
3, Rules 2, Counts 2, and one each for Unsubscribe, Settings, MailNew, ContactEditing,
Capabilities, CalendarEditing, Ai, AccountSetup). The engine is bundled, so these only matter
for a downgraded or detached older engine.

## 11. Duplicated logic

TS vs Rust:

1. Socket protocol types: `engine/protocol.ts` (954 l) and web `engine/types.ts`+`rpc.ts` vs `crates/tern-ipc`.
2. mbox framing and assembly: `mail/mbox.ts` + `features/account-sidebar/mbox-export-runner.ts` vs `apps/cli/src/mbox.rs` + `files.rs export_mbox`.
3. Pending/scheduled sends: `features/draft-list/store.ts pendingSends` vs `apps/cli/src/draft.rs scheduled_row`.
4. RSVP target choice: `features/events/rsvp-target.ts resolveRSVPTarget` vs `apps/engine/src/pim_api.rs stored_copy/is_own_calendar`.
5. New-mail decision: `features/unread-notifications/index.ts _onMessagesChanged` (unread, not loopback, inbox) vs `tern-sync mail_processor.rs is_new_mail`.
6. Unsubscribe: `features/list-unsubscribe/service.ts` (parse, one-click POST, best option) vs `apps/engine/src/unsubscribe.rs` + `tern_net::unsubscribe`.
7. Signature insertion: `features/composer-signature/{composer-extension,utils}.ts applySignature` vs `tern-compose/signature.rs`.
8. Tracker rules: `features/remove-tracking-pixels/*` vs `tern-compose/pixels.rs` + `desktop_data.json` (copied by a capture test).
9. Send checks: `data/drafts/editing-session.ts validateDraft*` (called from `features/composer/view.tsx`) vs `tern-compose/validate.rs` (`draft.validate`, unused by the desktop).
10. Templates: `plugins/composer-templates` reads `<config>/templates` vs `tern-compose/templates.rs` (`templates.*`, unused by the desktop).
11. Draft building: `data/drafts/draft-factory.ts` (JS fallback) vs `tern-compose/draft.rs`; web `features/composer/draft.ts` ("Reply/forward construction follows the desktop's DraftFactory") is a third copy.
12. Plaintext composer helpers: `components/composer-editor/plaintext.ts` vs `tern-compose/plaintext.rs`.
13. Search grammar: `mail/search/parser` (used by `features/thread-search/*`) vs `tern-store/search_grammar.rs`/`search_dates.rs`; web `features/search/query.ts` + `engine/mock/search.ts`.
14. Folder delimiter/UTF-7: `features/account-sidebar/sidebar-item.ts detectFolderSeparator` + imapUtf7 vs engine `SyncbackCategoryTask` path handling and `tern-client/utf7.rs`; web `engine/model.ts` has a third UTF-7 decoder.
15. Mail action semantics (archive/trash/spam per perspective and provider): `data/` TaskFactory + perspectives (used by thread-list features) vs `tern-client/tasks.rs` (CLI/MCP) vs web `features/mailbox/actions.ts`.
16. Plugin metadata shapes (send-later, reminders, tracking): desktop plugins vs `tern-client/metadata.rs`.
17. Attachment file names: desktop save path rules vs `tern-client/files.rs` ("attachment file names as the desktop saves them").
18. MCP policy validation: `features/preferences/tabs/mcp/policy.ts` vs `apps/mcp/src/policy.rs`.
19. Snooze/"morning" times: `utils/date.ts` presets vs `apps/cli/src/mail_actions.rs parse_until`.
20. Settings mirror: config.json and the engine's settings.json both hold the 16 engine-owned keys (`engine/supervisor/settings.ts`); `accounts` in config.json and engine `accounts.json`.
21. Google grant revocation: `engine/client/oauth-revocation.ts` vs `accounts_api.rs revoke`.
22. Quote/attribution/resurfacing texts rendered in TS (`intl/date-format.ts quoteTexts`, `engine/resurfacing.ts`) and pushed into engine settings.

Desktop vs web (TS twice):

- Connection security rule (`features/onboarding/connection-security.ts` vs web `settings/accounts/account-draft.ts`).
- Provider presets (`features/onboarding/account-providers.tsx` + provider settings JSON vs web `settings/accounts/presets.ts`).
- Integration provider table (`engine/client/integrations.ts PROVIDERS` vs web `settings/integrations/store.ts`).
- AI settings shape and capability checks (`engine/client/ai.ts`, `data/stores/capability.ts` vs web `features/ai/config.ts`, `settings/capabilities.ts`).
- Category/thread/message model helpers (desktop models vs web `engine/model.ts`, "keep the semantics in step").
- Undo-send delay and preferences (`core.sending.undoSend` vs web `lib/undo-send.ts` + localStorage `undoSendMs`; `markAsReadDelay` defaults copied).
- Search query editing (thread-search chips vs web `features/search/query.ts`, `suggest.ts`).
- Optimistic thread updates (web `engine/optimistic.ts`; the desktop relies on the engine's fast local phase).

## 12. Engine gaps for thin clients (from this area)

1. Live queries: `query` + a subscription that pushes row changes for that query (thread list page, contacts list, drafts, calendars). Today `subscribe` takes only `{accounts, classes}` and every client re-runs queries on deltas.
2. Thread rows with message summary (count, senders, has-draft, attachments) so the list doesn't open a subscription per thread.
3. Verb tasks for mail actions with perspective semantics (archive, trash, spam, not spam, remove-from-view, mark folder read by query).
4. Server-side undo stack (`undo.last/list/redo`).
5. `messages.raw` (bytes) for .eml save, Show Original, forward-as-attachment; engine-side mbox export with progress.
6. `draft.create {kind:'forward', asAttachment}`; `draft.send {then:'archive'}`; signature re-apply on an existing body.
7. `events.invitation {messageId}` (parsed invitation, our status, target and problem, conflicts) and engine-side calendar refresh when an invitation arrives; `events.search`.
8. `contacts.list` with perspectives/groups/search (contacts.search is autocomplete only).
9. `drafts.pending` / outbox view.
10. Richer `mail.new` (subject, from, snippet, threadId) so notifiers need no read.
11. Engine-owned localized texts (`locale`, `hourCycle` settings) instead of client-rendered `composing.quoteTexts` / `resurfacing.*`.
12. Remote-image policy and sender allowlist as settings; display-side tracker stripping (or the rules in a shared package).
13. Body-link unsubscribe detection.
14. Account list owned by the engine (drop config.json `accounts`). (Token revocation is already in `accounts_api.rs revoke`.)
15. Socket versions of the stdin-only methods (`account.test`, `account.reset`, `engine.resetDatabase`, `engine.setLogLevel`) under the local-only policy, so the supervisor can move to `tern`.
16. Engine extension hooks (draft before-send/transform, message arrived, task before-queue) for plugins.
17. Protocol codegen from tern-ipc (TS types for the shared package).
18. `tern mcp policy get/set` (CLI, not engine) so the desktop stops validating mcp.json itself.
19. Windows story for `tern` units (none today), or the desktop keeps a spawn path there.
