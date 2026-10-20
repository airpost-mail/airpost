# Mailspring Plugin System Architecture

This document provides a comprehensive overview of the Mailspring plugin system for developers looking to understand, extend, or improve the plugin infrastructure.

## Table of Contents

- [Overview](#overview)
- [Plugin Directory Locations](#plugin-directory-locations)
- [Plugin Structure](#plugin-structure)
- [Plugin Lifecycle](#plugin-lifecycle)
- [Extension Points & Registries](#extension-points--registries)
- [User Plugin Installation](#user-plugin-installation)
- [Theme System](#theme-system)
- [Trust Model](#trust-model)
- [Current Limitations](#current-limitations)
- [Developer Resources](#developer-resources)

---

## Overview

Mailspring uses a robust plugin architecture where features are implemented as "packages". The system is managed by two core files:

| File | Purpose |
|------|---------|
| `src/plugin-host/package-manager.ts` | Discovers, validates, and activates packages |
| `src/plugin-host/package.ts` | Represents individual plugin instances |

The `PackageManager` is instantiated as part of the `AppEnv` singleton during application startup (see `src/window/app-env.ts`).

---

## Plugin Directory Locations

Plugins are discovered from these directories in order:

| Directory | Purpose | Notes |
|-----------|---------|-------|
| `<resourcePath>/src/features/`, `plugins/`, `themes/` | Built-in | Core features (app code, see below), optional plugins (`isOptional`) and the `ui-*` themes; discovered as one name-sorted list |
| `<configDirPath>/packages/` | User-installed plugins | Skipped in safe mode |
| `<configDirPath>/dev/packages/` | Developer mode plugins | Only loaded in dev mode |
| `<resourcePath>/spec/fixtures/packages/` | Test packages | Only in spec mode |

Built-in features are not packages: they are app code in `src/features/<dir>/`, with no
`package.json`, listed in `src/features/index.ts` with their package name, `windowTypes`,
`syncInit` and a lazy `require` of `index.ts`. `PackageManager` wraps each in a
`BuiltinFeature` (`src/plugin-host/builtin-feature.ts`) and merges them by directory name with
the bundled plugins and themes, so they share one activation order and stay in
`AppEnv.packages` under their names. A user package with a feature's name still replaces it.
The directory name is also the `mailspring://<dir>/` host for the feature's assets. Features
are not loaded in spec mode and are never listed in Preferences > Plugins.

### Platform-Specific Config Directories

| Platform | Config Directory |
|----------|------------------|
| Linux | `~/.config/Tern/` |
| macOS | `~/Library/Application Support/Tern/` |
| Windows | `%APPDATA%\Tern\` |

In development mode, data is stored separately (e.g., `~/.config/Tern-dev/` on Linux). `--config-dir-path` overrides both.

---

## Plugin Structure

### Directory Layout

```
my-plugin/
├── package.json          # Required - metadata and configuration
├── lib/
│   └── main.ts          # Required - entry point with activate/deactivate
├── styles/              # Optional - LESS stylesheets (auto-loaded)
│   └── my-plugin.less
├── keymaps/             # Optional - keyboard shortcuts (auto-loaded)
│   └── my-plugin.json
├── menus/               # Optional - menu definitions (auto-loaded)
│   └── my-plugin.json
├── assets/              # Optional - images, sounds, etc.
└── spec/                # Optional - Jasmine test specs
```

### package.json Schema

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "displayName": "My Plugin",
  "description": "What the plugin does",
  "main": "./lib/main",
  "license": "GPL-3.0",

  "engines": {
    "mailspring": "*"
  },

  "windowTypes": {
    "default": true,
    "composer": true,
    "thread-popout": true
  },

  "isOptional": true,
  "isDefault": true,
  "syncInit": false,
  "isHiddenOnPluginsPage": false,

  "theme": false,

  "tern": {
    "engineMethods": ["engine.logs.tail"],
    "service": {
      "bin": { "darwin-arm64": "bin/mac/my-service", "linux-x64": "bin/linux/my-service" },
      "args": ["--quiet"]
    }
  }
}
```

### Field Descriptions

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Package identifier (kebab-case) |
| `version` | Yes | Semantic version |
| `main` | Yes* | Entry point path (required for non-theme plugins) |
| `engines.mailspring` | Yes | Semver range of plugin API versions the plugin works with, matched against `apiVersion` (`PLUGIN_API_VERSION` in `src/plugin-host/plugin-api-version.ts`, 1.27.0). `*` matches any |
| `displayName` | No | Human-readable name for UI |
| `description` | No | Plugin description |
| `windowTypes` | No | Which window types load this plugin |
| `isOptional` | No | If true, can be disabled by user |
| `isDefault` | No | If true, enabled by default |
| `syncInit` | No | If true, loads immediately during startup |
| `theme` | No | Set to `"ui"` for theme plugins |
| `tern.engineMethods` | No | The local-only engine methods the plugin calls through `Engine.request` (see [Engine](#engine)) |
| `tern.service` | No | A background program the OS runs beside the engine (see [Service Plugins](#service-plugins)) |

### Window Types

| Type | Description |
|------|-------------|
| `default` | Main application window |
| `composer` | Composer/draft windows |
| `thread-popout` | Thread popout windows |
| `emptyWindow` | Empty window |
| `all` | All window types |

---

## Plugin Lifecycle

### Entry Point (lib/main.ts)

Every plugin must export `activate()` and `deactivate()` functions:

```typescript
import { ComponentRegistry, ExtensionRegistry } from 'mailspring-exports';
import MyComponent from './my-component';
import MyExtension from './my-extension';

export function activate() {
  // Register UI components
  ComponentRegistry.register(MyComponent, {
    role: 'Composer:ActionButton'
  });

  // Register extensions
  ExtensionRegistry.MessageView.register(MyExtension);
}

export function deactivate() {
  // Clean up all registrations
  ComponentRegistry.unregister(MyComponent);
  ExtensionRegistry.MessageView.unregister(MyExtension);
}

// Optional: Persist state across reloads
export function serialize() {
  return { myState: 'value' };
}
```

### Optional Configuration Support

```typescript
// Define configuration schema
export const config = {
  myOption: {
    type: 'string',
    default: 'value',
    description: 'Description of the option'
  }
};

// Or legacy style
export const configDefaults = {
  myOption: 'value'
};

// Hook called after config is registered
export function activateConfig() {
  AppEnv.config.onDidChange('my-plugin.myOption', (newValue) => {
    // Handle config change
  });
}
```

### Loading Sequence

1. **Discovery**: `PackageManager.discoverPackages()` scans configured directories
2. **Filtering**: Packages filtered by `windowTypes` for current window
3. **Validation**: Checks that `engines.mailspring` is a range `PLUGIN_API_VERSION` satisfies
4. **Sync Loading**: Plugins with `syncInit: true` activate immediately
5. **Delayed Loading**: Other plugins activate after 2.5 second delay
6. **Resource Loading**: Keymaps, stylesheets, menus auto-loaded
7. **Activation**: `activate()` function called on main module

### Activation Flow (package-manager.ts)

```
activatePackages(windowType)
    │
    ├── For each available package:
    │   ├── Check windowTypes match
    │   ├── If syncInit → activatePackage() immediately
    │   └── Else → queue for delayed activation
    │
    └── After 2.5s timeout:
        └── activatePackage() for queued packages

activatePackage(pkg)
    │
    ├── Skip if already active
    ├── Skip if theme (handled separately)
    ├── Skip if optional and disabled in config
    ├── Verify PLUGIN_API_VERSION satisfies engines.mailspring
    │   (missing or unsatisfied: logged and skipped; later packages still load)
    │
    ├── Mark as active
    └── Call pkg.activate()

Package.activate()
    │
    ├── Load keymaps from keymaps/ directory
    ├── Load stylesheets from styles/ directory
    ├── Load menus from menus/ directory
    ├── require(main module)
    ├── Call module.activate()
    └── Register config schema if present
```

---

## Extension Points & Registries

### ComponentRegistry

Injects React components into UI locations:

```typescript
import { ComponentRegistry, WorkspaceStore } from 'mailspring-exports';

// Register with a role
ComponentRegistry.register(MyButton, {
  role: 'Composer:ActionButton'
});

// Register at a location
ComponentRegistry.register(MySidebar, {
  location: WorkspaceStore.Location.MessageListSidebar
});

// Register with mode constraints
ComponentRegistry.register(MyComponent, {
  role: 'ThreadListIcon',
  modes: ['list', 'split']
});
```

**Common Roles:**

| Role | Description |
|------|-------------|
| `Composer` | Main composer component |
| `Composer:ActionButton` | Buttons in composer action bar |
| `Composer:Footer` | Footer area in composer |
| `ThreadActionsToolbarButton` | Thread toolbar buttons |
| `ThreadListIcon` | Icons in thread list items |
| `ThreadListQuickAction` | Quick action buttons |
| `MessageListHeaders` | Headers above message list |
| `MessageHeader` | Message header area |
| `MessageHeaderStatus` | Status in message header |
| `MessageFooterStatus` | Status in message footer |
| `DraftList:DraftStatus` | Draft status indicators |
| `message:BodyHeader` | Headers within message body |

**Common Locations:**

| Location | Description |
|----------|-------------|
| `WorkspaceStore.Location.RootSidebar` | Left sidebar |
| `WorkspaceStore.Location.RootSidebar.Toolbar` | Sidebar toolbar |
| `WorkspaceStore.Location.ThreadList` | Thread list area |
| `WorkspaceStore.Location.MessageList` | Message list area |
| `WorkspaceStore.Location.MessageListSidebar` | Sidebar next to messages |
| `WorkspaceStore.Location.Center` | Main content area |

### ExtensionRegistry

Register behavioral extensions:

```typescript
import { ExtensionRegistry } from 'mailspring-exports';

// Available extension registries:
ExtensionRegistry.Composer.register(MyComposerExtension);
ExtensionRegistry.MessageView.register(MyMessageExtension);
ExtensionRegistry.ThreadList.register(MyThreadListExtension);
ExtensionRegistry.AccountSidebar.register(MySidebarExtension);
```

#### ComposerExtension

```typescript
class MyComposerExtension extends ComposerExtension {
  // Add custom send actions (e.g., "Send Later")
  static sendActions() {
    return [{
      title: 'Send Later',
      iconUrl: 'mailspring://my-plugin/assets/clock.png',
      isEnabled: ({ draft }) => true,
      performSendAction: ({ draft }) => { /* ... */ }
    }];
  }

  // Display warnings before sending
  static warningsForSending({ draft }) {
    return ['Warning message'];
  }

  // Modify new drafts
  static prepareNewDraft({ draft }) {
    draft.body = draft.body + '<signature>';
  }

  // Transform draft before sending
  static applyTransformsForSending({ draft, draftBodyRootNode }) {
    // Add tracking pixels, etc.
    return draft;
  }
}
```

#### MessageViewExtension

```typescript
class MyMessageExtension extends MessageViewExtension {
  // Modify message HTML before rendering
  static formatMessageBody({ message }) {
    message.body = processHTML(message.body);
  }

  // Work on the rendered body. Since plugin API 1.27.0 the body renders in a sandboxed,
  // cross-origin frame, so there is no `document`: `body` (a MessageBody) does the DOM work.
  static async renderedMessageBodyIntoDocument({ body, message, iframe }) {
    const links = await body.querySelectorAll('a[href^="https://tracker.example"]');
    for (const link of links) {
      await body.setAttributes(link.ref, { title: 'Tracked link' });
      await body.insertElement(link.ref, 'afterend', 'span', { class: 'tracked' }, ' (tracked)');
    }
  }

  // Filter which attachments to display
  static filterMessageFiles({ message, files }) {
    return files.filter(f => !f.filename.endsWith('.sig'));
  }
}
```

`MessageBody` (`src/message-frame/message-frame-bridge.ts`):

| Method | Does |
|--------|------|
| `querySelectorAll(selector)` | Elements matching a CSS selector: `{ref, tagName, attributes, text}` |
| `setAttributes(ref, attributes)` | Sets attributes (`null` removes one); event handler attributes are refused |
| `insertElement(ref, position, tagName, attributes?, text?)` | Adds an element and returns its ref; scripts, frames, forms, styles and document-level elements are refused |
| `getBoundingClientRect(ref)` | The element's box in app window coordinates, for popovers |
| `addEventListener(ref, 'click' \| 'mousedown', cb)` | Calls `cb({rect})` when the element is clicked |

Every call returns a promise. Refs are valid until the message renders again; calls after that
reject. Reading `document` in this hook throws, and the app warns once per extension. This is a
breaking change for plugins written against 1.26.0 that read `document`, although 1.27.0 is a
minor version.

### Engine

`Engine` (in `mailspring-exports`, `src/engine/engine-api.ts`) is the plugin API's way to the sync
engine. Each window has its own connection to the engine; requests made while it is down reject.

| Member | Does |
|--------|------|
| `Engine.request(method, params)` | Calls a socket method on the window's connection (reads, `query`, `metadata.list`, `integrations.*`) and resolves with the result, or rejects with the engine's error (`code`, `message`, `data`) |
| `Engine.subscribe(filter, cb)` | Calls `cb` for each notification on the connection except deltas (use `DatabaseStore.listen` for those). `filter` is `{methods?, accounts?}`; returns a disposable |
| `Engine.features` / `Engine.has(feature)` | The features the engine's `hello` listed; gate on these, not on versions |
| `Engine.capabilities` | What the engine can do now, e.g. `capabilities.integrations.translation.available`; null while disconnected or from an older engine |
| `Engine.connected` | Whether the window's connection is up |
| `Engine.onDidChange(cb)` | Calls `cb` when the engine connects, goes away or its capabilities change; returns a disposable |

```typescript
import { Engine } from 'mailspring-exports';

let sub;

export function activate() {
  sub = Engine.subscribe({ methods: ['mail.new'] }, ({ params }) => {
    console.log('new mail in', params.accountId);
  });
}

export function deactivate() {
  sub.dispose();
}
```

`hello` and `subscribe` belong to the window's connection and are refused by `Engine.request`.
The local-only methods (accounts, secrets, credentials, `server.*`, `engine.logs.tail`,
`engine.setLogLevel`, `engine.resetDatabase`, `engine.shutdown`) are refused unless the
plugin's package.json lists the method in `tern.engineMethods` and the user allowed it: the
first call asks, a yes is kept in `core.plugins.engineMethodApprovals`, a no lasts until the
app restarts. The calling plugin is found from the call's stack; a call from code outside the
app that belongs to no installed package is refused.

### Service Plugins

Logic that must run while the desktop is closed (send something at a time, act on new mail)
goes in a service: a separate program on the engine socket, like `tern` and `tern-mcp`, run
by launchd (macOS) or systemd `--user` (Linux). No plugin code runs inside the engine.

- `tern.service.bin` is a path inside the package, or an object keyed by Node's
  `<platform>-<arch>` (`darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`).
  `tern.service.args` is an optional list of arguments.
- Preferences > Plugins offers "Install service…", which runs the bundled
  `tern service install <package>`. Before anything is written it shows the binary, its
  SHA-256, the copy it will run and the exact unit text; the install then refuses a binary
  whose hash changed in between. `tern service uninstall <package>` removes it.
- Installing copies that one file to `<profile>/services/<package>/` and the unit runs the
  copy, so the binary must be self-contained (a compiled program, or a single script). An
  update of the plugin changes nothing until the service is installed again. Source, binary,
  hash and unit are recorded in `<profile>/services.json` (`tern service list`).
- The unit sets `TERN_CONFIG_DIR` (the profile) and `TERN_SERVICE_PACKAGE` (the package name,
  which is also the plugin id for its metadata). The engine unit (`serve --detached`) is
  installed first when it is missing; on systemd the service starts after it, on launchd it
  has no start order, so a service waits for the engine's socket and reconnects when the
  engine restarts. Units restart after a failed exit, not after a clean one.
- Due work comes from `metadata-expiration` deltas for the plugin's id (sent on every scan
  until the expiration is cleared), new mail from `mail.new`, lists from `metadata.list`.
  `crates/tern-client` finds and talks to the engine for Rust services.
- `examples/service-plugin` in the repo is a complete sample.

### Other Registries

| Registry | Purpose | Key Methods |
|----------|---------|-------------|
| `CommandRegistry` | Keyboard commands | `add()`, `dispatch()` |
| `SoundRegistry` | Notification sounds | `register()`, `playSound()` |
| `ServiceRegistry` | Service injection | `registerService()`, `withService()` |
| `DatabaseObjectRegistry` | Custom data models | `register()`, `deserialize()` |

---

## User Plugin Installation

### Current Implementation

The `PackageManager` provides installation functionality (see `package-manager.ts:152-179`):

```typescript
// Opens file dialog, validates, copies to packages/, activates
AppEnv.packages.installPackageManually();

// Opens plugin starter template repo
AppEnv.packages.createPackageManually();
```

**Access Methods:**
- Developer console: `$m.packages.installPackageManually()`
- Command: `window:install-package` (not in any menu by default)

### Manual Installation

1. Download/create plugin folder with valid `package.json`
2. Copy to `~/.config/Tern/packages/` (or platform equivalent)
3. Restart Tern (or call `installPackageFromPath()`)

### Validation Requirements

- Must have `package.json` file
- Must have `engines.mailspring` field
- Main entry point must exist (for non-themes)

---

## Theme System

Themes are plugins with special handling:

```json
{
  "name": "my-theme",
  "theme": "ui",
  "displayName": "My Theme",
  "engines": { "mailspring": "*" }
}
```

### Theme Picker

Located in `src/features/theme-picker/`:
- Accessible via Preferences > Appearance > "Change Theme..."
- Lists all discovered theme packages
- Supports live switching without restart
- Only one theme active at a time

### Built-in Themes

- `ui-light` (base theme)
- `ui-dark`
- `ui-darkside`
- `ui-taiga`
- `ui-less-is-more`

---

## Trust Model

Plugins are fully trusted code. They run in the app's windows with Node and can reach
everything the app can: files, the network, the engine socket, the user's mail. Nothing in the
plugin API is a security boundary. The `tern.engineMethods` check and its prompt only keep a
plugin from calling a method that writes accounts or secrets or touches the engine by accident
or without the user knowing (docs/tech-debt.md TD29). Install only plugins you trust.

A service plugin is a program that runs at login with the user's rights and the engine's
local methods, whether Tern is open or not. Preferences > Plugins shows what will run before
it installs it and records each plugin's install source and hash, but that is information,
not a sandbox.

---

## Current Limitations

| Limitation | Impact |
|------------|--------|
| Plugin management UI | Preferences > Plugins lists third-party plugins with their install source and hash, installs plugins and services; no enable/disable yet |
| No marketplace/store | No central discovery for community plugins |
| No enable/disable UI | Plugins require config file editing (themes have UI) |
| No update mechanism | Manual updates only |
| No sandboxing | Plugins have full API access |
| Limited version checking | `engines.mailspring: "*"` is common; the range is checked against the plugin API version, which only moves with the API |

### Disabled Plugins Configuration

Optional plugins can be disabled via config:

```json
{
  "core": {
    "disabledPackages": ["plugin-name-1", "plugin-name-2"]
  }
}
```

---

## Developer Resources

### Global APIs

**mailspring-exports:**
- `Actions` - Flux action dispatcher
- `DatabaseStore` - Read-only database queries
- `Stores` - Application state (DraftStore, MessageStore, TaskQueue, etc.)
- `Models` - Data models (Message, Thread, Contact, Account, etc.)
- `Tasks` - Async operations (SendDraftTask, ChangeLabelsTask, etc.)
- All registries

**mailspring-component-kit:**
- `InjectedComponent`, `InjectedComponentSet`
- `Modal`, `Menu`, `Flexbox`, `Spinner`
- `KeyCommandsRegion`, `ListensToFluxStore`

### Debugging

- Hot reload: `Ctrl+R` (Windows/Linux) or `Cmd+R` (macOS)
- Dev tools: Menu > Developer > Toggle Developer Tools
- Console access: `$m` provides `mailspring-exports`

### External Resources

- Plugin Starter: https://github.com/Foundry376/Mailspring-Plugin-Starter
- Theme Starter: https://github.com/Foundry376/Mailspring-Theme-Starter
- Community: https://community.getmailspring.com/

---

## Key Source Files Reference

| File | Purpose |
|------|---------|
| `src/plugin-host/package-manager.ts` | Package discovery and activation |
| `src/plugin-host/package.ts` | Package class implementation |
| `src/plugin-host/builtin-feature.ts` | Built-in feature, held like a package |
| `src/features/index.ts` | The built-in feature list |
| `src/window/app-env.ts` | AppEnv singleton, initializes PackageManager |
| `src/registries/component-registry.ts` | UI component registration |
| `src/registries/extension-registry.ts` | Extension registration |
| `src/extensions/composer-extension.ts` | ComposerExtension base class |
| `src/extensions/message-view-extension.ts` | MessageViewExtension base class |
| `src/components/injected-component.tsx` | Renders registered components |
| `src/plugin-api/mailspring-exports.ts` | Global API exports |
| `src/features/theme-picker/` | Theme selection UI (reference for plugin UI) |
