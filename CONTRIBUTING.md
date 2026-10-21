# Contributing to Tern

Bug reports, fixes, plugins and themes are welcome.

## Issues

File bugs and feature requests as GitHub issues on
[ternmail/ternmail](https://github.com/ternmail/ternmail/issues). Search existing issues,
closed ones included, first. For a bug, give the version or commit, the platform, the mail
provider, and steps to reproduce. Report security problems privately as described in
[SECURITY.md](SECURITY.md), not in an issue.

## Pull requests

Contributions are accepted under the project's licence, GPL-3.0-only (see LICENSE.md and
NOTICE.md). There is no CLA. New dependencies must be GPL-3.0-compatible (MIT, Apache-2.0,
ISC, BSD, MPL-2.0). Keep upstream copyright notices. Rust files start with
`// SPDX-License-Identifier: GPL-3.0-only`.

For anything larger than a fix, open an issue first. New preferences for niche behaviour and
changes to the plugin API need agreement before the code. A feature that can be a plugin or
theme is often better as one.

Before you open a pull request, run `make lint` and the tests for what you touched (below).

## Build and run from source

You need git, the Node version in `.node-version`, Rust (`rust-toolchain.toml` pins the
version; rustup installs it), and Python 3. Docker is optional; the local mail stack uses it.
`npm ci` rebuilds better-sqlite3 against Electron, which needs a C/C++ toolchain and `make`
(the Xcode Command Line Tools on macOS).

```bash
git clone https://github.com/ternmail/ternmail.git
cd ternmail
make setup     # npm ci for apps/desktop and apps/web, Python deps, debug Rust builds
make desktop   # the desktop on a sandbox profile in .dev/profile, with the debug engine
make help      # every target
```

`make desktop` never touches your real profile. For mail to test against, `make mail` runs
fake IMAP and SMTP accounts without Docker, and `make stack` runs them in Docker.

| Path | Contents |
|---|---|
| `apps/desktop/` | Electron client; see its README.md and CLAUDE.md |
| `apps/engine/` | Rust sync engine (`tern-engine`), run by the desktop as `apps/desktop/tern-engine` |
| `apps/cli/`, `apps/mcp/` | The `tern` command and the MCP server `tern-mcp` |
| `apps/backend/`, `apps/gateway/` | The Tern server |
| `apps/web/` | Web client |
| `crates/`, `packages/` | Shared Rust and TS libraries |
| `docs/` | Plans and design docs; desktop docs in `docs/desktop/` |

## Tests and linting

- `make test`: Rust workspace tests and the engine harness quick tier.
- `make test-desktop`: the desktop's Jasmine specs. `SPEC=<regex>` runs only the spec files
  whose name matches.
- `make lint`: rustfmt, clippy, cargo deny, and the desktop and web linters. In
  `apps/desktop`, `npm run lint` fixes what it can and `npm run typecheck` runs tsc.
- `make gate`: the merge gate.

## Desktop development

`npm start` in `apps/desktop` runs the app with `--dev` on the `Tern-dev` profile
(`~/Library/Application Support/Tern-dev`, `%APPDATA%\Tern-dev`, `~/.config/Tern-dev`);
`--config-dir-path` picks another. Developer > Toggle Developer Tools opens the console,
where `$m` is `mailspring-exports`. Developer > Reload (`Cmd+Option+L` on macOS,
`Ctrl+Alt+L` on Linux, `Ctrl+Shift+R` on Windows) reloads the window and picks up renderer
changes; main-process code needs a restart.

The language is set in Preferences > General. Left on automatic, `--lang` picks one:
`npm start -- --lang=de`. Translations are in
`apps/desktop/lang`; see [docs/desktop/LOCALIZATION.md](docs/desktop/LOCALIZATION.md).
Plugins and themes are covered in
[docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md](docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md).

## Code of Conduct

This project is released with a [Code of Conduct](CODE_OF_CONDUCT.md). By participating you
agree to abide by its terms.
