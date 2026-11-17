#!/usr/bin/env bash
# Builds the web client the gateway serves (apps/web, VITE_AIRPOST_ENGINE=gateway) into OUT/web
# and writes its third-party notices to OUT/airpost-web.THIRD-PARTY-NOTICES.txt.
#
#   scripts/build-web.sh OUT
#
# Runs `npm ci` in apps/web unless SKIP_NPM_CI is set (node_modules already matches the lock).

set -euo pipefail

[ $# -eq 1 ] || { sed -n '2,/^$/s/^# \{0,1\}//p' "$0" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB="$REPO_ROOT/apps/web"
OUT="$1"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac

cd "$WEB"
[ -n "${SKIP_NPM_CI:-}" ] || npm ci --no-audit --no-fund
rm -rf dist
VITE_AIRPOST_ENGINE=gateway npm run build
node "$REPO_ROOT/scripts/web-notices.mjs" "$WEB" "$WEB/dist" "$OUT/airpost-web.THIRD-PARTY-NOTICES.txt"
rm -rf "$OUT/web"
mkdir -p "$OUT"
cp -R "$WEB/dist" "$OUT/web"
echo "---> $OUT/web" >&2
