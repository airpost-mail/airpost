# Plugin API v2

Design for the next plugin API of the desktop app. Nothing here is implemented. v2 ships
alongside v1 in the next minor (1.35.0); v1 keeps working, unchanged, until 2.0.0 removes it.
Paths are relative to `apps/desktop/` unless they start with `docs/`, `packages/` or `examples/`.

Inputs that are settled and not reopened here: the desktop is a thin client of the engine; core
features are app code under `src/features/` and move from registry wiring to direct imports
wherever no plugin uses the slot; the UI kit is `src/components/kit/` (`docs/desktop/ui-kit.md`);
themes are CSS custom properties (`docs/desktop/theme-tokens.md`); no plugin code runs inside the
engine, background logic is a service plugin (`docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md`);
plugins are trusted code and nothing in the API is a security boundary (TD29).

Reference model: Obsidian. A plugin is a class extending `Plugin` with `onload()`/`onunload()`;
everything it adds goes through `this.add*`/`this.register*` and is removed automatically on
unload; the API is a module the app provides at runtime (`obsidian`), typed by a types-only
package; a plugin ships `manifest.json` (with `minAppVersion`), a prebuilt `main.js` and an
optional `styles.css`; a theme is `manifest.json` plus CSS that sets variables.

## 1. What plugins use today

### 1.1 Sources and counting

- **Bundled plugins**: the 13 packages in `plugins/` (activity, ai, composer-grammar-check,
  composer-templates, link-tracking, open-tracking, participant-profile, phishing-detection,
  send-later, send-reminders, thread-sharing, thread-snooze, translation), read at `a770bab75`.
- **Built-in features**: the 28 directories in `src/features/`. They are app code, not plugins;
  they are counted only to show which slots are core-only.
- **Third-party plugins**: the 14 repositories named in `docs/desktop/community-plugins.md` and
  `docs/plugins-inventory.md` whose source could be read, fetched 2026-10-20: Striffly/mailspring-avatars,
  NgoHuy/mailspring-keybase, mirkoschubert/mailspring-pgp, dinoboy197/mailspring-openpgp,
  rages4calm/mailspring-spoof-guard, jmanuel1/todoer, saint-james-fr/mailspring-sort-inbox,
  hburaktasyurek/Mailspring-Smart-Folders, Raymo111/mailspring-plaintext-signatures,
  kz6fittycent/mailspring-package-tracker, arnoo/mailspring-mute-plugin,
  EdGaz/mailspring-outgoing-undo, 3nws/refresh-on-focus, ruslansin/launcher-api-support.
  The three PGP plugins are forks of one codebase; they are counted as three repositories, and
  the tables say so where it matters.
- A count is the number of packages (or features, or repositories) that use the slot, not the
  number of calls. Tests are not counted.

### 1.2 Component slots (ComponentRegistry roles)

| Role | Bundled plugins | Features | Third-party | v2 method |
|---|---|---|---|---|
| `Composer:ActionButton` | 8: ai, composer-grammar-check, composer-templates, link-tracking, open-tracking, send-later, send-reminders, translation | 0 | 3: keybase, pgp, openpgp | `addComposerAction` |
| `ThreadActionsToolbarButton` | 3: send-reminders, thread-sharing, thread-snooze | 2: category-picker, thread-list | 1: mute | `addThreadToolbarButton` |
| `MessageListHeaders` | 3: ai, phishing-detection, send-reminders | 1: message-list | 0 | `addThreadHeader` |
| `message:BodyHeader` | 1: translation | 3: events, list-unsubscribe, message-autoload-images | 4: keybase, pgp, openpgp, package-tracker | `addMessageBanner` |
| `Thread:MailLabel` | 2: ai, thread-snooze | 0 | 1: avatars | `addThreadLabel` |
| `ThreadListIcon` | 1: open-tracking | 0 | 1: avatars | `addThreadListIcon` |
| `ThreadListQuickAction` | 1: thread-snooze | 0 | 1: mute | `addThreadQuickAction` |
| `MessageHeaderStatus` | 1: open-tracking | 0 | 1: spoof-guard | `addMessageHeaderItem` |
| `MessageListSidebar:ContactCard` | 1: participant-profile (2 components) | 0 | 0 | `addContactSidebarSection` |
| `DraftList:DraftStatus` | 1: send-later | 1: draft-list | 0 | `addDraftStatus` |
| `Composer:Footer` | 1: composer-templates | 0 | 0 | `addComposerFooter` |
| `ThreadListTimestamp` (one component, first wins, core fallback) | 1: send-reminders | 0 | 0 | `setThreadListTimestamp` |
| `MessageHeader` (one component, first wins) | 1: send-reminders | 0 | 0 | `setMessageHeaderNotice` |
| `Composer:RecipientChip` | 0 | 0 | 3: keybase, pgp, openpgp | `addRecipientChipItem` |
| `Composer:FromFieldComponents` | 0 | 0 | 1: plaintext-signatures | `addComposerFromFieldItem` |

### 1.3 Locations and screens

| Slot | Bundled plugins | Features | Third-party | v2 method |
|---|---|---|---|---|
| `WorkspaceStore.Location.RootSidebar.Toolbar` | 1: activity | 1: composer | 0 | `addSidebarToolbarButton` |
| `WorkspaceStore.Location.RootSidebar` | 0 | 2: account-sidebar, notifications | 1: smart-folders | `addSidebarSection` |
| `WorkspaceStore.Location.ThreadList.Toolbar` | 0 | 2: thread-list, thread-search | 1: sort-inbox | `addThreadListToolbarItem` |
| `WorkspaceStore.defineSheet` + a location of its own | 1: activity (`Activity` sheet, `ActivityContent`) | 5: contacts, draft-list, main-calendar, onboarding, preferences | 0 | `registerSheet` |
| `PreferencesUIStore.registerPreferencesTab` | 2: ai, composer-templates | 3: category-mapper, composer-signature, preferences | 6: keybase, pgp, openpgp, todoer, spoof-guard, plaintext-signatures | `addSettingsTab` |
| `AppEnv.commands.add` | 0 | 3: account-sidebar, contacts, theme-picker | 0 | `addCommand` |
| `keymaps/`, `menus/` directories | 0 | not counted | 0 | `addCommand` (`hotkeys`, `menu`) |

### 1.4 Behaviour extensions (ExtensionRegistry)

| Registry and hooks | Bundled plugins | Features | Third-party | v2 method |
|---|---|---|---|---|
| `AccountSidebar` (`sidebarItem`) | 4: activity, ai, send-reminders, thread-snooze | 0 | 0 | `addSidebarItem` |
| `MessageView` (`formatMessageBody`, `renderedMessageBodyIntoDocument`, `allowsRemoteImages`, `filterMessageFiles`) | 2: link-tracking, translation | 3: events, message-autoload-images, remove-tracking-pixels | 3: keybase, pgp, openpgp | `addMessageViewExtension` |
| `Composer` (`prepareNewDraft`, `warningsForSending`, `applyTransformsForSending`, `needsPerRecipientBodies`, `onSendSuccess`) | 1: composer-templates | 3: composer-signature, remove-tracking-pixels, send-and-archive | 1: plaintext-signatures | `addComposerExtension` |
| `Composer` (`sendActions`) | 0 | 1: send-and-archive | 0 | `addSendAction` |
| `ThreadList` (`cssClassNamesForThreadListItem`, `…Icon`) | 1: send-reminders | 0 | 0 | `addThreadListClasses` |

### 1.5 Other APIs plugins reach for

| Use | Bundled plugins | Third-party | v2 |
|---|---|---|---|
| Engine: `Engine.request`/`subscribe`/`events`/`has`, or deep imports of `src/engine/client/events` and `src/engine/protocol` | 4: ai; send-later, send-reminders, thread-snooze (deep imports) | 1 calls `AppEnv.mailsyncBridge` (refresh-on-focus) | `this.engine` |
| `DatabaseStore.listen` | 4: ai, send-later, send-reminders, thread-snooze | 2: mute, spoof-guard | `this.onDatabaseChange`, `this.onMetadataExpired` |
| `DatabaseStore` reads | several | 6: keybase, pgp, openpgp, smart-folders, spoof-guard, sort-inbox | `this.app.database` |
| Plugin metadata read (`metadataForPluginId`) | 7: activity, ai, link-tracking, open-tracking, send-later, send-reminders, thread-snooze | 0 | `this.metadata` |
| Plugin metadata write (`SyncbackMetadataTask`, `MetadataComposerToggleButton`) | 6: ai, send-later, send-reminders, thread-snooze, link-tracking, open-tracking | 1 patches `SyncbackMetadataTask.prototype` (outgoing-undo) | `this.metadata` |
| `Actions.queueTask(s)`, `TaskFactory` | several | 2: mute, spoof-guard | `this.app.tasks` |
| `AppEnv.config` | 5: activity, ai, composer-grammar-check, thread-snooze, translation | 4: smart-folders, todoer, spoof-guard, launcher-api-support | `this.settings` |
| `localStorage` | 2: participant-profile, translation | 2: mute, sort-inbox | `this.cache` |
| `Actions.<action>.listen` | 3: activity, composer-grammar-check, thread-snooze | 2: todoer, smart-folders | `this.onAction` |
| `Actions.openPopover` | 9 | 3: keybase, pgp, openpgp | `this.app.ui.openPopover`, kit `Popover` |
| `HasTutorialTip` | 7 | 0 | the `tip` option of the button methods |
| `mailspring://<package>/…` asset URLs | 8 | 5: keybase, pgp, openpgp, todoer, smart-folders | `this.assetUrl(path)` |
| Styles | 13 (`styles/`, LESS then, plain CSS now) | 8 LESS in `styles/`; the PGP forks use `stylesheets/`, which is not loaded | `styles.css` |
| `serialize()` | 8 export it; 2 return state that nothing reads back | 0 | dropped |
| Node and Electron directly (`fs`, `dns`, `electron`, `@electron/remote`, `dbus-native`) | some | 8 | unchanged; plugins are trusted |

### 1.6 Slots no plugin uses

Rendered by core, registered only by features or by nobody. These get no v2 method: features
import their components directly, and the slot stays available to v1 plugins (nothing in v1 is
removed) until 2.0.0.

| Slot | Registered by |
|---|---|
| `Composer`, `ThreadList` roles; `ThreadList`, `MessageList`, `MessageList.Toolbar`, `MessageListSidebar`, `Center`, `DraftList`, `DraftPreview`, `Contacts*`, `Calendar*`, `Preferences` locations; the sheet toolbars and `Global.Footer` | features only |
| `RootSidebar:Notifications`, `NavRail:Item` | features only (notifications; account-sidebar, main-calendar) |
| `MessageFooterStatus`, `MessageActionMenuItem`, `DraftActionsToolbarButton`, `Composer:ActionBarWorkspace`, `Calendar:Day:Banner`, `Calendar:Week:Banner`, `Calendar:Event` | nobody |
| `SearchBarResults`, `ContactSearchResults` (providers looked up through ComponentRegistry) | nobody |
| `SoundRegistry`, `ServiceRegistry`, `DatabaseObjectRegistry` | features only, or nobody |

`docs/plugins-inventory.md` lists gaps third-party authors hit (search providers, contact
enrichment, a pre-send hook). They have no users today, so v2 doesn't add them speculatively; each
is a later minor when a plugin needs it (decision 8).

### 1.7 Findings that shape v2

1. **Cleanup is by hand and gets missed.** Every v1 plugin pairs each `register` with an
   `unregister` in `deactivate()`. mailspring-avatars registers two components and unregisters
   one (the other line is commented out). Nothing notices, because `PackageManager.deactivatePackages()`
   is empty and plugins are only deactivated by a restart. That also rules out turning a plugin
   off or reloading it while the app runs.
2. **Component names are a global namespace.** ComponentRegistry keys on `displayName` and throws
   when a different component already has the name. Two plugins with a `TranslateButton` collide.
3. **Deep imports fill the gaps.** 8 of 13 bundled plugins import app files by relative path
   (`src/engine/client/events`, `src/engine/protocol`, `src/engine/resurfacing`,
   `src/mail/frame/bridge`, `src/intl/date-format`, `src/components/kit/keys`,
   `src/components/settings-select`). Third-party plugins do the same by `resourcePath`:
   sort-inbox patches `internal_packages/thread-list` and `src/flux/models/query.js`, paths that
   don't exist in Tern; smart-folders loads `@electron/remote` from the app's `node_modules`.
   Each of these is a missing API.
4. **Ranges don't bound anything.** 13 of the 14 third-party manifests have an open range
   (`>=1.6.3`, `>=0.3.0`, `>=1.0.8`, `>=1.23.0`, `*`); only outgoing-undo says `<2.0.0`. All 13
   would satisfy 2.0.0, so the range can't be what keeps a v1 plugin from loading there.
5. **Only prebuilt JavaScript loads in packaged builds.** Packaged builds don't ship the TypeScript
   compiler: `src/main/main.js` installs `compile-cache-ts-unsupported.js` outside dev mode, and
   a `.ts`, `.tsx`, `.jsx` or `.es6` main fails with a "Plugin must be compiled" dialog. Of the 14
   repositories, 7 ship a `lib/main.js`, 3 point `main` at `.es6` source (todoer and two PGP forks,
   which don't load in a packaged Tern) and 4 have no built main in the repository. LESS still
   compiles at runtime in packaged builds, for third-party packages only.
6. **Singular slots exist.** `ThreadListTimestamp` and `MessageHeader` render one component;
   the first registered wins. v2 keeps that and names them `set*` so the behaviour is visible.

## 2. Package format

### 2.1 Files

A v2 plugin, as installed:

```
my-plugin/
├── manifest.json   required
├── main.js         required: prebuilt CommonJS
├── styles.css      optional: plain CSS, reads the theme tokens
└── assets/         optional: anything served at tern-asset://my-plugin/…
```

A v2 theme:

```
my-theme/
├── manifest.json   required, "type": "theme"
└── tokens.css      required: custom properties on :root
```

`manifest.json` marks the package as v2. A directory with `manifest.json` is v2 even if it also
has a `package.json` (a plugin's source repository usually does); a directory with only
`package.json` is v1 and loads exactly as today.

### 2.2 manifest.json

```json
{
  "id": "package-tracker",
  "name": "Package Tracker",
  "version": "1.2.0",
  "minApiVersion": "1.35.0",
  "description": "Finds tracking numbers in messages.",
  "author": "Jane Doe",
  "authorUrl": "https://example.com",
  "license": "MIT",
  "windowTypes": ["default", "thread-popout"],
  "engineMethods": [],
  "engineFeatures": [],
  "service": null
}
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | `^[a-z0-9][a-z0-9-]*$`, equal to the directory name. It is the plugin's id everywhere: settings namespace, metadata plugin id, asset host, approvals. A v2 id can't be a built-in feature's name (refused, with the reason in Preferences > Plugins); it can be a bundled plugin's name, which replaces the bundled copy as v1 packages do today |
| `name`, `version` | yes | Display name; semver of the plugin itself |
| `minApiVersion` | yes | The plugin API version it needs. A v2 plugin loads when `PLUGIN_API_VERSION` is `>=minApiVersion <3.0.0` (section 11) |
| `main` | no | Defaults to `main.js` |
| `windowTypes` | no | Window types it loads in; defaults to `["default"]`. Same names as v1 (`default`, `composer`, `thread-popout`, `emptyWindow`, `all`) |
| `engineMethods` | no | Local-only engine methods it calls (section 6). Same as v1's `tern.engineMethods` |
| `engineFeatures` | no | Engine features (`hello.features`) it needs. Missing ones: listed but not activated, with the reason in Preferences > Plugins |
| `service` | no | Same as v1's `tern.service` |
| `type`, `scheme`, `sidebarScheme` | themes | `"type": "theme"`; the scheme fields as in `docs/desktop/theme-tokens.md` |

Third-party v2 plugins are always optional (the user can turn them off) and never `syncInit`:
they activate in the delayed batch, after the window's own features. `isHiddenOnPluginsPage`
and `isDefault` stay bundled-only.

### 2.3 main.js

CommonJS, with the plugin class as the default export (`module.exports = class …` or
`exports.default = class …`). Everything is bundled into the one file except the modules the app
provides, which stay external:

- `tern`, `tern/ui` (section 4)
- `react`, `react-dom`: the loader resolves them to the app's copies, so a plugin can't load a
  second React (hooks break across copies)
- `electron`, `@electron/remote`, Node built-ins

The starter (`examples/plugin-starter`, section 10) builds with esbuild (MIT) and marks exactly
these external. Source maps may be inlined; the TS compile cache isn't involved.

### 2.4 Loading

`PackageManager` gets a `PluginV2` entry next to `Package` and `BuiltinFeature`, discovered from
the same directories in the same order. Activation:

1. Validate the manifest; check `minApiVersion`, `windowTypes`, `engineFeatures`, and that `main`
   is JavaScript (outside dev mode).
2. Load `styles.css` through StyleManager at plugin priority, with a disposable.
3. `require(main)`, take the default export, check it extends `Plugin`, construct it with the
   app and the manifest, `await plugin.onload()`.
4. If `onload` throws or rejects, everything it registered is disposed, the stylesheet removed,
   and the error shown on the plugin's row in Preferences > Plugins (not only the console).

Deactivation calls `onunload()` and then disposes every registration in reverse order. Because
nothing has to be undone by hand, Preferences > Plugins can turn v2 plugins on and off without a
restart, and dev mode can reload one when its files change.

## 3. The Plugin base class

### 3.1 Lifecycle and disposal

```ts
export abstract class Plugin {
  readonly app: App;
  readonly manifest: PluginManifest;
  readonly engine: PluginEngine; // bound to this plugin (section 6)
  readonly settings: PluginSettings; // section 7
  readonly cache: PluginCache;
  readonly metadata: PluginMetadata;
  readonly dataDir: string; // <profile>/plugin-data/<id>/, created on first read

  onload(): void | Promise<void>;
  onunload(): void;

  /** Runs `fn` on unload. Every add* and register* below goes through this. */
  register(fn: () => void): void;
  registerDisposable<T extends { dispose(): void }>(d: T): T;
  registerInterval(id: number): number;
  registerDomEvent(target: EventTarget, type: string, cb: EventListener): void;

  assetUrl(path: string): string; // tern-asset://<id>/<path>
}
```

Each `add*`/`register*`/`on*` method returns a `Disposable`, so a plugin can remove one thing
early (`ai`'s "Needs reply" sidebar item follows a setting); the rest is removed on unload.

Components passed to the slot methods are wrapped once, with `displayName` set to
`<id>:<displayName>` (or `<id>:<method>-<n>` when the component has none), so names can't collide
across plugins and `displayName` stops being required. The wrapper renders the component with the
props the slot passes today, so the DOM, the `InjectedComponent` wrappers and the classes themes
target are unchanged.

### 3.2 Slot methods

All of them register through the v1 registries, so v1 and v2 plugins share slots, and order
follows activation order as it does now.

| Method | Over | Props the component gets |
|---|---|---|
| `addComposerAction(C, opts?)` | `Composer:ActionButton` | `draft`, `threadId`, `headerMessageId`, `session`, `isValidDraft` |
| `addComposerFooter(C)` | `Composer:Footer` | `draft`, `threadId`, `headerMessageId`, `session` |
| `addComposerFromFieldItem(C)` | `Composer:FromFieldComponents` | as the role passes today |
| `addRecipientChipItem(C)` | `Composer:RecipientChip` | `contact`, `collapsed` |
| `addThreadToolbarButton(C, opts?)` | `ThreadActionsToolbarButton` | `items` (threads) |
| `addThreadListToolbarItem(C)` | `Location.ThreadList.Toolbar` | none |
| `addThreadListIcon(C)` | `ThreadListIcon` | `thread` |
| `addThreadLabel(C)` | `Thread:MailLabel` | `thread`, `messages` |
| `addThreadQuickAction(C)` | `ThreadListQuickAction` | `thread` |
| `setThreadListTimestamp(C)` | `ThreadListTimestamp` (first wins) | `thread` |
| `addThreadListClasses({ item?, icon? })` | `ExtensionRegistry.ThreadList` | `(thread) => string` |
| `addThreadHeader(C)` | `MessageListHeaders` | `thread`, `messages` |
| `addMessageHeaderItem(C)` | `MessageHeaderStatus` | `message`, `thread`, `detailedHeaders` |
| `setMessageHeaderNotice(C)` | `MessageHeader` (first wins) | `message`, `thread`, `messages` |
| `addMessageBanner(C)` | `message:BodyHeader` | `message` |
| `addContactSidebarSection(C)` | `MessageListSidebar:ContactCard` | `contact` |
| `addDraftStatus(C)` | `DraftList:DraftStatus` | `draft` |
| `addSidebarItem((accountIds) => item)` | `ExtensionRegistry.AccountSidebar` | returns `{id, name, icon, perspective, insertAtTop?}` |
| `addSidebarSection(C)` | `Location.RootSidebar` | none |
| `addSidebarToolbarButton(C, opts?)` | `Location.RootSidebar.Toolbar` | none |

The props column is what core passes in `exposedProps` today; the types package declares one
props interface per slot from it, and the slot's spec (section 12) pins it. Button methods take
`opts.tip: {title, instructions}`, which applies `HasTutorialTip`.

### 3.3 Behaviour, screens and commands

| Method | Over | Notes |
|---|---|---|
| `addComposerExtension({ prepareNewDraft?, warningsForSending?, applyTransformsForSending?, needsPerRecipientBodies?, onSendSuccess? })` | `ExtensionRegistry.Composer` | Desktop-only hooks: they don't run for CLI, MCP or engine-executed sends |
| `addSendAction({ key, title, iconUrl?, isEnabled, perform })` | `ComposerExtension.sendActions` | |
| `addMessageViewExtension({ formatBody?, bodyRendered?, allowsRemoteImages?, filterFiles? })` | `ExtensionRegistry.MessageView` | `bodyRendered` gets `MessageBody` (`src/mail/frame/bridge.ts`), exported as a type |
| `registerSheet({ id, title, icon?, component, toolbar?, navRail? })` | `WorkspaceStore.defineSheet` + a location of its own | One column; opens with `this.app.workspace.openSheet(id)`. `navRail` adds a rail entry (decision 5) |
| `addSettingsTab({ id, title, group?, order?, keywords?, component })` | `PreferencesUIStore.registerPreferencesTab` | `component` may be a lazy `() => C`. Builds on the `SettingsPage`/`SettingsSection`/`SettingsRow` exports |
| `addCommand({ id, name, callback, hotkeys?, menu? })` | `AppEnv.commands`, the keymap, the menu | Command id is `<id>:<command>`. `hotkeys` are defaults the user can rebind in Preferences > Shortcuts; `menu` names a top-level menu to list it in |

Not in v2: `SoundRegistry`, `ServiceRegistry`, `DatabaseObjectRegistry`, the `keymaps/` and
`menus/` directories (`addCommand` covers both) and `serialize()`.

### 3.4 Events

| Method | Over |
|---|---|
| `onDatabaseChange(filter, cb)` | `DatabaseStore.listen`, filtered by `{objectClass?, type?}` |
| `onMetadataExpired(cb)` | `metadata-expiration` deltas for this plugin's metadata id (send-later, snooze, reminders) |
| `onAction(name, cb)` | `Actions[name].listen` |
| `listenTo(store, cb)` | `store.listen` for the stores in `this.app` |
| `this.engine.subscribe(filter, cb)`, `this.engine.onDidChange(cb)` | section 6 |

### 3.5 Example

```ts
import { Plugin, MailboxPerspective, localized } from 'tern';
import { ToolbarSnooze, QuickActionSnooze } from './buttons';
import { SnoozeMailLabel } from './mail-label';

export default class ThreadSnooze extends Plugin {
  onload() {
    this.addThreadToolbarButton(ToolbarSnooze, {
      tip: { title: localized('Handle it later!'), instructions: localized('Snooze this email…') },
    });
    this.addThreadQuickAction(QuickActionSnooze);
    this.addThreadLabel(SnoozeMailLabel);
    this.addSidebarItem((accountIds) => ({
      id: 'snoozed',
      name: localized('Snoozed'),
      icon: 'snoozed.png',
      perspective: MailboxPerspective.forStandardCategories(accountIds, 'snoozed'),
      insertAtTop: true,
    }));
    this.onMetadataExpired((threads) => this.unsnooze(threads));
  }
}
```

There is no `deactivate()`: unload removes all five registrations and the listener.

## 4. The API module

The app provides two modules on the module path, as it does for `mailspring-exports` today
(`src/packages/api/`, added with `app-module-path`):

- `tern`: `Plugin`, the types, and the runtime values plugins need.
- `tern/ui`: the kit (`src/components/kit/`) under its shadcn names, flat: `Button`,
  `IconButton`, `Switch`, `Checkbox`, `RadioGroup`, `Select…`, `TextField`, `Textarea`,
  `Dialog…`, `Popover…`, `DropdownMenu…` (the root is `DropdownMenu` here; v1 has to call it
  `DropdownMenuRoot`), `Tooltip…`, `Tabs…`, `SegmentedControl`, plus the settings building blocks
  (`SettingsPage`, `SettingsSection`, `SettingsRow`, `ConfigSwitchRow`) and the non-kit
  components plugins use (`RetinaImg`, `Flexbox`, `ScrollRegion`, `DatePickerPopover`,
  `DateInput`, `MailLabel`, `ContactProfilePhoto`, `MetadataComposerToggleButton`).

Both modules are lazy getters like the v1 barrels, so a window only loads what a plugin reads.

`tern` exports what the evidence in section 1 shows plugins use, typed:

- Models (classes, for `instanceof` and query attributes): `Thread`, `Message`, `Contact`,
  `Account`, `Folder`, `Label`, `Category`, `File`, `Event`, `MailboxPerspective`.
- Helpers: `localized`, `localizedReactFragment`, `DateUtils`, `RegExpUtils`, `DOMUtils`, `Utils`,
  `Rx`.
- Types: one props interface per slot, `MessageBody`, `EngineFeature`, `EngineCapabilities`,
  `PluginManifest`, the extension hook shapes.

`this.app` holds the instances:

| Member | Over | Evidence |
|---|---|---|
| `window` | `{ type, isMain }` from `AppEnv.getLoadSettings`, `AppEnv.isMainWindow` | 4 bundled, 1 third-party read `isMainWindow` |
| `accounts` | `AccountStore` (read) | |
| `workspace` | `FocusedPerspectiveStore`, `FocusedContentStore`, `WorkspaceStore` (`openSheet`, `onLayoutReady`) | 3 bundled, 5 third-party |
| `database` | `DatabaseStore` reads (`find`, `findAll`, `findBy`, `count`), over the engine | 6 third-party |
| `tasks` | `Actions.queueTask(s)`, `TaskFactory`, the Task classes | 2 third-party |
| `drafts` | `DraftStore.sessionForClientId`, compose actions | send-later, templates |
| `messages` | `MessageStore` (the open thread) | phishing-detection |
| `ui` | `openPopover`/`closePopover`, `openPreferences(tabId?)`, the native `showMessageBox`/`showOpenDialog`/`showSaveDialog` | 9 bundled, 4 third-party |
| `config` | `AppEnv.config`, raw | for bundled plugins whose keys predate v2, and for one-time moves to `this.settings` |

Anything a plugin needs that isn't here stays reachable through `mailspring-exports` until 2.0.0.
Additions to `tern` are minor versions, made when a plugin needs one (decision 3).

`window.tern` is the `tern` module, for the dev console; `window.$m` stays.

## 5. Old plugins keep working

- v1 packages (`package.json`, `activate`/`deactivate`) load through `Package` exactly as now:
  `engines.mailspring` checked as a semver range, `styles/*.less` compiled (against the LESS
  surface in `static/styles/compat/less`), `keymaps/`, `menus/`,
  `tern.engineMethods` and `tern.service` read, the stack-walking caller check for the engine.
- `mailspring-exports`, `mailspring-component-kit`, `mailspring-store`, `mailspring-observables`,
  `$m`, `mailspring://`, `AppEnv` and every registry stay as they are.
- `Plugin` is a layer over the same registries; it adds no slot v1 can't reach and changes no
  render path. Both kinds can be installed side by side and share slots and order.
- A v1 `package.json` may say `engines.tern` instead of `engines.mailspring`; both are read, with
  the same meaning, until 2.0.0.

## 6. Engine access and the permission prompt

`this.engine` is `Engine` (`src/engine/client/api.ts`) bound to the plugin:

```ts
interface PluginEngine {
  request<R>(method: string, params?: unknown): Promise<R>;
  subscribe(filter: { methods?: string[]; accounts?: string[] }, cb): Disposable; // auto-disposed
  onDidChange(cb): Disposable; // connected, gone, capabilities; auto-disposed
  has(feature: EngineFeature): boolean;
  readonly features: string[];
  readonly capabilities: EngineCapabilities | null;
  readonly connected: boolean;
}
```

- The caller is the plugin instance, so v2 needs no stack walk; `callerPackageFromStack` stays
  for v1 calls.
- `hello` and `subscribe` are refused, as in v1.
- A local-only method (`account.*` writes, `secrets.*`, `credentials.*`, `server.*`,
  `engine.logs.tail`, `engine.setLogLevel`, `engine.resetDatabase`, `engine.shutdown`) is refused
  unless `manifest.json` lists it in `engineMethods`. The first call asks with the existing
  native prompt ("Allow the plugin %1$@ to call %2$@?"); a yes is kept in
  `core.plugins.engineMethodApprovals` under the plugin id, a no lasts until restart. Bundled
  plugins aren't asked, as today.
- Preferences > Plugins shows each plugin's declared methods and the answers, with a way to
  revoke one.
- When an update adds a method to `engineMethods`, that method asks again; the others keep their
  answer (decision 6).
- `engineFeatures` gates activation (section 2.2), so a plugin doesn't have to check
  `has()` in `onload` for what it can't run without.
- `refresh-on-focus` reaches `AppEnv.mailsyncBridge.sendSyncMailNow()`. Its v2 path is
  `this.engine.request('sync.wake', { accountId? })`: the engine already has the method
  (`method::SYNC_WAKE` in `crates/tern-ipc`, handled in `apps/engine/src/socket.rs`), it wakes one
  account or, without `accountId`, every running one, and it is an open method, so it needs no
  `engineMethods` entry or prompt. `sendSyncMailNow()` sends the same request. Nothing has to be
  added to the engine.

The prompt is a guard against accidents, not a sandbox: a plugin still has Node (TD29).

## 7. Settings, cache, files and metadata

| Store | Backed by | For |
|---|---|---|
| `this.settings` | `AppEnv.config` under `plugins.<id>` | User settings. Synced to every window as config is today, in `config.json`, covered by profile reset and import. `get(key)`, `set(key, value)`, `onDidChange(key, cb)` (auto-disposed), `defaults({…})` |
| `this.cache` | `localStorage`, keys prefixed `plugin:<id>:` | Per-machine data that can be rebuilt (translation results, profile lookups) |
| `this.dataDir` | `<profile>/plugin-data/<id>/` | Files. Outside the plugin's directory, so an update or reinstall keeps them |
| `this.metadata` | `metadataForPluginId(id)`, `SyncbackMetadataTask`, `metadata.list` | Per-thread or per-message data the engine stores and syncs. `get(model)`, `set(model, value, {expiration?})`, `list(query)`, and `scope(otherId)` for a second id (ai's `tern-ai-triage`, send-reminders' `thread:send-reminders`) |

Obsidian keeps settings in `data.json` inside the plugin folder; config.json is used here
because it already syncs across windows and the profile tools know it (decision 7).

v1 plugins keep their keys. A plugin moving to v2 reads its old keys once through
`this.app.config` and writes them to `this.settings`; bundled plugins keep their current keys
(`core.composing.grammarCheck`, the AI keys) through `this.app.config`, since moving them is a
data migration with no user benefit.

## 8. Themes

- A v2 theme is `manifest.json` (`"type": "theme"`, `scheme`, optional `sidebarScheme`) and
  `tokens.css`, which sets the tokens of `docs/desktop/theme-tokens.md` on `:root` and nothing
  else. ThemeManager already loads a theme's `styles/tokens.css` after `defaults.css`; v2 reads
  `tokens.css` at the package root.
- `"scheme": "auto"` covers what `ui-automatic` does: `tokens.css` holds the dark values in a
  `@media (prefers-color-scheme: dark)` block.
- The theme picker and Preferences > Appearance preview a theme from the `--theme-preview-*`
  properties in its `styles/theme-colors.css` (a v1 LESS theme: its LESS variables;
  `docs/desktop/theme-tokens.md`, "Theme previews"); for v2 themes they read the same properties
  from `tokens.css` instead.
- v1 themes keep working until 2.0.0: token themes with `styles/tokens.css`, LESS-variables themes
  through the compat path, and the restyle-by-class themes (most community themes) through the
  legacy classes the kit still renders (`docs/desktop/ui-kit.md` section 7).
- A token-only theme can't restyle layout. Someone who wants that writes a plugin with a
  `styles.css`; that is a plugin, listed as one.
- At 2.0.0 the compat LESS path, LESS themes and the legacy kit classes go together.

## 9. Runtime compilation

| Code | Packaged build | Dev mode (`--dev`) |
|---|---|---|
| App code, built-in features, bundled plugins | TS transpiled by `build/build.js`; stylesheets are plain CSS, read as they are | TS compiled at runtime; plain CSS |
| v1 third-party packages | `.js` only (a TS/JSX/ES6 main gets the "must be compiled" dialog, as now); `styles/*.less` compiled at runtime | TS/JSX/ES6 and LESS compiled at runtime |
| v2 third-party plugins | `main.js` and `styles.css` only, no compiler involved | Also accepts a `.ts`/`.tsx` `main` and a `styles.less`, compiled by the existing caches, so a plugin in `<profile>/dev/packages/` can run from source; the loader reloads it when its files change |

So the TS compiler stays where it is (dev mode only), and runtime LESS is only for v1 packages and
dev mode: the app's own stylesheets, bundled plugins included, are plain CSS. v1 LESS imports
`ui-variables`, `ui-mixins`, `base/*` and `mixins/*` from `static/styles/compat/less`, not the app's
stylesheets.

## 10. Types package and starter

- `packages/plugin-api/` (`@tern/plugin-api`, `private: true`, GPL-3.0-only) holds the contract:
  `index.d.ts` declaring `module 'tern'` and `module 'tern/ui'`. It is self-contained (no paths
  into `apps/desktop`), so it can be copied out of the repository.
- It is written by hand rather than generated from the app's sources, so internals don't leak
  into it. The app checks itself against it: `scripts/check-plugin-api-dts.js` gains a check that
  `src/packages/api/tern.ts` and `tern/ui.ts` are assignable to the declared modules, and
  `npm run typecheck` runs it.
- The desktop links it with a `file:` dependency, like `@tern/email-dark`, so bundled plugins
  are type-checked against the same contract third-party plugins see.
- `examples/plugin-starter/` (next to `examples/service-plugin/`): a minimal plugin with
  `manifest.json`, `src/main.ts`, `styles.css`, an esbuild script with the externals of 2.3, and
  a `tsconfig.json` pointing at the types.
- Third-party authors get the types without npm (decision 4).

## 11. Naming

| v1 (stays until 2.0.0) | v2 |
|---|---|
| `mailspring-exports`, `mailspring-store`, `mailspring-observables` | `tern` |
| `mailspring-component-kit` | `tern/ui` |
| `window.$m` | `window.tern` |
| `mailspring://<package>/<path>` | `tern-asset://<id>/<path>`, and `this.assetUrl(path)` so plugins don't write the scheme. Same handler and load paths (`src/main/protocols/asset.ts`); the content security policies have to allow both. `tern://` is taken by the OS deep links, `tern-file:` by attachments |
| `engines.mailspring` in `package.json` | `minApiVersion` in `manifest.json`; `engines.tern` accepted in a v1 `package.json` |
| `tern.engineMethods`, `tern.service` in `package.json` | `engineMethods`, `service` in `manifest.json` |
| `activate`/`deactivate`/`serialize` exports | `class extends Plugin`, `onload`/`onunload` |
| `styles/*.less` | `styles.css` |

The old names are aliases, not copies: `mailspring://` and `tern-asset://` are one handler, and
`$m` and `window.tern` are different objects only because they are different APIs.

## 12. Versioning

- **1.35.0** adds v2: `tern`, `tern/ui`, `Plugin`, the manifest loader, `tern-asset:`,
  `window.tern`, `engines.tern`, v2 themes, the types package and the starter. Nothing in v1
  changes and nothing warns. The flat kit exports already in `mailspring-component-kit`
  (`Button`, `DropdownMenuRoot`, …) have no CHANGELOG entry or version line yet; they go in the
  same 1.35.0 entry unless a release ships before v2 does (decision 2).
- **1.35.x–1.3x**: each new v2 method or `tern` export is a minor bump with a CHANGELOG entry,
  as now. Moving the bundled plugins changes no API and needs no bump.
- **Deprecation**: once every bundled plugin is on v2 and the starter is published, the v1 names
  warn once per package through the existing `deprecate()` in `mailspring-exports.js`, and
  Preferences > Plugins marks v1 plugins "uses plugin API 1, which Tern 2.0 removes" (decision 1).
- **2.0.0**, at least six months after deprecation starts (the existing rule is one minor release
  or six months): v1 is removed. A package without `manifest.json` is listed and not loaded, with
  that reason, whatever its `engines` range says (finding 4). Removed with it: the v1 barrels and
  `$m`, `mailspring://`, `package.json` loading, `styles/` LESS for plugins, LESS-variables themes
  and the compat path, the legacy kit classes, the slots of section 1.6 as plugin slots, and the
  removals 2.0.0 already lists (`MailsyncProcess`, `KeyManager`, `MailRulesProcessor`,
  `IMAPSearchQueryBackend`, `QuerySubscriptionPool` internals, `MailspringTestUtils`,
  `ICSEventHelpers`, the deprecated `CalendarUtils` helpers).
- A v2 plugin loads for `>=minApiVersion <3.0.0`: 2.0.0 removes v1 only, so a plugin written for
  1.35.0 keeps loading on 2.x. A future change that breaks v2 is 3.0.0.

## 13. Migrating the bundled plugins

Each bundled plugin moves to `manifest.json` (id = its current name, so metadata ids don't
change), `class extends Plugin`, imports from `tern`/`tern/ui`, and `styles.css` in place of
`styles/` (their stylesheets are already plain CSS reading tokens and `static/styles/variables.css`). Deep imports are replaced where v2 has the API; what's left
stays an in-tree import, since bundled plugins ship with the app. One commit per plugin, each
with its specs green, in this order (smallest surface first):

| Plugin | Today | Becomes | Left over |
|---|---|---|---|
| phishing-detection | 1 role, `MessageStore` | `addThreadHeader`, `app.messages` | |
| thread-sharing | 1 role | `addThreadToolbarButton` | |
| participant-profile | 1 role ×2, `localStorage` | `addContactSidebarSection` ×2, `cache` | `intl/date-format` |
| open-tracking | 3 roles, metadata toggle | `addComposerAction`, `addThreadListIcon`, `addMessageHeaderItem`, `metadata` | |
| link-tracking | 1 role, MessageView | `addComposerAction`, `addMessageViewExtension` | `MessageBody` becomes a `tern` type |
| translation | 2 roles, MessageView, config, `localStorage` | `addComposerAction`, `addMessageBanner`, `addMessageViewExtension`, `settings`, `cache` | `messageFrameContent` |
| composer-templates | 2 roles, Composer ext, prefs tab | `addComposerAction`, `addComposerFooter`, `addComposerExtension`, `addSettingsTab` | |
| composer-grammar-check | 1 role, action listeners, config | `addComposerAction`, `onAction`, `app.config` | `GrammarCheckPluginAPI` (tier D, bundled-only) |
| send-later | 2 roles, `DatabaseStore.listen`, engine deep imports | `addComposerAction`, `addDraftStatus`, `onMetadataExpired`, `engine.has` | `SendActionsStore` |
| thread-snooze | 3 roles, AccountSidebar, engine deep imports | example in 3.5 | `engine/resurfacing` texts |
| send-reminders | 5 roles, ThreadList + AccountSidebar exts, engine deep imports | `addComposerAction`, `addThreadToolbarButton`, `setThreadListTimestamp`, `setMessageHeaderNotice`, `addThreadHeader`, `addThreadListClasses`, `addSidebarItem`, `metadata.scope` | `engine/resurfacing` texts |
| ai | 3 roles, prefs tab, AccountSidebar, `Engine.events` | `addThreadHeader`, `addComposerAction`, `addThreadLabel`, `addSettingsTab`, `addSidebarItem` (its disposable replaces the unregister/register refresh), `engine.onDidChange` | `components/settings-select` until `tern/ui` has it |
| activity | sheet + location, toolbar button, AccountSidebar, saved state | `registerSheet`, `addSidebarToolbarButton`, `addSidebarItem` | `kit/keys` |

Every v2 method gets a spec before the first plugin uses it: a fixture v2 plugin under
`tests/support/fixtures/packages/` registers into the slot, the spec renders the slot and checks
the component, its props and the wrapper classes, then unloads the plugin and checks it's gone.
These double as the "spec that renders it" for each slot that stays.

## 14. Stages (for when this is implemented)

1. `packages/plugin-api` contract and the assignability check.
2. Loader: manifest detection, `Plugin`, disposal, the slot and behaviour methods over the
   registries, `tern`/`tern/ui`, `window.tern`, `tern-asset:`, `styles.css`; fixture specs.
3. Engine binding, the v2 approval flow, Preferences > Plugins rows (errors, methods, on/off).
4. `settings`, `cache`, `dataDir`, `metadata`, the event methods.
5. v2 themes and token previews.
6. Dev mode: source mains, `styles.less`, reload on change.
7. Starter, `docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md` v2 section, CHANGELOG 1.35.0, `PLUGIN_API_VERSION`.
8. Bundled plugin migration (section 13).
9. Deprecation warnings; 2.0.0 later.

## 15. Decisions

Settled with the user on 2026-10-20.

1. **When v1 starts warning.** When the last bundled plugin is on v2 and the starter is out, not
   in 1.35.0. Warning before there is a migration path only adds noise.
2. **Version of the kit exports already in main.** They go into the 1.35.0 entry with v2. They get
   1.35.0 of their own (and v2 1.36.0) only if a release goes out first.
3. **Curated or full `tern` module.** Curated (section 4), grown on request, with
   `mailspring-exports` covering the rest until 2.0.0. Re-exporting all of v1 under new names
   would make every store and internal a typed promise kept through 3.0.0.
4. **How third-party authors get the types**, given the package isn't published to npm. Each
   GitHub release attaches `tern-plugin-api-<apiVersion>.tgz` (`npm pack` of
   `packages/plugin-api`); `npm install <url>` works with it and the starter pins one.
5. **Nav rail entries for plugin sheets.** No plugin uses `NavRail:Item` today; activity reaches
   its sheet from the account sidebar. `registerSheet` offers `navRail`, off by default, so a
   plugin with a screen of its own (activity, a todo list) can sit next to Mail and Calendar.
6. **Re-asking engine-method approvals on update.** Only methods an update adds to
   `engineMethods` ask again, not every update or hash change; Preferences > Plugins already
   shows the install hash.
7. **Where v2 settings live.** `config.json` under `plugins.<id>` (cross-window sync, reset and
   import already handle it), not an Obsidian-style `data.json` per plugin.
8. **Slots nobody uses (section 1.6).** No v2 methods for them now; at 2.0.0 they stop being
   plugin slots and features render their parts directly. The empty ones that look useful
   (`MessageActionMenuItem`, `MessageFooterStatus`, the calendar banners, search providers) get a
   v2 method in a later minor when a plugin asks.
9. **Module name.** `tern` and `tern/ui`, short like `obsidian`. Nothing in the desktop's
   `node_modules` is called `tern`; the npm package of that name (a JS code-analysis tool) would
   only matter to a plugin that bundles it, which esbuild's external list would then have to
   exclude by path.

### Open question: "sync now" from a plugin

Answered: no engine addition is needed. The socket method `sync.wake` (`accountId?`, none = all
accounts; `method::SYNC_WAKE` in `crates/tern-ipc/src/lib.rs`, handled in
`apps/engine/src/socket.rs`, open class in `crates/tern-ipc/src/policy.rs`) triggers a sync now,
and `sendSyncMailNow()` already uses it. `refresh-on-focus` moves to
`this.engine.request('sync.wake', {})` (section 6).
