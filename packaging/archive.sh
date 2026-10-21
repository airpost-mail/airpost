#!/usr/bin/env bash
# Packs the release archives: tern, tern-engine, tern-mcp and tern-gateway for one target, the
# web client the gateway serves, and the licence and notices for all of it.
#
#   packaging/archive.sh --dist DIR --web DIR [--source FILE] [--out DIR] TARGET...
#
#   --dist    build-release.sh --gateway's output (OUT_DIR): DIR/<target>/tern-engine, …
#   --web     build-web.sh's output: DIR/web and DIR/tern-web.THIRD-PARTY-NOTICES.txt
#   --source  SOURCE.txt (.github/scripts/source-offer.sh), the GPLv3 6(d) offer; CI only
#   --out     where the archives go (default: target/release-archives)
#
# Writes OUT/tern-<version>-<target>.tar.gz, with Linux debug info in
# tern-<version>-<target>-debug.tar.gz, and OUT/SHA256SUMS over every archive in OUT.
# The version is [workspace.package] version in Cargo.toml (VERSION overrides). Layout, the
# same prefix layout Homebrew, the .deb/.rpm and install.sh unpack to:
#
#   tern-<version>-<target>/bin/{tern,tern-engine,tern-mcp,tern-gateway}
#   tern-<version>-<target>/share/tern-gateway/web/          (tern-gateway serve --web-root)
#   tern-<version>-<target>/share/doc/tern/                   LICENSE.md, NOTICE.md, notices
#
# Sign macOS binaries in DIST before packing (apps/engine/packaging/macos/sign.sh).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARIES=(tern tern-engine tern-mcp tern-gateway)
# Each notice file in DIST/<target>, and its name in the archive. tern-mcp's covers tern too:
# tern's crates are a subset of tern-mcp's.
NOTICES=(
  THIRD-PARTY-NOTICES.txt:tern-engine.THIRD-PARTY-NOTICES.txt
  tern-mcp.THIRD-PARTY-NOTICES.txt:tern-mcp.THIRD-PARTY-NOTICES.txt
  tern-gateway.THIRD-PARTY-NOTICES.txt:tern-gateway.THIRD-PARTY-NOTICES.txt
)

die() { printf 'archive.sh: %s\n' "$*" >&2; exit 1; }
log() { printf -- '---> %s\n' "$*" >&2; }

dist="" web="" source="" out="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/release-archives"
targets=()
while [ $# -gt 0 ]; do
  case "$1" in
    --dist) dist="$2"; shift 2 ;;
    --web) web="$2"; shift 2 ;;
    --source) source="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    -h | --help) sed -n '2,/^$/s/^# \{0,1\}//p' "$0"; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) targets+=("$1"); shift ;;
  esac
done
[ -n "$dist" ] && [ -n "$web" ] && [ ${#targets[@]} -gt 0 ] || die "usage: archive.sh --dist DIR --web DIR [--source FILE] [--out DIR] TARGET..."
[ -f "$web/web/index.html" ] || die "$web/web/index.html missing (packaging/build-web.sh)"
[ -f "$web/tern-web.THIRD-PARTY-NOTICES.txt" ] || die "$web/tern-web.THIRD-PARTY-NOTICES.txt missing"
[ -z "$source" ] || [ -f "$source" ] || die "$source missing"

VERSION="${VERSION:-$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml")}"
[ -n "$VERSION" ] || die "no version in Cargo.toml"

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

# Owner root and no macOS metadata (._ files, xattrs such as com.apple.provenance, which GNU
# tar warns about) in the archive.
tar_create() {
  local archive="$1" dir="$2" name="$3"
  local opts=(--owner=0 --group=0 --numeric-owner)
  tar --version 2>/dev/null | grep -q bsdtar && opts=(--uid 0 --gid 0 --no-xattrs --no-mac-metadata)
  COPYFILE_DISABLE=1 tar "${opts[@]}" -czf "$archive" -C "$dir" "$name"
}

mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

for target in "${targets[@]}"; do
  src="$dist/$target"
  name="tern-$VERSION-$target"
  root="$work/$name"
  mkdir -p "$root/bin" "$root/share/tern-gateway" "$root/share/doc/tern"
  for b in "${BINARIES[@]}"; do
    [ -f "$src/$b" ] || die "$src/$b missing (build-release.sh --gateway)"
    install -m 755 "$src/$b" "$root/bin/$b"
  done
  cp -R "$web/web" "$root/share/tern-gateway/web"
  doc="$root/share/doc/tern"
  install -m 644 "$REPO_ROOT/LICENSE.md" "$REPO_ROOT/NOTICE.md" "$doc/"
  # The web client's brand/ images are trademarks, outside the GPL.
  install -m 644 "$REPO_ROOT/assets/brand/LICENSE.md" "$doc/TRADEMARKS.md"
  for n in "${NOTICES[@]}"; do
    [ -f "$src/${n%%:*}" ] || die "$src/${n%%:*} missing (build-release.sh needs cargo-about)"
    install -m 644 "$src/${n%%:*}" "$doc/${n#*:}"
  done
  install -m 644 "$web/tern-web.THIRD-PARTY-NOTICES.txt" "$doc/"
  [ -z "$source" ] || install -m 644 "$source" "$doc/SOURCE.txt"
  tar_create "$out/$name.tar.gz" "$work" "$name"
  log "$out/$name.tar.gz"

  if compgen -G "$src/*.debug" >/dev/null; then
    dbg="$work/$name-debug"
    mkdir -p "$dbg/lib/debug"
    cp "$src"/*.debug "$dbg/lib/debug/"
    install -m 644 "$REPO_ROOT/LICENSE.md" "$REPO_ROOT/NOTICE.md" "$dbg/"
    tar_create "$out/$name-debug.tar.gz" "$work" "$name-debug"
    log "$out/$name-debug.tar.gz"
  fi
  rm -rf "$root" "$work/$name-debug"
done

(cd "$out" && rm -f SHA256SUMS && sha256 ./*.tar.gz | sed 's| \./| |' > SHA256SUMS)
log "$out/SHA256SUMS"
