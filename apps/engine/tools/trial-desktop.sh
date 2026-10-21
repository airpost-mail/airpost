#!/usr/bin/env bash
# Run the desktop client from source on the Rust engine, with a trial profile imported from a
# copy of an existing Mailspring profile, so the real profile and the running Mailspring.app are
# never touched.
#
#   apps/engine/tools/trial-desktop.sh [--accounts EMAIL,...] [--fresh] [--from DIR] [--to DIR]
#
# --accounts keep only these accounts in the copy (config.json and database); default: all
# --from     profile to copy (default: ~/Library/Application Support/Mailspring)
# --to       trial profile (default: ~/tern-trial); reused on later runs unless --fresh
# --fresh    delete the trial profile and copy it again
#
# A new trial profile starts empty. The Mailspring profile is copied (filtered by --accounts) to
# <to>.mailspring, and the desktop's first launch imports that copy with
# --import-from-mailspring, as Import from Mailspring does: edgehill.db becomes tern.db, which
# the Rust engine's migrate moves aside before it resyncs every account from scratch;
# import-legacy then carries over local drafts, snooze/send-later/reminder metadata and sync
# cursors. The import reads the saved passwords from the macOS keychain ("Mailspring Safe
# Storage"), so macOS asks to let the dev Electron binary read it, and stores them under "Tern
# Safe Storage".
set -euo pipefail

repo="$(cd "$(dirname "$0")/../../.." && pwd)"
from="$HOME/Library/Application Support/Mailspring"
to="$HOME/tern-trial"
fresh=0
accounts=""
while [ $# -gt 0 ]; do
  case "$1" in
    --fresh) fresh=1 ;;
    --accounts) accounts="$2"; shift ;;
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

to="${to%/}"
staging="$to.mailspring"
case "$(cd "$staging" 2>/dev/null && pwd -P)" in
  "$(cd "$from" && pwd -P)") echo "$staging must not be the source profile" >&2; exit 1 ;;
esac

if [ "$fresh" = 1 ]; then
  for dir in "$to" "$staging"; do
    if [ -d "$dir" ]; then
      log "removing $dir"
      rm -rf "$dir"
    fi
  done
fi

import_args=()
if [ ! -f "$to/config.json" ]; then
  # A profile left by an import that did not finish is Tern's (marked) or empty; anything else
  # is not this script's to replace.
  if [ -d "$to" ] && [ -n "$(ls -A "$to")" ] && [ ! -f "$to/.tern-profile" ]; then
    echo "$to holds files but is not a Tern profile; pick another --to" >&2
    exit 1
  fi
  log "copying profile $from -> $staging"
  rm -rf "$staging"
  mkdir -p "$staging"
  # Caches, crash dumps, locks and logs are not needed; the database is copied separately.
  rsync -a \
    --exclude 'Cache' --exclude 'Code Cache' --exclude 'GPUCache' --exclude 'GPUPersistentCache' \
    --exclude '*DawnCache' --exclude 'DawnWebGPUCache' --exclude 'Crashpad' --exclude 'compile-cache' \
    --exclude 'Singleton*' --exclude 'mailsync-*.log*' --exclude 'edgehill.db*' \
    "$from/" "$staging/"
  # The live database is in WAL mode and in use by Mailspring.app; the backup API gives a
  # consistent copy without stopping it.
  log "copying edgehill.db (online backup)"
  sqlite3 "$from/edgehill.db" ".backup '$staging/edgehill.db'"
  if [ -n "$accounts" ]; then
    log "keeping only: $accounts"
    python3 -I - "$staging" "$accounts" <<'PY'
import json, sqlite3, sys
to, wanted = sys.argv[1], [e.strip().lower() for e in sys.argv[2].split(",") if e.strip()]
cfg_path = f"{to}/config.json"
cfg = json.load(open(cfg_path))
accts = cfg["*"].get("accounts") or []
keep = [a for a in accts if a.get("emailAddress", "").lower() in wanted]
missing = set(wanted) - {a["emailAddress"].lower() for a in keep}
if missing:
    sys.exit(f"not in the profile: {', '.join(sorted(missing))}")
cfg["*"]["accounts"] = keep
json.dump(cfg, open(cfg_path, "w"), indent=2)
ids = [a["id"] for a in keep]
db = sqlite3.connect(f"{to}/edgehill.db")
marks = ",".join("?" * len(ids))
tables = [n for (n,) in db.execute("SELECT name FROM sqlite_master WHERE type='table'")]
for t in tables:
    if "accountId" in [r[1] for r in db.execute(f'PRAGMA table_info("{t}")')]:
        db.execute(f'DELETE FROM "{t}" WHERE accountId NOT IN ({marks})', ids)
for (key,) in db.execute("SELECT id FROM _State WHERE id LIKE 'cursor-%'").fetchall():
    if not any(key.endswith(i) for i in ids):
        db.execute("DELETE FROM _State WHERE id = ?", (key,))
db.commit()
db.execute("VACUUM")
print(f"kept {len(keep)} of {len(accts)} accounts", file=sys.stderr)
PY
  fi
  # Copied into the trial profile by the import.
  printf '%s\n' "${accounts:-all}" > "$staging/.tern-trial-accounts"
  # The import runs only into a profile with no config.json; an empty one is also marked as
  # Tern's, so Reset Accounts and Settings works on it.
  rm -rf "$to"
  mkdir -p "$to"
  import_args=(--import-from-mailspring="$staging")
else
  if [ -n "$accounts" ] && [ "$(cat "$to/.tern-trial-accounts" 2>/dev/null)" != "$accounts" ]; then
    echo "$to was made with different accounts; pass --fresh to recopy" >&2
    exit 1
  fi
  log "reusing $to (pass --fresh to copy again)"
fi

# tern-mcp and tern too: Preferences > Agents finds them next to tern-engine, as in a packaged build.
log "building the Rust engine, tern-mcp and tern"
(cd "$repo" && cargo build --release -p tern-engine -p tern-mcp -p tern)
for bin in tern-engine tern-mcp tern; do
  cp "$repo/target/release/$bin" "$repo/apps/desktop/$bin"
done

# A checkout from before the flatten still has the tooling-only node_modules (no react, and a
# Node-built better-sqlite3), so test for the app's runtime deps rather than the dir.
if [ ! -d "$repo/apps/desktop/node_modules/react" ] ||
  ! (cd "$repo/apps/desktop" && node scripts/check-native-abi.js >/dev/null 2>&1); then
  log "installing desktop dependencies (npm ci)"
  (cd "$repo/apps/desktop" && npm ci)
fi

log "launching the desktop on the Rust engine with profile $to"
log "engine log: $to/engine.log"
cd "$repo/apps/desktop"
exec npm start -- --config-dir-path="$to" ${import_args[@]+"${import_args[@]}"}
