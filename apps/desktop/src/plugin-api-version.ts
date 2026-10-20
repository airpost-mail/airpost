/**
 * The version of the plugin API, matched as a semver range against a plugin's
 * `engines.mailspring`. It moves with the API, not the app: a minor bump for additions, a
 * major one only when a deprecated export is removed (docs/desktop-client-plan.md section 6).
 *
 * 1.27.0: `Engine` (src/engine-api.ts) and `tern.engineMethods`;
 * MessageViewExtension.renderedMessageBodyIntoDocument gets `body` (MessageBody) instead of the
 * document, which is in a cross-origin frame now.
 *
 * 1.28.0: `AI` (src/ai.ts), the AI settings, and the `ai-summary` and `ai-reply` capabilities.
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
 */
export const PLUGIN_API_VERSION = '1.33.0';
