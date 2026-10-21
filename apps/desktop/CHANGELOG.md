# Changelog

The plugin API log of the Tern desktop app. Mailspring's release notes up to 1.26.0 are in
this file's git history.

## Unreleased (Tern)

No plugin API change:

- The built-in features moved from `<resourcePath>/features/<dir>/lib/` to `<resourcePath>/src/features/<dir>/` and have no `package.json` any more. `AppEnv.packages` lists them under the same names, they activate in the same order, and `mailspring://<dir>/…` asset URLs are unchanged. A plugin that requires a feature's files by path has to use the new path.
- `src/` is grouped by domain (`data/`, `engine/`, `mail/`, `packages/`, `window/`, `config/`, `main/`, `components/`, `utils/`) and file names drop the prefix or suffix their dir already states (`src/flux/stores/draft-store.ts` is `src/data/drafts/store.ts`). `mailspring-exports`, `mailspring-component-kit`, `mailspring-store` and `mailspring-observables` resolve as before; a plugin that requires files under `src/` by path has to use the new paths.
- `BaseStore` is defined in `src/data/stores/base.ts` and the Rx helpers in `src/data/query/observables.ts`; `mailspring-store` and `mailspring-observables` re-export them and resolve as before.
- A package's tests can be `tests/*.test.*` files as well as `specs/*-spec.*`, and `--test-file-pattern` is an alias of `--spec-file-pattern`. The pattern matches the test file's path relative to the core or package root.

Plugin API 1.35.0:

- Package assets are served under `tern-asset://<package>/<path>`. `mailspring://` is the same handler and keeps working; the content security policies allow both.
- `engines.tern` in `package.json` is read before `engines.mailspring`; either one is enough.
- `AppEnv.engineBridge` is the sync engine bridge; `AppEnv.mailsyncBridge` returns the same object.
- `EngineProcess` and `EngineProcessExit` are the names of `MailsyncProcess` and `MailsyncProcessExit`, which stay as aliases.
- The Developer menu's command is `window:open-engine-logs` (it was `window:open-mailsync-logs`).
- `MAILSPRING_FORCE_BACKGROUND_PORTAL` is no longer read; use `TERN_FORCE_BACKGROUND_PORTAL`.
- `RegExpUtils.commandRegex()` matches `tern:` links only.

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
