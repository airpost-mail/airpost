import { BrowserWindow, Menu, app, dialog } from 'electron';
import path from 'path';
import fs from 'fs';
import url from 'url';
import { EventEmitter } from 'events';
import { developerToolsAvailable } from '../../platform/developer-tools';
import {
  XDG_DATA_PATHS,
  getFirstExistingPath,
  isWaylandSession,
} from '../../platform/linux-desktop';

import {
  attemptEarlyRendererCrashRecovery,
  isPrimaryWindow,
} from './hardware-acceleration-recovery';
import { shouldBlockFrameNavigation } from '../frame/navigation';
import { attachAppWindowContents } from '../ipc/app-api';

let WindowIconPath: string | null = null;
let idNum = 0;

const appWebContents = new WeakSet<Electron.WebContents>();

/**
 * True for the renderers of Tern's own windows. Other top-level windows (print
 * preview, quick preview, "show original") display untrusted content and must not be
 * able to drive the main process.
 */
export function isAppWindowContents(contents: Electron.WebContents) {
  return appWebContents.has(contents);
}

export interface AppWindowSettings {
  frame?: boolean;
  title?: string;
  width?: number;
  height?: number;
  hidden?: boolean;
  toolbar?: boolean;
  resizable?: boolean;
  pathToOpen?: string;
  isSpec?: boolean;
  devMode?: boolean;
  windowKey?: string;
  safeMode?: boolean;
  neverClose?: boolean;
  mainWindow?: boolean;
  windowType?: string;
  initialPath?: string;
  resourcePath?: string;
  exitWhenDone?: boolean;
  configDirPath?: string;
  autoHideMenuBar?: boolean;
  bounds?: Electron.Rectangle;
  coldStartOnly?: boolean;
  initializeInBackground?: boolean;
  titleBarStyle?: Electron.BrowserWindowConstructorOptions['titleBarStyle'];
  bootstrapScript?: string;
  appVersion?: string;
  shellLoadTime?: number;
  // Allow additional properties for extensibility
  [key: string]: unknown;
}

// What WindowLauncher passes: its defaults always carry both paths.
export type AppWindowOptions = AppWindowSettings & { resourcePath: string; configDirPath: string };

type AppBrowserWindow = BrowserWindow & {
  loadSettings: AppWindowSettings;
  loadSettingsChangedSinceGetURL: boolean;
  updateLoadSettings: (newSettings?: AppWindowSettings) => void;
};

export default class AppWindow extends EventEmitter {
  static includeShellLoadTime = true;

  public windowType: string | undefined;
  public browserWindow: AppBrowserWindow;
  public devMode: boolean | undefined;
  public safeMode: boolean | undefined;

  private loaded = false;
  private isSpec: boolean | undefined;
  public windowKey: string;
  private neverClose: boolean | undefined;
  private mainWindow: boolean | undefined;
  private resourcePath: string;
  private exitWhenDone: boolean | undefined;
  private configDirPath: string;

  private isWindowClosing = false;

  constructor(settings: AppWindowOptions) {
    super();

    // toolbar is present too, but passed through to client-side
    const { frame, title, width, height, resizable, pathToOpen, autoHideMenuBar, titleBarStyle } =
      settings;
    this.isSpec = settings.isSpec;
    this.devMode = settings.devMode;
    this.safeMode = settings.safeMode;
    this.neverClose = settings.neverClose;
    this.mainWindow = settings.mainWindow;
    this.windowType = settings.windowType;
    this.exitWhenDone = settings.exitWhenDone;
    this.configDirPath = settings.configDirPath;

    if (settings.windowKey) {
      this.windowKey = settings.windowKey;
    } else {
      this.windowKey = `${this.windowType}-${idNum}`;
      idNum += 1;
    }

    // Normalize to make sure drive letter case is consistent on Windows
    this.resourcePath = settings.resourcePath && path.normalize(settings.resourcePath);

    const webPreferences: Electron.WebPreferences = {
      nodeIntegration: true,
      contextIsolation: false,
      webviewTag: true,
      // Withholding the capability, rather than only the ways to ask for it, is what makes
      // the console unreachable in a packaged build: @electron/remote stays enabled for these
      // windows (below), so any code running in them could otherwise call openDevTools() on
      // its own webContents. Electron turns such calls into silent no-ops.
      devTools: developerToolsAvailable(),
    };
    const browserWindowOptions: Electron.BrowserWindowConstructorOptions = {
      show: false,
      title: title || 'Tern',
      frame,
      width,
      height,
      resizable,
      titleBarStyle,
      webPreferences,
      autoHideMenuBar,
    };

    if (this.neverClose || this.isSpec) {
      // Prevents DOM timers from being suspended when the main window is hidden.
      // Means there's not an awkward catch-up when you re-show the main window.
      // For spec windows, this is critical: the hidden spec window would otherwise
      // throttle setTimeout calls to ~1Hz, making each test take ~1s and causing
      // the full test suite to exceed CI time limits.
      webPreferences.backgroundThrottling = false;
    }

    // Don't set icon on Windows so the exe's ico will be used as window and
    // taskbar's icon. See https://github.com/atom/atom/issues/4811 for more.
    if (process.platform === 'linux') {
      if (!WindowIconPath) {
        WindowIconPath = getFirstExistingPath(XDG_DATA_PATHS, path.join('pixmaps', 'tern.png'));
        if (!WindowIconPath) {
          WindowIconPath = path.resolve(this.resourcePath, 'static', 'images', 'tern.png');
        }
      }
      browserWindowOptions.icon = WindowIconPath;
    }

    this.browserWindow = Object.assign(new BrowserWindow(browserWindowOptions), {
      loadSettings: settings,
      loadSettingsChangedSinceGetURL: false,
      updateLoadSettings: this.updateLoadSettings,
    });
    appWebContents.add(this.browserWindow.webContents);

    // Constrain every <webview> guest to low-privilege preferences regardless of
    // the attributes on the tag, and strip any preload it requests. The only
    // legitimate guest (the onboarding sign-in view in
    // src/components/webview.tsx) displays remote web content and needs none
    // of these privileges. Do not relax this. See GHSA-x8wg-258g-v28h.
    this.browserWindow.webContents.on('will-attach-webview', (_event, webPreferences, params) => {
      delete (webPreferences as any).preload;
      delete (params as any).preload;
      delete (params as any).webpreferences;
      delete (params as any).nodeintegration;
      delete (params as any).nodeintegrationinsubframes;
      webPreferences.nodeIntegration = false;
      webPreferences.nodeIntegrationInSubFrames = false;
      webPreferences.contextIsolation = true;
      webPreferences.sandbox = true;
      webPreferences.devTools = developerToolsAvailable();
    });

    // Tern's code reaches main over ipc/app-api.ts; remote stays enabled for v1 plugins that
    // load @electron/remote themselves, until plugin API 2.0.
    require('@electron/remote/main').enable(this.browserWindow.webContents);
    attachAppWindowContents(this.browserWindow.webContents, (template) =>
      Menu.buildFromTemplate(template)
    );

    this.handleEvents();

    const loadSettings = Object.assign({}, settings);
    loadSettings.appVersion = global.application.version;
    loadSettings.resourcePath = this.resourcePath;
    if (loadSettings.devMode == null) {
      loadSettings.devMode = false;
    }
    if (loadSettings.safeMode == null) {
      loadSettings.safeMode = false;
    }
    if (loadSettings.mainWindow == null) {
      loadSettings.mainWindow = this.mainWindow;
    }
    if (loadSettings.windowType == null) {
      loadSettings.windowType = 'default';
    }

    // Only send to the first non-spec window created
    if (AppWindow.includeShellLoadTime && !this.isSpec) {
      AppWindow.includeShellLoadTime = false;
      if (loadSettings.shellLoadTime == null) {
        loadSettings.shellLoadTime = Date.now() - global.shellStartTime;
      }
    }

    loadSettings.initialPath = pathToOpen;

    let stats: fs.Stats | false = false;
    if (pathToOpen) {
      try {
        stats = fs.statSync(pathToOpen);
      } catch (e) {
        // path doesn't exist
      }
    }
    if (pathToOpen && stats && stats.isFile && stats.isFile()) {
      loadSettings.initialPath = path.dirname(pathToOpen);
    }

    this.browserWindow.loadSettings = loadSettings;

    (this.browserWindow.once as any)('window:loaded', () => {
      this.loaded = true;
      if (this.browserWindow.loadSettingsChangedSinceGetURL) {
        this.browserWindow.webContents.send(
          'load-settings-changed',
          this.browserWindow.loadSettings
        );
      }
      this.emit('window:loaded');
    });

    // On Wayland, Electron's ready-to-show event is broken (DidMeaningfulLayout never
    // fires - see https://github.com/electron/electron/issues/48859). Calling show()
    // much later (at window:loaded time) also fails silently because the Wayland surface
    // was never committed. However, show() works reliably at did-finish-load time, when
    // the HTML is loaded, themes/styles are applied, and React root is mounted - the UI
    // is nearly complete. This is the same workaround used by FreeTube and Signal Desktop.
    //
    // When --background is requested on Wayland we must still show briefly to commit the
    // Wayland surface (otherwise show() silently fails). Once the window finishes
    // initializing (window:loaded) we hide it again so the net effect matches what the
    // user asked for: Tern running silently in the background.
    if (isWaylandSession()) {
      this.browserWindow.webContents.once('did-finish-load', () => {
        if (!this.browserWindow.isDestroyed() && !this.browserWindow.isVisible()) {
          const initInBackground = this.browserWindow.loadSettings?.initializeInBackground;
          this.browserWindow.show();
          if (initInBackground) {
            this.once('window:loaded', () => {
              if (!this.browserWindow.isDestroyed()) {
                this.browserWindow.hide();
              }
            });
          } else {
            this.browserWindow.focus();
          }
        }
      });
    }

    this.browserWindow.loadURL(this.getURL(loadSettings));
    if (this.isSpec) {
      this.browserWindow.focusOnWebView();
    }
  }

  updateLoadSettings = (newSettings: AppWindowSettings = {}) => {
    this.loaded = true;
    this.setLoadSettings({ ...this.browserWindow.loadSettings, ...newSettings });
  };

  loadSettings(): AppWindowSettings {
    return this.browserWindow.loadSettings;
  }

  // This gets called when we want to turn a WindowLauncher.EMPTY_WINDOW
  // into a new kind of custom popout window.
  //
  // The windowType will change which will cause a new set of plugins to
  // load.
  setLoadSettings(loadSettings: AppWindowSettings) {
    this.browserWindow.loadSettings = loadSettings;
    this.browserWindow.loadSettingsChangedSinceGetURL = true;
    this.browserWindow.webContents.send('load-settings-changed', loadSettings);
  }

  getURL(loadSettings: AppWindowSettings) {
    this.browserWindow.loadSettingsChangedSinceGetURL = false;

    return url.format({
      protocol: 'file',
      pathname: `${this.resourcePath}/static/index.html`,
      slashes: true,
      query: { loadSettings: JSON.stringify(loadSettings) },
    });
  }

  handleEvents() {
    // Also see logic in `AppEnv::onBeforeUnload` and
    // `WindowEventHandler::AddUnloadCallback`. Classes like the DraftStore
    // and ActionBridge intercept the closing of windows and perform
    // action.
    //
    // This uses the DOM's `beforeunload` event.
    this.browserWindow.on('close', (event: Electron.Event) => {
      if (global.application.isQuitting()) {
        return;
      }

      const isLastWindow = global.application.windowManager.getVisibleWindowCount() === 1;
      // The configuration value may be `undefined` when it has not been manually set to true in the preferences
      // This check against false prevents that Tern is closed when configuring the first mail account
      const isTrayEnabled = global.application.config.get('core.workspace.systemTray') !== false;
      const runWithoutWindowsOpen = isTrayEnabled || process.platform === 'darwin';

      if (isLastWindow && !runWithoutWindowsOpen) {
        // Tray indicator is switched off, closing the last window should quit the application.
        app.quit();
        return;
      }

      if (this.neverClose) {
        // For neverClose windows (like the main window) simply hide and
        // take out of full screen as long as the tray indicator is switched on.
        // Tray indicator is switched on therefore hiding the main window only.
        event.preventDefault();
        if (this.browserWindow.isFullScreen()) {
          this.browserWindow.once('leave-full-screen', () => {
            this.browserWindow.hide();
          });
          this.browserWindow.setFullScreen(false);
        } else {
          this.browserWindow.hide();
        }
        // HOWEVER! If the neverClose window is broken and is not actually loaded and
        // no other windows are visible, quit because the user may not be able to.
        if (!this.isSpec) {
          global.application.windowManager.quitWinLinuxIfNoWindows();
        }
      }
    });

    this.browserWindow.on('focus', () => {
      this.browserWindow.webContents.send('browser-window-focus');
    });

    this.browserWindow.on('blur', () => {
      this.browserWindow.webContents.send('browser-window-blur');
    });

    this.browserWindow.on('show', () => {
      this.browserWindow.webContents.send('browser-window-show');
    });

    this.browserWindow.on('hide', () => {
      this.browserWindow.webContents.send('browser-window-hide');
    });

    this.browserWindow.webContents.on('will-navigate', (event, url) => {
      event.preventDefault();
    });

    this.browserWindow.webContents.on('will-frame-navigate', (event) => {
      const initiatorUrl =
        event.initiator && !event.initiator.isDestroyed() ? event.initiator.url : null;
      if (
        shouldBlockFrameNavigation({ url: event.url, isMainFrame: event.isMainFrame, initiatorUrl })
      ) {
        event.preventDefault();
      }
    });

    this.browserWindow.webContents.setWindowOpenHandler(({ url, frameName, disposition }) => {
      return { action: 'deny' };
    });

    this.browserWindow.on('unresponsive', () => {
      if (this.isSpec) {
        return;
      }
      if (!this.loaded) {
        return;
      }
      if (this.devMode) {
        return;
      }

      const chosen = dialog.showMessageBoxSync(this.browserWindow, {
        type: 'warning',
        buttons: ['Close', 'Keep Waiting'],
        message: 'Tern is not responding',
        detail: 'Would you like to force close it or keep waiting?',
      });
      if (chosen === 0) {
        this.browserWindow.destroy();
      }
    });

    this.browserWindow.webContents.on('render-process-gone', (event, details) => {
      const killed = details.reason === 'killed';
      if (killed) {
        // Killed means that the app is exiting and the browser window is being
        // forceably cleaned up. Carry on, do not try to reload the window.
        this.browserWindow.destroy();
        return;
      }

      if (
        attemptEarlyRendererCrashRecovery({
          app,
          configDirPath: this.configDirPath,
          loaded: this.loaded,
          primaryWindow: isPrimaryWindow({
            mainWindow: this.mainWindow,
            windowType: this.windowType,
          }),
          reason: details.reason,
        })
      ) {
        return;
      }

      if (this.exitWhenDone) {
        app.exit(100);
        return;
      }

      if (this.neverClose) {
        this.browserWindow.reload();
      } else {
        const chosen = dialog.showMessageBoxSync({
          type: 'warning',
          buttons: ['Close Window', 'Reload', 'Keep It Open'],
          message: 'Tern has crashed',
          detail: 'Please report this issue at https://github.com/ternmail/ternmail/issues.',
        });
        if (chosen === 0) {
          this.browserWindow.destroy();
        } else if (chosen === 1) {
          this.browserWindow.reload();
        }
      }
    });

    if (this.isSpec) {
      // Workaround for https://github.com/atom/electron/issues/380
      // Don't focus the window when it is being blurred during close or
      // else the app will crash on Windows.
      if (process.platform === 'win32') {
        this.browserWindow.on('close', () => {
          this.isWindowClosing = true;
        });
      }

      // Spec window's web view should always have focus
      this.browserWindow.on('blur', () => {
        if (!this.isWindowClosing) {
          this.browserWindow.focusOnWebView();
        }
      });
    }
  }

  sendMessage(message: string, detail?: unknown) {
    this.waitForLoad(() => this.browserWindow.webContents.send(message, detail));
  }

  sendCommand(command: string, ...args: unknown[]) {
    if (this.isSpecWindow()) {
      if (!global.application.sendCommandToFirstResponder(command)) {
        switch (command) {
          case 'window:reload':
            return this.reload();
          case 'window:toggle-dev-tools':
            return this.toggleDevTools();
          case 'window:close':
            return this.close();
          default:
        }
      }
    } else if (this.isWebViewFocused()) {
      return this.sendCommandToBrowserWindow(command, ...args);
    } else {
      if (!global.application.sendCommandToFirstResponder(command)) {
        return this.sendCommandToBrowserWindow(command, ...args);
      }
    }
  }

  sendCommandToBrowserWindow(command: string, ...args: unknown[]) {
    this.browserWindow.webContents.send('command', command, ...args);
  }

  getDimensions() {
    const [x, y] = this.browserWindow.getPosition();
    const [width, height] = this.browserWindow.getSize();
    return { x, y, width, height };
  }

  close() {
    this.browserWindow.close();
  }

  hide() {
    this.browserWindow.hide();
  }

  show() {
    this.browserWindow.show();
  }

  showWhenLoaded() {
    this.waitForLoad(() => {
      this.show();
      this.focus();
    });
  }

  waitForLoad(fn: () => void) {
    if (this.loaded) {
      fn();
    } else {
      this.once('window:loaded', fn);
    }
  }

  focus() {
    this.browserWindow.focus();
  }

  minimize() {
    this.browserWindow.minimize();
  }

  maximize() {
    this.browserWindow.maximize();
  }

  restore() {
    this.browserWindow.restore();
  }

  isFocused() {
    return this.browserWindow.isFocused();
  }

  isMinimized() {
    return this.browserWindow.isMinimized();
  }

  isVisible() {
    return this.browserWindow.isVisible();
  }

  isLoaded() {
    return this.loaded;
  }

  isWebViewFocused() {
    return this.browserWindow.webContents.isFocused();
  }

  isSpecWindow() {
    return this.isSpec;
  }

  reload() {
    this.browserWindow.reload();
  }

  toggleDevTools() {
    if (!developerToolsAvailable()) return;
    this.browserWindow.webContents.toggleDevTools();
  }
}
