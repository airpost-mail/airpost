import { ipcRenderer } from 'electron';
import { Emitter, Disposable } from 'event-kit';
import path from 'path';
import fs from 'fs';
import rtlcss from 'rtlcss';
import { localized, isRTL } from '../intl/localize';
import LessCompileCache from './compile-cache-less';
import PackageManager from './manager';
import type Package from './package';
import { bundledAssetRoots } from './bundled-packages';

const CONFIG_THEME_KEY = 'core.theme';
const CONFIG_USE_SYSTEM_ACCENT_KEY = 'core.appearance.useSystemAccent';
const CONFIG_LIGHT_THEME_KEY = 'core.appearance.lightThemeName';
const CONFIG_DARK_THEME_KEY = 'core.appearance.darkThemeName';
const SYSTEM_ACCENT_SOURCE_PATH = 'system-accent:dynamic';
// The token sheets get synthetic source paths so EmailFrameStylesStore can find them on every
// platform, and so the LESS reload loops leave the compat sheet to activateThemePackage().
export const TOKEN_DEFAULTS_SOURCE_PATH = 'tokens:defaults';
export const THEME_TOKENS_SOURCE_PATH = 'tokens:theme';
const THEME_TOKENS_FILE = 'tokens.css';
const THEME_VARIABLES_FILE = 'ui-variables.less';
// The core stylesheets, in cascade order (they were static/styles/index.less's imports, and
// font-awesome its hoisted `@import (css)`). Paths under static/styles without the .css
// extension. Each loads as its own <style> at core priority, after the token defaults and
// before email-frame and the kit.
export const CORE_STYLESHEETS = [
  '../font-awesome.min.css',
  'variables',
  'common-ui-elements',
  'normalize',
  'type',
  'inputs',
  'buttons',
  'dropdowns',
  'workspace',
  'resizable',
  'selection',
  'utilities',
  'components/menu',
  'components/switch',
  'components/settings',
  'components/tokenizing-text-field',
  'components/extra',
  'components/list-tabular',
  'components/disclosure-triangle',
  'components/button-dropdown',
  'components/scroll-region',
  'components/spinner',
  'components/unsafe',
  'components/key-commands-region',
  'components/contenteditable',
  'components/editable-list',
  'components/outline-view',
  'components/fixed-popover',
  'components/date-picker-popover',
  'components/label-color-popover',
  'components/modal',
  'components/date-input',
  'components/empty-list-state',
  'components/syncing-list-state',
  'components/date-picker',
  'components/mini-month-view',
  'components/time-picker',
  'components/table',
  'components/editable-table',
  'components/contact-profile-photo',
  'components/multiselect-dropdown',
  'components/tutorial-overlay',
  'components/attachment-items',
  'components/search-bar',
  'components/code-snippet',
  'components/copy-button',
  'components/webview',
  'components/propose-time-popover',
  'components/nav-rail',
];
// The UI kit's plain-CSS sheets in static/styles/kit, in dependency order. Loaded raw after
// the core stylesheets at the same priority, so on a specificity tie the kit beats core
// and package and theme styles beat the kit (docs/desktop/ui-kit.md section 5).
const KIT_STYLESHEETS = [
  'base',
  'button',
  'icon-button',
  'switch',
  'checkbox',
  'radio-group',
  'segmented-control',
  'text-field',
  'textarea',
  'dialog',
  'popover',
  'tooltip',
  'dropdown-menu',
  'menu',
  'select',
  'tabs',
];

export type ThemeScheme = 'light' | 'dark';
export const AUTOMATIC_THEME_NAME = 'ui-automatic';
export const LIGHT_THEME_NAME = 'ui-light';
export const DARK_THEME_NAME = 'ui-dark';

function buildSystemAccentCSS(color: string): string {
  return `:root {
  --system-accent: ${color};
  --system-accent-dark: color-mix(in srgb, ${color}, black 10%);
}`;
}

/**
 * LESS for a LESS-variables theme's tokens: the literal base, then the theme's variables, then
 * the token mapping, so the theme's values win even if its file doesn't import the base.
 * Compile it with static/styles/compat ahead of the LESS surface (static/styles/compat/less) in
 * the import path.
 */
export function compatTokensSource(resourcePath: string, themeStylesPath: string) {
  const compat = path.join(resourcePath, 'static', 'styles', 'compat');
  const lessPath = (p: string) => JSON.stringify(p.split(path.sep).join('/'));
  return [
    `@import (reference) ${lessPath(path.join(compat, 'base', 'ui-variables.less'))};`,
    `@import (reference) ${lessPath(path.join(themeStylesPath, THEME_VARIABLES_FILE))};`,
    `@import ${lessPath(path.join(compat, 'tokens.less'))};`,
  ].join('\n');
}

function parseScheme(value: unknown): ThemeScheme | null {
  return value === 'light' || value === 'dark' ? value : null;
}

/**
 * The ThemeManager observes the user's theme selection and loads the stylesheets. The app's own
 * (static/styles, src/features, the bundled plugins and themes) are plain CSS, read as they are
 * (through rtlcss for an RTL locale). LESS is compiled only for third-party packages from the
 * profile, with the theme's variables in the @import path and the LESS surface in
 * static/styles/compat/less (v1 compatibility, dropped at 2.0); when the theme changes, their
 * stylesheets are recompiled against the new theme.
 *
 * Themes are theme tokens: CSS custom properties on :root (docs/desktop/theme-tokens.md).
 * static/styles/tokens/defaults.css loads at core priority with the Light values;
 * the active theme's styles/tokens.css loads after it at theme priority. A theme
 * with only a styles/ui-variables.less gets its tokens from static/styles/compat/tokens.less,
 * compiled against its variables, and its LESS compiles against the literal
 * static/styles/compat/base/ui-variables.less. The theme's scheme (package.json `scheme` and
 * `sidebarScheme`, or for a LESS-variables theme the lightness of its
 * backgrounds) becomes the theme-scheme-* and theme-sidebar-scheme-* body classes.
 *
 * This class is loosely based on Atom's Theme Manager but:
 *  - Only one theme is active at a time and always overrides ui-light
 *  - Theme packages are never "activated" by the package manager,
 *    they are only placed in the LESS import path.
 *  - ThemeManager directly updates <style> tags when recompiling LESS.
 */
export default class ThemeManager {
  private activeThemePackage: Package | null = null;
  private baseThemeOnly = false;
  private emitter = new Emitter();
  private styleSheetDisposablesBySourcePath: { [sourcePath: string]: Disposable } = {};
  private themeTokensDisposable: Disposable | null = null;
  private lessCache: LessCompileCache;
  private packageManager: PackageManager;
  private configDirPath: string;
  private resourcePath: string;
  private safeMode: boolean;
  // Read once like LessCompileCache does; specs set it to check the RTL path.
  rtl = isRTL;
  private rtlCache = new Map<string, { css: string; rtl: string }>();

  private themeValueCache: { emailTextColor?: string } = {};

  private _systemAccentColor: string | null = null;
  private _systemAccentDisposable: Disposable | null = null;
  private _systemDarkMode = false;

  constructor({
    packageManager,
    resourcePath,
    configDirPath,
    safeMode,
  }: {
    packageManager: PackageManager;
    resourcePath: string;
    configDirPath: string;
    safeMode: boolean;
  }) {
    this.packageManager = packageManager;
    this.resourcePath = resourcePath;
    this.configDirPath = configDirPath;
    this.safeMode = safeMode;

    // Prime the system dark-mode state synchronously so the initial theme
    // resolution in activateThemePackage() picks the right light/dark variant
    // without a flash of the wrong theme.
    this._systemDarkMode = !!ipcRenderer.sendSync('get-system-dark-mode-sync');

    this.lessCache = new LessCompileCache({
      configDirPath: this.configDirPath,
      resourcePath: this.resourcePath,
      importPaths: this.getImportPaths(),
    });

    AppEnv.config.onDidChange(CONFIG_THEME_KEY, () => this.updateThemePackageAndRecomputeLESS());
    AppEnv.config.onDidChange(CONFIG_USE_SYSTEM_ACCENT_KEY, () => this.applySystemAccent());
    AppEnv.config.onDidChange(CONFIG_LIGHT_THEME_KEY, () => {
      if (this.isAutomaticModeSelected()) this.updateThemePackageAndRecomputeLESS();
    });
    AppEnv.config.onDidChange(CONFIG_DARK_THEME_KEY, () => {
      if (this.isAutomaticModeSelected()) this.updateThemePackageAndRecomputeLESS();
    });

    ipcRenderer.on('system-accent-color-changed', (_event, color: string | null) => {
      this._systemAccentColor = color;
      this.applySystemAccent();
    });
    ipcRenderer.invoke('get-system-accent-color').then((color: string | null) => {
      this._systemAccentColor = color;
      this.applySystemAccent();
    });

    ipcRenderer.on('system-dark-mode-changed', (_event, darkMode: boolean) => {
      this._systemDarkMode = !!darkMode;
      if (this.isAutomaticModeSelected()) {
        this.updateThemePackageAndRecomputeLESS();
      }
    });
  }

  // New users (no `core.theme` saved in config) default to automatic mode so
  // the app follows their OS appearance. Existing users keep whatever they set.
  // In spec/safe mode we fall back to the concrete light theme so headless
  // test environments don't follow the auto-resolution path (which depends on
  // packages like ui-dark/ui-automatic not being present in the spec fixtures).
  private getConfiguredThemeName(): string {
    const configured = AppEnv.config.get(CONFIG_THEME_KEY);
    if (configured) return configured;
    if (this.safeMode || AppEnv.inSpecMode()) return LIGHT_THEME_NAME;
    return AUTOMATIC_THEME_NAME;
  }

  private isAutomaticModeSelected() {
    return !this.baseThemeOnly && this.getConfiguredThemeName() === AUTOMATIC_THEME_NAME;
  }

  // Called from the onboarding window to disable any custom theme
  forceBaseTheme() {
    this.baseThemeOnly = true;
    this.updateThemePackageAndRecomputeLESS();
  }

  updateThemePackageAndRecomputeLESS() {
    // Import paths first: activating a LESS-variables theme compiles its compat tokens.
    if (this.lessCache) {
      this.lessCache.setImportPaths(this.getImportPaths());
    }
    this.activateThemePackage();

    // reload all stylesheets attached to the dom
    for (const styleEl of Array.from(document.head.querySelectorAll('[source-path]'))) {
      if ((styleEl as any).sourcePath.endsWith('.less')) {
        styleEl.textContent = this.cssContentsOfStylesheet((styleEl as any).sourcePath);
      }
    }
    this.emitter.emit('did-change-active-themes');
  }

  reloadCoreStyles() {
    console.log('Reloading /static, the features and the bundled packages to incorporate changes');
    const reloadStylesIn = (folder: string) => {
      (fs.readdirSync(folder, { recursive: true }) as string[])
        .map((f) => path.join(folder, f))
        .filter((stylePath) => stylePath.endsWith('.css'))
        .forEach((stylePath) => {
          const styleEl = document.head.querySelector(`[source-path="${stylePath}"]`);
          if (styleEl) styleEl.textContent = this.cssContentsOfStylesheet(stylePath);
        });
    };
    reloadStylesIn(`${this.resourcePath}/static`);
    for (const root of bundledAssetRoots(this.resourcePath)) {
      reloadStylesIn(root);
    }
    if (this.activeThemePackage) this.loadThemeTokens(this.activeThemePackage);
  }

  // Essential: Invoke `callback` when style sheet changes associated with
  // updating the list of active themes have completed.
  //
  // * `callback` {Function}
  //
  onDidChangeActiveThemes(callback: () => void) {
    return this.emitter.on('did-change-active-themes', callback);
  }

  getBaseTheme() {
    return this.packageManager.getPackageNamed('ui-light');
  }

  getActiveTheme() {
    if (this.baseThemeOnly) {
      return this.getBaseTheme();
    }
    const configured = this.getConfiguredThemeName();
    if (configured === AUTOMATIC_THEME_NAME) {
      const resolvedName = this._systemDarkMode
        ? this.getConfiguredDarkThemeName()
        : this.getConfiguredLightThemeName();
      return this.packageManager.getPackageNamed(resolvedName) || this.getBaseTheme();
    }
    return this.packageManager.getPackageNamed(configured) || this.getBaseTheme();
  }

  getConfiguredLightThemeName(): string {
    return AppEnv.config.get(CONFIG_LIGHT_THEME_KEY) || LIGHT_THEME_NAME;
  }

  getConfiguredDarkThemeName(): string {
    return AppEnv.config.get(CONFIG_DARK_THEME_KEY) || DARK_THEME_NAME;
  }

  setLightTheme(packageName: string) {
    AppEnv.config.set(CONFIG_LIGHT_THEME_KEY, packageName);
  }

  setDarkTheme(packageName: string) {
    AppEnv.config.set(CONFIG_DARK_THEME_KEY, packageName);
  }

  // Returns the theme the user selected, which may be "ui-automatic" (unresolved).
  getActiveThemeSetting() {
    if (this.baseThemeOnly) {
      return this.getBaseTheme();
    }
    return (
      this.packageManager.getPackageNamed(this.getConfiguredThemeName()) || this.getBaseTheme()
    );
  }

  getAvailableThemes() {
    return this.packageManager.getAvailablePackages().filter((p) => p.isTheme());
  }

  // Set the active theme.
  //  * `packageName` {string} - the theme package to activate
  //
  setActiveTheme(packageName: string) {
    AppEnv.config.set(CONFIG_THEME_KEY, packageName);
    // because we're observing the config, changes will be applied
  }

  activateThemePackage() {
    const next = this.getActiveTheme();
    if (this.activeThemePackage === next) {
      return;
    }

    // Turn off the old active theme and enable the new theme. This
    // allows the theme to have code and random stylesheets of it's own.
    if (this.activeThemePackage) {
      this.activeThemePackage.deactivate();
    }
    const { scheme, sidebarScheme } = this.loadThemeTokens(next);
    next.activate();

    // Update the body classList to include the theme name so plugin
    // developers can alter their plugin's styles based on the theme.
    for (const cls of Array.from(document.body.classList)) {
      if (cls.startsWith('theme-')) {
        document.body.classList.remove(cls);
      }
    }
    document.body.classList.add(`theme-${this.getBaseTheme().name}`);
    document.body.classList.add(`theme-${this.getActiveTheme().name}`);
    document.body.classList.add(`theme-scheme-${scheme}`);
    document.body.classList.add(`theme-sidebar-scheme-${sidebarScheme}`);

    this.themeValueCache = {};
    this.activeThemePackage = next;
  }

  getImportPaths() {
    const paths = [this.getBaseTheme().getStylesheetsPath()];
    const active = this.getActiveTheme();
    if (active) {
      // Ahead of the LESS surface, so the theme's `@import 'base/ui-variables'` gets the literals.
      if (this.usesCompatTokens(active)) paths.unshift(this.compatStylesPath());
      paths.unshift(active.getStylesheetsPath());
    }
    return paths;
  }

  /** A theme that sets LESS variables and no tokens, so its tokens come from static/styles/compat/tokens.less. */
  usesCompatTokens(theme: Pick<Package, 'getStylesheetsPath'>) {
    const styles = theme.getStylesheetsPath();
    return (
      !fs.existsSync(path.join(styles, THEME_TOKENS_FILE)) &&
      fs.existsSync(path.join(styles, THEME_VARIABLES_FILE))
    );
  }

  private compatStylesPath() {
    return path.join(this.resourcePath, 'static', 'styles', 'compat');
  }

  /** The theme's tokens as CSS: its styles/tokens.css, or the compat tokens. Empty for neither. */
  themeTokensCSS(theme: Package): string {
    const tokensPath = path.join(theme.getStylesheetsPath(), THEME_TOKENS_FILE);
    if (fs.existsSync(tokensPath)) return fs.readFileSync(tokensPath, 'utf8');
    if (!this.usesCompatTokens(theme)) return '';
    const source = compatTokensSource(this.resourcePath, theme.getStylesheetsPath());
    try {
      return this.lessCache.cssForFile(path.join(this.compatStylesPath(), 'theme.less'), source);
    } catch (error) {
      console.error(`Error compiling theme tokens for ${theme.name}`, error);
      return '';
    }
  }

  /** package.json `scheme` / `sidebarScheme`; else what compat tokens derived; else light. */
  themeSchemes(
    theme: Package,
    tokensCSS: string
  ): { scheme: ThemeScheme; sidebarScheme: ThemeScheme } {
    const derived = (prop: string) =>
      parseScheme(new RegExp(`${prop}:\\s*(light|dark)`).exec(tokensCSS)?.[1]);
    const scheme = theme.scheme() || derived('--compat-scheme') || 'light';
    const sidebarScheme = theme.sidebarScheme() || derived('--compat-sidebar-scheme') || scheme;
    return { scheme, sidebarScheme };
  }

  private loadThemeTokens(theme: Package) {
    if (this.themeTokensDisposable) {
      this.themeTokensDisposable.dispose();
      this.themeTokensDisposable = null;
    }
    const css = this.themeTokensCSS(theme);
    if (css) {
      this.themeTokensDisposable = AppEnv.styles.addStyleSheet(css, {
        sourcePath: THEME_TOKENS_SOURCE_PATH,
        priority: 1,
      });
    }
    return this.themeSchemes(theme, css);
  }

  // Writes a <style> tag setting the --system-accent CSS custom properties
  // when the user opted into system accent and the OS provided a color.
  // Otherwise removes the tag so per-theme fallbacks in LESS take over.
  applySystemAccent() {
    const enabled = AppEnv.config.get(CONFIG_USE_SYSTEM_ACCENT_KEY) !== false;
    const color = enabled ? this._systemAccentColor : null;

    if (this._systemAccentDisposable) {
      this._systemAccentDisposable.dispose();
      this._systemAccentDisposable = null;
    }
    if (color) {
      this._systemAccentDisposable = AppEnv.styles.addStyleSheet(buildSystemAccentCSS(color), {
        sourcePath: SYSTEM_ACCENT_SOURCE_PATH,
        priority: 2,
      });
    }
  }

  // Section: Private
  // ------

  requireStylesheet(stylesheetPath: string) {
    const sourcePath = this.resolveStylesheetPath(stylesheetPath);
    if (!sourcePath) {
      throw new Error(localized(`Could not find a file at path '%@'`, stylesheetPath));
    }
    const content = this.cssContentsOfStylesheet(sourcePath);
    this.styleSheetDisposablesBySourcePath[sourcePath] = AppEnv.styles.addStyleSheet(content, {
      priority: -1,
      sourcePath,
    });
  }

  loadStaticStylesheets() {
    const defaults = path.join(this.resourcePath, 'static', 'styles', 'tokens', 'defaults.css');
    this.styleSheetDisposablesBySourcePath[TOKEN_DEFAULTS_SOURCE_PATH] =
      AppEnv.styles.addStyleSheet(fs.readFileSync(defaults, 'utf8'), {
        sourcePath: TOKEN_DEFAULTS_SOURCE_PATH,
        priority: -1,
      });
    for (const sheet of this.coreStylesheetPaths()) this.requireStylesheet(sheet);
    this.requireStylesheet(path.join(this.resourcePath, 'static', 'styles', 'email-frame.css'));
    for (const name of KIT_STYLESHEETS) {
      this.requireStylesheet(
        path.join(this.resourcePath, 'static', 'styles', 'kit', `${name}.css`)
      );
    }
  }

  /** CORE_STYLESHEETS as files, in order. */
  coreStylesheetPaths() {
    const styles = path.join(this.resourcePath, 'static', 'styles');
    return CORE_STYLESHEETS.map((name) => {
      const file = path.join(styles, name.endsWith('.css') ? name : `${name}.css`);
      if (!fs.existsSync(file)) throw new Error(`Missing core stylesheet ${file}`);
      return file;
    });
  }

  /** static/styles/variables.css, which the message iframe gets along with the tokens. */
  coreVariablesPath() {
    return path.join(this.resourcePath, 'static', 'styles', 'variables.css');
  }

  /**
   * The app's own stylesheets, the ones that were LESS: under static/styles except the kit, the
   * tokens and the compat surface, and the features' and bundled packages' styles.
   */
  isAppStylesheet(stylesheetPath: string) {
    const inside = (dir: string) => {
      const rel = path.relative(dir, stylesheetPath);
      return !!rel && !rel.startsWith('..') && !path.isAbsolute(rel);
    };
    const styles = path.join(this.resourcePath, 'static', 'styles');
    if (inside(styles)) {
      return !['kit', 'tokens', 'compat'].some((dir) => inside(path.join(styles, dir)));
    }
    return bundledAssetRoots(this.resourcePath).some(inside);
  }

  resolveStylesheetPath(stylesheetPath: string) {
    if (path.extname(stylesheetPath).length > 0) {
      const p = path.resolve(__dirname, stylesheetPath);
      return fs.existsSync(p) ? p : null;
    }
    for (const ext of ['css', 'less']) {
      const p = path.resolve(__dirname, `${stylesheetPath}.${ext}`);
      if (fs.existsSync(p)) return p;
    }
    return null;
  }

  cssContentsOfStylesheet(stylesheetPath: string) {
    const ext = path.extname(stylesheetPath);

    if (ext === '.less') {
      return this.cssContentsOfLessStylesheet(stylesheetPath);
    } else if (ext === '.css') {
      const css = fs.readFileSync(stylesheetPath, 'utf8');
      // LESS output went through rtlcss (LessCompileCache), so the CSS that replaced it does too.
      return this.rtl && this.isAppStylesheet(stylesheetPath)
        ? this.rtlCSS(stylesheetPath, css)
        : css;
    } else {
      throw new Error(localized(`Tern does not support stylesheets with the extension: %@`, ext));
    }
  }

  private rtlCSS(stylesheetPath: string, css: string) {
    const cached = this.rtlCache.get(stylesheetPath);
    if (cached && cached.css === css) return cached.rtl;
    const rtl = rtlcss.process(css);
    this.rtlCache.set(stylesheetPath, { css, rtl });
    return rtl;
  }

  // Third-party LESS only (v1 compatibility, dropped at 2.0): the app's own stylesheets are CSS.
  cssContentsOfLessStylesheet(lessStylesheetPath: string) {
    try {
      const less = fs.readFileSync(lessStylesheetPath, 'utf8').toString();
      return this.lessCache.cssForFile(lessStylesheetPath, less);
    } catch (error) {
      // less throws objects with `line` for syntax errors.
      const failure = typeof error === 'object' && error !== null ? error : {};
      const errorMessage = 'message' in failure ? failure.message : undefined;
      const line = 'line' in failure ? failure.line : undefined;
      let message = `Error loading Less stylesheet: ${lessStylesheetPath}`;
      let detail: unknown = errorMessage;

      if (line !== undefined) {
        message = `Error compiling Less stylesheet: ${lessStylesheetPath}`;
        detail = `Line number: ${line}\n${errorMessage}`;
      }
      console.error(message, { detail, dismissable: true });
      console.error(detail);

      ipcRenderer.send('encountered-theme-error', { message, detail });

      return '';
    }
  }

  // Helpers for theme values

  getEmailTextColor() {
    if (!this.themeValueCache.emailTextColor) {
      // Quite surprising that Chrome is cool with this...
      const innerBodyEl = document.createElement('body');
      const wrapEl = document.createElement('span');
      wrapEl.className = 'ignore-in-parent-frame';
      wrapEl.appendChild(innerBodyEl);
      document.body.appendChild(wrapEl);

      const style = window.getComputedStyle(innerBodyEl);
      this.themeValueCache.emailTextColor = style.getPropertyValue('color');
      wrapEl.remove();
    }
    return this.themeValueCache.emailTextColor;
  }
}
