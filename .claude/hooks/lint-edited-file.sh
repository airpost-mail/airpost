#!/usr/bin/env bash
# PostToolUse hook: run eslint --fix on the single apps/desktop file that was just edited.
#
# `npm run lint` globs all of src, plugins, themes and tests, which is far too
# slow to run after every edit. eslint's cost here is startup-dominated (~4s to
# load the @typescript-eslint, react, import and jsx-a11y plugins), so linting one
# file costs about the same as linting three -- but a fraction of the full pass.
#
# Exits 2 with eslint's report on stderr when unfixable problems remain, which
# feeds the errors back to Claude rather than leaving them to be discovered later.
set -uo pipefail

file=$(jq -r '.tool_input.file_path // .tool_response.filePath // ""')

case "$file" in
  */apps/desktop/src/*.ts | */apps/desktop/src/*.tsx | \
  */apps/desktop/tests/*.ts | */apps/desktop/tests/*.tsx | \
  */apps/desktop/plugins/*.ts | */apps/desktop/plugins/*.tsx | \
  */apps/desktop/themes/*.ts | */apps/desktop/themes/*.tsx) ;;
  *) exit 0 ;;
esac

[ -f "$file" ] || exit 0

# The desktop dir of the checkout the file is in, so edits in a worktree lint against that
# worktree's config; falls back to the project's own checkout when it has no node_modules.
desktop="${file%%/apps/desktop/*}/apps/desktop"
if [ ! -x "$desktop/node_modules/.bin/eslint" ]; then
  desktop="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}/apps/desktop"
fi
[ -x "$desktop/node_modules/.bin/eslint" ] || exit 0
cd "$desktop" || exit 0

# Relative to the desktop dir: with an absolute path under a symlinked dir (/tmp on macOS)
# typescript-eslint doesn't match the file to tsconfig.json's include.
target="$file"
case "$file" in "$desktop"/*) target="${file#"$desktop"/}" ;; esac

if ! output=$("./node_modules/.bin/eslint" --fix -c .eslintrc "$target" 2>&1); then
  printf 'eslint found problems it could not fix in %s:\n%s\n' "$file" "$output" >&2
  exit 2
fi
exit 0
