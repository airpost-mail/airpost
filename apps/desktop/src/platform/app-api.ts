// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { ipcRenderer } from 'electron';
import type {
  AppApiInvoke,
  AppApiSend,
  AppApiSync,
  AppFacts,
  DialogParent,
  EditCommand,
  InvokeArgs,
  InvokeResult,
  MenuItemTemplate,
  MenuPosition,
  MessageBoxOptions,
  PrintWindowOptions,
  QuickPreviewCapture,
  QuickPreviewDisplay,
  StaticPageOptions,
  SyncArgs,
  SyncResult,
  UpdaterState,
  WindowCommand,
  WindowState,
} from './app-api-channels';

/*
The windows' client for the main-process Electron APIs (channels in app-api-channels.ts, handlers
in main/ipc/app-api.ts). It lives in platform/ so the platform modules can use it, but only window
code calls it. clipboard, shell, webFrame and ipcRenderer work in a window; import those from
'electron' directly.

Dialog functions take the parent explicitly: 'window' for the calling window (a sheet on macOS),
'main' for the main window, 'none' for an app-modal dialog with no parent.
*/

/** The IPC calls, swappable in specs. */
export const AppApiTransport = {
  invoke<K extends keyof AppApiInvoke>(
    channel: K,
    ...args: InvokeArgs<K>
  ): Promise<InvokeResult<K>> {
    return ipcRenderer.invoke(channel, ...args);
  },
  send<K extends keyof AppApiSend>(channel: K, ...args: AppApiSend[K]): void {
    ipcRenderer.send(channel, ...args);
  },
  sendSync<K extends keyof AppApiSync>(channel: K, ...args: SyncArgs<K>): SyncResult<K> {
    return ipcRenderer.sendSync(channel, ...args);
  },
};

export const Dialogs = {
  showMessageBox(options: MessageBoxOptions, parent: DialogParent) {
    return AppApiTransport.invoke('app-api:dialog:show-message-box', options, parent);
  },

  /** showMessageBox, resolving with the index of the clicked button. */
  async ask(options: MessageBoxOptions, parent: DialogParent): Promise<number> {
    const { response } = await Dialogs.showMessageBox(options, parent);
    return response;
  },

  /**
   * The blocking form of `ask`, for synchronous public APIs only. Returns `cancelId` (or -1)
   * if main can't show the dialog.
   */
  askSync(options: MessageBoxOptions, parent: DialogParent): number {
    const response = AppApiTransport.sendSync(
      'app-api:sync:dialog:show-message-box',
      options,
      parent
    );
    return typeof response === 'number' ? response : (options.cancelId ?? -1);
  },

  showOpenDialog(options: Electron.OpenDialogOptions, parent: DialogParent) {
    return AppApiTransport.invoke('app-api:dialog:show-open-dialog', options, parent);
  },

  showSaveDialog(options: Electron.SaveDialogOptions, parent: DialogParent) {
    return AppApiTransport.invoke('app-api:dialog:show-save-dialog', options, parent);
  },

  showErrorBox(title: string, content: string) {
    return AppApiTransport.invoke('app-api:dialog:show-error-box', title, content);
  },
};

let facts: AppFacts | null = null;

export const App = {
  /** isPackaged and the locale, read from main once per window. */
  facts(): AppFacts {
    if (!facts) facts = AppApiTransport.sendSync('app-api:sync:facts');
    return facts;
  },
  getPath(name: InvokeArgs<'app-api:app:get-path'>[0]) {
    return AppApiTransport.invoke('app-api:app:get-path', name);
  },
  quit() {
    AppApiTransport.send('app-api:app:quit');
  },
  relaunch() {
    AppApiTransport.send('app-api:app:relaunch');
  },
  exit(code: number) {
    AppApiTransport.send('app-api:app:exit', code);
  },
  crashMainProcess() {
    AppApiTransport.send('app-api:app:crash-main-process');
  },
  /** Main's handling of mailto: and tern: links. */
  openUrl(url: string) {
    AppApiTransport.send('app-api:app:open-url', url);
  },
  isQuitting() {
    return AppApiTransport.invoke('app-api:app:is-quitting');
  },
  /** Synchronous, for beforeunload callbacks. */
  openWindowCount(): number {
    return AppApiTransport.sendSync('app-api:sync:open-window-count');
  },
  getLoginItemSettings(options?: Electron.LoginItemSettingsOptions) {
    return AppApiTransport.invoke('app-api:app:get-login-item-settings', options);
  },
  setLoginItemSettings(settings: Electron.Settings) {
    return AppApiTransport.invoke('app-api:app:set-login-item-settings', settings);
  },
  isDefaultProtocolClient(scheme: string) {
    return AppApiTransport.invoke('app-api:app:is-default-protocol-client', scheme);
  },
  setAsDefaultProtocolClient(scheme: string) {
    return AppApiTransport.invoke('app-api:app:set-as-default-protocol-client', scheme);
  },
  removeAsDefaultProtocolClient(scheme: string) {
    return AppApiTransport.invoke('app-api:app:remove-as-default-protocol-client', scheme);
  },
  isInApplicationsFolder() {
    return AppApiTransport.invoke('app-api:app:is-in-applications-folder');
  },
  registerDefaultMailClient() {
    return AppApiTransport.invoke('app-api:app:register-default-mail-client');
  },
  updaterState(): Promise<UpdaterState> {
    return AppApiTransport.invoke('app-api:updater:get-state');
  },
  shouldUseDarkColors(): boolean {
    return AppApiTransport.sendSync('app-api:sync:should-use-dark-colors');
  },
  /** Linux windows don't inherit main's environment (electron/electron#3306). */
  mainProcessEnv() {
    return AppApiTransport.sendSync('app-api:sync:main-env');
  },
};

/** The main process's FileListCache, shared by every window. */
export const FileListCache = {
  get imageData(): string {
    return AppApiTransport.sendSync('app-api:sync:file-list-cache-image-data');
  },
  set imageData(imageData: string) {
    AppApiTransport.send('app-api:file-list-cache:set-image-data', imageData);
  },
};

/** The system clipboard, which only main can reach in Electron 44. */
export const Clipboard = {
  writeText(text: string) {
    AppApiTransport.send('app-api:clipboard:write-text', text);
  },
};

function command(cmd: WindowCommand) {
  AppApiTransport.send('app-api:window:command', cmd);
}

/**
 * The calling window. Reads are synchronous, from one snapshot of the window per call; commands
 * are sent without waiting.
 */
export const CurrentWindow = {
  state(): WindowState {
    return AppApiTransport.sendSync('app-api:sync:window-state');
  },
  getBounds(): Electron.Rectangle {
    return CurrentWindow.state().bounds;
  },
  getSize(): [number, number] {
    const { width, height } = CurrentWindow.state().bounds;
    return [width, height];
  },
  getPosition(): [number, number] {
    const { x, y } = CurrentWindow.state().bounds;
    return [x, y];
  },
  getMinimumSize(): [number, number] {
    return CurrentWindow.state().minimumSize;
  },
  isVisible() {
    return CurrentWindow.state().isVisible;
  },
  isMaximized() {
    return CurrentWindow.state().isMaximized;
  },
  isMinimized() {
    return CurrentWindow.state().isMinimized;
  },
  isFullScreen() {
    return CurrentWindow.state().isFullScreen;
  },
  isFocused() {
    return CurrentWindow.state().isFocused;
  },
  isResizable() {
    return CurrentWindow.state().resizable;
  },
  isDevToolsOpened() {
    return CurrentWindow.state().isDevToolsOpened;
  },

  close: () => command({ op: 'close' }),
  show: () => command({ op: 'show' }),
  hide: () => command({ op: 'hide' }),
  focus: () => command({ op: 'focus' }),
  center: () => command({ op: 'center' }),
  maximize: () => command({ op: 'maximize' }),
  unmaximize: () => command({ op: 'unmaximize' }),
  minimize: () => command({ op: 'minimize' }),
  setFullScreen: (flag: boolean) => command({ op: 'setFullScreen', flag }),
  setTitle: (title: string) => command({ op: 'setTitle', title }),
  setSize: (width: number, height: number) => command({ op: 'setSize', width, height }),
  setPosition: (x: number, y: number) => command({ op: 'setPosition', x, y }),
  setBounds: (bounds: Partial<Electron.Rectangle>) => command({ op: 'setBounds', bounds }),
  setMinimumSize: (width: number, height: number) =>
    command({ op: 'setMinimumSize', width, height }),
  setResizable: (flag: boolean) => command({ op: 'setResizable', flag }),
  setMenuBarVisibility: (flag: boolean) => command({ op: 'setMenuBarVisibility', flag }),
  setSheetOffset: (offset: number) => command({ op: 'setSheetOffset', offset }),
  /** macOS Quick Look. */
  previewFile: (path: string, displayName?: string) =>
    command({ op: 'previewFile', path, displayName }),
  showDefinitionForSelection: () => command({ op: 'showDefinitionForSelection' }),

  /** A webContents edit command, as the Edit menu runs it. */
  edit(cmd: EditCommand) {
    AppApiTransport.send('app-api:window:edit', cmd);
  },
};

export type CurrentWindowHandle = typeof CurrentWindow;

export const Screen = {
  getPrimaryDisplay(): Electron.Display {
    return AppApiTransport.sendSync('app-api:sync:primary-display');
  },
  getDisplayMatching(rect: Electron.Rectangle): Electron.Display {
    return AppApiTransport.sendSync('app-api:sync:display-matching', rect);
  },
};

export const Spellchecker = {
  availableLanguages() {
    return AppApiTransport.invoke('app-api:spellchecker:get-available-languages');
  },
  setLanguages(languages: string[]) {
    return AppApiTransport.invoke('app-api:spellchecker:set-languages', languages);
  },
  addWord(word: string) {
    AppApiTransport.send('app-api:spellchecker:add-word', word);
  },
  removeWord(word: string) {
    AppApiTransport.send('app-api:spellchecker:remove-word', word);
  },
};

/** A file in its own window with JavaScript and Node off. */
export function openStaticPage(options: StaticPageOptions) {
  AppApiTransport.send('app-api:window:open-static-page', options);
}

export const QuickPreviewWindows = {
  display(options: QuickPreviewDisplay) {
    return AppApiTransport.invoke('app-api:quickpreview:display', options);
  },
  capture(options: QuickPreviewCapture) {
    return AppApiTransport.invoke('app-api:quickpreview:capture', options);
  },
};

export const PrintWindows = {
  /** Opens the print preview of print.html in the temp directory; write that file first. */
  open(options: PrintWindowOptions) {
    return AppApiTransport.invoke('app-api:print:open', options);
  },
};

/** A native menu item with its click handler, as call sites build it. */
export interface ContextMenuItem extends Omit<MenuItemTemplate, 'id' | 'submenu'> {
  click?: () => void;
  submenu?: ContextMenuItem[];
}

/**
 * Splits a template into the serialisable form main builds the menu from, and the click
 * handlers by the ids given to the items that have one.
 */
export function serializeMenu(template: ContextMenuItem[]) {
  const handlers = new Map<string, () => void>();
  const walk = (items: ContextMenuItem[]): MenuItemTemplate[] =>
    items.map((item) => {
      const { click, submenu, ...rest } = item;
      const out: MenuItemTemplate = { ...rest };
      if (click) {
        const id = String(handlers.size);
        handlers.set(id, click);
        out.id = id;
      }
      if (submenu) out.submenu = walk(submenu);
      return out;
    });
  return { template: walk(template), handlers };
}

export const Menus = {
  /**
   * Shows a native context menu at the pointer (or `position`, in window coordinates) and runs
   * the clicked item's handler. Resolves once the menu has closed.
   */
  async popup(template: ContextMenuItem[], position: MenuPosition = {}): Promise<void> {
    const { template: serialised, handlers } = serializeMenu(template);
    const id = await AppApiTransport.invoke('app-api:menu:popup', serialised, position);
    const handler = id !== null ? handlers.get(id) : undefined;
    if (handler) handler();
  },

  /** The application menu as a popup, for windows without a menu bar. */
  popupApplicationMenu() {
    return AppApiTransport.invoke('app-api:menu:popup-application-menu');
  },
};
