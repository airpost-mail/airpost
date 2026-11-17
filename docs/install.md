# Installing Airpost

Airpost runs on macOS and Linux and comes in two parts, installed separately:

- **The desktop app** (Electron, `apps/desktop`): mail, calendar and contacts in a window. It
  bundles its own engine and the `airpost` command (Preferences > General links it onto your
  PATH).
- **The command-line programs**, for machines without the desktop or for running the engine
  on its own: `airpost` (the CLI), `airpost-engine` (the sync engine), `airpost-mcp` (the MCP server
  for AI agents, `docs/mcp.md`) and `airpost-gateway` (serves the web client, `apps/gateway`),
  plus the built web client in `share/airpost-gateway/web`. For an agent that uses the CLI
  instead of MCP, `airpost skill install` adds its skill to Claude Code and Codex; the CLI has
  full access to your mail, as you do.

Not yet released: the commands below need the first `vX.Y.Z` release (`.github/workflows/release.yml`)
and the `airpost/homebrew-tap` repository. Until then, build from source (`CONTRIBUTING.md`)
or build the archives locally (see the end of this page).

## The command-line programs

### Install script (macOS, Linux)

    curl -fsSL https://github.com/airpost-mail/airpost/releases/latest/download/install.sh | sh

It picks the archive for your OS and CPU, checks it against the release's `SHA256SUMS`, and
installs to `~/.local/bin` and `~/.local/share`. Options, after `sh -s --`:

| | |
|---|---|
| `--version 0.2.0` | a given release (default: the newest) |
| `--system` | `/usr/local`, with sudo |
| `--prefix DIR` | `DIR/bin` and `DIR/share` |
| `--uninstall` | remove what the script installed in that prefix |

Upgrade by running it again, then `airpost engine restart`.

### Homebrew (macOS, Linux)

    brew install airpost/tap/airpost

The formula has no `brew services` entry, on purpose: `airpost engine install` writes and owns
the engine's LaunchAgent or systemd unit, and a second unit from Homebrew would start a second
engine on the same profile. After `brew upgrade airpost`, run `airpost engine install` and `airpost
engine restart`.

### Debian, Ubuntu (.deb) and Fedora, RHEL, openSUSE (.rpm)

Download `airpost-cli_<version>-1_<amd64|arm64>.deb` or `airpost-cli-<version>-1.<x86_64|aarch64>.rpm`
from the release, then:

    sudo apt install ./airpost-cli_*.deb
    sudo dnf install ./airpost-cli-*.rpm

The package is `airpost-cli`, so it installs next to the desktop app's `airpost` package. It puts the
programs in `/usr/bin`, the web client in `/usr/share/airpost-gateway/web` and the notices in
`/usr/share/doc/airpost-cli`, and ships no systemd units: `airpost engine install` writes your user
unit. Needs glibc 2.31 or newer (Debian 11, Ubuntu 20.04, RHEL 9).

### Arch Linux (AUR)

    yay -S airpost-cli-bin

### Nix

    nix profile install github:airpost/airpost
    nix run github:airpost/airpost -- status

Built from source by `flake.nix` (`distribution/nix/package.nix`).

### Archives

`airpost-<version>-<target>.tar.gz` for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`universal-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, with
`SHA256SUMS` next to them. Each unpacks to `bin/`, `share/airpost-gateway/web/` and
`share/doc/airpost/` (`LICENSE.md`, `NOTICE.md`, `TRADEMARKS.md` and the third-party notices).
Linux debug info is in `-debug.tar.gz`.

## After installing

    airpost engine install                # the engine in the background, started at login
    airpost account add you@example.com
    airpost status

On a machine without a desktop keyring (a server), keep the engine's secrets in a key file
instead, and let the user's units run without a login session:

    airpost engine install --key-file ~/.config/airpost-engine.key
    loginctl enable-linger "$USER"

The web client:

    airpost-gateway serve --web-root <prefix>/share/airpost-gateway/web
    airpost-gateway pair

with `<prefix>` `~/.local`, `/usr`, `$(brew --prefix)` or the Nix profile. Remote access and
TLS are in `apps/gateway/README.md`.

## The desktop app

- macOS: `brew install --cask airpost/tap/airpost`, or `Airpost-AppleSilicon.zip` / `Airpost.zip`
  (Intel) from the release. (`--cask` is needed: the formula has the same name.)
- Linux: the `airpost` .deb or .rpm, or the snap.

Windows isn't supported yet: there is no Windows build of the engine, and its socket is a
Unix socket only.

## Building the archives yourself

    apps/engine/packaging/build-release.sh --gateway     # needs cargo-about for the notices
    scripts/build-web.sh target/release-web
    scripts/archive.sh --dist target/engine-dist --web target/release-web aarch64-apple-darwin
    AIRPOST_DOWNLOAD_URL=file://$PWD/target/release-archives sh distribution/install.sh --prefix /tmp/airpost

`distribution/linux/build-packages.sh` makes the .deb and .rpm from Linux archives (nfpm).
`scripts/bump.sh` sets a release's version and checksums in the Homebrew formula and cask
(`distribution/homebrew/`) and the PKGBUILD (`distribution/aur/`).
