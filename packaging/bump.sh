#!/usr/bin/env bash
# Sets the version and checksums in the package manager recipes from a release's SHA256SUMS.
#
#   packaging/bump.sh VERSION SHA256SUMS [FORMULA [PKGBUILD]]
#       the Homebrew formula and the AUR PKGBUILD, from a vX.Y.Z release
#   packaging/bump.sh --cask VERSION SHA256SUMS [CASK]
#       the Homebrew cask, from a desktop-vX.Y.Z release (Airpost-AppleSilicon.zip, Airpost.zip)
#
# The files default to the ones in packaging/; pass copies to render them elsewhere (the
# release workflow attaches rendered ones to the release).

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cask=""
if [ "${1:-}" = --cask ]; then cask=1; shift; fi
[ $# -ge 2 ] || { sed -n '2,/^$/s/^# \{0,1\}//p' "$0" >&2; exit 2; }
version="${1#v}" sums="$2"
shift 2

if [ -n "$cask" ]; then
  set -- "${1:-$HERE/homebrew/Casks/airpost.rb}"
else
  set -- "${1:-$HERE/homebrew/Formula/airpost.rb}" "${2:-$HERE/aur/PKGBUILD}"
fi

python3 -I - "$cask" "$version" "$sums" "$@" <<'PY'
import re, sys

cask, version, sums_path, *files = sys.argv[1:]
sums = {}
with open(sums_path) as f:
    for line in f:
        parts = line.split()
        if len(parts) == 2 and re.fullmatch(r"[0-9a-f]{64}", parts[0]):
            sums[parts[1].lstrip("*")] = parts[0]

def need(name):
    if name not in sums:
        sys.exit(f"bump.sh: {sums_path} has no {name}")
    return sums[name]

def edit(path, fn):
    with open(path) as f:
        text = f.read()
    new = fn(text)
    with open(path, "w") as f:
        f.write(new)
    print(f"---> {path}", file=sys.stderr)

def sub1(pattern, repl, text):
    new, n = re.subn(pattern, repl, text, count=1, flags=re.M)
    if n != 1:
        sys.exit(f"bump.sh: no match for {pattern}")
    return new

if cask:
    def cask_fn(text):
        text = sub1(r'^(  version )"[^"]*"', rf'\g<1>"{version}"', text)
        text = sub1(r'(arm:\s+)"[0-9a-f]{64}"', rf'\g<1>"{need("Airpost-AppleSilicon.zip")}"', text)
        return sub1(r'(intel:\s+)"[0-9a-f]{64}"', rf'\g<1>"{need("Airpost.zip")}"', text)
    edit(files[0], cask_fn)
    sys.exit(0)

formula, pkgbuild = files

def formula_fn(text):
    # Homebrew reads the version from the urls. Each url names its target; the sha256 line
    # after it gets that archive's sum.
    def url_sha(m):
        name = f"airpost-{version}-{m.group(3)}.tar.gz"
        return f'{m.group(1)}v{version}/{name}"\n{m.group(4)}"{need(name)}"'
    text, n = re.subn(
        r'(url "[^"]*/download/)(v[^/]+)/airpost-[^"]+?-([a-z0-9_]+-(?:apple-darwin|unknown-linux-gnu))\.tar\.gz"\n(\s*sha256 )"[0-9a-f]{64}"',
        url_sha, text)
    if n != 4:
        sys.exit(f"bump.sh: expected 4 url/sha256 pairs in {formula}, found {n}")
    return text

def pkgbuild_fn(text):
    text = sub1(r"^pkgver=.*$", f"pkgver={version}", text)
    text = sub1(r"^pkgrel=.*$", "pkgrel=1", text)
    for arch in ("x86_64", "aarch64"):
        name = f"airpost-{version}-{arch}-unknown-linux-gnu.tar.gz"
        text = sub1(rf"^sha256sums_{arch}=\('[0-9a-f]{{64}}'\)", f"sha256sums_{arch}=('{need(name)}')", text)
    return text

edit(formula, formula_fn)
edit(pkgbuild, pkgbuild_fn)
PY
