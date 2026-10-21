/* eslint global-require: 0 */
/* eslint import/no-dynamic-require: 0 */
import _ from 'underscore';
import path from 'path';
import { ipcRenderer } from 'electron';
import { Emitter } from 'event-kit';
import { mapSourcePosition } from 'source-map-support';
import { localized, isRTL, initializeLocalization } from '../intl/localize';

import { APIError } from '../data/errors';
import WindowEventHandler, { UnloadCallback } from './event-handler';
import { isWaylandSession } from '../platform/linux-desktop';
import {
  App,
  CurrentWindow,
  CurrentWindowHandle,
  Dialogs,
  FileListCache,
  Screen,
} from '../platform/app-api';

function ensureInteger(f: number, fallback: number) {
  let int = f;
  if (isNaN(f) || f === undefined || f === null) {
    int = fallback;
  }
  return Math.round(int);
}

let loadSettings = JSON.parse(decodeURIComponent(window.location.search.substr(14)));

// A property of a thrown value or other untyped input, which can be anything.
function errorField(error: unknown, key: string): unknown {
  if ((typeof error !== 'object' && typeof error !== 'function') || error === null) {
    return undefined;
  }
  return Reflect.get(error, key);
}

// Electron's dialogs show a missing message or detail as empty.
function dialogText(value: unknown) {
  return value == null ? '' : String(value);
}

// What the main process's AutoUpdateManager sends with update-available.
export interface UpdateDetails {
  releaseVersion?: string;
  releaseNotes?: string;
}

// Essential: AppEnv global for dealing with packages, themes, menus, and the window.
//
// The singleton of this class is always available as the `AppEnv` global.
export default class AppEnvConstructor {
  // Returns the load settings hash associated with the current window.

  emitter = new Emitter();
  loadTime: number | null = null;
  config: import('../config/config').default;
  keymaps: import('./keymap-manager').default;
  commands: import('./command-registry').default;
  packages: import('../packages/manager').default;
  styles: import('../packages/style-manager').default;
  themes: import('../packages/theme-manager').default;
  spellchecker: typeof import('./spellchecker').default;
  menu: import('./menu-manager').default;
  windowEventHandler: import('./event-handler').default;
  actionBridge: import('../data/action-bridge').default;
  engineBridge: import('../engine/client/bridge').default;
  errorLogger: any;
  savedState: any;
  isReloading = false;

  // True once the constructor finishes. Guards against config IPC arriving
  // re-entrantly mid-boot, before our other singletons are assigned.
  bootComplete = false;

  /*
  Section: Construction and Destruction
  */
  constructor() {
    // self-assign us to `window` so things called from this function can reference
    // window.AppEnv early.
    window.AppEnv = this;

    this.enhanceEventObject();
    this.setupErrorLogger();
    this.restoreWindowState();

    const { devMode, safeMode, resourcePath, configDirPath, windowType } = this.getLoadSettings();
    const specMode = this.inSpecMode();

    initializeLocalization({ configDirPath });

    // Add 'src/packages/api/' to module search path.
    const pluginApiPath = path.join(resourcePath, 'src', 'packages', 'api');
    require('app-module-path').addPath(pluginApiPath);

    const Config = require('../config/config').default;
    const KeymapManager = require('./keymap-manager').default;
    const CommandRegistry = require('./command-registry').default;
    const PackageManager = require('../packages/manager').default;
    const ThemeManager = require('../packages/theme-manager').default;
    const StyleManager = require('../packages/style-manager').default;
    const MenuManager = require('./menu-manager').default;

    document.body.classList.add(`platform-${process.platform}`);
    document.body.classList.add(`window-type-${windowType}`);

    // Make react.js faster
    if (!devMode && process.env.NODE_ENV == null) {
      process.env.NODE_ENV = 'production';
    }

    // Setup config and load it immediately so it's available to our singletons
    // and doesn't emit events later when it loads
    this.config = new Config({ configDirPath, resourcePath });
    this.loadConfig();

    this.keymaps = new KeymapManager({ configDirPath, resourcePath });
    this.commands = new CommandRegistry();
    this.packages = new PackageManager({
      devMode,
      configDirPath,
      resourcePath,
      safeMode,
      specMode,
    });
    this.styles = new StyleManager();
    this.themes = new ThemeManager({
      packageManager: this.packages,
      configDirPath,
      resourcePath,
      safeMode,
    });
    this.themes.activateThemePackage();

    this.spellchecker = require('./spellchecker').default;
    this.menu = new MenuManager({ resourcePath });
    if (process.platform === 'win32') {
      CurrentWindow.setMenuBarVisibility(false);
    }

    this.windowEventHandler = new WindowEventHandler();

    // Adds the Rx.Observable.from* helpers.
    require('../data/query/observables');

    // The plugin API barrel (`mailspring-exports`). Requiring it registers the models and
    // tasks with the DatabaseObjectRegistry and loads the stores nothing else requires.
    require('../packages/api/mailspring-exports');

    const ActionBridge = require('../data/action-bridge').default;
    this.actionBridge = new ActionBridge(ipcRenderer);

    const EngineBridge = require('../engine/client/bridge').default;
    this.engineBridge = new EngineBridge();

    process.title = `Tern ${this.getWindowType()}`;
    this.onWindowPropsReceived(() => {
      process.title = `Tern ${this.getWindowType()}`;
    });

    this.bootComplete = true;
  }

  // This ties window.onerror and process.uncaughtException,handledRejection
  // to the publically callable `reportError` method. This will take care of
  // reporting errors if necessary and hooking into error handling
  // callbacks.
  //
  // Start our error reporting to the backend and attach error handlers
  // to the window and the Bluebird Promise library, converting things
  // back through the sourcemap as necessary.
  setupErrorLogger() {
    const ErrorLogger = require('../platform/error-logger');
    this.errorLogger = new ErrorLogger({
      inSpecMode: this.inSpecMode(),
      inDevMode: this.inDevMode(),
      resourcePath: this.getLoadSettings().resourcePath,
    });

    // https://developer.mozilla.org/en-US/docs/Web/API/GlobalEventHandlers/onerror
    window.onerror = (message, url, line, column, originalError) => {
      if (!originalError && !message) return;
      if (!originalError) originalError = new Error(`${message}`);

      if (!this.inDevMode() || url === undefined || line === undefined || column === undefined) {
        return this.reportError(originalError, { url, line, column });
      }
      const { line: newLine, column: newColumn } = mapSourcePosition({ source: url, line, column });
      return this.reportError(originalError, { url, line: newLine, column: newColumn });
    };

    process.on('uncaughtException', (error: Error) => {
      this.reportError(error);
    });

    process.on('unhandledRejection', (error: Error) => {
      this.reportError(error);
    });

    window.addEventListener('unhandledrejection', (e) => {
      // This event is supposed to look like {reason, promise}, according to
      // https://developer.mozilla.org/en-US/docs/Web/API/PromiseRejectionEvent
      // In practice, it can have different shapes, so we make our best guess
      if (e instanceof Error) {
        this.reportError(e);
        return;
      }
      if (e.reason) {
        this.reportError(e.reason);
        return;
      }
      if ((e as any).detail && (e as any).detail.reason) {
        this.reportError((e as any).detail.reason);
        return;
      }
    });

    return null;
  }

  // Public: report an error through the `ErrorLogger`
  //
  // The difference between this and `ErrorLogger.reportError` is that
  // `AppEnv.reportError` hooks into test failures and dev tool popups.
  //
  reportError(error: unknown, extra: any = {}) {
    // Check if this error should be ignored and not reported to Sentry
    // Errors marked noSentry have already been displayed to the user via a dialog.
    if (errorField(error, 'noSentry')) {
      return;
    }

    const errorMessage = `${error}`.toLowerCase();

    // ResizeObserver errors happen infrequently but spam Sentry with thousands of reports
    if (errorMessage.includes('resizeobserver') || errorMessage.includes('resize observer')) {
      return;
    }

    // File system errors that are commonly "file not found" or similar
    if (errorMessage.includes('enoent')) {
      return;
    }

    // Authentication errors - these are user configuration issues, not bugs
    if (
      errorMessage.includes('authentication error') ||
      errorMessage.includes('check your username and password') ||
      (errorMessage.includes('smtp') && errorMessage.includes('authentication'))
    ) {
      return;
    }

    try {
      const matchedPackages = this._findPluginsFromError(error);
      extra.pluginIds = matchedPackages.map((pkg) => pkg.name);

      // Errors thrown from user-installed third-party (community) plugins are
      // outside our control (stale APIs, missing deps, plugin bugs) and
      // reporting them to Sentry creates noise we cannot act on — mirrors the
      // reasoning in PackageManager.activatePackage for activation failures.
      // Only skip when the crash's culprit frame (the top of the stack,
      // i.e. where it was actually thrown) is inside a community plugin —
      // not merely because some plugin frame appears elsewhere in the call
      // chain. That keeps a core regression reachable through a plugin's
      // callback from being silently swallowed.
      const culpritPackages = this._culpritPackagesFromError(error);
      if (
        culpritPackages.length > 0 &&
        culpritPackages.every((pkg) => !pkg.directory.startsWith(this.packages.resourcePath))
      ) {
        return;
      }
    } catch (err) {
      // can happen when an error is thrown very early
      extra.pluginIds = [];
    }

    if (error instanceof APIError) {
      // API Errors are logged by our backend and happen all the time (offline, etc.)
      // Don't clutter the front-end metrics with these.
      return;
    }

    if (this.inSpecMode()) {
      const jasmine = global.jasmine || window.jasmine;
      if (jasmine) {
        jasmine.getEnv().currentSpec.fail(error);
      }
    } else if (this.inDevMode()) {
      if (!this.isDevToolsOpened()) {
        this.openDevTools();
        this.executeJavaScriptInDevTools("DevToolsAPI.showPanel('console')");
      }
    }

    const message = errorField(error, 'message');
    if (
      typeof message === 'string' &&
      message &&
      ['EROFS', 'EPIPE', 'ENOSPC', 'EBUSY', 'EACCES', 'UNKNOWN: unknown error, open'].find(
        (prefix) => message.startsWith(prefix)
      )
    ) {
      // Don't fill Sentry with "couldn't open an attachment" type errors.
      return;
    }

    this.errorLogger.reportError(error, extra);
  }

  _findPluginsFromError(error: unknown) {
    const stack = errorField(error, 'stack');
    if (!stack || typeof stack !== 'string') {
      return [];
    }
    const stackPaths = stack.match(/((?:\/[\w-_]+)+)/g) || [];
    const stackPathComponents = [...new Set(stackPaths.flatMap((p: string) => p.split('/')))];

    return this.packages
      .getActivePackages()
      .filter((pkg) => stackPathComponents.includes(path.basename(pkg.directory)));
  }

  // Like `_findPluginsFromError`, but only considers the top frame of the
  // stack (the culprit — where the error was actually thrown), not every
  // frame in the call chain. V8 stacks are newest-first, so that's the
  // first "at ..." line after the message.
  _culpritPackagesFromError(error: unknown) {
    const stack = errorField(error, 'stack');
    if (!stack || typeof stack !== 'string') {
      return [];
    }
    const topFrame = stack.split('\n')[1] || '';
    const stackPaths = topFrame.match(/((?:\/[\w-_]+)+)/g) || [];
    const stackPathComponents = [...new Set(stackPaths.flatMap((p: string) => p.split('/')))];

    return this.packages
      .getActivePackages()
      .filter((pkg) => stackPathComponents.includes(path.basename(pkg.directory)));
  }

  // The v1 plugin API name of engineBridge.
  get mailsyncBridge() {
    return this.engineBridge;
  }

  /*
  Section: Event Subscription
  */

  isMainWindow() {
    return !!this.getLoadSettings().mainWindow;
  }

  isEmptyWindow() {
    return this.getWindowType() === 'emptyWindow';
  }

  isComposerWindow() {
    return this.getWindowType() === 'composer';
  }

  isThreadWindow() {
    return this.getWindowType() === 'thread-popout';
  }

  getWindowType() {
    return this.getLoadSettings().windowType;
  }

  // Public: Is the current window in development mode?
  inDevMode() {
    return this.getLoadSettings().devMode;
  }

  // Public: Is the current window in safe mode?
  inSafeMode() {
    return this.getLoadSettings().safeMode;
  }

  // Public: Is the current window running specs?
  inSpecMode() {
    return this.getLoadSettings().isSpec;
  }

  // Public: Get the version of Tern.
  //
  // Returns the version text {String}.
  private appVersion?: string;

  getVersion() {
    return this.appVersion != null
      ? this.appVersion
      : (this.appVersion = this.getLoadSettings().appVersion);
  }

  // Public: Determine whether the current version is an official release.
  isReleasedVersion() {
    // Check if the release contains a 7-character SHA prefix
    return !/\w{7}/.test(this.getVersion());
  }

  // Public: Get the directory path to Tern's configuration area.
  getConfigDirPath() {
    return this.getLoadSettings().configDirPath;
  }

  // Public: Get the time taken to completely load the current window.
  //
  // This time include things like loading and activating packages, creating
  // DOM elements for the editor, and reading the config.
  //
  // Returns the {Number} of milliseconds taken to load the window or null
  // if the window hasn't finished loading yet.
  getWindowLoadTime() {
    return this.loadTime;
  }

  // Public: Get the load settings for the current window.
  //
  // Returns an {Object} containing all the load setting key/value pairs.
  getLoadSettings() {
    return loadSettings;
  }

  /*
  Section: Managing The Tern Window
  */

  // Essential: Close the current window.
  close() {
    return CurrentWindow.close();
  }

  quit() {
    return App.quit();
  }

  // Essential: Get the size of current window.
  //
  // Returns an {Object} in the format `{width: 1000, height: 700}`
  getSize() {
    const [width, height] = CurrentWindow.getSize();
    return { width, height };
  }

  // Essential: Set the size of current window.
  //
  // * `width` The {Number} of pixels.
  // * `height` The {Number} of pixels.
  setSize(width: number, height: number) {
    return CurrentWindow.setSize(ensureInteger(width, 100), ensureInteger(height, 100));
  }

  setMinimumWidth(minWidth: number) {
    const { minimumSize, bounds } = CurrentWindow.state();
    CurrentWindow.setMinimumSize(ensureInteger(minWidth, 0), minimumSize[1]);

    if (minWidth > bounds.width) {
      CurrentWindow.setSize(minWidth, bounds.height);
    }
  }

  // Essential: Get the position of current window.
  //
  // Returns an {Object} in the format `{x: 10, y: 20}`
  getPosition() {
    const [x, y] = CurrentWindow.getPosition();
    return { x, y };
  }

  // Essential: Set the position of current window.
  //
  // * `x` The {Number} of pixels.
  // * `y` The {Number} of pixels.
  setPosition(x: number, y: number) {
    return CurrentWindow.setPosition(ensureInteger(x, 0), ensureInteger(y, 0));
  }

  // Extended: Get the current window. A handle to the BrowserWindow methods windows may call on
  // themselves (platform/app-api.ts), not the BrowserWindow.
  getCurrentWindow(): CurrentWindowHandle {
    return CurrentWindow;
  }

  // Extended: Move current window to the center of the screen.
  center() {
    if (process.platform === 'linux') {
      const dimensions = this.getWindowDimensions();
      const { x, y, width, height } = dimensions;
      const display =
        Screen.getDisplayMatching({ x, y, width, height }) || Screen.getPrimaryDisplay();

      return this.setPosition(
        display.bounds.x + (display.bounds.width - width) / 2,
        display.bounds.y + (display.bounds.height - height) / 2
      );
    } else {
      return CurrentWindow.center();
    }
  }

  // Extended: Focus the current window. Note: this will not open the window
  // if it is hidden.
  focus() {
    CurrentWindow.focus();
    return window.focus();
  }

  // Extended: Show the current window.
  show() {
    return CurrentWindow.show();
  }

  isVisible() {
    return CurrentWindow.isVisible();
  }

  // Extended: Hide the current window.
  hide() {
    return CurrentWindow.hide();
  }

  // Extended: Reload the current window.
  reload() {
    this.isReloading = true;
    return ipcRenderer.send('call-webcontents-method', 'reload');
  }

  // Public: The windowProps passed when creating the window via `newWindow`.
  //
  getWindowProps() {
    return this.getLoadSettings().windowProps || {};
  }

  // Public: If your package declares hot-loaded window types, `onWindowPropsReceived`
  // fires when your hot-loaded window is about to be shown so you can update
  // components to reflect the new window props.
  //
  // - callback: A function to call when window props are received, just before
  //   the hot window is shown. The first parameter is the new windowProps.
  //
  onWindowPropsReceived(callback: (...args: unknown[]) => void) {
    return this.emitter.on('window-props-received', callback);
  }

  // Extended: Is the current window maximized?
  isMaximixed() {
    return CurrentWindow.isMaximized();
  }

  maximize() {
    return CurrentWindow.maximize();
  }

  minimize() {
    return CurrentWindow.minimize();
  }

  // Extended: Is the current window in full screen mode?
  isFullScreen() {
    return CurrentWindow.isFullScreen();
  }

  // Extended: Set the full screen state of the current window.
  setFullScreen(fullScreen = false) {
    CurrentWindow.setFullScreen(fullScreen);
    if (fullScreen) {
      return document.body.classList.add('fullscreen');
    }
    return document.body.classList.remove('fullscreen');
  }

  // Extended: Toggle the full screen state of the current window.
  toggleFullScreen() {
    return this.setFullScreen(!this.isFullScreen());
  }

  // Get the dimensions of this window.
  //
  // Returns an {Object} with the following keys:
  //   * `x`      The window's x-position {Number}.
  //   * `y`      The window's y-position {Number}.
  //   * `width`  The window's width {Number}.
  //   * `height` The window's height {Number}.
  getWindowDimensions() {
    const { bounds, isMaximized, isFullScreen } = CurrentWindow.state();
    const { x, y, width, height } = bounds;
    return { x, y, width, height, maximized: isMaximized, fullScreen: isFullScreen };
  }

  // Set the dimensions of the window.
  //
  // The window will be centered if either the x or y coordinate is not set
  // in the dimensions parameter. If x or y are omitted the window will be
  // centered. If height or width are omitted only the position will be changed.
  //
  // * `dimensions` An {Object} with the following keys:
  //   * `x` The new x coordinate.
  //   * `y` The new y coordinate.
  //   * `width` The new width.
  //   * `height` The new height.
  setWindowDimensions({
    x,
    y,
    width,
    height,
  }: {
    x?: number;
    y?: number;
    width?: number;
    height?: number;
  }) {
    if (x != null && y != null && width != null && height != null) {
      return CurrentWindow.setBounds({ x, y, width, height });
    } else if (width != null && height != null) {
      return this.setSize(width, height);
    } else if (x != null && y != null) {
      return this.setPosition(x, y);
    }
    return this.center();
  }

  // Returns true if the dimensions are useable, false if they should be ignored.
  // Work around for https://github.com/atom/electron/issues/473
  isValidDimensions({
    x,
    y,
    width,
    height,
  }: { x?: number; y?: number; width?: number; height?: number } = {}) {
    if (x === undefined || y === undefined || width === undefined || height === undefined) {
      return false;
    }
    return width > 0 && height > 0 && x + width > 0 && y + height > 0;
  }

  getDefaultWindowDimensions() {
    let { width, height } = Screen.getPrimaryDisplay().workAreaSize;
    let x = 0;
    let y = 0;

    const MAX_WIDTH = 1440;
    if (width > MAX_WIDTH) {
      x = Math.floor((width - MAX_WIDTH) / 2);
      width = MAX_WIDTH;
    }

    const MAX_HEIGHT = 900;
    if (height > MAX_HEIGHT) {
      y = Math.floor((height - MAX_HEIGHT) / 2);
      height = MAX_HEIGHT;
    }

    return { x, y, width, height };
  }

  restoreWindowDimensions() {
    const settings = this.getLoadSettings();

    let dimensions = this.savedState.windowDimensions;
    if (!this.isValidDimensions(dimensions)) {
      dimensions = this.getDefaultWindowDimensions();
    }

    // Amazingly, Wayland doesn't let apps adjust the positions of their windows, only their size.
    // This is wild and users seem to dislike it, but we'll let the app use it's default position,
    // and avoid interfering with any userland scripts / extensions users have to save their settings.
    if (!isWaylandSession()) {
      this.setWindowDimensions(dimensions);
    }

    if (dimensions.maximized && !settings.initializeInBackground) {
      this.maximize();
    }
    if (dimensions.fullScreen) {
      this.setFullScreen(true);
    }
  }

  storeWindowDimensions() {
    const dimensions = this.getWindowDimensions();
    if (this.isValidDimensions(dimensions)) {
      this.savedState.windowDimensions = dimensions;
    }
  }

  storeColumnWidth({ id, width }: { id: string; width: number }) {
    if (this.savedState.columnWidths == null) {
      this.savedState.columnWidths = {};
    }
    this.savedState.columnWidths[id] = width;
  }

  getColumnWidth(id: string) {
    if (this.savedState.columnWidths == null) {
      this.savedState.columnWidths = {};
    }
    return this.savedState.columnWidths[id];
  }

  storeThreadListVerticalHeight(height: number) {
    this.savedState.threadListVerticalHeight = height;
  }

  getThreadListVerticalHeight() {
    return this.savedState.threadListVerticalHeight;
  }

  async startWindow() {
    const { windowType } = this.getLoadSettings();

    this.themes.loadStaticStylesheets();
    this.initializeBasicSheet();
    this.initializeReactRoot();
    this.packages.activatePackages(windowType);

    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => {
        window.requestAnimationFrame(() => {
          window.requestAnimationFrame(() => {
            this.keymaps.loadKeymaps();
            this.menu.update();

            ipcRenderer.send('window-command', 'window:loaded');
          });
        });
      });
    });
  }

  // Call this method when establishing a real application window.
  async startRootWindow() {
    this.restoreWindowDimensions();
    CurrentWindow.setMinimumSize(875, 250);
    await this.startWindow();
  }

  // Initializes a secondary window.
  // NOTE: If the `packageLoadingDeferred` option is set (which is true for
  // hot windows), the packages won't be loaded until `populateHotWindow`
  // gets fired.
  async startSecondaryWindow() {
    await this.startWindow();
    ipcRenderer.on('load-settings-changed', (event, loadSettings) => {
      let url = window.location.href.substr(0, window.location.href.indexOf('loadSettings='));
      url += `loadSettings=${encodeURIComponent(JSON.stringify(loadSettings))}`;
      window.history.replaceState('', '', url);

      this.populateHotWindow(loadSettings);
    });
  }

  // We setup the initial Sheet for hot windows. This is the default title
  // bar, stoplights, etc. This saves ~100ms when populating the hot
  // windows.
  initializeBasicSheet() {
    const WorkspaceStore = require('../data/stores/workspace').default;
    if (!WorkspaceStore.Sheet.Main) {
      WorkspaceStore.defineSheet(
        'Main',
        { root: true },
        {
          popout: ['Center'],
        }
      );
    }
  }

  // Updates the window load settings - called when the app is ready to
  // display a hot-loaded window. Causes listeners registered with
  // `onWindowPropsReceived` to receive new window props.
  //
  // This also means that the windowType has changed and a different set of
  // plugins needs to be loaded.
  populateHotWindow(newLoadSettings: { [key: string]: unknown }) {
    const previousWindowType = loadSettings.windowType;
    loadSettings = newLoadSettings;

    document.body.classList.remove(`window-type-${previousWindowType}`);
    document.body.classList.add(`window-type-${loadSettings.windowType}`);

    this.packages.activatePackages(loadSettings.windowType);

    this.emitter.emit(
      'window-props-received',
      loadSettings.windowProps != null ? loadSettings.windowProps : {}
    );

    if (CurrentWindow.isResizable() !== loadSettings.resizable) {
      CurrentWindow.setResizable(loadSettings.resizable);
    }

    if (!loadSettings.hidden) {
      this.displayWindow();
    }
  }

  // Launches a new window via the main/WindowLauncher.
  //
  // If you pass a `windowKey` in the options, and that windowKey already
  // exists, it'll show that window instead of spawing a new one. This is
  // useful for places like popout composer windows where you want to
  // simply display the draft instead of spawning a whole new window for
  // the same draft.
  //
  // `options` are documented in main/WindowLauncher
  newWindow(options = {}) {
    return ipcRenderer.send('new-window', options);
  }

  saveWindowStateAndUnload() {
    this.packages.deactivatePackages();
    this.saveWindowState();
  }

  /*
  Section: Messaging the User
  */

  displayWindow({ maximize }: { maximize?: boolean } = {}) {
    if (this.inSpecMode()) {
      return;
    }
    this.show();
    this.focus();
    if (maximize) this.maximize();
  }

  /*
  Section: Managing the Dev Tools
  */

  // Extended: Open the dev tools for the current window.
  openDevTools() {
    return ipcRenderer.send('call-webcontents-method', 'openDevTools');
  }

  isDevToolsOpened() {
    return CurrentWindow.isDevToolsOpened();
  }

  // Extended: Toggle the visibility of the dev tools for the current window.
  toggleDevTools() {
    return ipcRenderer.send('call-webcontents-method', 'toggleDevTools');
  }

  // Extended: Execute code in dev tools.
  executeJavaScriptInDevTools(code: string) {
    return ipcRenderer.send('call-devtools-webcontents-method', 'executeJavaScript', code);
  }

  /*
  Section: Private
  */

  initializeReactRoot() {
    // Put state back into sheet-container? Restore app state here
    const item = document.createElement('tern-workspace');
    item.setAttribute('id', 'sheet-container');
    item.setAttribute('class', 'sheet-container');
    item.setAttribute('tabIndex', '-1');
    if (isRTL) {
      item.setAttribute('dir', 'rtl');
      // Portals (menus, popovers, tooltips) render outside the workspace.
      document.documentElement.setAttribute('dir', 'rtl');
    }

    const React = require('react');
    const ReactDOM = require('react-dom');
    const SheetContainer = require('../components/sheets/container').default;
    const { UIProvider } = require('../components/kit/provider');
    ReactDOM.render(
      React.createElement(UIProvider, null, React.createElement(SheetContainer)),
      item
    );

    if (this.inSpecMode()) {
      // The spec runner's page has it.
      document.querySelector('#jasmine-content')!.appendChild(item);
    } else {
      document.body.appendChild(item);
    }
  }

  loadConfig() {
    this.config.setSchema(null, {
      type: 'object',
      properties: _.clone(require('../config/schema').default),
    });
    this.config.load();
  }

  exit(status: number) {
    App.exit(status);
  }

  async showOpenDialog(options: Electron.OpenDialogOptions, callback: (paths: string[]) => void) {
    const result = await Dialogs.showOpenDialog(options, 'window');
    callback(result.filePaths);
  }

  async showSaveDialog(options: Electron.SaveDialogOptions, callback: (path: string) => void) {
    if (options.title == null) {
      options.title = 'Save File';
    }
    const result = await Dialogs.showSaveDialog(options, 'window');
    callback(result.filePath);
  }

  async showErrorDialog(
    messageData: unknown,
    { showInMainWindow, detail }: { showInMainWindow?: boolean; detail?: string } = {}
  ) {
    let message: unknown;
    let title: unknown;
    if (_.isString(messageData) || _.isNumber(messageData)) {
      message = messageData;
      title = 'Error';
    } else if (_.isObject(messageData)) {
      message = errorField(messageData, 'message');
      title = errorField(messageData, 'title');
    } else {
      throw new Error(`Must pass a valid message to show dialog: ${message}`);
    }

    const parent = showInMainWindow ? 'main' : 'none';

    if (!detail) {
      return Dialogs.ask(
        {
          type: 'warning',
          buttons: [localized('Okay')],
          message: dialogText(title),
          detail: dialogText(message),
        },
        parent
      );
    }

    const result = await Dialogs.ask(
      {
        type: 'warning',
        buttons: [localized('Okay'), localized('Show Detail')],
        message: dialogText(title),
        detail: dialogText(message),
      },
      parent
    );
    if (result === 1) {
      const Actions = require('../data/actions');
      const CodeSnippet = require('../components/code-snippet').default;
      Actions.openModal({
        component: CodeSnippet({ intro: message, code: detail, className: 'error-details' }),
        width: 500,
        height: 300,
      });
    }
  }

  // The main process's fileListCache, shared by every window.
  fileListCache() {
    return FileListCache;
  }

  getWindowStateKey() {
    return `window-state-${this.getWindowType()}`;
  }

  saveWindowState() {
    const stateString = JSON.stringify(this.savedState);
    window.localStorage.setItem(this.getWindowStateKey(), stateString);
  }

  restoreWindowState() {
    try {
      const stateString = window.localStorage.getItem(this.getWindowStateKey());
      if (stateString != null) {
        this.savedState = JSON.parse(stateString);
      }
    } catch (error) {
      console.warn(`Error parsing window state: ${errorField(error, 'stack')}`, error);
    }
    if (!this.savedState) {
      this.savedState = {};
    }
  }

  crashMainProcess() {
    App.crashMainProcess();
  }

  crashRenderProcess() {
    process.crash();
  }

  onUpdateAvailable(callback: (details: UpdateDetails) => void) {
    return this.emitter.on('update-available', callback);
  }

  updateAvailable(details: UpdateDetails) {
    this.emitter.emit('update-available', details);
  }

  // Lets multiple components register beforeUnload callbacks.
  // The callbacks are expected to return either true or false.
  //
  // Note: If you return false to cancel the window close, you /must/ perform
  // work and then call finishUnload. We do not support cancelling quit!
  //
  // Also see logic in main/AppWindow::handleEvents where we listen
  // to the browserWindow.on 'close' event to catch "unclosable" windows.
  onBeforeUnload(callback: UnloadCallback) {
    return this.windowEventHandler.addUnloadCallback(callback);
  }

  onReadyToUnload(callback: () => void) {
    return this.windowEventHandler.addReadyToUnloadCallback(callback);
  }

  removeUnloadCallback(callback: UnloadCallback) {
    this.windowEventHandler.removeUnloadCallback(callback);
  }

  enhanceEventObject() {
    const overriddenStop = Event.prototype.stopPropagation;
    Event.prototype.stopPropagation = function stopPropagation(this: Event, ...args: []) {
      this.propagationStopped = true;
      return overriddenStop.apply(this, args);
    };
    Event.prototype.isPropagationStopped = function isPropagationStopped(this: Event) {
      return this.propagationStopped;
    };
  }
}
