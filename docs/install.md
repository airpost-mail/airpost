# Installing Tern

Tern comes in two parts, installed separately:

- **The desktop app** (Electron, `apps/desktop`): mail, calendar and contacts in a window. It
  bundles its own engine and the `tern` command (Preferences > General links it onto your
  PATH).
- **The command-line programs**, for machines without the desktop or for running the engine
  on its own: `tern` (the CLI), `tern-engine` (the sync engine), `tern-mcp` (the MCP server
  for AI agents, `docs/mcp.md`) and `tern-gateway` (serves the web client, `apps/gateway`),
  plus the built web client in `share/tern-gateway/web`.

Not yet released: the commands below need the first `vX.Y.Z` release (`.github/workflows/release.yml`)
and the `ternmail/homebrew-tap` repository. Until then, build from source (`CONTRIBUTING.md`)
or build the archives locally (see the end of this page).

## The command-line programs

### Install script (macOS, Linux)

    curl -fsSL https://github.com/ternmail/ternmail/releases/latest/download/install.sh | sh

It picks the archive for your OS and CPU, checks it against the release's `SHA256SUMS`, and
installs to `~/.local/bin` and `~/.local/share`. Options, after `sh -s --`:

| | |
|---|---|
| `--version 0.2.0` | a given release (default: the newest) |
| `--system` | `/usr/local`, with sudo |
| `--prefix DIR` | `DIR/bin` and `DIR/share` |
| `--uninstall` | remove what the script installed in that prefix |

Upgrade by running it again, then `tern engine restart`.

### Homebrew (macOS, Linux)

    brew install ternmail/tap/tern

The formula has no `brew services` entry, on purpose: `tern engine install` writes and owns
the engine's LaunchAgent or systemd unit, and a second unit from Homebrew would start a second
engine on the same profile. After `brew upgrade tern`, run `tern engine install` and `tern
engine restart`.

### Debian, Ubuntu (.deb) and Fedora, RHEL, openSUSE (.rpm)

Download `tern-cli_<version>-1_<amd64|arm64>.deb` or `tern-cli-<version>-1.<x86_64|aarch64>.rpm`
from the release, then:

    sudo apt install ./tern-cli_*.deb
    sudo dnf install ./tern-cli-*.rpm

The package is `tern-cli`, so it installs next to the desktop app's `tern` package. It puts the
programs in `/usr/bin`, the web client in `/usr/share/tern-gateway/web` and the notices in
`/usr/share/doc/tern-cli`, and ships no systemd units: `tern engine install` writes your user
unit. Needs glibc 2.31 or newer (Debian 11, Ubuntu 20.04, RHEL 9).

### Arch Linux (AUR)

    yay -S tern-cli-bin

### Nix

    nix profile install github:ternmail/ternmail
    nix run github:ternmail/ternmail -- status

Built from source by `flake.nix` (`packaging/nix/package.nix`).

### Archives

`tern-<version>-<target>.tar.gz` for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`universal-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, with
`SHA256SUMS` next to them. Each unpacks to `bin/`, `share/tern-gateway/web/` and
`share/doc/tern/` (`LICENSE.md`, `NOTICE.md`, `TRADEMARKS.md` and the third-party notices).
Linux debug info is in `-debug.tar.gz`.

## After installing

    tern engine install                # the engine in the background, started at login
    tern account add you@example.com
    tern status

On a machine without a desktop keyring (a server), keep the engine's secrets in a key file
instead, and let the user's units run without a login session:

    tern engine install --key-file ~/.config/tern-engine.key
    loginctl enable-linger "$USER"

The web client:

    tern-gateway serve --web-root <prefix>/share/tern-gateway/web
    tern-gateway pair

with `<prefix>` `~/.local`, `/usr`, `$(brew --prefix)` or the Nix profile. Remote access and
TLS are in `apps/gateway/README.md`.

## The desktop app

- macOS: `brew install --cask ternmail/tap/tern`, or `Tern-AppleSilicon.zip` / `Tern.zip`
  (Intel) from the release. (`--cask` is needed: the formula has the same name.)
- Linux: the `tern` .deb or .rpm, or the snap.
- Windows: the installer.

## Building the archives yourself

    apps/engine/packaging/build-release.sh --gateway     # needs cargo-about for the notices
    packaging/build-web.sh target/release-web
    packaging/archive.sh --dist target/engine-dist --web target/release-web aarch64-apple-darwin
    TERN_DOWNLOAD_URL=file://$PWD/target/release-archives sh packaging/install.sh --prefix /tmp/tern

`packaging/linux/build-packages.sh` makes the .deb and .rpm from Linux archives (nfpm).
`packaging/bump.sh` sets a release's version and checksums in the Homebrew formula and cask
(`packaging/homebrew/`) and the PKGBUILD (`packaging/aur/`).
