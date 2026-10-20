---
name: plugin-development
description: Context for work on the desktop app's plugin system (apps/desktop) — creating, modifying or debugging plugins and themes, PackageManager/Package loading, discovery and activation, ComponentRegistry/ExtensionRegistry and the other registries, Preferences > Plugins, or anything in app/internal_packages/. Read before changing plugin-related code.
---

# Plugin development

Use this skill when working on code related to the desktop app's plugin system, including:
- Creating, modifying, or debugging plugins/packages
- Working with the PackageManager or Package classes
- Modifying plugin loading, discovery, or activation logic
- Working with ComponentRegistry, ExtensionRegistry, or other registries
- Plugin management UI (Preferences > Plugins)
- Working with themes (which are a type of plugin)
- Modifying code in `apps/desktop/app/internal_packages/`

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
`apps/desktop/app/src/plugin-api-version.ts` and an entry in `apps/desktop/CHANGELOG.md`.

## Key Source Files

Paths are relative to `apps/desktop/`.

| File | Purpose |
|------|---------|
| `app/src/package-manager.ts` | Package discovery, validation, and activation |
| `app/src/package.ts` | Package class representing individual plugins |
| `app/src/app-env.ts` | AppEnv singleton that initializes PackageManager |
| `app/src/plugin-api-version.ts` | The plugin API version matched against `engines.mailspring` |
| `app/src/registries/component-registry.ts` | UI component registration |
| `app/src/registries/extension-registry.ts` | Extension registration |
| `app/src/extensions/composer-extension.ts` | ComposerExtension base class |
| `app/src/extensions/message-view-extension.ts` | MessageViewExtension base class |
| `app/src/components/injected-component.tsx` | Renders registered components |
| `app/internal_packages/theme-picker/` | Theme UI (reference for plugin management UI) |

## Development Guidelines

1. **Plugin Validation**: All plugins must have `engines.mailspring` in package.json
2. **Lifecycle Compliance**: Plugins must export `activate()` and `deactivate()` functions
3. **Clean Deactivation**: Always unregister components/extensions in `deactivate()`
4. **Window Types**: Consider which window types a plugin should load in
5. **Optional Plugins**: Use `isOptional: true` for user-disableable plugins

## Testing Plugin Changes

- `npm start` in `apps/desktop` runs in dev mode on the `Tern-dev` profile; `make desktop` from the repo root runs on `.dev/profile` instead
- Reload the window with Developer > Reload (`Cmd+Option+L` on macOS, `Ctrl+Alt+L` on Linux, `Ctrl+Shift+R` on Windows); `Cmd/Ctrl+R` is Reply
- Access `$m` in dev console for `mailspring-exports`
- User plugins go in `<profile>/packages/`, e.g. `~/Library/Application Support/Tern-dev/packages/` or `~/.config/Tern-dev/packages/` during development
