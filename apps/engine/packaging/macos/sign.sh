#!/usr/bin/env bash
# Signs engine binaries with the hardened runtime and checks the result.
#
#   sign.sh <binary>...
#
# APPLE_SIGNING_IDENTITY is the "Developer ID Application" certificate (name or SHA-1) in
# the keychain; "-" signs ad hoc, which checks everything except the certificate and
# cannot be notarized. Entitlements come from entitlements.plist next to this script
# (ENTITLEMENTS overrides). The engine needs no hardened-runtime exceptions: no JIT, no
# unsigned executable memory, no DYLD_* variables, no libraries outside the binary.
#
# Inside the desktop app the binary is re-signed by @electron/osx-sign with
# apps/desktop/build/resources/mac/entitlements.child.plist; this script is for the
# engine shipped on its own and for checking a build before that.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IDENTITY="${APPLE_SIGNING_IDENTITY:?set APPLE_SIGNING_IDENTITY (a Developer ID Application identity, or - for ad hoc)}"
ENTITLEMENTS="${ENTITLEMENTS:-$HERE/entitlements.plist}"

[ $# -gt 0 ] || { echo "usage: sign.sh <binary>..." >&2; exit 2; }
plutil -lint -s "$ENTITLEMENTS"

timestamp=(--timestamp)
# Apple's timestamp service only countersigns certificate signatures.
[ "$IDENTITY" = - ] && timestamp=(--timestamp=none)

for bin in "$@"; do
  echo "---> Signing $bin" >&2
  codesign --force --sign "$IDENTITY" --options runtime "${timestamp[@]}" \
    --entitlements "$ENTITLEMENTS" "$bin"
  codesign --verify --strict --verbose=2 "$bin"
  info="$(codesign --display --verbose=4 "$bin" 2>&1)"
  grep -q 'flags=.*runtime' <<<"$info" || { echo "$bin: hardened runtime flag missing" >&2; exit 1; }
  # Every slice of a universal binary must carry the signature.
  for arch in $(lipo -archs "$bin"); do
    codesign --verify --strict --arch "$arch" "$bin"
  done
  codesign --display --entitlements - --xml "$bin" 2>/dev/null | plutil -lint -s - >/dev/null
  grep -E '^(Identifier|Format|CodeDirectory|Authority|Timestamp|TeamIdentifier)' <<<"$info" >&2 || true
done
