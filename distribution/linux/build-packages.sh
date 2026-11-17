#!/usr/bin/env bash
# Builds the airpost-cli .deb and .rpm from Linux release archives (scripts/archive.sh) with
# nfpm (https://nfpm.goreleaser.com; `go install github.com/goreleaser/nfpm/v2/cmd/nfpm@v2.43.0`
# or a release binary). NFPM overrides the nfpm binary.
#
#   distribution/linux/build-packages.sh OUT ARCHIVE...
#
# Writes OUT/airpost-cli_<version>-1_<amd64|arm64>.deb and OUT/airpost-cli-<version>-1.<x86_64|aarch64>.rpm.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NFPM="${NFPM:-nfpm}"
[ $# -ge 2 ] || { sed -n '2,/^$/s/^# \{0,1\}//p' "$0" >&2; exit 2; }
out="$1"
shift
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac
command -v "$NFPM" >/dev/null || { echo "build-packages.sh: nfpm not found (NFPM=...)" >&2; exit 1; }
mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

for archive in "$@"; do
  name="$(basename "$archive" .tar.gz)"
  case "$name" in
    airpost-*-x86_64-unknown-linux-gnu) arch=amd64 target=x86_64-unknown-linux-gnu ;;
    airpost-*-aarch64-unknown-linux-gnu) arch=arm64 target=aarch64-unknown-linux-gnu ;;
    *) echo "build-packages.sh: $archive isn't a Linux release archive" >&2; exit 1 ;;
  esac
  version="${name#airpost-}"
  version="${version%-"$target"}"
  tar -xzf "$archive" -C "$work"
  root="$work/$name"
  cat "$root/share/doc/airpost/NOTICE.md" "$root/share/doc/airpost/LICENSE.md" > "$root/copyright"
  for packager in deb rpm; do
    (cd "$root" && VERSION="$version" ARCH="$arch" \
      "$NFPM" package --config "$HERE/nfpm.yaml" --packager "$packager" --target "$out/")
  done
  rm -rf "$root"
done
