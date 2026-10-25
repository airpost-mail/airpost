// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { localized } from '../../intl/localize';
import type {
  AppApiInvoke,
  AppApiSend,
  AppApiSync,
  DialogParent,
  EditCommand,
  InvokeArgs,
  InvokeResult,
  MenuItemTemplate,
  StaticPageOptions,
  SyncArgs,
  SyncResult,
  UpdaterState,
  WindowCommand,
  WindowState,
} from '../../platform/app-api-channels';
import { AppApiEvent } from '../../platform/app-api-channels';

/*
The main-process side of platform/app-api-channels.ts. Every channel answers Tern's own windows
only: quick preview, print and the static pages show untrusted content and get nothing.
*/

type MenuLike = Pick<Electron.Menu, 'popup'>;

type InvokeHandler<K extends keyof AppApiInvoke> = (
  event: Electron.IpcMainInvokeEvent,
  ...args: InvokeArgs<K>
) => InvokeResult<K> | Promise<InvokeResult<K>>;

/** Everything the handlers touch, so specs can pass fakes. `createAppApiHost` is the real one. */
export interface AppApiHost {
  dialog: Pick<
    Electron.Dialog,
    'showMessageBox' | 'showOpenDialog' | 'showSaveDialog' | 'showErrorBox'
  >;
  app: Pick<
    Electron.App,
    | 'getPath'
    | 'getLocale'
    | 'quit'
    | 'relaunch'
    | 'emit'
    | 'getLoginItemSettings'
    | 'setLoginItemSettings'
    | 'isDefaultProtocolClient'
    | 'setAsDefaultProtocolClient'
    | 'removeAsDefaultProtocolClient'
    | 'isInApplicationsFolder'
  > & { readonly isPackaged: boolean };
  screen: Pick<Electron.Screen, 'getPrimaryDisplay' | 'getDisplayMatching'>;
  nativeTheme: Pick<Electron.NativeTheme, 'shouldUseDarkColors'>;
  clipboard: Pick<Electron.Clipboard, 'writeText'>;
  buildMenu(template: Electron.MenuItemConstructorOptions[]): MenuLike;
  isTrustedSender(contents: Electron.WebContents): boolean;
  windowFor(contents: Electron.WebContents): Electron.BrowserWindow | null;
  mainWindow(): Electron.BrowserWindow | null;
  applicationMenu(): MenuLike | null;
  isQuitting(): boolean;
  openUrl(url: string): void;
  openWindowCount(): number;
  fileListCache: { imageData: string };
  updaterState(): UpdaterState;
  registerDefaultMailClient(): Promise<{ error: string | null; didMakeDefault: boolean }>;
  openStaticPage(options: StaticPageOptions): void;
  exitProcess(code: number): void;
  crashProcess(): void;
}

/** What `createAppApiHost` needs from the Application. */
export interface AppApiApplication {
  getMainWindow(): Electron.BrowserWindow | null;
  isQuitting(): boolean;
  openUrl(url: string): void;
  windowManager: { getOpenWindowCount(): number };
  fileListCache: { imageData: string };
  applicationMenu?: { menu?: Electron.Menu };
  autoUpdateManager?: {
    getState(): string;
    getReleaseDetails(): { releaseVersion?: string; releaseNotes?: string };
  };
}

// The grace period after a context menu closes before it counts as dismissed. On macOS the
// item's click can arrive after the popup's close callback.
export const MenuDismissGraceMs = 100;

export function handleInvoke<K extends keyof AppApiInvoke>(
  ipcMain: Electron.IpcMain,
  host: Pick<AppApiHost, 'isTrustedSender'>,
  channel: K,
  fn: InvokeHandler<K>
) {
  ipcMain.handle(channel, (event, ...args: unknown[]) => {
    if (!host.isTrustedSender(event.sender)) {
      throw new Error(`${channel} is only available to Tern windows`);
    }
    // Args are typed by the channel map; the renderer client is the only sender.
    return fn(event, ...(args as InvokeArgs<K>));
  });
}

function onSend<K extends keyof AppApiSend>(
  ipcMain: Electron.IpcMain,
  host: Pick<AppApiHost, 'isTrustedSender'>,
  channel: K,
  fn: (event: Electron.IpcMainEvent, ...args: AppApiSend[K]) => void
) {
  ipcMain.on(channel, (event, ...args: unknown[]) => {
    if (!host.isTrustedSender(event.sender)) return;
    fn(event, ...(args as AppApiSend[K]));
  });
}

function onSync<K extends keyof AppApiSync>(
  ipcMain: Electron.IpcMain,
  host: Pick<AppApiHost, 'isTrustedSender'>,
  channel: K,
  fn: (event: Electron.IpcMainEvent, ...args: SyncArgs<K>) => SyncResult<K>
) {
  ipcMain.on(channel, (event, ...args: unknown[]) => {
    // A sync sender blocks until returnValue is set, so it is set on every path.
    if (!host.isTrustedSender(event.sender)) {
      event.returnValue = null;
      return;
    }
    try {
      event.returnValue = fn(event, ...(args as SyncArgs<K>));
    } catch (err) {
      console.error(`${channel} failed:`, err);
      event.returnValue = null;
    }
  });
}

function dialogParent(
  host: AppApiHost,
  sender: Electron.WebContents,
  parent: DialogParent
): Electron.BrowserWindow | null {
  if (parent === 'window') return host.windowFor(sender);
  if (parent === 'main') return host.mainWindow();
  return null;
}

export function runWindowCommand(win: Electron.BrowserWindow, command: WindowCommand) {
  switch (command.op) {
    case 'close':
      return win.close();
    case 'show':
      return win.show();
    case 'hide':
      return win.hide();
    case 'focus':
      return win.focus();
    case 'center':
      return win.center();
    case 'maximize':
      return win.maximize();
    case 'unmaximize':
      return win.unmaximize();
    case 'minimize':
      return win.minimize();
    case 'setFullScreen':
      return win.setFullScreen(!!command.flag);
    case 'setTitle':
      return win.setTitle(String(command.title));
    case 'setSize':
      return win.setSize(command.width, command.height);
    case 'setPosition':
      return win.setPosition(command.x, command.y);
    case 'setBounds':
      return win.setBounds(command.bounds);
    case 'setMinimumSize':
      return win.setMinimumSize(command.width, command.height);
    case 'setResizable':
      return win.setResizable(!!command.flag);
    case 'setMenuBarVisibility':
      return win.setMenuBarVisibility(!!command.flag);
    case 'setSheetOffset':
      return win.setSheetOffset(command.offset);
    case 'previewFile':
      return win.previewFile(command.path, command.displayName);
    case 'showDefinitionForSelection':
      return win.webContents.showDefinitionForSelection();
    default:
      console.error(`Unknown window command ${JSON.stringify(command)}`);
  }
}

const EditCommands: EditCommand[] = [
  'copy',
  'cut',
  'paste',
  'pasteAndMatchStyle',
  'undo',
  'redo',
  'selectAll',
];

export function windowState(win: Electron.BrowserWindow): WindowState {
  const [minWidth, minHeight] = win.getMinimumSize();
  return {
    bounds: win.getBounds(),
    minimumSize: [minWidth, minHeight],
    isVisible: win.isVisible(),
    isMaximized: win.isMaximized(),
    isMinimized: win.isMinimized(),
    isFullScreen: win.isFullScreen(),
    isFocused: win.isFocused(),
    isDevToolsOpened: win.webContents.isDevToolsOpened(),
    resizable: win.isResizable(),
  };
}

/**
 * Electron menu options for a serialised template. An item with an `id` reports it through
 * `onClick`; everything else (roles, separators, submenus) behaves as Electron makes it.
 */
export function menuFromTemplate(
  template: MenuItemTemplate[],
  onClick: (id: string) => void
): Electron.MenuItemConstructorOptions[] {
  return template.map((item) => {
    const { id, submenu, ...rest } = item;
    const options: Electron.MenuItemConstructorOptions = { ...rest };
    if (id !== undefined) {
      options.id = id;
      options.click = () => onClick(id);
    }
    if (submenu) {
      options.submenu = menuFromTemplate(submenu, onClick);
    }
    return options;
  });
}

export function popupMenu(
  host: Pick<AppApiHost, 'buildMenu'>,
  win: Electron.BrowserWindow | null,
  template: MenuItemTemplate[],
  position: { x?: number; y?: number } = {}
) {
  return new Promise<string | null>((resolve) => {
    let settled = false;
    const finish = (id: string | null) => {
      if (settled) return;
      settled = true;
      resolve(id);
    };
    const menu = host.buildMenu(menuFromTemplate(template, (id) => finish(id)));
    const options: Electron.PopupOptions = {
      callback: () => setTimeout(() => finish(null), MenuDismissGraceMs),
    };
    if (win) options.window = win;
    if (typeof position.x === 'number') options.x = Math.round(position.x);
    if (typeof position.y === 'number') options.y = Math.round(position.y);
    menu.popup(options);
  });
}

/** The spelling and edit menu of an editable area, built in main so it needs no round trips. */
export function editableContextMenuTemplate(
  contents: Pick<
    Electron.WebContents,
    'replaceMisspelling' | 'cut' | 'copy' | 'paste' | 'pasteAndMatchStyle'
  > & { session: Pick<Electron.Session, 'addWordToSpellCheckerDictionary'> },
  params: Pick<Electron.ContextMenuParams, 'misspelledWord' | 'dictionarySuggestions' | 'editFlags'>
): Electron.MenuItemConstructorOptions[] {
  const template: Electron.MenuItemConstructorOptions[] = [];
  if (params.misspelledWord) {
    if (params.dictionarySuggestions && params.dictionarySuggestions.length > 0) {
      for (const suggestion of params.dictionarySuggestions) {
        template.push({ label: suggestion, click: () => contents.replaceMisspelling(suggestion) });
      }
    } else {
      template.push({ label: localized('No Guesses Found'), enabled: false });
    }
    template.push({ type: 'separator' });
    template.push({
      label: localized('Learn Spelling'),
      click: () => {
        contents.session.addWordToSpellCheckerDictionary(params.misspelledWord);
      },
    });
    template.push({ type: 'separator' });
  }
  template.push(
    { label: localized('Cut'), enabled: params.editFlags.canCut, click: () => contents.cut() },
    { label: localized('Copy'), enabled: params.editFlags.canCopy, click: () => contents.copy() },
    {
      label: localized('Paste'),
      enabled: params.editFlags.canPaste,
      click: () => contents.paste(),
    },
    {
      label: localized('Paste and Match Style'),
      enabled: params.editFlags.canPaste,
      click: () => contents.pasteAndMatchStyle(),
    }
  );
  return template;
}

/**
 * The webContents listeners of an app window: the edit menu of editable areas, and trackpad
 * scroll gestures forwarded to the window (SwipeContainer listens for them).
 */
export function attachAppWindowContents(
  contents: Electron.WebContents,
  buildMenu: AppApiHost['buildMenu']
) {
  contents.on('context-menu', (_event, params) => {
    if (!params.isEditable) return;
    buildMenu(editableContextMenuTemplate(contents, params)).popup({});
  });
  contents.on('input-event', (_event, input) => {
    if (input.type === 'gestureScrollBegin') {
      contents.send(AppApiEvent.GestureScroll, 'begin');
    } else if (input.type === 'gestureScrollEnd') {
      contents.send(AppApiEvent.GestureScroll, 'end');
    }
  });
}

export function registerAppApiIPC(ipcMain: Electron.IpcMain, host: AppApiHost) {
  const invoke = <K extends keyof AppApiInvoke>(channel: K, fn: InvokeHandler<K>) =>
    handleInvoke(ipcMain, host, channel, fn);
  const send = <K extends keyof AppApiSend>(
    channel: K,
    fn: (event: Electron.IpcMainEvent, ...args: AppApiSend[K]) => void
  ) => onSend(ipcMain, host, channel, fn);
  const sync = <K extends keyof AppApiSync>(
    channel: K,
    fn: (event: Electron.IpcMainEvent, ...args: SyncArgs<K>) => SyncResult<K>
  ) => onSync(ipcMain, host, channel, fn);

  // Dialogs
  invoke('app-api:dialog:show-message-box', (event, options, parent) => {
    const win = dialogParent(host, event.sender, parent);
    return win ? host.dialog.showMessageBox(win, options) : host.dialog.showMessageBox(options);
  });
  invoke('app-api:dialog:show-open-dialog', (event, options, parent) => {
    const win = dialogParent(host, event.sender, parent);
    return win ? host.dialog.showOpenDialog(win, options) : host.dialog.showOpenDialog(options);
  });
  invoke('app-api:dialog:show-save-dialog', (event, options, parent) => {
    const win = dialogParent(host, event.sender, parent);
    return win ? host.dialog.showSaveDialog(win, options) : host.dialog.showSaveDialog(options);
  });
  invoke('app-api:dialog:show-error-box', (_event, title, content) =>
    host.dialog.showErrorBox(String(title), String(content))
  );

  // App
  invoke('app-api:app:get-path', (_event, name) => host.app.getPath(name));
  invoke('app-api:app:get-login-item-settings', (_event, options) =>
    host.app.getLoginItemSettings(options)
  );
  invoke('app-api:app:set-login-item-settings', (_event, settings) =>
    host.app.setLoginItemSettings(settings)
  );
  invoke('app-api:app:is-default-protocol-client', (_event, scheme) =>
    host.app.isDefaultProtocolClient(scheme)
  );
  invoke('app-api:app:set-as-default-protocol-client', (_event, scheme) =>
    host.app.setAsDefaultProtocolClient(scheme)
  );
  invoke('app-api:app:remove-as-default-protocol-client', (_event, scheme) =>
    host.app.removeAsDefaultProtocolClient(scheme)
  );
  invoke('app-api:app:is-in-applications-folder', () => host.app.isInApplicationsFolder());
  invoke('app-api:app:is-quitting', () => host.isQuitting());
  invoke('app-api:app:register-default-mail-client', () => host.registerDefaultMailClient());
  invoke('app-api:updater:get-state', () => host.updaterState());
  send('app-api:app:quit', () => host.app.quit());
  send('app-api:app:relaunch', () => host.app.relaunch());
  send('app-api:app:exit', (_event, code) => {
    host.app.emit('will-exit');
    host.exitProcess(code);
  });
  send('app-api:app:crash-main-process', () => host.crashProcess());
  send('app-api:app:open-url', (_event, url) => host.openUrl(String(url)));
  send('app-api:file-list-cache:set-image-data', (_event, imageData) => {
    host.fileListCache.imageData = String(imageData);
  });
  send('app-api:clipboard:write-text', (_event, text) => {
    host.clipboard.writeText(String(text)).catch((err: Error) => {
      console.error('Writing to the clipboard failed:', err);
    });
  });
  sync('app-api:sync:facts', () => ({
    isPackaged: host.app.isPackaged,
    locale: host.app.getLocale(),
  }));
  sync('app-api:sync:main-env', () => ({ ...process.env }));
  sync('app-api:sync:file-list-cache-image-data', () => host.fileListCache.imageData);
  sync('app-api:sync:should-use-dark-colors', () => host.nativeTheme.shouldUseDarkColors);
  sync('app-api:sync:open-window-count', () => host.openWindowCount());

  // Screen
  sync('app-api:sync:primary-display', () => host.screen.getPrimaryDisplay());
  sync('app-api:sync:display-matching', (_event, rect) => host.screen.getDisplayMatching(rect));

  // The sender's window
  send('app-api:window:command', (event, command) => {
    const win = host.windowFor(event.sender);
    if (win) runWindowCommand(win, command);
  });
  send('app-api:window:edit', (event, command) => {
    if (!EditCommands.includes(command)) {
      console.error(`Edit command ${command} is not permitted`);
      return;
    }
    event.sender[command]();
  });
  sync('app-api:sync:window-state', (event) => {
    const win = host.windowFor(event.sender);
    if (!win) throw new Error('app-api:sync:window-state has no window');
    return windowState(win);
  });
  send('app-api:window:open-static-page', (_event, options) => host.openStaticPage(options));

  // Menus
  invoke('app-api:menu:popup', (event, template, position) =>
    popupMenu(host, host.windowFor(event.sender), template, position)
  );
  invoke('app-api:menu:popup-application-menu', (event) => {
    const menu = host.applicationMenu();
    const win = host.windowFor(event.sender);
    if (menu) menu.popup(win ? { window: win } : {});
  });

  // The sender's session spellchecker
  invoke(
    'app-api:spellchecker:get-available-languages',
    (event) => event.sender.session.availableSpellCheckerLanguages || []
  );
  invoke('app-api:spellchecker:set-languages', (event, languages) =>
    event.sender.session.setSpellCheckerLanguages(languages)
  );
  send('app-api:spellchecker:add-word', (event, word) => {
    event.sender.session.addWordToSpellCheckerDictionary(String(word));
  });
  send('app-api:spellchecker:remove-word', (event, word) => {
    event.sender.session.removeWordFromSpellCheckerDictionary(String(word));
  });
}

export function createAppApiHost(
  application: AppApiApplication,
  isTrustedSender: (contents: Electron.WebContents) => boolean,
  developerToolsAvailable: () => boolean
): AppApiHost {
  const electron: typeof Electron = require('electron');
  const { BrowserWindow, Menu } = electron;
  return {
    dialog: electron.dialog,
    app: electron.app,
    screen: electron.screen,
    nativeTheme: electron.nativeTheme,
    clipboard: electron.clipboard,
    buildMenu: (template) => Menu.buildFromTemplate(template),
    isTrustedSender,
    windowFor: (contents) => BrowserWindow.fromWebContents(contents),
    mainWindow: () => application.getMainWindow(),
    applicationMenu: () => application.applicationMenu?.menu || null,
    isQuitting: () => application.isQuitting(),
    openUrl: (url) => application.openUrl(url),
    openWindowCount: () => application.windowManager.getOpenWindowCount(),
    get fileListCache() {
      return application.fileListCache;
    },
    updaterState: () => {
      const updater = application.autoUpdateManager;
      if (!updater) return { state: 'unsupported' };
      return { state: updater.getState(), ...updater.getReleaseDetails() };
    },
    registerDefaultMailClient: () =>
      new Promise((resolve) => {
        const WindowsUpdater = require('../autoupdate/win32-updater');
        WindowsUpdater.createRegistryEntries(
          { allowEscalation: true, registerDefaultIfPossible: true },
          (err: Error | null | undefined, didMakeDefault?: boolean) =>
            resolve({ error: err ? err.message : null, didMakeDefault: !!didMakeDefault })
        );
      }),
    openStaticPage: ({ filePath, title, width, height }) => {
      const win = new BrowserWindow({
        width: width || 800,
        height: height || 600,
        title,
        webPreferences: {
          javascript: false,
          nodeIntegration: false,
          devTools: developerToolsAvailable(),
        },
      });
      win.loadURL(`file://${filePath}`).catch((err: Error) => {
        console.error(`${title} window failed to load:`, err);
      });
    },
    exitProcess: (code) => process.exit(code),
    crashProcess: () => process.crash(),
  };
}
