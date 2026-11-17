#!/bin/sh
# Installs Airpost's command-line programs (airpost, airpost-engine, airpost-mcp, airpost-gateway and the web
# client the gateway serves) from a GitHub release, after checking the archive against the
# release's SHA256SUMS.
#
#   curl -fsSL https://github.com/airpost-mail/airpost/releases/latest/download/install.sh | sh
#   curl -fsSL …/install.sh | sh -s -- [--version X.Y.Z] [--prefix DIR | --system] [--uninstall]
#
#   --version X.Y.Z  that release (AIRPOST_VERSION); default the newest stable vX.Y.Z release
#   --prefix DIR     install into DIR/bin and DIR/share (AIRPOST_PREFIX); default ~/.local
#   --system         --prefix /usr/local, with sudo when it isn't writable
#   --uninstall      remove what an install into the prefix put there
#
# AIRPOST_DOWNLOAD_URL replaces the release's download directory (a mirror, or file:///… for a
# local build: scripts/archive.sh's output). macOS (arm64, x86_64) and Linux with glibc
# (x86_64, aarch64) only.

set -eu

REPO="airpost-mail/airpost"
version="${AIRPOST_VERSION:-}"
prefix="${AIRPOST_PREFIX:-}"
system=""
uninstall=""

say() { printf '%s\n' "$*"; }
err() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || err "--version needs a value"; version="$2"; shift 2 ;;
    --version=*) version="${1#*=}"; shift ;;
    --prefix) [ $# -ge 2 ] || err "--prefix needs a value"; prefix="$2"; shift 2 ;;
    --prefix=*) prefix="${1#*=}"; shift ;;
    --system) system=1; shift ;;
    --uninstall) uninstall=1; shift ;;
    -h | --help) sed -n '2,/^$/s/^# \{0,1\}//p' "$0" 2>/dev/null || say "see https://docs.airpost.sh/install"; exit 0 ;;
    *) err "unknown argument $1 (--help)" ;;
  esac
done
version="${version#v}"

if [ -n "$system" ]; then
  [ -z "$prefix" ] || err "--system and --prefix both given"
  prefix=/usr/local
fi
[ -n "$prefix" ] || prefix="${HOME:?}/.local"
case "$prefix" in /*) ;; *) prefix="$PWD/$prefix" ;; esac

# Root-owned prefixes go through sudo; nothing else does.
sudo=""
probe="$prefix"
while [ ! -e "$probe" ]; do probe="$(dirname "$probe")"; done
if [ ! -w "$probe" ] && [ "$(id -u)" != 0 ]; then
  command -v sudo >/dev/null || err "$prefix isn't writable and sudo isn't installed"
  sudo=sudo
  say "Installing into $prefix needs sudo."
fi

BINARIES="airpost airpost-engine airpost-mcp airpost-gateway"

if [ -n "$uninstall" ]; then
  for b in $BINARIES; do $sudo rm -f "$prefix/bin/$b"; done
  $sudo rm -rf "$prefix/share/airpost-gateway" "$prefix/share/doc/airpost"
  say "Removed Airpost's programs from $prefix."
  say "Units written by 'airpost engine install' stay: run 'airpost engine uninstall' first to remove them."
  say "Your mail profile is untouched."
  exit 0
fi

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    # An x86_64 shell under Rosetta on Apple silicon still gets the native build.
    [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ] && arch=arm64
    case "$arch" in
      arm64 | aarch64) target=aarch64-apple-darwin ;;
      x86_64) target=x86_64-apple-darwin ;;
      *) err "no build for macOS on $arch" ;;
    esac
    ;;
  Linux)
    if ldd --version 2>&1 | grep -qi musl; then
      err "no build for musl-based Linux (Alpine); the binaries need glibc 2.31 or newer"
    fi
    case "$arch" in
      x86_64 | amd64) target=x86_64-unknown-linux-gnu ;;
      aarch64 | arm64) target=aarch64-unknown-linux-gnu ;;
      *) err "no build for Linux on $arch" ;;
    esac
    ;;
  *) err "no build for $os; Airpost runs on macOS and Linux" ;;
esac

if command -v curl >/dev/null; then
  fetch() { curl --proto '=https,file,http' -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null; then
  fetch() { wget -q "$1" -O "$2"; }
else
  err "needs curl or wget"
fi
if command -v sha256sum >/dev/null; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  err "needs sha256sum or shasum to check the download"
fi

tmp="$(mktemp -d 2>/dev/null || mktemp -d -t airpost-install)"
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -z "$version" ] && [ -z "${AIRPOST_DOWNLOAD_URL:-}" ]; then
  # The repo also publishes desktop, engine-v* and cli/v* releases, so GitHub's "latest" may
  # not be one of these: take the newest release tagged vX.Y.Z (no pre-release suffix).
  fetch "https://api.github.com/repos/$REPO/releases?per_page=50" "$tmp/releases.json" ||
    err "couldn't list the releases (set AIRPOST_VERSION to skip this)"
  version="$(tr ',' '\n' < "$tmp/releases.json" |
    sed -n 's/.*"tag_name": *"v\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' | head -n 1)"
  [ -n "$version" ] || err "no vX.Y.Z release found"
fi
base="${AIRPOST_DOWNLOAD_URL:-https://github.com/$REPO/releases/download/v$version}"

say "Downloading $base/SHA256SUMS"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || err "couldn't download $base/SHA256SUMS"
# Without --version (a local AIRPOST_DOWNLOAD_URL), the sums name the version.
pattern="airpost-${version:-[0-9][^ ]*}-$target.tar.gz"
line="$(grep -E "^[0-9a-f]{64}  $(printf '%s' "$pattern" | sed 's/\./\\./g')\$" "$tmp/SHA256SUMS" | head -n 1 || true)"
[ -n "$line" ] || err "SHA256SUMS lists no archive for $target"
expected="${line%% *}"
archive="${line##* }"

say "Downloading $base/$archive"
fetch "$base/$archive" "$tmp/$archive" || err "couldn't download $base/$archive"
actual="$(sha256 "$tmp/$archive")"
[ "$actual" = "$expected" ] || err "checksum mismatch for $archive: expected $expected, got $actual"
say "Checksum OK ($expected)"

mkdir "$tmp/x"
tar -xzf "$tmp/$archive" -C "$tmp/x"
src="$tmp/x/${archive%.tar.gz}"
[ -x "$src/bin/airpost" ] || err "$archive has no bin/airpost"

$sudo mkdir -p "$prefix/bin" "$prefix/share/doc"
for b in $BINARIES; do
  dest="$prefix/bin/$b"
  if [ -L "$dest" ]; then
    say "Replacing $dest, a link to $(readlink "$dest")"
  fi
  # Copy then rename: a running engine keeps its binary, and the new one takes its place.
  $sudo cp "$src/bin/$b" "$dest.new"
  $sudo chmod 755 "$dest.new"
  $sudo mv -f "$dest.new" "$dest"
done
$sudo rm -rf "$prefix/share/airpost-gateway" "$prefix/share/doc/airpost"
$sudo cp -R "$src/share/airpost-gateway" "$prefix/share/airpost-gateway"
$sudo cp -R "$src/share/doc/airpost" "$prefix/share/doc/airpost"

installed="${archive#airpost-}"
installed="${installed%-"$target".tar.gz}"
say ""
say "Installed Airpost $installed into $prefix:"
for b in $BINARIES; do say "  $prefix/bin/$b"; done
say "  $prefix/share/airpost-gateway/web (the web client)"
say "  $prefix/share/doc/airpost (licence and third-party notices)"
case ":$PATH:" in
  *":$prefix/bin:"*) ;;
  *)
    say ""
    say "$prefix/bin isn't on your PATH. Add it, e.g. in ~/.profile:"
    say "  export PATH=\"$prefix/bin:\$PATH\""
    ;;
esac
say ""
say "Next steps:"
say "  airpost engine install          run the engine in the background, started at login"
say "  airpost account add you@example.com"
say "  airpost status"
say "  airpost-gateway serve --web-root $prefix/share/airpost-gateway/web   the web client, then 'airpost-gateway pair'"
if [ "$os" = Linux ]; then
  say ""
  say "Without a desktop keyring (a server), keep the engine's secrets in a key file:"
  say "  airpost engine install --key-file ~/.config/airpost-engine.key"
  say "To keep it running after you log out: loginctl enable-linger \"\$USER\""
fi
say ""
say "Upgrade: run this script again, then 'airpost engine restart'. Remove: --uninstall."
