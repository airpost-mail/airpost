# Contributing to Airpost

Bug reports, fixes, plugins and themes are welcome.

## Issues

File bugs and feature requests as GitHub issues on
[airpost/airpost](https://github.com/airpost-mail/airpost/issues); the issue forms ask for what
we need (version, platform, mail provider, engine log lines). Search existing issues, closed
ones included, first. Report security problems privately as described in
[SECURITY.md](SECURITY.md), not in an issue.

## Pull requests

Contributions are accepted under the project's licence, GPL-3.0-only (see LICENSE.md and
NOTICE.md). There is no CLA. New dependencies must be GPL-3.0-compatible (MIT, Apache-2.0,
ISC, BSD, MPL-2.0); CI checks this for crates (`cargo deny`) and npm packages
(`.github/scripts/check-npm-licenses.mjs`). Keep upstream copyright notices. Rust files start
with `// SPDX-License-Identifier: GPL-3.0-only`. Don't add Mailspring branding or calls to
`*.getmailspring.com`.

For anything larger than a fix, open an issue first. New preferences for niche behaviour and
changes to the plugin API need agreement before the code. A feature that can be a plugin or
theme is often better as one.

Before you open a pull request, run `make lint` and the tests for what you touched (below).
Keep a pull request to one change; CI runs the workflows whose paths it touches.

### Commit messages

One line, `area: what changed`, describing the result rather than the work, e.g.
`desktop: an installed on-demand service counts as running in Preferences` or
`airpost-client: snap profile path uses the airpost snap name`. The area is the app or crate
(`desktop`, `engine`, `airpost` for the CLI, `web`, `backend`, `gateway`, `airpost-mcp`, a crate
name), or `docs`, `ci`, `harness`, `packaging`. Add a body when the why isn't obvious.

## Build and run from source

Development happens on macOS and Linux; the Makefile needs bash. On Windows the engine only
has to compile (CI checks it).

You need:

- git, Python 3, and the Node major version in `.node-version` (`make setup` checks it).
- Rust through rustup; `rust-toolchain.toml` pins the version and rustup installs it.
- A C/C++ toolchain and `make` (the Xcode Command Line Tools on macOS, `build-essential` on
  Debian and Ubuntu): `npm ci` rebuilds better-sqlite3 against Electron.
- `cargo install --locked cargo-deny` for `make lint`.
- Docker, optionally: `make stack` and the harness's Docker tier use it.

```bash
git clone https://github.com/airpost-mail/airpost.git
cd airpost
make setup     # npm ci for apps/desktop and apps/web, Python deps in .venv, debug Rust builds
make desktop   # the desktop on a sandbox profile in .dev/profile, with the debug engine
make help      # every target
```

`make desktop` never touches your real profile. For mail to test against, `make mail` runs
fake IMAP and SMTP accounts without Docker (alice and bob @airpost.test, password airpostpass), and
`make stack` runs them in Docker.

The other parts:

- CLI: `make airpost ARGS="status"` runs the debug `airpost` against the dev desktop's engine.
- Web client: `make web` starts the Vite dev server on an in-browser mock engine; see
  [apps/web/README.md](apps/web/README.md) for running it against a real engine.
- Server: `make stack PROFILE=backend` adds the backend to the Docker stack; see
  [apps/backend/README.md](apps/backend/README.md) and [docs/self-hosting.md](docs/self-hosting.md).

| Path | Contents |
|---|---|
| `apps/desktop/` | Electron client; see its README.md and CLAUDE.md |
| `apps/engine/` | Rust sync engine (`airpost-engine`), run by the desktop as `apps/desktop/airpost-engine`; see its CLAUDE.md |
| `apps/cli/`, `apps/mcp/` | The `airpost` command and the MCP server `airpost-mcp` ([docs/mcp.md](docs/mcp.md)) |
| `apps/backend/`, `apps/gateway/` | The Airpost server, and the gateway that serves the web client from an engine |
| `apps/web/` | Web client |
| `crates/`, `packages/` | Shared Rust and TS libraries |
| `docs/` | Design docs and plans; desktop docs in `docs/desktop/` |

## Tests and linting

- `make test`: Rust workspace tests and the engine harness quick tier (fake servers, no
  Docker, a few minutes). One harness case: `.venv/bin/python -m pytest apps/engine/test -n 0
  -k <name>` with `AIRPOST_ENGINE_BIN=target/debug/airpost-engine`; see
  [apps/engine/test/README.md](apps/engine/test/README.md) for tiers and server kinds.
- `make test-desktop`: the desktop's Jasmine specs. `SPEC=<regex>` runs only the spec files
  whose name matches.
- Web client: `npm test`, `npm run lint` and `npm run typecheck` in `apps/web`.
- `make lint`: rustfmt, clippy, cargo deny, the npm licence check, and the desktop and web
  linters. In `apps/desktop`, `npm run lint` fixes what it can and `npm run typecheck` runs
  tsc.
- `make gate`: the merge gate (lint, tests, release builds, the harness on one serve engine).

## Desktop development

`npm start` in `apps/desktop` runs the app with `--dev` on the `Airpost-dev` profile
(`~/Library/Application Support/Airpost-dev`, `%APPDATA%\Airpost-dev`, `~/.config/Airpost-dev`);
`--config-dir-path` picks another. It needs an engine at `apps/desktop/airpost-engine`, which
`make desktop` links. Developer > Toggle Developer Tools opens the console, where `$m` is
`mailspring-exports`. Developer > Reload (`Cmd+Option+L` on macOS, `Ctrl+Alt+L` on Linux,
`Ctrl+Shift+R` on Windows) reloads the window and picks up renderer changes; main-process code
needs a restart. Developer > Open Engine Logs opens the folder with the engine's logs.

The language is set in Preferences > General. Left on automatic, `--lang` picks one:
`npm start -- --lang=de`. Translations are in
`apps/desktop/lang`; see [docs/desktop/LOCALIZATION.md](docs/desktop/LOCALIZATION.md).
Plugins and themes are covered in
[docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md](docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md).

## Code of Conduct

This project is released with a [Code of Conduct](CODE_OF_CONDUCT.md). By participating you
agree to abide by its terms.
