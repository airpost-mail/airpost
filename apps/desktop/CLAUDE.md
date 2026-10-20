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

# Run linting (prettier runs as an eslint rule); lint:check reports without fixing
npm run lint
npm run lint:check

# TypeScript type checking, then the plugin API .d.ts check (scripts/check-plugin-api-dts.js)
npm run typecheck

# Check that better-sqlite3 is built for Electron (postinstall runs it too)
npm run check:native

# Run all tests, or only spec files whose path matches a regex
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

Tern's desktop app is an Electron email client written in TypeScript with React 17, forked from Mailspring. TypeScript and LESS are compiled at runtime; there is no bundler. It uses a plugin architecture where features are implemented as internal packages. Plugin API names keep their Mailspring spelling (`mailspring-exports`, `engines.mailspring`, `window.$m`, `AppEnv`) because third-party plugins depend on them.

### Key Directories

- **`src/`** - Core application source code
  - `main/` - Main process code (application lifecycle, window management, auto-updates, the engine supervisor)
  - `shared/` - Modules both the main process and the windows load (config, intl, the engine binary and protocol types, IPC channel names, compile cache)
  - `plugin-api/` - The plugin API barrels (`mailspring-exports`, `mailspring-component-kit`, `mailspring-store`, `mailspring-observables`), on the module path as bare names
  - `window/` - Window startup and window-level managers (AppEnv, the bootstraps, keymaps, menus, spellchecker, the KeyManager facade)
  - `plugin-host/` - Package discovery and loading, themes and styles (PackageManager, Package, ThemeManager, the plugin API version)
  - `engine/` - The windows' side of the engine: the socket connection, `Engine` (engine-api), engine events, accounts, drafts, PIM, integrations, AI
  - `platform/` - OS integration (native notifications, do-not-disturb, Linux themes, start at login)
  - `utils/` - DOM, date, calendar, regexp, canvas and backoff helpers
  - `flux/` - Flux-based state management (actions, stores, models, tasks, mailbox perspectives, mail rule templates)
  - `components/` - Reusable React UI components, including the sheets (`components/sheets/`) and the higher-order components (`components/decorators/`)
  - `services/` - Application services (search, sanitization, etc.)
  - `registries/` - Extension registries (components, extensions, database objects)

- **`features/`** - Built-in packages that always load (composer, message-list, thread-list, preferences, main-calendar, etc.)
- **`plugins/`** - Bundled optional packages (`isOptional: true`, can be turned off via `core.disabledPackages`), e.g. activity, send-later, translation
- **`themes/`** - The `ui-*` themes
- **`vendor/`** - Quick-preview libraries (pdf.js, Prism, SheetJS, DOMPurify) loaded by `src/quickpreview/renderer.html`

All three package dirs sit at the same depth, so packages import core code as `../../../src/...`. `src/shared/bundled-packages.ts` lists them for the package manager, the `mailspring:` protocol handler, ThemeManager and the spec loader.

> **IMPORTANT:** Application source code lives in `src/`, `features/` and `plugins/`. When searching for usages of a module, symbol, or pattern, always search all of them. Searching only `src/` will miss a large portion of the codebase and lead to incomplete changes.

- **`spec/`** - Jasmine test specs

Code outside `src/main/` and `src/shared/` runs in the windows and must not import `src/main/`; `src/main/` and `src/shared/` must not import window code, except `flux/models/utils` and the two registries it loads (`database-object-registry`, `serializable-registry`), which main uses. `import/no-restricted-paths` in `.eslintrc` checks both directions and fails lint.

### Dependencies and packaging

There is one `package.json` and one `node_modules`. Runtime code goes in `dependencies` (or `optionalDependencies`); tests and tooling go in `devDependencies`, which the packager prunes. `import/no-extraneous-dependencies` (with `packageDir: "."`, so run eslint from this directory) fails a runtime import of a devDependency in `src/`, `features/`, `plugins/` or `themes/`; `spec/` and package `specs/` dirs may use them. Only the top-level entries listed in `shippedTopLevel` in `build/build.js` are packaged, so a new top-level runtime dir has to be added there or it silently won't ship. Package `specs/` dirs never ship.

### Core Modules

**Global exports for plugins:**
- `mailspring-exports` - Core APIs: Actions, Stores, Models, Tasks, Utils, database access
- `mailspring-component-kit` - Reusable UI components

**Flux Architecture:**
- **Models** (`flux/models/`) - Data models: Message, Thread, Contact, Account, Folder, Label, etc.
- **Stores** (`flux/stores/`) - Application state: DatabaseStore, DraftStore, AccountStore, etc.
- **Tasks** (`flux/tasks/`) - Async operations: SendDraftTask, ChangeFolderTask, etc.
- **Actions** (`flux/actions.ts`) - Application-wide action dispatcher

### Plugin Structure

Each package in `features/`, `plugins/` and `themes/` has:
- `package.json` - Metadata with `windowTypes` specifying where plugin loads
- `lib/main.ts` - Entry point with `activate()` and `deactivate()` lifecycle hooks
- `lib/` - Plugin source code
- `styles/` - LESS stylesheets
- `keymaps/` - Keyboard shortcut definitions

## Core Data Flow: Sync Engine, Tasks, and Observable Database

**Important:** The UI is read-only with respect to the database. All database modifications happen in the Rust sync engine (`apps/engine`, bundled as `mailsync-rs` next to `package.json`). The Electron app requests changes via Tasks, and the sync engine streams entity changes back to create a real-time UI.

### Sync Engine Communication

One engine process (`mailsync-rs --mode serve`) runs every account, started by the main process at launch, with zero accounts too (docs/engine-protocol-plan.md section 4, docs/desktop-client-plan.md):

1. **Main process → engine, stdin** (`main/engine-supervisor.ts`, `main/engine-host.ts`, `shared/engine-process.ts` `EngineServeProcess`): JSON-RPC calls that carry credentials or manage the engine: `account.add`/`remove`/`test`/`reset`, `engine.resetDatabase`/`setLogLevel`/`shutdown`. stdout carries their responses and rotated secrets only.
2. **Each window ↔ engine, Unix socket** (`engine-connection.ts`): JSON-RPC 2.0 over newline-delimited JSON at `<configDir>/run/engine.sock`: `hello`, `subscribe`, `task.queue`, reads, and the delta notifications.

```
┌──────────────┐   stdin/stdout (JSON-RPC)   ┌──────────────────┐
│ Main process │ ◀─────────────────────────▶ │                  │
└──────────────┘                             │  mailsync-rs     │
┌──────────────┐   socket (JSON-RPC, deltas) │  --mode serve    │
│ Each window  │ ◀─────────────────────────▶ │                  │
└──────────────┘                             └──────────────────┘
```

Windows reach the stdin-only calls over IPC (`supervisor-channels.ts`). The `MailsyncBridge` in the main window listens to `Actions.queueTask` and sends tasks on its connection; every window applies the deltas it reads.

### Task System (`flux/tasks/`)

Tasks represent operations the user wants to perform (send email, star thread, move to folder). They are **persisted models** stored in the database.

**Task Lifecycle:**
1. UI calls `Actions.queueTask(new SomeTask({...}))`
2. `MailsyncBridge._onQueueTask()` validates and sends `task.queue` on the main window's socket connection
3. Sync engine executes the task (local changes + remote API calls)
4. Sync engine persists task status updates and emits deltas
5. Task completion triggers `onSuccess()` or `onError()` callbacks

**Task States** (`flux/tasks/task.ts`):
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

### Task Queue (`flux/stores/task-queue.ts`)

The TaskQueue store observes Task model changes from the database and provides:
- `queue()` - Active tasks
- `completed()` - Finished tasks
- `waitForPerformLocal(task)` - Promise that resolves when task runs locally
- `waitForPerformRemote(task)` - Promise that resolves when task fully completes

### Observable Database Pattern

**Database is read-only in Electron** (`flux/stores/database-store.ts`):
- `DatabaseStore.inTransaction()` throws - writes are not allowed
- Uses SQLite in WAL mode via better-sqlite3 for concurrent reads
- The sync engine exclusively handles writes

**Change Records** (`flux/stores/database-change-record.ts`):

When the sync engine modifies data, it emits JSON deltas that become `DatabaseChangeRecord` objects:
```typescript
{
  type: 'persist' | 'unpersist',
  objectClass: 'Thread' | 'Message' | ...,
  objects: Model[],
  objectsRawJSON: object[]
}
```

**Reactive Queries** (`flux/models/query-subscription.ts`):

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

**ObservableListDataSource** (`flux/stores/observable-list-data-source.ts`):

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

- Reload the window with Developer > Reload: `Cmd+Option+L` (macOS), `Ctrl+Alt+L` (Linux), `Ctrl+Shift+R` (Windows). It reruns renderer code and recompiles edited files (the compile cache is keyed on file path and contents); main-process code in `src/main` needs a full restart. `Cmd/Ctrl+R` is Reply
- Dev tools accessible via Menu > Developer > Toggle Developer Tools
- In dev tools console, `$m` provides access to `mailspring-exports` for debugging
- Profiles live under the OS app-data dir: `Tern`, `Tern-dev` (`--dev`), `Tern-spec` (specs), e.g. `~/Library/Application Support/Tern-dev/`; `--config-dir-path` overrides, and the snap uses `$SNAP_USER_COMMON`. `make desktop` from the repo root runs on `.dev/profile`. On first launch an empty default `Tern`/`Tern-dev` profile is copied from `Mailspring`/`Mailspring-dev` (`src/main/mailspring-profile-import.ts`), which is only read; spec mode, the snap and `--config-dir-path` don't import
- The app name is Tern, so safeStorage's keychain item is `Tern Safe Storage`. Credentials in an imported profile are encrypted under `Mailspring Safe Storage`; `src/main/mailspring-keychain-migration.ts` (`OLD_APP_NAME`) reads that item, read-only, and `src/main/key-manager.ts` re-encrypts them under Tern's
- The interface language is set in Preferences > General (`core.intl.language`) and read at launch, so relaunch to apply it. Arabic, Persian, Hebrew and Kurdish run right-to-left: the workspace gets `dir="rtl"` and every stylesheet goes through `rtlcss`

## Claude Hooks

A `PostToolUse` hook in the repo root's `.claude/settings.json` runs `.claude/hooks/lint-edited-file.sh` after each edit to a `.ts`/`.tsx` file under `apps/desktop/src`, `features`, `plugins` or `themes`. It runs `eslint --fix` on that one file and reports anything it couldn't fix. The root `.claude/skills/` has `app-screenshots` (CDP captures of the dev app) and `plugin-development`.
