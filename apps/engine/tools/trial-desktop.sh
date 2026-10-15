#!/usr/bin/env bash
# Run the desktop client from source on the Rust engine, against a copy of an existing
# Mailspring profile, so the real profile and the running Mailspring.app are never touched.
#
#   apps/engine/tools/trial-desktop.sh [--fresh] [--from DIR] [--to DIR]
#
# --from  profile to copy (default: ~/Library/Application Support/Mailspring)
# --to    trial profile (default: ~/tern-trial); reused on later runs unless --fresh
# --fresh delete the trial profile and copy it again
#
# The Rust engine's migrate moves the copied edgehill.db aside and resyncs every account from
# scratch; import-legacy then carries over local drafts, snooze/send-later/reminder metadata and
# sync cursors. Secrets stay in the macOS keychain ("Mailspring Safe Storage"), which the trial
# shares with the real app; macOS may ask once to let the dev Electron binary read it.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../../.." && pwd)"
from="$HOME/Library/Application Support/Mailspring"
to="$HOME/tern-trial"
fresh=0
while [ $# -gt 0 ]; do
  case "$1" in
    --fresh) fresh=1 ;;
    --from) from="$2"; shift ;;
    --to) to="$2"; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

log() { printf -- '---> %s\n' "$*" >&2; }

[ -f "$from/config.json" ] || { echo "no Mailspring profile at $from" >&2; exit 1; }
case "$(cd "$to" 2>/dev/null && pwd -P)" in
  "$(cd "$from" && pwd -P)") echo "--to must not be the source profile" >&2; exit 1 ;;
esac

if [ "$fresh" = 1 ] && [ -d "$to" ]; then
  log "removing $to"
  rm -rf "$to"
fi

if [ ! -f "$to/config.json" ]; then
  log "copying profile $from -> $to"
  mkdir -p "$to"
  # Caches, crash dumps, locks and logs are not needed; the database is copied separately.
  rsync -a \
    --exclude 'Cache' --exclude 'Code Cache' --exclude 'GPUCache' --exclude 'GPUPersistentCache' \
    --exclude '*DawnCache' --exclude 'DawnWebGPUCache' --exclude 'Crashpad' --exclude 'compile-cache' \
    --exclude 'Singleton*' --exclude 'mailsync-*.log*' --exclude 'edgehill.db*' \
    "$from/" "$to/"
  # The live database is in WAL mode and in use by Mailspring.app; the backup API gives a
  # consistent copy without stopping it.
  log "copying edgehill.db (online backup)"
  sqlite3 "$from/edgehill.db" ".backup '$to/edgehill.db'"
else
  log "reusing $to (pass --fresh to copy again)"
fi

log "building the Rust engine"
(cd "$repo" && cargo build --release -p mailsync)
cp "$repo/target/release/mailsync" "$repo/apps/desktop/app/mailsync-rs"

if [ ! -d "$repo/apps/desktop/node_modules" ]; then
  log "installing desktop dependencies (npm ci, first run only)"
  (cd "$repo/apps/desktop" && npm ci)
fi

log "launching the desktop on the Rust engine with profile $to"
log "engine logs: $to/mailsync-<accountId>.log"
cd "$repo/apps/desktop"
MAILSYNC_ENGINE=rust exec npm start -- --config-dir-path="$to"
