# Thin clients: bundled plugins and behaviour features

Scope: `apps/desktop/plugins/*` (13) and the behaviour-like `apps/desktop/src/features/*`, read on
main at 9c1ba543f. Paths are relative to `apps/desktop/` unless they start with `apps/`, `crates/`
or `docs/`. "serve" = `tern-engine --mode serve`, which the desktop always runs (apps/desktop/CLAUDE.md,
"One engine process"), so every `!engine.has(Feature.X)` fallback below is dead code against the
bundled engine and only serves an older external engine.

Verdicts: **engine** (behaviour moves to / already is in the engine; the TS goes), **client** (UI
slot, editor, OS integration, client prefs), **split** (both halves named), **delete** (dead or
duplicate, nothing to move).

Sizes are non-test TS/TSX lines. "Node" = fs/path/electron/child_process/os/tld/`platform/app-api`.

## 1. Scheduling and follow-up (send-later, undo send, snooze, reminders)

| Path | Logic / state | Node/Electron/DB | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `plugins/send-later/lib/main.ts` `handleMetadataExpiration` | On `metadata-expiration` deltas: clears `send-later` metadata, opens a DraftEditingSession and `Actions.sendDraft` for rows marked `clientFinalize` or with a non-engine `actionKey`; clears schedule of non-drafts when no executor | `DatabaseStore.listen`, main window only | delete (keep only until engine has send hooks) | `tern_sync::expiration_executor` (feature `expiration-executor`): finalises and sends due `send-later` rows incl. `isUndoSend`, retries 1/2/4/8 min, `error` on metadata, server claim (`MetadataBackend::claim`), `SEND_ACTIONS = ["send","send-and-archive"]` | The only live case is `clientFinalize` (third-party `applyTransformsForSending` or a plugin send action). Needs engine `draft.transform` + extension send actions (sec. 6), then this goes |
| `plugins/send-later/lib/{button,popover,status}.tsx` | Composer button writes `{expiration, clientFinalize?}` via `session.changes.addPluginMetadata`; draft-list status shows schedule/`error`, cancel = SyncbackMetadataTask | `TaskQueue.findTasks(SendDraftTask)` | client | metadata storage; `draft.send {delayMs}` writes the same row | An RPC `draft.schedule {accountId, headerMessageId, at, after?}` / `draft.unschedule` so clients don't hand-write metadata JSON (web `features/composer/send-later.ts` and CLI `followup.rs` write it too) |
| `src/data/stores/send-actions.ts` `SendActionsStore` | Collects `sendActions` from Composer extensions; `ENGINE_SEND_ACTION_KEYS`; `needsClientFinalize` (true when any Composer ext owns `applyTransformsForSending`); preferred action from `core.sending.defaultSendType` | config | split | `draft.send {after: "send"|"send-and-archive"}` (crates/tern-ipc/src/drafts.rs `DraftSendParams.after`) | Engine-side registry of send actions (`after` is a closed list); `core.sending.defaultSendType` not in `SETTINGS_KEYS` |
| `src/features/send-and-archive/extension.ts` | `performSendAction`: SendDraftTask + `TaskFactory.tasksForArchiving` | `DatabaseStore.modelify` | delete (fallback) / client (menu entry) | `draft.send {after: "send-and-archive"}`; `src/data/drafts/store.ts` `viaEngine` already routes there | nothing; the JS perform is the no-`draft-pipeline` fallback |
| `src/data/drafts/store.ts` send path (`viaEngine`, `queueUndoOnlyTask`) | Undo send: writes `send-later {isUndoSend}` itself without draft-pipeline; with it calls `draft.send {delayMs, bodies}` | — | split | `draft.send`, `draft.cancelSend`, `pending_sends` (bodies kept with the send) | — |
| `plugins/thread-snooze/lib/store.ts`, `utils.ts` (`moveThreads`, `markUnreadOrResurfaceThreads`) | Snooze = ChangeLabelsTask/ChangeFolderTask built from CategoryStore + SyncbackMetadataTask `{expiration}`; unsnooze on expiry (fallback), resurfacing via `DraftFactory.createDraftForResurfacing` + silent SendDraftTask | `DatabaseStore.listen`, `AppEnv.config` | engine (delete the TS) | `threads.snooze` / `threads.unsnooze` (`crates/tern-tasks/src/mail_actions.rs` `snooze_tasks`, `unsnooze_tasks`, feature `snooze`); executor unsnoozes + resurfaces (`core.notifications.unsnoozeToTop`) | **Desktop never calls `threads.snooze`** (web `features/mailbox/snooze.ts` and CLI `mail_actions.rs::run_snooze` do). Blocker: desktop's snooze is undoable (`canBeUndone: true`), `threads.snooze` returns tasks with no undo story -> needs engine undo (sec. 6) |
| `plugins/thread-snooze/lib/{buttons,popover,mail-label}.tsx`, `account-sidebar-extension.ts`, `actions.ts` | Toolbar/quick action/popover with date presets; "Snoozed" label; sidebar item | — | client | — | — |
| `plugins/send-reminders/lib/store.ts` | Fallback: reply baseline (`lastReplyTimestamp`), clear on reply, notify on expiry + resurfacing message; live: `_onFocusedContentChanged` acknowledges `shouldNotify` by writing `{}` | `DatabaseStore.listen`, `FocusedContentStore` | engine (fallback delete) / client (ack trigger) | executor + `Thread::reminder_update` on every thread save (tern-store), `SEND_REMINDERS_PLUGIN` | RPC `threads.acknowledgeReminder` (or `threads.remind {until|null}`) so the focus-ack isn't raw metadata; web has no reminders at all |
| `plugins/send-reminders/lib/utils.ts` (`updateReminderMetadata`, `assertMetadataShape`, `updateDraftReminderMetadata` under `thread:send-reminders`) | Metadata shape checks and the draft->thread promotion convention | — | engine | promotion of `thread:<plugin>` on send (tern-sync); `crates/tern-client/src/metadata.rs` has the same shapes for CLI/MCP | One RPC `threads.remind` / draft field; until then generated TS types from tern-client shapes |
| `plugins/send-reminders/lib/mailbox-perspective.ts` | "Reminders" view: `Thread.pluginMetadata.contains('send-reminders')` | DatabaseStore query | client (over shared query client) | `query` handles `contains`/`containsAny` on metadata (crates/tern-store/tests/fixtures/model_queries.json) | — |
| `plugins/send-reminders/lib/{composer-button,toolbar-button,popover*,headers,thread-timestamp,thread-list-extension,account-sidebar-extension}` | UI over metadata (5 roles + ThreadList + AccountSidebar ext) | — | client | — | — |
| `src/engine/resurfacing.ts` + `src/engine/supervisor/settings.ts` `_writeResurfacing` | Localised resurfacing HTML pushed into engine settings `resurfacing.snoozeBody/reminderBody` | — | engine | settings keys; executor reads them (`SNOOZE_BODY_KEY`, English fallback) | Engine-side localisation (a `locale` setting + catalog), else every client must push strings; same for `composing.quoteTexts` and `draft.validate` messages ("the desktop's English ones", validate.rs) |

## 2. Tracking and Activity

| Path | Logic / state | Node/Electron/DB | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `plugins/open-tracking/lib/*` (button, icon, message-status, popover) | Composer toggle writes `open-tracking` metadata (MetadataComposerToggleButton); thread-list icon and header status read `open_count`/opens | — | client | feature `tracking`: engine tracks on send (`crates/tern-compose/src/tracking.rs`, per-recipient bodies, Tern URL format); opens come back via metadata backend | `draft.update {tracking: {opens, links}}` or `draft.send {track}` instead of raw metadata (CLI `--track-opens` writes metadata too) |
| `plugins/link-tracking/lib/button.tsx` | Composer toggle (`link-tracking` metadata) | — | client | same | same |
| `plugins/link-tracking/lib/message-extension.tsx` `renderedMessageBodyIntoDocument` | Restores original hrefs (incl. Mailspring `redirect_url`), inserts click dots, popover | `MessageBody` bridge (`src/mail/frame/bridge.ts`) | client | sent copy stays untracked (tracking.rs), so href restore only matters for old/other-client mail | — |
| `plugins/activity/lib/{event-store.tsx,events.ts,data-source.ts,plugin-helpers.ts}` | Flattens open/link metadata into events (`eventsForMessage`, `foldRepeatedEvents`, `pluckByEmail`), unseen/unnotified via `AppEnv.savedState`, throttled native notifications (`notificationInterval`), `core.notifications.enabledForRepeatedTrackingEvents` | `Rx.Observable.fromQuery` (500 msgs), `NativeNotifications` | split | nothing for events; `metadata.list` | `tracking.events {accounts?, since?, limit}` + `tracking.event` notification (engine sees metadata updates from the backend first). Same flattening exists in `apps/cli/src/followup.rs` (`tern tracking`) and `apps/mcp/src/tools/followup.rs`. Notification display stays client |
| `plugins/activity/lib/engagement/stats.ts`, `dashboard/root.tsx` (815 lines: `firstOpenDelays`, `rateBuckets`, `delayBuckets`, `median`, `newThreadStat`), `message-scan.ts` | Full scan of every non-draft message in range, 500 per animation frame, in the renderer; per-recipient engagement, reply rates, time-to-open | DatabaseStore `.background()` chunks | engine | — | `tracking.stats {accounts, from, to, groupBy}` aggregate in tern-store; renderer scan is the heaviest DB use of any plugin and impossible for web/mobile |
| `plugins/activity/lib/csv-export.ts` | CSV with formula-injection escaping | `fs` | client (save dialog) | — | could be `tracking.stats {format:"csv"}` if CLI wants it |
| `plugins/activity/lib/{root,feed/*,list/*,dashboard/metrics-components,engagement/board,engagement/card,timespan*}` | Sheet, feed table, dashboard UI | `electron` (shell) in `dashboard/root.tsx`, `feed/table.tsx` | client | — | — |

## 3. Composer: templates, signatures, pixels, grammar, translation, AI reply

| Path | Logic / state | Node/Electron/DB | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `plugins/composer-templates/lib/store.ts` (412) | Reads/writes/renames/deletes `<config>/templates/*.html`, `fs.watch`, welcome template, `_onInsertTemplateId` (replace above signature, set subject unless reply) | `fs`, `path`, `platform/app-api` Dialogs | engine | `templates.list|get|save|delete` (apps/engine/src/templates_api.rs, same dir/format), `draft.create {template}`, `tern_compose::templates` insert rules | Desktop doesn't call them. `templates.save/delete` are LocalOnly for relayed clients and the `templates` settings key exists but is unused -> web keeps its own templates in localStorage (`apps/web/src/features/composer/templates.ts`, TODO(engine)). Decide one store; add `templates.rename`, `templates.insert {draft}` for an open draft |
| `plugins/composer-templates/lib/file.ts` | `<meta name="subject">` parse/stringify | — | delete | `tern_compose::templates` | — |
| `plugins/composer-templates/lib/composer-extension.ts` `warningsForSending` | "empty template area" warning | — | delete | `draft.validate` (`crates/tern-compose/src/validate.rs`: empty area + `template-variable`) | Desktop and web never call `draft.validate` (only defined in `src/engine/protocol.ts`) |
| `plugins/composer-templates/lib/{picker,status-bar,preferences-templates}.tsx` | Picker, footer, prefs tab | `electron`, `fs` (prefs) | client | — | — |
| `src/features/composer-signature/composer-extension.ts`, `utils.ts` `applySignature`, `src/data/stores/signature.ts` (`signatureForEmail`) | Default signature per From address, inserted on new drafts | `AppEnv.config` `signatures`/`defaultSignatures` | engine (JS path delete) | `tern_compose::signature`, `settings` (`signatures`, `defaultSignatures` in `SETTINGS_KEYS`), `draft.create` applies it; desktop skips it via `ENGINE_PREPARED_EXTENSIONS` | — (the JS path is the no-`draft-pipeline` fallback) |
| `src/features/composer-signature/templates.tsx` (506), `template-picker`, `photo-picker`, `preferences-signatures`, `composer-dropdown`, `account-default-picker` | Signature template HTML rendering from fields (`data`), editors, From-field dropdown | `platform/app-api` | split | stores `{body, data}` | Template rendering is client-only, so web/CLI can store but not render a template signature; move the renderer to the shared TS package or engine `signatures.render` |
| `src/features/remove-tracking-pixels/index.ts` (691 incl. rules) | (a) `prepareNewDraft`: strips trackers from quoted text; (b) `formatMessageBody`: strips trackers/hidden remote images on every message view | — | (a) delete, (b) engine | (a) `crates/tern-compose/src/pixels.rs` + `desktop_data.json` rules, applied by `draft.create` | (b) nothing: engine never strips on read; web shows trackers (only CSP blocks them). Needs `messages.get {render: true}` / a sanitised body field |
| `plugins/composer-grammar-check/lib/{store,service}.ts` | Per-draft block cache, dirty blocks, abort, dismissed rules; LanguageTool via `integrations.grammar` | `AppEnv.config` `core.composing.grammarCheck`, `…Language`, `…DisabledRules` | split | `integrations.grammar` (tern-integrations) | Editor state stays client; the three config keys aren't engine settings, so web/CLI can't share disabled rules; `integrations.*` require per-call `config` and are LocalOnly for relayed clients |
| `plugins/composer-grammar-check/lib/toggle.tsx` | Toolbar toggle | — | client | — | — |
| `plugins/translation/lib/service.ts` `translateMessageBody` | Removes quoted HTML, walks DOM for translatable text (skips monospace/tiny), 5000-char cap, LRU snippet cache, re-merges, appends quote | DOMParser in renderer | split | `integrations.translate {format:"html"}`, `integrations.detect` | `messages.translate {accountId, messageId, target}` returning HTML (block extraction + cache in engine) so web/mobile get it |
| `plugins/translation/lib/message-header.tsx` | Detects language with Chromium CLD (`static/extensions/chrome-i18n/main.js` via postMessage), auto/never lists `core.translation.automatic/disabled`, results in `localStorage` `translated-<id>` | `platform/app-api`, localStorage | split | `integrations.detect` (provider, costs a call) | Local detection in engine (whatlang-style) so detection isn't Electron-only; the two config lists as engine settings |
| `plugins/translation/lib/composer-button.tsx` | Translate draft text | — | client | `integrations.translate` | — |
| `plugins/ai/lib/client.ts` (445) | Stream ids, `ai.delta` collection, error-key mapping, key moving (`moveKeysToEngine`, KeyManager), `providerParams` | — | split -> shared TS package | `ai.*`, `ai-keys`, `capabilities.ai` | Same code in `apps/web/src/features/ai/api.ts` (`AiError`, `toAiError`, `newStreamId`, `errorMessage`) |
| `plugins/ai/lib/triage.ts` (`answered`, `chipCategory`, `correctCategory`, `triageInbox`, `listTriaged`) | Which chip a thread shows, "answered since" rule | — | engine | `ai.triage.run/list/progress`, `tern-ai-triage` metadata, tern-store `triaged_threads` (same "answered" rule) | `answered`/`chipCategory` exist 3x (desktop, `apps/web/src/features/ai/triage.ts`, Rust). Put `triage.chip` on Thread JSON or have the shared package own it |
| `plugins/ai/lib/needs-reply.ts` `NeedsReplySubscription` | Re-lists `ai.triage.list` on every Thread delta (debounced 500 ms), builds DatabaseStore IN query | `DatabaseStore.listen` | client over live query | `ai.triage.list` | A live query / subscription for `ai.triage.list` (web `needs-reply.ts` polls the same way) |
| `plugins/ai/lib/{thread-summary,reply-suggestions,reply-insert,triage-chip,preferences-ai}.tsx` | In-memory summary cache keyed by thread+messages, `worthSummarizing`, insertion into editor, prefs | — | client (cache could be engine) | `ai.summarize`, `ai.suggestReplies`, `ai.draftReply`, `ai.cancel`, `ai.usage` | Summary cache duplicated in `apps/web/src/features/ai/summary-memory.ts`; an engine-side cache (`ai.summarize {cached:true}`) would serve both |
| `src/engine/client/integrations.ts` (551) | `PROVIDERS`, `readSettings`, builds `config` for every `integrations.*` call, error mapping | config | split -> shared TS package | provider logic in `crates/tern-integrations`; `core.integrations.*` mirrored into engine settings | Engine should default `config` from its own `core.integrations.<kind>` setting (AI already does this with `provider` id). Then `integrations.*` can be Open for relayed clients like `ai.*` |

## 4. Message view: warnings, unsubscribe, images, invitations, profile

| Path | Logic / state | Node/Electron/DB | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `plugins/phishing-detection/lib/main.tsx` | Reply-To registered domain != From registered domain -> banner | `require('tld')` (2011 PSL), `MessageStore` | engine (logic delete) / client (render `warnings`) | `warnings` on Message JSON from `messages.get`/`threads.get` (apps/engine/src/warnings.rs, feature `message-warnings`) | Desktop recomputes and never reads `warnings`; web gets them from `threads.get` but renders nothing. `warnings` comes only from the read API, not on deltas/`query` rows, so the desktop's DatabaseStore models don't carry it |
| `src/features/list-unsubscribe/service.ts`, `index.tsx` | `parseListUnsubscribeHeader`, `chooseBestUnsubscribeOption`, body-link scan with cheerio (`findBodyUnsubscribeLink`, LRU cache), `performEngineUnsubscribe` | `electron.shell`, `platform/app-api`, cheerio | split | `messages.unsubscribe` (apps/engine/src/unsubscribe.rs, `tern_net::unsubscribe`): one-click POST, mailto via `unsubscribe_send_task`, web link returned | Availability on Message JSON (`unsubscribe: {method}`) so clients don't parse headers (web `reader/unsubscribe.ts` checks `hListUnsub`); body-link scan in engine ("left to the client" today) |
| `src/features/list-unsubscribe/header.tsx` | Banner + confirm | — | client | — | — |
| `src/features/message-autoload-images/store.ts` | Remote-image policy: `core.reading.autoloadImages`, not in Spam, sender allowlist in `<config>/autoload-images-whitelist.txt`, per-message temp allow; regex to show the banner | `fs`, `path` | split | — | Engine setting `reading.autoloadImages` + `reading.imageSenders` and `remoteImages: "allowed"|"blocked"|"none"` on Message JSON; CSP stays client. Web has a per-browser `loadRemoteImages` only (`apps/web/src/lib/preferences.ts`) |
| `src/features/message-autoload-images/{header,extension,actions}.tsx` | Banner, `allowsRemoteImages` hook | — | client | — | — |
| `src/features/events/header.tsx` (834) | Reads the ICS attachment from disk, parses with ical.js, method (request/reply/cancel/counter), finds stored copies via DatabaseStore (`Event.icsuid`), renders RSVP | `fs`, `electron.ipcRenderer`, DatabaseStore | split | `events.expand` (times), `events.rsvp` (builds REPLY/COUNTER, decides own copy in `pim_api.rs` `stored_copy` -> `Ours/None/NotOurs`) | `messages.invitation {accountId, messageId}` -> `{method, summary, times, organizer, myStatus, target|problem}`; web `features/calendar/ics.ts` parses ICS a third time |
| `src/features/events/rsvp-target.ts` `resolveRSVPTarget` | Which copy is ours: `not-on-a-calendar|read-only|not-ours|ambiguous` | — | engine | `pim_api.rs::stored_copy` (same decision for the write) | expose the resolution (above) |
| `src/features/events/invitation-calendar-refresh.ts` | On arriving messages with ICS (`rulesReady`), throttled `calendar.sync` per account (1/min) | `DatabaseStore.listen`, main window only | engine | `calendar.sync`, `rulesReady` flag is the engine's | An engine "message arrived" hook does this in tern-sync; today it only happens while a desktop window is open |
| `src/features/events/index.tsx` `HideICSAttachmentExtension.filterMessageFiles` | Hides duplicate ICS parts | — | client | — | (could be an annotation from `messages.invitation`) |
| `plugins/participant-profile/lib/data-source.ts` | Enrichment via `integrations.enrich`, 200-entry localStorage LRU (`pp-cache-v3-*`), skips `likelyNonHumanEmail` | localStorage | split | `integrations.enrich` with a 1-day in-memory cache | Persistent engine cache (contact-level `enrichment` on Contact JSON); `likelyNonHumanEmail` patterns are already in `tern_compose::data` |
| `plugins/participant-profile/lib/sidebar-*.tsx` | Profile card, related threads (DatabaseStore query by participant) | `src/intl/date-format` | client | `query` | — |
| `plugins/thread-sharing/lib/button.tsx` | Disabled stub, copy mailbox permalink | — | client | — | Sharing doesn't exist in engine or `apps/backend` |

## 5. Other behaviour-bearing features

| Path | Logic / state | Node/Electron/DB | Verdict | Engine already has | Missing |
|---|---|---|---|---|---|
| `src/features/unread-notifications/index.ts` (376) | OS notifications for new mail; mail.new batching (100 ms), dedupe set (2000), sound quiet period; fallback picks new mail from `headersSyncComplete` | `NativeNotifications`, `SoundRegistry`, DatabaseStore | client (fallback delete) | `mail.new` (feature `mail-new`) decides what is new | — |
| `src/features/undo-redo/toast.tsx` + `src/data/stores/undo-redo.ts` (193) + Task `createUndoTask(s)` | Undo stack in the window; undo tasks built client-side (ChangeFolderTask uses engine `undoPlacements`) | DatabaseStore | split | `undoPlacements` recorded by tern-tasks local phase; `draft.cancelSend`; SyncbackEventTask `restore` | Server-side undo (`task.undo {taskId}` / `undo.last`), listed in the `TODO(cli-parity)` block of apps/cli/src/cli.rs; needed before snooze/archive move to engine RPCs |
| `src/features/thread-search/query-subscription.ts` + `src/mail/search/parser.ts` (398) | Parses the search grammar in TS, `structuredSearch` -> SQL | DatabaseStore | engine | `query.search {grammar:true}`, `search` (feature `search-grammar`) | — (duplicate; web already sends raw grammar) |
| `src/features/account-sidebar/mbox-export-runner.ts` | Assembles mbox from the engine's staged `.eml` files, incrementally | `fs`, TaskQueue | engine | GetManyRFC2822Task staging; `apps/cli/src/mbox.rs` writes mbox itself | Engine writes the mbox (export task with `format: mbox`), so the runner and CLI framing collapse |
| `src/features/category-mapper` | Role mapping prefs -> ChangeRoleMappingTask | — | client | task | — |
| `src/features/draft-list` | Draft list + send status | DatabaseStore | client | — | — |
| `src/features/notifications`, `system-tray`, `print`, `custom-sounds`, `custom-fonts`, `theme-picker`, `mode-switch`, `attachments`, `category-picker`, `message-list`, `thread-list`, `composer`, `onboarding`, `preferences`, `contacts`, `main-calendar` | UI, OS integration, window state | various (onboarding 6, preferences 14 Node files) | client | — | out of this note's scope except where listed above |

## 6. Duplicated logic

Same thing computed in TS and Rust, or in desktop and web (and CLI/MCP):

1. Phishing check: `plugins/phishing-detection/lib/main.tsx` (tld) vs `apps/engine/src/warnings.rs` (attached in `read_api.rs` to `threads.get`/`messages.get` only). Desktop ignores `warnings`, web receives and drops them.
2. Send checks: `DraftEditingSession.validateDraftForSending`/`validateDraftRecipients` (`src/data/drafts/editing-session.ts`) + templates `warningsForSending` vs `crates/tern-compose/src/validate.rs` (`draft.validate`); web's own "Send without a subject?" in `features/composer/useComposerSession.ts`. Neither client calls `draft.validate`.
3. Templates: desktop `plugins/composer-templates/lib/{store,file}.ts` (fs) vs `tern_compose::templates` + `templates_api.rs` vs web localStorage `tern.composer.templates` (a third, disjoint store).
4. Signatures: `SignatureStore.signatureForEmail` (desktop) vs web `features/settings` `signatureForEmail` + `features/composer/signature.ts` `withSignature` vs `tern_compose::settings`/`signature`. Web inserts its own block (`data-tern-signature`) instead of the engine's `<signature>`.
5. Draft building: desktop `src/data/drafts/factory.ts` (570, fallback) and web `features/composer/draft.ts` (`participantsForReply`, `replyAttribution`, `quoteForReply`, `quoteForForward`, `subjectWithPrefix`) vs `tern_compose::{draft,participants,quote}` (`draft.create`). Web never calls `draft.create`/`draft.send`; it queues SyncbackDraftTask/SendDraftTask JSON (`apps/web/src/engine/tasks.ts`).
6. Tracking pixels: `remove-tracking-pixels` draft rules vs `pixels.rs` (same rules via `desktop_data.json`; desktop caps `.+` at 256, Rust doesn't).
7. Snooze: desktop `thread-snooze/lib/utils.ts` `moveThreads` vs `tern_tasks::mail_actions::snooze_tasks`; snooze presets in `thread-snooze/lib/popover.tsx`, web `features/mailbox/snooze.ts` and CLI `parse_until` (three date vocabularies; presets can stay client, parsing could be one).
8. Expiration fallbacks: `send-later/main.ts`, `thread-snooze/store.ts`, `send-reminders/store.ts` and web `features/composer/send-later-runner.ts` all re-implement the executor for engines without `expiration-executor`.
9. Undo send: desktop writes `send-later {isUndoSend}` itself on the fallback path; web `useComposerSession.ts` either writes the same metadata (`sendByEngine`) or runs a tab timer; engine has `draft.send {delayMs}`.
10. Send actions: `ENGINE_SEND_ACTION_KEYS` (`send-actions.ts`) vs `SEND_ACTIONS` (`expiration_executor.rs`) vs `DraftSendParams.after`.
11. Tracking/activity events: `plugins/activity/lib/events.ts` vs `apps/cli/src/followup.rs` vs `apps/mcp/src/tools/followup.rs` (all flatten `open-tracking`/`link-tracking` metadata client-side; metadata shapes also in `crates/tern-client/src/metadata.rs`).
12. Triage chip / answered: `plugins/ai/lib/triage.ts` vs `apps/web/src/features/ai/triage.ts` vs tern-store `triaged_threads`.
13. AI client plumbing: `plugins/ai/lib/client.ts` vs `apps/web/src/features/ai/api.ts`; summary cache `thread-summary.tsx` vs web `summary-memory.ts`; `needs-reply.ts` in both.
14. Capabilities: `src/data/stores/capability.ts` (395) vs `apps/web/src/features/settings/capabilities.ts`; integration provider tables `src/engine/client/integrations.ts` `PROVIDERS` vs web `features/settings/integrations/providers.ts` vs `crates/tern-integrations`.
15. Unsubscribe header parsing: `list-unsubscribe/service.ts` vs `tern_net::unsubscribe`; web checks `hListUnsub` itself.
16. RSVP target: `events/rsvp-target.ts` vs `pim_api.rs::stored_copy`; ICS parsing in `events/header.tsx` (ical.js), web `features/calendar/ics.ts`, and tern-pim/tern-dav.
17. Search grammar: `src/mail/search/parser.ts` vs the engine's `search-grammar`.
18. mbox framing: `src/mail/mbox.ts` + `mbox-export-runner.ts` vs `apps/cli/src/mbox.rs`.
19. Address equivalence / non-human patterns: `Utils.toEquivalentEmailForm`, `likelyNonHumanEmail` vs `tern_compose::{contacts,data}`; web `features/composer/recipients.ts` has a third address parser.

## 7. Hook points an engine extension API needs

Today the only engine-side extension point is the out-of-process service plugin
(docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md "Service Plugins", `examples/service-plugin`): a program
on the socket that reads `metadata-expiration` deltas for its id, `mail.new`, `metadata.list`, and
queues tasks. Everything is after-the-fact and asynchronous; nothing can veto, transform or annotate.

| Hook | Who needs it (bundled / third-party) | Today |
|---|---|---|
| `draft.prepare` (new/reply/forward built) | signature, pixels (built in); plaintext-signatures | `prepareNewDraft`, desktop only; engine built-ins hard-coded in `tern_compose::draft::prepare_new_draft` |
| `draft.validate` (warnings/errors before send) | templates (built in); PGP forks | `warningsForSending`, desktop only |
| `draft.transform` (per-recipient bodies before send) | open/link tracking (built in, `tracking.rs`); keybase/pgp/openpgp | `applyTransformsForSending`/`needsPerRecipientBodies` desktop only; `clientFinalize` exists only because of this |
| `send.action` (custom after-send action) | send-and-archive (built in via `after`) | `sendActions`, desktop only; `after` is a closed list |
| `task.before` / `task.after` (task lifecycle, incl. `onSendSuccess`) | outgoing-undo (patches `SyncbackMetadataTask.prototype`), mute, spoof-guard | none; services see Task deltas after the fact |
| `message.received` (arrival, `rulesReady`) | rules, triage (built in), invitation calendar refresh (desktop today), package-tracker, mute | `mail.new` notification (async, no ordering vs rules/notifications) |
| `message.annotate` (fields on Message JSON for every client: `warnings`, unsubscribe, remote-images policy, invitation summary, extension badges) | phishing (built in), spoof-guard, package-tracker, keybase | `warnings` only, fixed set, read API only |
| `message.render` (body transform served to clients) | remove-tracking-pixels; PGP decrypt | `formatMessageBody`, desktop only |
| `metadata.expired` / `schedule` (periodic, due work) | send-later, snooze, reminders (built in); sample service | `metadata-expiration` deltas re-sent each scan; no timer API |
| `settings` (namespaced, schema'd, relayed policy) | every plugin with account-level prefs | `SETTINGS_KEYS` is a closed list and `setting_error` refuses any other key, so `plugins.<id>.*` can't live in the engine |
| `rpc` (extension methods callable by any client, routed by the engine to the extension) | any plugin with a UI half on web/mobile | none: a service is just another socket client; other clients can't call it |
| `capabilities` (extension announces features so clients gate UI) | all | `hello.features` is engine-only |

## 8. docs/desktop/plugin-api-v2.md: what holds, changes, splits

Holds (UI-extension API, desktop client): sec. 2 package format and loader (manifest, `main.js`,
`styles.css`, disposal), sec. 3.1 Plugin base class, sec. 3.2 slot methods (all 20 are UI), 3.3
`registerSheet`/`addSettingsTab`/`addCommand`, sec. 4 `tern/ui`, sec. 6 permission prompt for
local-only methods, sec. 8 themes, 9 compilation, 10 types package, 11 naming, 12 versioning.

Changes:
- The settled input "no plugin code runs inside the engine, background logic is a service plugin"
  stays true for process isolation, but services become **engine extensions** with registered
  hooks (sec. 7 above). The plan should say they stay out of process (socket, language-agnostic,
  timeouts on synchronous hooks) rather than in-engine.
- Manifest: `service` becomes an `engine` block (`bin`, `hooks`, `settings` schema, `methods`,
  `features`); the UI half keeps `main`/`windowTypes`. One manifest, two halves.
- 3.3 `addComposerExtension`: `prepareNewDraft`, `warningsForSending`, `applyTransformsForSending`,
  `needsPerRecipientBodies`, `onSendSuccess` move to engine hooks (`draft.prepare`, `draft.validate`,
  `draft.transform`, `task.after`). The doc already notes they "don't run for CLI, MCP or
  engine-executed sends"; that is the reason to move them, and `clientFinalize` goes with them.
- 3.3 `addSendAction`: split. UI registers the menu entry; `perform` is an engine `send.action`.
- 3.3 `addMessageViewExtension`: split. `formatBody` -> engine `message.render`; `allowsRemoteImages`
  -> engine policy (setting + Message field); `bodyRendered` (DOM decoration: link dots,
  translation swap) and `filterFiles` stay UI.
- 3.4 events: `onDatabaseChange` -> live query subscription on the shared TS client (DatabaseStore
  goes away); `onMetadataExpired` leaves the UI API entirely (due work never runs in a window).
  `this.app.database` -> the shared client's `query`.
- 4 `tern` models: classes come from the shared `packages/` client (generated from `crates/tern-wire`
  / tern-ipc), not the desktop's `src/data/models`.
- 7 storage: `this.settings` splits into client prefs (config.json, as now) and account-level
  plugin settings in the engine store (`plugins.<id>.*`, needs the engine change above);
  `this.metadata` holds and should gain typed helpers for built-ins (`draft.schedule`,
  `threads.remind`, tracking toggles) instead of raw JSON.
- 13 migration table: send-later, thread-snooze, send-reminders drop `onMetadataExpired`/engine
  deep imports (the executor owns it; snooze calls `threads.snooze`); phishing-detection becomes
  "render `message.warnings`" with no `MessageStore`; composer-templates uses `templates.*` and
  drops `addComposerExtension`; ai's needs-reply uses a live `ai.triage.list`; activity reads
  `tracking.events`/`tracking.stats`.

Splits: bundled plugins become UI-only, because their engine halves are already built in
(executor, `threads.snooze`, `tracking`, `templates`, `message-warnings`, `unsubscribe`,
`ai-triage`). Third-party plugins (PGP x3, spoof-guard, package-tracker, mute, outgoing-undo,
plaintext-signatures) are the ones that need the engine-extension hooks. UI extensions are per
client: v2 is the desktop's; the web needs its own slot API later (not in this doc).
