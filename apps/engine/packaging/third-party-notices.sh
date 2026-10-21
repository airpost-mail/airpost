#!/usr/bin/env bash
# Writes THIRD-PARTY-NOTICES.txt for a Rust binary: the licences of every crate in its normal
# and build dependency graph (cargo-about, with the repo root's about.toml and about.hbs), then
# the NOTICE files those crates ship (Apache-2.0 section 4(d) asks for them; cargo-about does
# not collect them).
#
#   third-party-notices.sh OUTPUT [MANIFEST]
#
# MANIFEST defaults to apps/engine/Cargo.toml (the `tern-engine` engine); the backend image uses
# apps/backend/Cargo.toml. Needs cargo-about 0.9 (`cargo install --locked cargo-about
# --features cli`) and python3. Fails when a crate's licence cannot be identified or is not
# in about.toml's accepted list.

set -euo pipefail

[ $# -ge 1 ] || { sed -n '3,/^$/s/^# \{0,1\}//p' "$0" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
OUT="$1"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
MANIFEST="${2:-$REPO_ROOT/apps/engine/Cargo.toml}"
command -v cargo-about >/dev/null || { echo "third-party-notices.sh: cargo-about not installed" >&2; exit 1; }

mkdir -p "$(dirname "$OUT")"
cargo about generate --locked --fail -c "$REPO_ROOT/about.toml" -m "$MANIFEST" \
  -o "$OUT" "$REPO_ROOT/about.hbs"

meta="$(mktemp)"
trap 'rm -f "$meta"' EXIT
cargo metadata --locked --format-version 1 --manifest-path "$MANIFEST" > "$meta"

python3 -I - "$meta" "$OUT" <<'PY'
import json, os, re, sys

meta_path, out_path = sys.argv[1], sys.argv[2]
with open(out_path, encoding="utf-8") as f:
    notices = f.read()
used = set(re.findall(r"^  (\S+) (\S+)(?: <[^>]*>)?$", notices, re.M))
with open(meta_path, encoding="utf-8") as f:
    packages = json.load(f)["packages"]

sections = []
for pkg in sorted(packages, key=lambda p: (p["name"], p["version"])):
    if (pkg["name"], pkg["version"]) not in used:
        continue
    root = os.path.dirname(pkg["manifest_path"])
    for name in sorted(os.listdir(root)):
        path = os.path.join(root, name)
        if re.match(r"NOTICE", name, re.I) and os.path.isfile(path):
            with open(path, encoding="utf-8", errors="replace") as f:
                text = f.read().strip()
            sections.append(f"{pkg['name']} {pkg['version']}: {name}\n\n{text}\n")

with open(out_path, "a", encoding="utf-8") as f:
    f.write("\n" + "=" * 80 + "\nNOTICE files\n" + "=" * 80 + "\n\n")
    f.write("\n".join(sections) if sections else "No crate in this graph ships a NOTICE file.\n")
PY
echo "---> $OUT" >&2
