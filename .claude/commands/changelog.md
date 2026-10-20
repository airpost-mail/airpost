# Plugin API changelog entry

Add the entry for a plugin API change to `apps/desktop/CHANGELOG.md`.

Tern's desktop changelog is the plugin API log: one `Plugin API X.Y.Z:` block per version
under `## Unreleased (Tern)`, newest first. App release numbering isn't decided yet, so don't
bump `apps/desktop/app/package.json`, its lockfile or the Linux metainfo
(`apps/desktop/build/resources/linux/com.ternmail.tern.metainfo.xml.in`).

## Instructions

1. Read `PLUGIN_API_VERSION` and its doc comment in `apps/desktop/app/src/plugin-api-version.ts`.
   The comment lists what each version added; the changelog needs a block for each version
   it has no block for yet.

2. Find what changed for plugins since the last logged version:
   ```bash
   git log --oneline -- apps/desktop/app/src/plugin-api-version.ts apps/desktop/app/src/global
   ```
   Read the diffs of the commits that moved the version.

3. Write the block. Each bullet says what a plugin author can now use or must change: new
   exports and manifest fields, deprecations (and the version that removes them), and
   anything that breaks plugins within their `engines.mailspring` range. Name the export in
   backticks; point to `docs/desktop/PLUGIN_SYSTEM_ARCHITECTURE.md` when it documents the
   change.

```markdown
Plugin API 1.X.0:

- `NewExport.method(args)`: what it does for a plugin.
- `OldExport` is deprecated and removed in 2.0.0: what it does now, and what to use instead.
```

$ARGUMENTS
