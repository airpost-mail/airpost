#!/usr/bin/env bash
# shellcheck disable=SC2329  # build_* are called as build_$platform
# Builds the release engine (`mailsync`), the MCP server (`tern-mcp`, apps/mcp) and the CLI
# (`tern`, apps/cli) for macOS and Linux and optionally places them where the desktop build
# picks them up (apps/desktop/mailsync-rs, apps/desktop/tern-mcp and
# apps/desktop/tern, unpacked from the asar by apps/desktop/build/build.js).
#
#   build-release.sh [macos] [linux] [--install-desktop[=ARCH]]
#
# With no platform, builds the host's. Outputs, under $OUT_DIR (default
# $CARGO_TARGET_DIR/engine-dist, or target/engine-dist):
#
#   aarch64-apple-darwin/mailsync     x86_64-apple-darwin/mailsync   universal-apple-darwin/mailsync
#   x86_64-unknown-linux-gnu/mailsync aarch64-unknown-linux-gnu/mailsync
#
# and tern-mcp and tern next to each mailsync.
# Each target directory also gets LICENSE.md, NOTICE.md and THIRD-PARTY-NOTICES.txt (the
# engine crates' licences, from third-party-notices.sh) and tern-mcp.THIRD-PARTY-NOTICES.txt
# (tern-mcp's, which also cover tern: its crates are a subset of tern-mcp's), which ship next
# to the binaries. Without
# cargo-about the notices are skipped with a warning; TERN_REQUIRE_NOTICES=1 (set in CI) makes
# that an error.
#
# macOS needs both Rust targets installed; with rustup the script adds them for the
# toolchain in rust-toolchain.toml. Linux binaries are built natively on a Linux host (the
# other arch with a cross gcc if one is installed) or, on macOS, the same way in a Debian
# bullseye container (glibc 2.31, older than any distro the desktop's Linux packages
# support). Linux debug info is split into mailsync.debug next to each binary.
#
# --install-desktop copies one mailsync to apps/desktop/mailsync-rs and its tern-mcp and
# tern to apps/desktop/tern-mcp and apps/desktop/tern, and the notices to
# apps/desktop/mailsync-rs.THIRD-PARTY-NOTICES.txt for build.js. ARCH is arm64, x64 or
# universal (macOS only); the default is the arch build.js packages for: x64 when
# OVERRIDE_TO_INTEL is set, else the host's.
#
# The binary is unsigned; macOS signing and notarization are in packaging/macos/.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TOOLCHAIN="${RUST_TOOLCHAIN:-$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$REPO_ROOT/rust-toolchain.toml")}"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
case "$TARGET_DIR" in /*) ;; *) TARGET_DIR="$REPO_ROOT/$TARGET_DIR" ;; esac
export CARGO_TARGET_DIR="$TARGET_DIR"
OUT_DIR="${OUT_DIR:-$TARGET_DIR/engine-dist}"
case "$OUT_DIR" in /*) ;; *) OUT_DIR="$PWD/$OUT_DIR" ;; esac
DESKTOP_DIR="$REPO_ROOT/apps/desktop"
BINARIES=(mailsync tern-mcp tern)
LINUX_IMAGE="${LINUX_IMAGE:-rust:$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$REPO_ROOT/rust-toolchain.toml")-bullseye}"
# Electron 44's floor; also above arm64's 11.0 minimum.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"

HOST_OS="$(uname -s)"
HOST_ARCH="$(uname -m)"
[ "$HOST_ARCH" = arm64 ] && HOST_ARCH=aarch64

cd "$REPO_ROOT"

log() { printf -- '---> %s\n' "$*" >&2; }
die() { printf 'build-release.sh: %s\n' "$*" >&2; exit 1; }

find_rustup() {
  command -v rustup 2>/dev/null && return
  # Homebrew's rustup is keg-only and not on PATH.
  for p in /opt/homebrew/opt/rustup/bin/rustup /usr/local/opt/rustup/bin/rustup; do
    [ -x "$p" ] && { echo "$p"; return; }
  done
  return 0
}
RUSTUP="$(find_rustup)"

cargo_build() {
  local target="$1"
  shift
  if [ -n "$RUSTUP" ]; then
    "$RUSTUP" target add --toolchain "$TOOLCHAIN" "$target" >/dev/null
    # Run the toolchain's binaries directly: `rustup run` leaves PATH alone, so a cargo or
    # rustc installed outside rustup (Homebrew's) would still win.
    local bin
    bin="$(dirname "$("$RUSTUP" which --toolchain "$TOOLCHAIN" cargo)")"
    PATH="$bin:$PATH" RUSTC="$bin/rustc" "$bin/cargo" build --release --locked -p mailsync \
      -p tern-mcp -p tern --target "$target" "$@"
  else
    cargo build --release --locked -p mailsync -p tern-mcp -p tern --target "$target" "$@" ||
      die "cargo build for $target failed; without rustup the $target standard library must already be installed"
  fi
}

# Generated once per run on the host (the Docker build reuses it through $OUT_DIR): the crate
# graph is the same for every target listed in about.toml.
generate_notices() {
  [ -n "${TERN_SKIP_NOTICES:-}" ] && return 0
  if ! command -v cargo-about >/dev/null; then
    [ -n "${TERN_REQUIRE_NOTICES:-}" ] && die "cargo-about not installed (cargo install --locked cargo-about --features cli)"
    log "Not writing THIRD-PARTY-NOTICES.txt: cargo-about not installed"
    return 0
  fi
  log "Writing $OUT_DIR/THIRD-PARTY-NOTICES.txt"
  "$REPO_ROOT/apps/engine/packaging/third-party-notices.sh" "$OUT_DIR/THIRD-PARTY-NOTICES.txt"
  log "Writing $OUT_DIR/tern-mcp.THIRD-PARTY-NOTICES.txt"
  "$REPO_ROOT/apps/engine/packaging/third-party-notices.sh" \
    "$OUT_DIR/tern-mcp.THIRD-PARTY-NOTICES.txt" "$REPO_ROOT/apps/mcp/Cargo.toml"
}

collect() {
  local target="$1"
  mkdir -p "$OUT_DIR/$target"
  local b
  for b in "${BINARIES[@]}"; do
    cp "$TARGET_DIR/$target/release/$b" "$OUT_DIR/$target/$b"
  done
  cp "$REPO_ROOT/LICENSE.md" "$REPO_ROOT/NOTICE.md" "$OUT_DIR/$target/"
  for b in THIRD-PARTY-NOTICES.txt tern-mcp.THIRD-PARTY-NOTICES.txt; do
    if [ -f "$OUT_DIR/$b" ]; then cp "$OUT_DIR/$b" "$OUT_DIR/$target/"; fi
  done
  log "$OUT_DIR/$target/mailsync"
}

build_macos() {
  [ "$HOST_OS" = Darwin ] || die "macOS binaries can only be built on macOS"
  local t
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    log "Building $t"
    cargo_build "$t"
    collect "$t"
  done
  mkdir -p "$OUT_DIR/universal-apple-darwin"
  local b
  for b in "${BINARIES[@]}"; do
    lipo -create -output "$OUT_DIR/universal-apple-darwin/$b" \
      "$OUT_DIR/aarch64-apple-darwin/$b" "$OUT_DIR/x86_64-apple-darwin/$b"
  done
  cp "$OUT_DIR/aarch64-apple-darwin/"{LICENSE.md,NOTICE.md} "$OUT_DIR/universal-apple-darwin/"
  for b in THIRD-PARTY-NOTICES.txt tern-mcp.THIRD-PARTY-NOTICES.txt; do
    if [ -f "$OUT_DIR/$b" ]; then cp "$OUT_DIR/$b" "$OUT_DIR/universal-apple-darwin/"; fi
  done
  lipo "$OUT_DIR/universal-apple-darwin/mailsync" -verify_arch arm64 x86_64
  log "$OUT_DIR/universal-apple-darwin/mailsync ($(lipo -archs "$OUT_DIR/universal-apple-darwin/mailsync"))"
}

# Cross gcc and the env cargo/cc need for $1 on a host of arch $2, printed as KEY=VALUE.
linux_cross_env() {
  local target="$1" host_arch="$2" arch="${1%%-*}"
  [ "$arch" = "$host_arch" ] && return 0
  local gcc="$arch-linux-gnu-gcc" var
  var="$(echo "$target" | tr 'a-z-' 'A-Z_')"
  echo "CARGO_TARGET_${var}_LINKER=$gcc"
  echo "CC_$(echo "$target" | tr - _)=$gcc"
}

# The release profile keeps line tables (debug = 1), which on Linux end up in the binary
# (~100 MB against ~20 MB without). Ship the stripped binary and keep the DWARF next to it
# as <binary>.debug, linked by .gnu_debuglink.
split_linux_debug() {
  local bin="$1" arch="$2" objcopy=objcopy name
  [ "$arch" != "$HOST_ARCH" ] && objcopy="$arch-linux-gnu-objcopy"
  if ! command -v "$objcopy" >/dev/null; then
    log "Not splitting debug info: $objcopy not installed"
    return
  fi
  name="$(basename "$bin")"
  (
    cd "$(dirname "$bin")"
    "$objcopy" --only-keep-debug "$name" "$name.debug"
    "$objcopy" --strip-debug --add-gnu-debuglink="$name.debug" "$name"
  )
}

build_linux_native() {
  local t arch b
  for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    arch="${t%%-*}"
    if [ "$arch" != "$HOST_ARCH" ] && ! command -v "$arch-linux-gnu-gcc" >/dev/null; then
      log "Skipping $t: $arch-linux-gnu-gcc not installed (Debian/Ubuntu: gcc-${arch//_/-}-linux-gnu, or build on an $arch host)"
      continue
    fi
    log "Building $t"
    (
      while IFS= read -r kv; do export "${kv?}"; done < <(linux_cross_env "$t" "$HOST_ARCH")
      cargo_build "$t"
    )
    collect "$t"
    for b in "${BINARIES[@]}"; do
      split_linux_debug "$OUT_DIR/$t/$b" "$arch"
    done
    if command -v objdump >/dev/null; then
      log "$t needs glibc $(objdump -T "$OUT_DIR/$t/mailsync" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
    fi
  done
}

# Runs build_linux_native in a container, with the cross gcc for the other arch installed.
build_linux_docker() {
  command -v docker >/dev/null || die "building Linux binaries on $HOST_OS needs Docker"
  local cross_pkgs
  case "$HOST_ARCH" in
    aarch64) cross_pkgs="gcc-x86-64-linux-gnu libc6-dev-amd64-cross" ;;
    x86_64) cross_pkgs="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;;
    *) die "unsupported host arch $HOST_ARCH" ;;
  esac
  log "Building Linux binaries in $LINUX_IMAGE"
  mkdir -p "$TARGET_DIR/linux-docker" "$OUT_DIR"
  # RUST_TOOLCHAIN: the image's toolchain is the one rust-toolchain.toml names, but rustup
  # would install it again under the shorter channel name.
  docker run --rm \
    -v "$REPO_ROOT":/src:ro \
    -v "$TARGET_DIR/linux-docker":/target \
    -v "$OUT_DIR":/out \
    -v tern-engine-cargo-registry:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/target -e OUT_DIR=/out -e TERN_SKIP_NOTICES=1 \
    "$LINUX_IMAGE" bash -euo pipefail -c "
      apt-get update -qq >/dev/null
      DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $cross_pkgs >/dev/null
      RUST_TOOLCHAIN=\$(rustup default | cut -d' ' -f1) /src/apps/engine/packaging/build-release.sh linux
    "
}

build_linux() {
  if [ "$HOST_OS" = Linux ]; then build_linux_native; else build_linux_docker; fi
}

install_desktop() {
  local arch="$1" target
  if [ -z "$arch" ]; then
    if [ -n "${OVERRIDE_TO_INTEL:-}" ]; then arch=x64; else arch="$HOST_ARCH"; fi
  fi
  case "$HOST_OS:$arch" in
    Darwin:arm64 | Darwin:aarch64) target=aarch64-apple-darwin ;;
    Darwin:x64 | Darwin:x86_64) target=x86_64-apple-darwin ;;
    Darwin:universal) target=universal-apple-darwin ;;
    Linux:arm64 | Linux:aarch64) target=aarch64-unknown-linux-gnu ;;
    Linux:x64 | Linux:x86_64) target=x86_64-unknown-linux-gnu ;;
    *) die "no desktop binary for $HOST_OS $arch" ;;
  esac
  local b
  for b in "${BINARIES[@]}"; do
    [ -f "$OUT_DIR/$target/$b" ] || die "$OUT_DIR/$target/$b was not built"
  done
  install -m 755 "$OUT_DIR/$target/mailsync" "$DESKTOP_DIR/mailsync-rs"
  install -m 755 "$OUT_DIR/$target/tern-mcp" "$DESKTOP_DIR/tern-mcp"
  # Preferences > Plugins runs it for `tern service install`.
  install -m 755 "$OUT_DIR/$target/tern" "$DESKTOP_DIR/tern"
  log "Installed $target into $DESKTOP_DIR/mailsync-rs, $DESKTOP_DIR/tern-mcp and $DESKTOP_DIR/tern"
  if [ -f "$OUT_DIR/$target/THIRD-PARTY-NOTICES.txt" ]; then
    install -m 644 "$OUT_DIR/$target/THIRD-PARTY-NOTICES.txt" \
      "$DESKTOP_DIR/mailsync-rs.THIRD-PARTY-NOTICES.txt"
  fi
  if [ -f "$OUT_DIR/$target/tern-mcp.THIRD-PARTY-NOTICES.txt" ]; then
    install -m 644 "$OUT_DIR/$target/tern-mcp.THIRD-PARTY-NOTICES.txt" \
      "$DESKTOP_DIR/tern-mcp.THIRD-PARTY-NOTICES.txt"
  fi
}

platforms=()
install=""
install_arch=""
for arg in "$@"; do
  case "$arg" in
    macos | linux) platforms+=("$arg") ;;
    --install-desktop) install=1 ;;
    --install-desktop=*) install=1 install_arch="${arg#*=}" ;;
    -h | --help) sed -n '3,/^$/s/^# \{0,1\}//p' "$0"; exit 0 ;;
    *) die "unknown argument $arg (see --help)" ;;
  esac
done
if [ ${#platforms[@]} -eq 0 ]; then
  case "$HOST_OS" in
    Darwin) platforms=(macos) ;;
    Linux) platforms=(linux) ;;
    *) die "unsupported host $HOST_OS" ;;
  esac
fi

mkdir -p "$OUT_DIR"
generate_notices
for p in "${platforms[@]}"; do "build_$p"; done
[ -n "$install" ] && install_desktop "$install_arch"
exit 0
