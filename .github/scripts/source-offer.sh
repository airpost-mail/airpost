#!/usr/bin/env bash
# Writes OUT_DIR/SOURCE.txt, the directions to the Corresponding Source that GPLv3 section 6(d)
# requires next to every binary we publish, and makes sure TAG names the commit being built so
# the link outlives the branch. Run in a GitHub Actions job with `contents: write` and GH_TOKEN.
#
#   source-offer.sh TAG OUT_DIR WHAT
#
# WHAT names the artifact in the text ("the Tern desktop app for macOS"). With --no-tag the tag
# must already exist at the commit (the engine's `engine-v*` tags are pushed by hand).

set -euo pipefail

create=1
if [ "${1:-}" = --no-tag ]; then create=""; shift; fi
[ $# -eq 3 ] || { sed -n '2,/^$/s/^# \{0,1\}//p' "$0" >&2; exit 2; }
tag="$1" out="$2" what="$3"
: "${GITHUB_SERVER_URL:?}" "${GITHUB_REPOSITORY:?}" "${GITHUB_SHA:?}"
repo_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY"

# The commit a tag names; annotated tags are peeled.
tag_commit() {
  local ref type sha
  ref="$(gh api "repos/$GITHUB_REPOSITORY/git/ref/tags/$tag" --jq '.object.type + " " + .object.sha' 2>/dev/null)" || return 0
  read -r type sha <<< "$ref"
  if [ "$type" = tag ]; then
    gh api "repos/$GITHUB_REPOSITORY/git/tags/$sha" --jq .object.sha
  else
    echo "$sha"
  fi
}

existing="$(tag_commit)"
if [ -z "$existing" ]; then
  [ -n "$create" ] || { echo "source-offer.sh: tag $tag does not exist" >&2; exit 1; }
  # Parallel matrix jobs race to create the same tag; the loser re-reads it.
  if ! gh api "repos/$GITHUB_REPOSITORY/git/refs" -f ref="refs/tags/$tag" -f sha="$GITHUB_SHA" >/dev/null; then
    existing="$(tag_commit)"
    [ -n "$existing" ] || { echo "source-offer.sh: could not create tag $tag" >&2; exit 1; }
  fi
fi
if [ -n "$existing" ] && [ "$existing" != "$GITHUB_SHA" ]; then
  echo "source-offer.sh: tag $tag points at $existing, not $GITHUB_SHA" >&2
  exit 1
fi

mkdir -p "$out"
cat > "$out/SOURCE.txt" <<EOF
This is $what, built from commit $GITHUB_SHA.

Tern is free software: you can redistribute it and/or modify it under the terms of the GNU
General Public License, version 3 only (GPL-3.0-only). It comes with no warranty. The licence
and the third-party notices ship with the program.

The complete corresponding source code for this build is the tag $tag:

  $repo_url/tree/$tag
  $repo_url/archive/refs/tags/$tag.tar.gz

The Rust and npm dependencies are pinned by Cargo.lock and apps/desktop/package-lock.json in
that tree. Engine releases also carry an archive of the
source with every crate vendored.
EOF
echo "---> $out/SOURCE.txt ($tag)" >&2
