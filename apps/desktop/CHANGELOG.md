# Changelog

The plugin API log of the Tern desktop app. Mailspring's release notes up to 1.26.0 are in
this file's git history.

## Unreleased (Tern)

No plugin API change:

- The built-in features moved from `<resourcePath>/features/<dir>/lib/` to `<resourcePath>/src/features/<dir>/` and have no `package.json` any more. `AppEnv.packages` lists them under the same names, they activate in the same order, and `mailspring://<dir>/…` asset URLs are unchanged. A plugin that requires a feature's files by path has to use the new path.
- `src/` is grouped by domain (`data/`, `engine/`, `mail/`, `packages/`, `window/`, `config/`, `main/`, `components/`, `utils/`) and file names drop the prefix or suffix their dir already states (`src/flux/stores/draft-store.ts` is `src/data/drafts/store.ts`). `mailspring-exports`, `mailspring-component-kit`, `mailspring-store` and `mailspring-observables` resolve as before; a plugin that requires files under `src/` by path has to use the new paths.
- `BaseStore` is defined in `src/data/stores/base.ts` and the Rx helpers in `src/data/query/observables.ts`; `mailspring-store` and `mailspring-observables` re-export them and resolve as before.
- A package's tests can be `tests/*.test.*` files as well as `specs/*-spec.*`, and `--test-file-pattern` is an alias of `--spec-file-pattern`. The pattern matches the test file's path relative to the core or package root.
- The app's stylesheets are plain CSS: `static/styles`, the built-in features, the bundled plugins and themes. LESS compiles only for third-party packages (LESS-variables themes and v1 plugins with `styles/*.less`), as v1 compatibility that is removed in 2.0.0. That LESS keeps `@import 'ui-variables'`, `'ui-mixins'`, `'base/…'` and `'mixins/…'` (now in `static/styles/compat/less`) with the variables and mixins it had, but can no longer `@import` the app's own stylesheets (`buttons`, `type`, `workspace`, …).
- A package's `styles/` without an `index.*` loads its stylesheets sorted by name, `foo.less` sorting as `foo.css`; they loaded in directory order, which is the same on macOS.
- With a LESS-variables theme, the app's styles get the theme's values through the tokens and the `static/styles/variables.css` custom properties. A variable that only aliases a token without being its source (`@btn-default-bg-color`, `@dropdown-default-*`, `@border-color-primary`, `@accent-primary-dark`, …) no longer restyles the app on its own; the theme's own LESS still sees it (docs/desktop/theme-tokens.md).
- The theme previews in Preferences > Appearance and the theme picker read the `--theme-preview-*` properties of a theme's `styles/theme-colors.css`. A v1 theme without one previews its LESS variables (`ui-variables.less`, `theme-colors.less`) as before.
- With Save drafts to server on (the account setting `save_drafts_to_server`, on when absent), another device's revision of a draft reaches an open `DraftEditingSession` as a persist of the draft, sometimes under a new id with the same `headerMessageId`. The session takes it in the fields not edited since the last save and not carried by a save in flight, and ignores a non-draft message with the draft's `headerMessageId`; `session.draft().id` follows the new id.
- The engine types `mailspring-exports` re-exports (`EngineCapabilities`, `EngineCapability`, the `Ai*` and `Rules*` results, `CalendarConflict`) are generated from crates/tern-ipc (`@tern/client`) with the same fields.

Plugin API 1.39.0:

- `TaskFactory`'s builders return `IntentTask`s: one task (in an array where they returned one per account) for threads of any accounts, which EngineBridge runs as the engine's intent (`threads.archive`, `threads.trash`, `threads.spam`, `threads.notSpam`, `threads.markRead`, `threads.star`) when it is queued with `Actions.queueTask(s)`. The engine decides what the action means on each account; a plugin that read `folder`, `labelsToAdd` or `sourceFolderIds` off the returned tasks gets `intent` and `params` instead. `taskForSettingUnread({canBeUndone: false})` still returns a `ChangeUnreadTask`. Waiting on an `IntentTask` (`TaskQueue.waitForPerformRemote`) never resolves; the tasks the engine queues arrive as Task deltas.
- `IntentTask` and `IntentTask.forThreads(intent, threads, params, source, description)` queue any `threads.*` or `categories.*` intent.
- `MailboxPerspective.tasksForRemovingItems` returns a `threads.removeFromView` IntentTask, and `actionsForReceivingThreads` `threads.move`, `threads.label`, `threads.star` and `threads.markRead` ones. `MailboxPerspective.removalKind(threads)` says what removing does (`'trash'`, `'archive'`, `'unstar'` or null).
- `UndoRedoStore` follows the engine's undo log: intents, snoozes and sends inside their undo window are entries of their own, tasks queued with `canBeUndone` of a class the engine can undo are handed to `undo.record` once their local phases ran, and Cmd-Z sends the entry's token to `undo.apply`. Only the app's own entries are undone, never the CLI's or an agent's. `UndoBlock.entry` is the entry; `UndoRedoStore.describe(token, text)` sets a toast's text. A plugin's own task class, and an undo-only task without an engine token, are still reversed with `createUndoTasks()`.

Plugin API 1.38.0:

- The engine runs the checks before a send (`draft.validate`). The `isValidDraft` prop of `Composer:ActionButton` components returns a `Promise<boolean>`: await it before sending or scheduling (a plugin that tests the returned value as a boolean always goes ahead). It saves the draft first, and its dialogs no longer block the window.
- `DraftEditingSession.validateForSending()` resolves with `{recipientErrors, recipientWarnings, miscErrors, miscWarnings}`: the engine's findings in the app's language, then the `warningsForSending` hooks' warnings. `validateDraftForSending` and `validateDraftRecipients` are deprecated and removed in 2.0.0; they throw. The engine also warns about template variables left unfilled.
- `ModelQuery.structuredSearch()` takes the search text (`'from:ann in:inbox'`) as well as what `SearchQueryParser.parse` made of it, which now keeps its text (`sourceText`). The engine runs these queries, whatever `core.engine.reads` says; an expression built by hand, without text, throws. The SQL backend for them is gone.
- The composer-templates plugin reads and writes templates through the engine (`templates.list|get|save|delete`) and registers no `ComposerExtension`; `draft.validate` gives its empty-template warning. phishing-detection shows the engine's `warnings` (`messages.get`), and the `tld` package is no longer bundled.
- `Actions.destroyDraft` deletes the draft in the engine (`draft.delete`), which cancels a scheduled send with it and refuses a draft being sent. Opening, saving and previewing an attachment asks the engine for its path (`files.get`), which fetches a file that isn't on disk yet.

Plugin API 1.37.0:

- `DraftFactory.createNewDraft`, `createDraftForReply`, `createDraftForForward` and `createDraftForMailto` always ask the engine (`draft.create`) and reject when it isn't running or refuses; the desktop no longer builds these drafts itself. `DraftFactory.createDraft` (fields only) is unchanged.
- Every `prepareNewDraft` hook runs on a new draft, whoever built it. The built-in signature and tracking-pixel hooks are gone, as the engine adds the default signature and strips tracking pixels from the quoted message.
- `DraftFactory.prepareBodyForQuoting` and `DraftFactory.createDraftForResurfacing` are deprecated and removed in 2.0.0. They throw: the engine quotes the message in the drafts it builds and sends the resurfacing messages of snoozes and reminders.
- `EventRSVPTask.forReplying` is deprecated and removed in 2.0.0. It throws; use `Engine.request('events.rsvp', …)`.
- The app requires an engine with the serve features (accounts, settings, the draft pipeline, the expiration executor, unsubscribe, counts, AI); the fallbacks for an older engine are gone. Snoozed threads, send reminders and scheduled sends are handled by the engine's expiration executor only, except a send-later draft marked `clientFinalize`, which the send-later plugin still sends.

Plugin API 1.36.0:

- `Actions.composeReply` with `behavior: 'prefer-existing'` reopens the thread's unsent reply draft (the newest one answering that message, else the newest reply draft of the thread) instead of saving it as a new draft: the composers keep their editing session, `prepareNewDraft` hooks don't run on it again, and it is focused in the open thread (or popped out with `popout`). Recipients change only when switching between Reply and Reply All on the message the draft answers. The built-in Reply, Reply All, reply area and thread-list menu items use `prefer-existing`; `prefer-existing-if-pristine` keeps its meaning.
- `DraftFactory.replyDraftsInThread(message)` and `DraftFactory.participantsForReplyUpdate(draft, {type, message})`; `updateDraftForReply` uses the latter.
- `MessageStore.focusDraft(headerMessageId)` expands a draft of the open thread for the message list to focus; `MessageStore.takeDraftToFocus()` reads and clears the request.

Plugin API 1.35.0:

- Package assets are served under `tern-asset://<package>/<path>`. `mailspring://` is the same handler and keeps working; the content security policies allow both.
- `engines.tern` in `package.json` is read before `engines.mailspring`; either one is enough.
- `AppEnv.engineBridge` is the sync engine bridge; `AppEnv.mailsyncBridge` returns the same object.
- `EngineProcess` and `EngineProcessExit` are the names of `MailsyncProcess` and `MailsyncProcessExit`, which stay as aliases.
- The Developer menu's command is `window:open-engine-logs` (it was `window:open-mailsync-logs`).
- `MAILSPRING_FORCE_BACKGROUND_PORTAL` is no longer read; use `TERN_FORCE_BACKGROUND_PORTAL`.
- `RegExpUtils.commandRegex()` matches `tern:` links only.
- Windows are created with `webviewTag: false`, so a `<webview>` stays an inert element. The `Webview` component is deprecated (a package that reads it is warned once) and removed in 2.0.0; open pages in the browser with `shell.openExternal`.
- Package assets under `<config-dir>/dev/packages` are served in dev mode only, as PackageManager loads packages from there only in dev mode, and nothing from the profile's `packages` in safe mode.

Plugin API 1.34.0:

- `TestUtils` replaces `MailspringTestUtils`, and `RegExpUtils.signatureRegex()` and `RegExpUtils.commandRegex()` replace `mailspringSignatureRegex()` and `mailspringCommandRegex()`. The old names keep working, are deprecated (a package that reads them is warned once) and are removed in 2.0.0.
- The Linux background portal is forced with `TERN_FORCE_BACKGROUND_PORTAL=true`; `MAILSPRING_FORCE_BACKGROUND_PORTAL` is read when it is unset.

Plugin API 1.32.0:

- The engine expands, creates and edits calendar events and contacts (`events.expand`, `events.create`, `events.update`, `events.rsvp`, `contacts.get`, `contacts.create`, `contacts.update`, `contacts.import`, `contacts.export`; features `calendar-editing` and `contact-editing`). Call them with `Engine.request`.
- `ICSEventHelpers` is deprecated and removed in 2.0.0: every function throws. Its types `CreateEventOptions`, `Occurrence`, `UpdateTimesOptions`, `RecurrenceExceptionResult` and `RecurrenceInfo` are gone; `CalendarConflict` is now the engine's conflict (`eventId`, `calendarId`, `icsuid`, `title`, `start`, `end`).
- `CalendarUtils.resolveIanaZone` and `CalendarUtils.createVTIMEZONEString` are deprecated and removed in 2.0.0: they throw, and a package that reads them is warned once. The engine writes the VTIMEZONEs of the events it builds. `CalendarUtils.registerTimezones` registers only the file's own VTIMEZONEs.
- `SyncbackEventTask` has `restore` and `tz`: an undo or redo re-sends its snapshot, which the engine applies over the copy stored by then.

Plugin API 1.31.0:

- `ComponentRegistry.listenShared(callback)`: one registry listener shared by every caller, for components rendered many times (thread rows).
- `MessageStore.items()` carries `body` for expanded messages only; it is `undefined` on collapsed ones until they are expanded. Read the body with `DatabaseStore.find(Message, id).include(Message.attributes.body)` when you need it. Thread changes no longer re-read the open thread's messages.

Plugin API 1.27.0:

- New `Engine` export (`request`, `subscribe`, `features`, `has`, `capabilities`, `connected`, `onDidChange`) and the `tern.engineMethods` manifest field. See docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md.
- `tern.service` manifest block: a plugin's background program, installed from Preferences > Plugins (`tern service install`) as a launchd or systemd user unit that runs after the engine, also while Tern is closed. See docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md, Service Plugins.
- Preferences > Plugins lists third-party plugins with where they were installed from and a hash of what was installed.
- Breaking for plugins that read `document`: message bodies render in a sandboxed cross-origin frame, so `MessageViewExtension.renderedMessageBodyIntoDocument` gets `body` (a `MessageBody`) instead, and reading `document` throws. Plugins with `engines.mailspring` `^1.26.0` still load.
