---
name: plugin-development
description: Context for work on the desktop app's plugin system (apps/desktop) — creating, modifying or debugging plugins and themes, PackageManager/Package loading, discovery and activation, ComponentRegistry/ExtensionRegistry and the other registries, Preferences > Plugins, or anything in apps/desktop/src/features/, plugins/ or themes/. Read before changing plugin-related code.
---

# Plugin development

Use this skill when working on code related to the desktop app's plugin system, including:
- Creating, modifying, or debugging plugins/packages
- Working with the PackageManager or Package classes
- Modifying plugin loading, discovery, or activation logic
- Working with ComponentRegistry, ExtensionRegistry, or other registries
- Plugin management UI (Preferences > Plugins)
- Working with themes (which are a type of plugin)
- Modifying code in `apps/desktop/src/features/`, `plugins/` or `themes/`

## Required Reading

Before making changes to plugin-related code, read the plugin system architecture
documentation at `docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md` (repo root). It covers:
- Plugin directory locations and discovery
- Plugin structure and package.json schema
- Plugin lifecycle (activate/deactivate)
- Extension points and registries (ComponentRegistry, ExtensionRegistry, etc.)
- The `Engine` export, `tern.engineMethods` and service plugins
- User plugin installation mechanisms and the trust model
- Theme system implementation

Changes to the plugin API also need a `PLUGIN_API_VERSION` bump in
`apps/desktop/src/packages/api-version.ts` and an entry in `apps/desktop/CHANGELOG.md`.

## Key Source Files

Paths are relative to `apps/desktop/`.

| File | Purpose |
|------|---------|
| `src/packages/manager.ts` | Package discovery, validation, and activation |
| `src/packages/package.ts` | Package class representing individual plugins |
| `src/packages/builtin-feature.ts` | A built-in feature held in PackageManager like a package |
| `src/packages/resources.ts` | Keymap, menu and stylesheet loading shared by both |
| `src/features/index.ts` | The built-in feature list: name, window types, `syncInit`, lazy `require` |
| `src/window/app-env.ts` | AppEnv singleton that initializes PackageManager |
| `src/packages/api-version.ts` | The plugin API version matched against `engines.mailspring` |
| `src/packages/component-registry.ts` | UI component registration |
| `src/packages/extension-registry.ts` | Extension registration |
| `src/packages/composer-extension.ts` | ComposerExtension base class |
| `src/packages/message-view-extension.ts` | MessageViewExtension base class |
| `src/components/injected/component.tsx` | Renders registered components |
| `src/features/theme-picker/` | Theme UI (reference for plugin management UI) |

## Development Guidelines

1. **Plugin Validation**: All plugins must have `engines.mailspring` in package.json
2. **Lifecycle Compliance**: Plugins must export `activate()` and `deactivate()` functions
3. **Clean Deactivation**: Always unregister components/extensions in `deactivate()`
4. **Window Types**: Consider which window types a plugin should load in
5. **Optional Plugins**: Use `isOptional: true` for user-disableable plugins
6. **Built-in Features**: A feature in `src/features/<dir>/` has no package.json; add it to `src/features/index.ts` (and to the order in `tests/packages/builtin-features.test.ts`). Its directory name is the `mailspring://` asset host and its place in the activation order, which features share with the bundled plugins

## Testing Plugin Changes

- `npm start` in `apps/desktop` runs in dev mode on the `Tern-dev` profile; `make desktop` from the repo root runs on `.dev/profile` instead
- Reload the window with Developer > Reload (`Cmd+Option+L` on macOS, `Ctrl+Alt+L` on Linux, `Ctrl+Shift+R` on Windows); `Cmd/Ctrl+R` is Reply
- Access `$m` in dev console for `mailspring-exports`
- User plugins go in `<profile>/packages/`, e.g. `~/Library/Application Support/Tern-dev/packages/` or `~/.config/Tern-dev/packages/` during development
