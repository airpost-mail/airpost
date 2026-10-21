# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
# Install dependencies; postinstall rebuilds the native runtime deps for Electron.
# Use `npm ci` (or `npm install`), never a bare `npm rebuild`: that puts better-sqlite3 back
# on Node's ABI.
npm ci

# Run the app in development mode (uses --dev flag, data stored in Tern-dev folder)
npm start

# Run linting (prettier runs as an eslint rule); lint:check reports without fixing. Both pass
# --rulesdir scripts/eslint-rules for raw-controls, which warns on raw <button>, <select>,
# checkbox/radio inputs and .btn classes outside src/components/kit; run eslint by hand with it too.
npm run lint
npm run lint:check

# TypeScript type checking, then the plugin API .d.ts check (scripts/check-plugin-api-dts.js)
npm run typecheck

# Check that better-sqlite3 is built for Electron (postinstall runs it too)
npm run check:native

# Run all tests, or only the test files whose path matches a regex (--test-file-pattern works too)
npm test
npm test -- --spec-file-pattern='calendar'

# Run window-specific tests
npm run test-window

# TypeScript type checking in watch mode
npm run tsc-watch

# Build for production
npm run build
```

## Architecture Overview

Tern's desktop app is an Electron email client written in TypeScript with React 17, forked from Mailspring. TypeScript and LESS are compiled at runtime; there is no bundler. Built-in features are app code in `src/features/` that loads through the same package manager as plugins, and optional features are bundled plugins. Plugin API names keep their Mailspring spelling (`mailspring-exports`, `engines.mailspring`, `window.$m`, `AppEnv`) because third-party plugins depend on them.

### Key Directories

- **`src/`** - Core application source code, one directory per domain
  - `main/` - Main process code, grouped into `windows/`, `frame/` (the message frame scheme), `protocols/` (the `mailspring:` and `tern-file:` schemes), `ipc/`, `platform/` (menu, tray, taskbar, OS hooks), `profile/` (profile marker, reset, Mailspring import, KeyManager) and `autoupdate/`; `main.js`, `application.ts` and `permission-policy.ts` sit at its root
  - `intl/` - Localization (`intl/localize.ts`: `localized`, the language list, RTL) and date/time formats in the app's locale and clock (`intl/date-format.ts`); both processes load it
  - `platform/` - The Electron/OS runtime both processes load: the TypeScript compile caches, the error logger, the devtools gate, the `shell` patch, default mail client registration and Linux desktop facts (XDG paths, the .desktop file name, Wayland)
  - `config/` - The config store, keys, schema and IPC channels; `migrator.ts` and `persistence-manager.ts` are main-process only
  - `window/` - Window startup and window-level managers (AppEnv, the bootstraps, keymaps, menus, commands, spellchecker, the KeyManager facade), with OS integration in `window/platform/` (native notifications, do-not-disturb, Linux themes, start at login)
  - `engine/` - Everything that talks to the sync engine. The root files load in both processes: the protocol types (`protocol.ts`), the engine binary and error mapping (`process.ts`) and the supervisor's IPC channels (`channels.ts`), plus the unsnooze/reminder texts the settings mirror sends (`resurfacing.ts`). `engine/supervisor/` is main-process only: it starts and supervises the engine process (host, supervisor, systemd/launchd unit, settings). `engine/client/` is the windows' side: the socket connection, `Engine` (`engine/client/api.ts`), the bridge, engine events, accounts, drafts, PIM, integrations, AI
  - `data/` - The local data layer: actions, models, attributes, queries (`data/query/`), stores, the database (`data/database/`), drafts (`data/drafts/`), tasks, calendar ICS parsing and all-day dates (`data/calendar/`), mailbox perspectives, mail rule templates
  - `mail/` - Message content: body processing (`mail/body/`), the sandboxed message frame and its protocol (`mail/frame/`), search queries (`mail/search/`), attachment previews (`mail/quick-preview/`), the content security policies, the `tern-file:` scheme, attachment safety checks, IMAP folder-name encoding, .eml and .mbox
  - `components/` - Reusable React UI components, including the UI kit (`components/kit/`), the sheets, lists, injected components, the higher-order components (`components/decorators/`) and the thread drag image and drag types
  - `packages/` - The plugin system: package discovery and loading, themes and styles, the component/extension/service/sound registries, the extension base classes and the plugin API version; `packages/api/` holds only the plugin API barrels (`mailspring-exports`, `mailspring-component-kit`, `mailspring-store`, `mailspring-observables`), which are on the module path as bare names
  - `utils/` - Generic helpers: DOM, date, regexp, backoff schedulers, the shared store listener, file: URL checks and atomic file writes
  - `features/` - The built-in features that always load (composer, message-list, thread-list, preferences, main-calendar, etc.), one directory each, listed in `features/index.ts`

- **`plugins/`** - Bundled optional packages (`isOptional: true`, can be turned off via `core.disabledPackages`), e.g. activity, send-later, translation
- **`themes/`** - The `ui-*` themes
- **`vendor/`** - Quick-preview libraries (pdf.js, Prism, SheetJS, DOMPurify) loaded by `src/mail/quick-preview/renderer.html`

Plugins and themes import core code as `../../../src/...`. `src/packages/bundled-packages.ts` lists the plugin and theme roots and `src/features` for the package manager, the `mailspring:` protocol handler, ThemeManager and the test loader.

> **IMPORTANT:** Application source code lives in `src/` (including `src/features/`) and `plugins/`. When searching for usages of a module, symbol, or pattern, always search all of them. Searching only `src/` will miss a large portion of the codebase and lead to incomplete changes.

- **`tests/`** - Jasmine tests, `tests/<area>/<subject>.test.ts(x)`. Areas are coarse and don't mirror `src/`: one per `src/` domain (`main/`, `intl/`, `platform/`, `config/`, `window/`, `engine/`, `data/`, `mail/`, `components/`, `packages/`, `utils/`) and, for the built-in features, product areas (`calendar/`, `composer/`, `mailbox/`, `preferences/`, `onboarding/`, `notifications/`). Tests that load code from several places go in `tests/integration/`; a test for one bundled plugin or theme goes in that package's `tests/`. `tests/support/` is never loaded as tests: the runner and loader (`runner/`), `fixtures/`, the jasmine and spec-env globals (`types/`), `test-utils.ts` (the plugin API's `TestUtils`), the kit helpers and the engine fakes (`fake-serve-engine.ts`, `fake-pim-engine.ts`, `mock-engine.ts`).

Main-process code is `src/main/`, `src/engine/supervisor/`, `src/config/migrator.ts` and `src/config/persistence-manager.ts`; code outside them runs in the windows and must not import them. The main process loads only those, `src/intl/`, `src/platform/`, `src/config/`, the `src/engine/` root files and single files in `packages/` (`bundled-packages.ts`), `mail/` (`attachment-safety.ts`, `content-security-policy.ts`, `files-scheme.ts`, `frame/protocol.ts`), `window/` (`external-fetch.ts`, for its types) and `utils/` (`file-url.ts`, `fs.ts`), all listed in the main-process override in `.eslintrc`, so none of these may import window code (including `src/engine/client/`), except `data/models/utils` and the two registries it loads (`database-object-registry`, `serializable-registry`), which main uses. `import/no-restricted-paths` in `.eslintrc` checks both directions and fails lint.

### Dependencies and packaging

There is one `package.json` and one `node_modules`. Runtime code goes in `dependencies` (or `optionalDependencies`); tests and tooling go in `devDependencies`, which the packager prunes. `import/no-extraneous-dependencies` (with `packageDir: "."`, so run eslint from this directory) fails a runtime import of a devDependency in `src/`, `plugins/` or `themes/`; `tests/` and package `tests/` dirs may use them. Only the top-level entries listed in `shippedTopLevel` in `build/build.js` are packaged, so a new top-level runtime dir has to be added there or it silently won't ship. `tests/` and package `tests/` dirs never ship.

### Core Modules

**Global exports for plugins:**
- `mailspring-exports` - Core APIs: Actions, Stores, Models, Tasks, Utils, database access
- `mailspring-component-kit` - Reusable UI components

**Data layer:**
- **Models** (`data/models/`) - Data models: Message, Thread, Contact, Account, Folder, Label, etc.
- **Stores** (`data/stores/`, `data/database/`, `data/drafts/`) - Application state: DatabaseStore, DraftStore, AccountStore, etc.
- **Tasks** (`data/tasks/`) - Async operations: SendDraftTask, ChangeFolderTask, etc.
- **Actions** (`data/actions.ts`) - Application-wide action dispatcher

### Plugin Structure

Each package in `plugins/` and `themes/` has:
- `package.json` - Metadata with `windowTypes` specifying where plugin loads
- `lib/main.ts` - Entry point with `activate()` and `deactivate()` lifecycle hooks
- `lib/` - Plugin source code
- `styles/` - LESS stylesheets
- `keymaps/` - Keyboard shortcut definitions

A built-in feature in `src/features/<dir>/` has no `package.json`: its entry in `src/features/index.ts` gives its package name (`list-unsubscribe` is `unsubscribe`), `windowTypes`, `syncInit` and a lazy `require` of its `index.ts` (which exports `activate()`/`deactivate()`). Source files sit in the feature folder; `styles/`, `keymaps/` and asset folders are subfolders loaded as for a package. `PackageManager` wraps each one in a `BuiltinFeature` and sorts features and bundled plugins together by directory name, so the activation order, and with it the order toolbar buttons, message headers and composer extensions register in, stays what it was (`tests/packages/builtin-features.test.ts` pins it). The directory name is the `mailspring://<dir>/` asset host, so don't rename one.

## Core Data Flow: Sync Engine, Tasks, and Observable Database

**Important:** The UI is read-only with respect to the database. All database modifications happen in the Rust sync engine (`apps/engine`, bundled as `mailsync-rs` next to `package.json`). The Electron app requests changes via Tasks, and the sync engine streams entity changes back to create a real-time UI.

### Sync Engine Communication

One engine process (`mailsync-rs --mode serve`) runs every account, started by the main process at launch, with zero accounts too:

1. **Main process → engine, stdin** (`engine/supervisor/supervisor.ts`, `engine/supervisor/host.ts`, `engine/process.ts` `EngineServeProcess`): JSON-RPC calls that carry credentials or manage the engine: `account.add`/`remove`/`test`/`reset`, `engine.resetDatabase`/`setLogLevel`/`shutdown`. stdout carries their responses and rotated secrets only.
2. **Each window ↔ engine, Unix socket** (`engine/client/connection.ts`): JSON-RPC 2.0 over newline-delimited JSON at `<configDir>/run/engine.sock`: `hello`, `subscribe`, `task.queue`, reads, and the delta notifications.

```
┌──────────────┐   stdin/stdout (JSON-RPC)   ┌──────────────────┐
│ Main process │ ◀─────────────────────────▶ │                  │
└──────────────┘                             │  mailsync-rs     │
┌──────────────┐   socket (JSON-RPC, deltas) │  --mode serve    │
│ Each window  │ ◀─────────────────────────▶ │                  │
└──────────────┘                             └──────────────────┘
```

Windows reach the stdin-only calls over IPC (`engine/channels.ts`). The `MailsyncBridge` in the main window listens to `Actions.queueTask` and sends tasks on its connection; every window applies the deltas it reads.

### Task System (`data/tasks/`)

Tasks represent operations the user wants to perform (send email, star thread, move to folder). They are **persisted models** stored in the database.

**Task Lifecycle:**
1. UI calls `Actions.queueTask(new SomeTask({...}))`
2. `MailsyncBridge._onQueueTask()` validates and sends `task.queue` on the main window's socket connection
3. Sync engine executes the task (local changes + remote API calls)
4. Sync engine persists task status updates and emits deltas
5. Task completion triggers `onSuccess()` or `onError()` callbacks

**Task States** (`data/tasks/task.ts`):
- `local` - Not yet executed
- `remote` - Local phase complete, waiting for remote
- `complete` - Finished successfully
- `cancelled` - Cancelled before completion

**Key Task Classes:**
- `SendDraftTask`, `DestroyDraftTask` - Email composition
- `ChangeLabelsTask`, `ChangeFolderTask` - Organization
- `ChangeStarredTask`, `ChangeUnreadTask` - Status flags
- `SyncbackMetadataTask` - Plugin metadata sync
- `SyncbackEventTask` - Calendar event sync

**Undoable Tasks:**

Tasks can support undo/redo by implementing `canBeUndone` and `createUndoTask()`. The `UndoRedoStore` automatically registers tasks with `canBeUndone = true` for undo.

Two patterns exist:
1. **Toggle pattern** (`ChangeStarredTask`): Undo simply flips a boolean flag
2. **Snapshot pattern** (`SyncbackMetadataTask`, `SyncbackEventTask`): Store original state in `undoData`, swap on undo

```typescript
// Snapshot pattern example
const undoData = { ics: event.ics, recurrenceStart: event.recurrenceStart };
event.ics = newIcs;  // Modify after capturing
Actions.queueTask(SyncbackEventTask.forUpdating({ event, undoData, description: 'Edit event' }));
```

See `docs/desktop/undo-redo-task-pattern.md` (repo root) for detailed implementation guide.

### Task Queue (`data/stores/task-queue.ts`)

The TaskQueue store observes Task model changes from the database and provides:
- `queue()` - Active tasks
- `completed()` - Finished tasks
- `waitForPerformLocal(task)` - Promise that resolves when task runs locally
- `waitForPerformRemote(task)` - Promise that resolves when task fully completes

### Observable Database Pattern

**Database is read-only in Electron** (`data/database/store.ts`):
- `DatabaseStore.inTransaction()` throws - writes are not allowed
- Uses SQLite in WAL mode via better-sqlite3 for concurrent reads
- The sync engine exclusively handles writes

**Change Records** (`data/database/change-record.ts`):

When the sync engine modifies data, it emits JSON deltas that become `DatabaseChangeRecord` objects:
```typescript
{
  type: 'persist' | 'unpersist',
  objectClass: 'Thread' | 'Message' | ...,
  objects: Model[],
  objectsRawJSON: object[]
}
```

**Reactive Queries** (`data/query/subscription.ts`):

`QuerySubscription` provides live-updating query results:
```typescript
// Subscribe to all unread threads
const subscription = new QuerySubscription(
  DatabaseStore.findAll(Thread).where({ unread: true })
);
subscription.addCallback((threads) => this.setState({ threads }));

// Subscription automatically updates when DatabaseStore triggers
```

**Observable Integration** (`Rx.Observable.fromQuery`):

Wrap queries as RxJS observables for reactive UI updates:
```typescript
Rx.Observable.fromQuery(DatabaseStore.findAll(Thread))
  .subscribe(threads => this.updateUI(threads));
```

**ObservableListDataSource** (`components/list/observable-data-source.ts`):

Adapts QuerySubscription for virtualized list components (MultiselectList), supporting:
- Windowed/paginated data loading
- Selection state management
- Automatic updates from database changes

### Data Flow Summary

```
User Action → Actions.queueTask() → MailsyncBridge → socket (task.queue) → Sync Engine
                                                                          │
                                                                          ▼
UI Updates ← QuerySubscription ← DatabaseStore.trigger() ← socket delta notifications
```

## Comment Style

Write for a technical reader who prefers self-documenting code. Prefer clearer names and
smaller functions over commentary; a comment that restates the code should be deleted.

- State **current behavior and rationale** — why the code is the way it is, and what a
  reader would otherwise get wrong. Never narrate thought process, stream of
  consciousness, or the path you took to the answer.
- Never describe history: where code used to live, what it was folded out of, what the
  previous implementation did, or what a diff changed. Git records that.
- Cite external evidence when it exists — a Sentry issue ID (`MAILSPRING-CLIENT-AC`), an
  upstream commit or PR in mailcore2/libetpan/Electron, a spec section, a provider quirk.
  These justify code that otherwise looks arbitrary and are the most valuable comments in
  the codebase.
- Paragraph-length comments should be rare, and almost always attach to a function, class,
  or module rather than sitting inline. Inline comments belong on one non-obvious line and
  should be one or two lines long.

## Development Notes

- Reload the window with Developer > Reload: `Cmd+Option+L` (macOS), `Ctrl+Alt+L` (Linux), `Ctrl+Shift+R` (Windows). It reruns renderer code and recompiles edited files (the compile cache is keyed on file path and contents); main-process code (`src/main`, `src/engine/supervisor`) needs a full restart. `Cmd/Ctrl+R` is Reply
- Dev tools accessible via Menu > Developer > Toggle Developer Tools
- In dev tools console, `$m` provides access to `mailspring-exports` for debugging
- Profiles live under the OS app-data dir: `Tern`, `Tern-dev` (`--dev`), `Tern-spec` (specs), e.g. `~/Library/Application Support/Tern-dev/`; `--config-dir-path` overrides, and the snap uses `$SNAP_USER_COMMON`. `make desktop` from the repo root runs on `.dev/profile`. Nothing is imported from Mailspring on its own: onboarding and Preferences > Accounts offer Import from Mailspring when `<appData>/Mailspring` has a config.json (not in spec mode or the snap). With no accounts in Tern it empties the profile, restarts and copies the Mailspring profile in before `ready` (`edgehill.db` becomes `tern.db`); with accounts it only adds the Mailspring accounts Tern lacks. `--import-from-mailspring=<dir>` imports into a profile with no config.json (`apps/engine/tools/trial-desktop.sh` uses it). The Mailspring profile is only read (`src/main/profile/mailspring-import.ts`, `mailspring-import-flow.ts`)
- The app name is Tern, so safeStorage's keychain item is `Tern Safe Storage`. Mailspring's saved passwords are encrypted under `Mailspring Safe Storage` (DPAPI and Mailspring's Local State on Windows); the import's keychain step reads them with `src/main/profile/mailspring-keychain.ts`, read-only, and stores them under Tern's. KeyManager never reads Mailspring's key
- The interface language is set in Preferences > General (`core.intl.language`) and read at launch, so relaunch to apply it. Arabic, Persian, Hebrew and Kurdish run right-to-left: the workspace gets `dir="rtl"` and every stylesheet goes through `rtlcss`

## Claude Hooks

A `PostToolUse` hook in the repo root's `.claude/settings.json` runs `.claude/hooks/lint-edited-file.sh` after each edit to a `.ts`/`.tsx` file under `apps/desktop/src`, `plugins`, `themes` or `tests`. It runs `eslint --fix` on that one file and reports anything it couldn't fix. The root `.claude/skills/` has `app-screenshots` (CDP captures of the dev app) and `plugin-development`.
