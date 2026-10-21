#!/usr/bin/env bash
# Submits signed engine binaries to Apple's notary service and waits for the verdict.
#
#   notarize.sh [--dry-run] <binary>...
#
# Credentials, either set:
#   APPLE_API_KEY_PATH, APPLE_API_KEY_ID, APPLE_API_ISSUER   (App Store Connect API key), or
#   APPLE_ID, APPLE_ID_PASSWORD (app-specific), APPLE_TEAM_ID (the desktop workflow's names).
#
# The binaries are zipped with ditto and submitted together. A bare Mach-O cannot be
# stapled, so Gatekeeper looks the ticket up online; inside the desktop app the app's own
# notarization covers the engine. --dry-run checks the signatures and credentials and
# prints the notarytool command without submitting; it exits 1 if a submit would fail.

set -euo pipefail

dry_run=""
if [ "${1:-}" = --dry-run ]; then
  dry_run=1
  shift
fi
[ $# -gt 0 ] || { echo "usage: notarize.sh [--dry-run] <binary>..." >&2; exit 2; }

problems=0
for bin in "$@"; do
  info="$(codesign --display --verbose=4 "$bin" 2>&1)" || { echo "$bin: not signed" >&2; problems=1; continue; }
  grep -q 'flags=.*runtime' <<<"$info" || { echo "$bin: no hardened runtime (sign.sh)" >&2; problems=1; }
  if grep -q 'flags=.*adhoc' <<<"$info"; then
    echo "$bin: signed ad hoc; notarization needs a Developer ID signature" >&2
    problems=1
  elif ! grep -q '^Timestamp=' <<<"$info"; then
    echo "$bin: no secure timestamp (sign.sh signs with --timestamp)" >&2
    problems=1
  fi
done

auth=()
if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
  auth=(--key "$APPLE_API_KEY_PATH" --key-id "${APPLE_API_KEY_ID:?}" --issuer "${APPLE_API_ISSUER:?}")
elif [ -n "${APPLE_ID:-}" ]; then
  auth=(--apple-id "$APPLE_ID" --password "${APPLE_ID_PASSWORD:?}" --team-id "${APPLE_TEAM_ID:?}")
else
  echo "no notarization credentials (APPLE_API_KEY_PATH/_ID/_ISSUER or APPLE_ID/_PASSWORD/APPLE_TEAM_ID)" >&2
  problems=1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
zip="$work/tern-engine-notarize.zip"
mkdir "$work/payload"
for bin in "$@"; do
  # Slices of the same name (per-arch builds) go in separate directories.
  dir="$work/payload/$(basename "$(dirname "$bin")")"
  mkdir -p "$dir"
  cp -p "$bin" "$dir/"
done
ditto -c -k --norsrc --noextattr --keepParent "$work/payload" "$zip"

redacted=()
for a in "${auth[@]+"${auth[@]}"}"; do
  [ "$a" = "${APPLE_ID_PASSWORD:-}" ] && a='***'
  redacted+=("$a")
done
echo "xcrun notarytool submit $zip ${redacted[*]+"${redacted[*]}"} --wait --output-format json" >&2

if [ -n "$dry_run" ]; then
  unzip -l "$zip" >&2
  if [ "$problems" = 0 ]; then
    echo "dry run: ready to submit" >&2
    exit 0
  fi
  echo "dry run: not ready (see above)" >&2
  exit 1
fi
[ "$problems" = 0 ] || exit 1

result="$(xcrun notarytool submit "$zip" "${auth[@]}" --wait --output-format json)"
echo "$result"
status="$(plutil -extract status raw - <<<"$result")"
if [ "$status" != Accepted ]; then
  id="$(plutil -extract id raw - <<<"$result")"
  xcrun notarytool log "$id" "${auth[@]}" >&2 || true
  exit 1
fi
