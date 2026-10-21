# Tern desktop

The Electron client of Tern (React 17, TypeScript and LESS compiled at runtime, no bundler),
forked from Mailspring. Mail sync runs in the Rust engine (`apps/engine`), which the app
starts as `tern-engine` from this directory.

## Run from source

Install the Node version in the repo's `.node-version` and the Rust toolchain in
`rust-toolchain.toml`. From the repo root:

```bash
make setup     # npm ci here and in apps/web, debug builds of the Rust binaries
make desktop   # links the debug engine, tern and tern-mcp into this dir, starts on .dev/profile
```

Or from this directory, with an engine installed as `tern-engine`
(`../engine/packaging/build-release.sh --install-desktop` builds and copies one):

```bash
npm ci
npm start      # --dev: runs on the Tern-dev profile
npm start -- --config-dir-path=/tmp/tern-profile
```

## Commands

```bash
npm run lint:check   # eslint with prettier as a rule; `npm run lint` fixes
npm run typecheck
npm test             # Jasmine specs; needs ./tern-engine
npm test -- --spec-file-pattern='calendar'
npm run build        # package for the host platform into dist/
```

`make test-desktop` from the repo root runs the specs too.

## Layout

- `src/` holds the core, one directory per domain: main process in `main/`, code both processes load in `shared/`, config in `config/`, window startup in `window/`, the windows' engine client in `engine/`, stores, models and tasks in `data/`, message content in `mail/`, shared UI in `components/`, package loading and the plugin API barrels (`packages/api/`) in `packages/`.
- `src/features/` holds the built-in features (always loaded, app code); `plugins/` (bundled, can be turned off) and `themes/` (`ui-*`) hold the bundled packages.
- `vendor/` holds the quick-preview libraries (pdf.js, Prism, SheetJS, DOMPurify).
- `tests/` holds the tests (`tests/<area>/*.test.ts`, helpers and fixtures in `tests/support/`).
- `build/`, `scripts/` and `snap/` hold the build and dev tooling; they don't ship.
- One `package.json`: runtime deps in `dependencies` (native ones rebuilt for Electron by the
  postinstall), tooling in `devDependencies`.

## Docs

- [CLAUDE.md](CLAUDE.md): architecture, the engine protocol, conventions.
- [docs/desktop/](../../docs/desktop/): the plugin system, theme tokens, localization, plans.
- [CHANGELOG.md](CHANGELOG.md): the plugin API log.
- [CONTRIBUTING.md](../../CONTRIBUTING.md) and [SECURITY.md](../../SECURITY.md) at the repo root.

Licensed under GPL-3.0-only; see the root LICENSE.md and NOTICE.md.
