// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import path from 'path';
import LessCompileCache from './compile-cache-less';
import type Package from './package';

/**
 * The colours the theme previews draw (the tiles in Preferences > Appearance and in the theme
 * picker). They aren't the theme's tokens: a preview shows every theme at once, beside the
 * active one, so each theme names its preview colours in styles/theme-colors.css as
 * `--theme-preview-<name>` properties, and the previews read them from that file.
 */
export const THEME_PREVIEW_FILE = 'theme-colors.css';

/** Preview colour -> the LESS variable it was, which a LESS-variables theme still sets. */
const LESS_VARIABLES: [string, string][] = [
  ['sidebar', '@source-list-bg'],
  ['content', '@background-primary'],
  ['accent', '@accent-primary'],
  ['text', '@text-color'],
  ['background', '@background-secondary'],
  ['active', '@component-active-color'],
  ['toolbar', '@toolbar-background-color'],
  ['panel', '@panel-background-color'],
  ['swatch-border', '@btn-border-color'],
  ['font', '@font-family'],
  ['font-weight', '@font-weight-semi-bold'],
  ['ring', '@blue'],
];

/** Preview colour name (without `--theme-preview-`) -> CSS value. */
export type ThemePreviewColors = Record<string, string>;

type PreviewTheme = Pick<Package, 'name' | 'getStylesheetsPath'>;

let lessCache: LessCompileCache | null = null;

/** The `--theme-preview-*` properties set in a stylesheet's text. */
export function parseThemePreviewColors(css: string): ThemePreviewColors {
  const colors: ThemePreviewColors = {};
  const text = css.replace(/\/\*[\s\S]*?\*\//g, '');
  for (const m of text.matchAll(/--theme-preview-([a-z-]+)\s*:\s*([^;}]+)/g)) {
    colors[m[1]] = m[2].trim().replace(/\s+/g, ' ').replace(/\( /g, '(').replace(/ \)/g, ')');
  }
  return colors;
}

/** A v1 theme's LESS that sets its preview colours (Mailspring themes ship theme-colors.less). */
const LESS_FILES = ['ui-variables.less', 'theme-colors.less'];

/**
 * A theme's preview colours: from its styles/theme-colors.css; for a v1 theme with LESS
 * variables (styles/ui-variables.less, or a Mailspring theme's theme-colors.less), from those;
 * otherwise Light's (the theme picker always drew such a theme in Light's colours). Throws if
 * the LESS doesn't compile.
 */
export function themePreviewColors(theme: PreviewTheme): ThemePreviewColors {
  const styles = theme.getStylesheetsPath();
  const file = path.join(styles, THEME_PREVIEW_FILE);
  if (fs.existsSync(file)) return parseThemePreviewColors(fs.readFileSync(file, 'utf8'));
  const lessFiles = LESS_FILES.filter((f) => fs.existsSync(path.join(styles, f)));
  if (lessFiles.length) return lessThemePreviewColors(theme, lessFiles);
  const { resourcePath } = AppEnv.getLoadSettings();
  const light = path.join(resourcePath, 'themes', 'ui-light', 'styles', THEME_PREVIEW_FILE);
  return parseThemePreviewColors(fs.readFileSync(light, 'utf8'));
}

/** The colours as a rule for a preview document (the theme picker's iframe). */
export function themePreviewColorsCSS(colors: ThemePreviewColors) {
  const decls = Object.entries(colors).map(
    ([name, value]) => `  --theme-preview-${name}: ${value};`
  );
  return `:root {\n${decls.join('\n')}\n}`;
}

/**
 * v1 compatibility, goes with LESS support at 2.0: a v1 theme has no theme-colors.css, so its
 * preview colours are its LESS variables, compiled over the literal base the way the theme
 * picker always compiled them.
 */
function lessThemePreviewColors(theme: PreviewTheme, lessFiles: string[]): ThemePreviewColors {
  const { configDirPath, resourcePath } = AppEnv.getLoadSettings();
  const styles = theme.getStylesheetsPath();
  const importPaths = [
    ...new Set([
      styles,
      path.join(resourcePath, 'static', 'styles', 'compat'),
      AppEnv.themes.getBaseTheme().getStylesheetsPath(),
    ]),
  ];
  if (lessCache) {
    lessCache.setImportPaths(importPaths);
  } else {
    lessCache = new LessCompileCache({ configDirPath, resourcePath, importPaths });
  }
  const lessPath = (p: string) => JSON.stringify(p.split(path.sep).join('/'));
  // The theme's variables after the rule: LESS takes a variable's last definition. compat is in
  // the import path, so `base/ui-variables` is its literal base/ui-variables.less.
  const less = [
    `@import "ui-variables";`,
    `.theme-preview {`,
    ...LESS_VARIABLES.map(([name, variable]) => `  --theme-preview-${name}: ${variable};`),
    `}`,
    `@import "base/ui-variables";`,
    ...lessFiles.map((f) => `@import ${lessPath(path.join(styles, f))};`),
  ].join('\n');
  const css = lessCache.cssForFile(path.join(styles, 'airpost-theme-preview.less'), less);
  return parseThemePreviewColors(css);
}
