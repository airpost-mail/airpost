#!/usr/bin/env bash
# PostToolUse hook: run eslint --fix on the single apps/desktop file that was just edited.
#
# `npm run lint` globs all of app/src, app/features, app/plugins and app/themes, which is far too
# slow to run after every edit. eslint's cost here is startup-dominated (~4s to
# load the @typescript-eslint, react, import and jsx-a11y plugins), so linting one
# file costs about the same as linting three -- but a fraction of the full pass.
#
# Exits 2 with eslint's report on stderr when unfixable problems remain, which
# feeds the errors back to Claude rather than leaving them to be discovered later.
set -uo pipefail

file=$(jq -r '.tool_input.file_path // .tool_response.filePath // ""')

case "$file" in
  */apps/desktop/app/src/*.ts | */apps/desktop/app/src/*.tsx | \
  */apps/desktop/app/features/*.ts | */apps/desktop/app/features/*.tsx | \
  */apps/desktop/app/plugins/*.ts | */apps/desktop/app/plugins/*.tsx | \
  */apps/desktop/app/themes/*.ts | */apps/desktop/app/themes/*.tsx) ;;
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

if ! output=$("./node_modules/.bin/eslint" --fix -c .eslintrc "$file" 2>&1); then
  printf 'eslint found problems it could not fix in %s:\n%s\n' "$file" "$output" >&2
  exit 2
fi
exit 0
