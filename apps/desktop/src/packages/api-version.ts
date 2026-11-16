/**
 * The version of the plugin API, matched as a semver range against a plugin's
 * `engines.airpost` (or `engines.mailspring`). It moves with the API, not the app: a minor bump
 * for additions, a major one only when a deprecated export is removed.
 *
 * 1.27.0: `Engine` (src/engine/client/api.ts) and `airpost.engineMethods`;
 * MessageViewExtension.renderedMessageBodyIntoDocument gets `body` (MessageBody) instead of the
 * document, which is in a cross-origin frame now.
 *
 * 1.28.0: `AI` (src/engine/client/ai.ts), the AI settings, and the `ai-summary` and `ai-reply` capabilities.
 *
 * 1.29.0: the `ai-triage` capability and the `ai.triage.*` types.
 *
 * 1.30.0: the `rules` feature and the `rules.*` types; the engine runs the mail rules, and
 * `MailRulesProcessor` is deprecated (its `processMessages` does nothing).
 *
 * 1.31.0: `ComponentRegistry.listenShared`; `MessageStore.items()` carries `body` for expanded
 * messages only (undefined on collapsed ones, TD40), and Thread deltas no longer re-read the
 * open thread's messages.
 *
 * 1.32.0: the `calendar-editing` and `contact-editing` features; the engine expands, builds and
 * edits events and vCards. `ICSEventHelpers` is deprecated (every function throws), so are
 * the `CalendarUtils` VTIMEZONE helpers `resolveIanaZone` and `createVTIMEZONEString` (they
 * throw), all removed in 2.0.0; SyncbackEventTask gains `restore` and `tz`.
 *
 * 1.33.0: the Preferences building blocks in `mailspring-component-kit` (`SettingsPage`,
 * `SettingsSection`, `SettingsRow`, `ConfigSwitchRow`, `ProviderCard` and the rest of
 * src/components/settings.tsx); `PreferencesUIStore.TabItem` takes `group` and `keywords` for
 * the sidebar. Tabs that name no group are listed under "From plugins".
 *
 * 1.34.0: `TestUtils`, `RegExpUtils.signatureRegex` and `RegExpUtils.commandRegex`; the old
 * names `MailspringTestUtils`, `mailspringSignatureRegex` and `mailspringCommandRegex` are
 * deprecated aliases, removed in 2.0.0.
 *
 * 1.35.0: Airpost names for what plugins see under Mailspring's: the `airpost-asset://` asset scheme,
 * `engines.airpost`, `AppEnv.engineBridge`, `EngineProcess` and `EngineProcessExit`. The old names
 * (`mailspring://`, `engines.mailspring`, `AppEnv.mailsyncBridge`, `MailsyncProcess`) stay as
 * aliases. Windows have no `<webview>` tag, and the `Webview` component is deprecated (it renders
 * nothing), removed in 2.0.0.
 *
 * 1.36.0: `Actions.composeReply` with `prefer-existing` reopens the thread's reply draft through
 * its editing session; `DraftFactory.replyDraftsInThread`, `DraftFactory.participantsForReplyUpdate`,
 * `MessageStore.focusDraft` and `MessageStore.takeDraftToFocus`.
 *
 * 1.37.0: the engine builds every new, reply, forward and mailto draft; `DraftFactory`'s
 * `prepareBodyForQuoting` and `createDraftForResurfacing` and `EventRSVPTask.forReplying` are
 * deprecated (they throw), removed in 2.0.0.
 *
 * 1.38.0: the engine checks drafts before a send (`draft.validate`): the composer's
 * `isValidDraft` resolves a Promise, `DraftEditingSession.validateForSending()` replaces
 * `validateDraftForSending` and `validateDraftRecipients` (deprecated, they throw), and
 * `structuredSearch()` takes the search text and runs in the engine.
 *
 * 1.39.0: the engine's intents and undo log. `IntentTask` (a `threads.*` or `categories.*`
 * intent queued with `Actions.queueTask`); TaskFactory and
 * `MailboxPerspective.tasksForRemovingItems` return IntentTasks, one for threads of any
 * accounts, and `MailboxPerspective.removalKind`; `UndoRedoStore` follows the engine's log
 * (`UndoBlock.entry`, `UndoRedoStore.describe`) and hands tasks queued with `canBeUndone` to
 * `undo.record`.
 */
export const PLUGIN_API_VERSION = '1.39.0';
