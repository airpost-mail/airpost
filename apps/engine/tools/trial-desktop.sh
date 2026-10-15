#!/usr/bin/env bash
# Run the desktop client from source on the Rust engine, against a copy of an existing
# Mailspring profile, so the real profile and the running Mailspring.app are never touched.
#
#   apps/engine/tools/trial-desktop.sh [--accounts EMAIL,...] [--fresh] [--from DIR] [--to DIR]
#
# --accounts keep only these accounts in the copy (config.json and database); default: all
# --from     profile to copy (default: ~/Library/Application Support/Mailspring)
# --to       trial profile (default: ~/tern-trial); reused on later runs unless --fresh
# --fresh    delete the trial profile and copy it again
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
  if [ -n "$accounts" ]; then
    log "keeping only: $accounts"
    python3 -I - "$to" "$accounts" <<'PY'
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
  printf '%s\n' "${accounts:-all}" > "$to/.tern-trial-accounts"
else
  if [ -n "$accounts" ] && [ "$(cat "$to/.tern-trial-accounts" 2>/dev/null)" != "$accounts" ]; then
    echo "$to was made with different accounts; pass --fresh to recopy" >&2
    exit 1
  fi
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
