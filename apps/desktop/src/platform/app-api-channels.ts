// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The channels between the windows and the main process for the Electron APIs only main has:
dialogs, app, the window's BrowserWindow and webContents, screen, native menus, the session's
spellchecker and the quick preview windows. `main/ipc/app-api.ts` answers them for Tern's own
windows only (`isAppWindowContents`); `platform/app-api.ts` is the windows' typed client.

Each map gives a channel's arguments and result. Invoke channels are `ipcRenderer.invoke` /
`ipcMain.handle`; send channels are fire-and-forget; sync channels are `ipcRenderer.sendSync`
and serve the few reads that synchronous APIs (`AppEnv.getSize()`, `isFullScreen()`, the
window dimensions saved in `beforeunload`) return directly.

`platform/developer-tools.js` and `static/index.js` are plain JavaScript loaded before the
TypeScript compile cache, so they spell out 'app-api:sync:facts' and 'app-api:sync:main-env'.
*/

/** Which window a dialog is attached to: the sender's, the main window's, or none (app-modal). */
export type DialogParent = 'window' | 'main' | 'none';

/** Electron's MessageBoxOptions without the parts that can't cross IPC. */
export type MessageBoxOptions = Omit<Electron.MessageBoxOptions, 'icon' | 'signal'> & {
  /** A file path. */
  icon?: string;
};

export interface WindowState {
  bounds: Electron.Rectangle;
  minimumSize: [number, number];
  isVisible: boolean;
  isMaximized: boolean;
  isMinimized: boolean;
  isFullScreen: boolean;
  isFocused: boolean;
  isDevToolsOpened: boolean;
  resizable: boolean;
}

/** The sender's BrowserWindow methods a window may call on itself. */
export type WindowCommand =
  | { op: 'close' }
  | { op: 'show' }
  | { op: 'hide' }
  | { op: 'focus' }
  | { op: 'center' }
  | { op: 'maximize' }
  | { op: 'unmaximize' }
  | { op: 'minimize' }
  | { op: 'setFullScreen'; flag: boolean }
  | { op: 'setTitle'; title: string }
  | { op: 'setSize'; width: number; height: number }
  | { op: 'setPosition'; x: number; y: number }
  | { op: 'setBounds'; bounds: Partial<Electron.Rectangle> }
  | { op: 'setMinimumSize'; width: number; height: number }
  | { op: 'setResizable'; flag: boolean }
  | { op: 'setMenuBarVisibility'; flag: boolean }
  | { op: 'setSheetOffset'; offset: number }
  | { op: 'previewFile'; path: string; displayName?: string }
  | { op: 'showDefinitionForSelection' };

/** The sender's webContents edit commands (Edit menu and the core:copy-style commands). */
export type EditCommand =
  | 'copy'
  | 'cut'
  | 'paste'
  | 'pasteAndMatchStyle'
  | 'undo'
  | 'redo'
  | 'selectAll';

/** A native menu item that can cross IPC. Main reports back the `id` of the clicked item. */
export interface MenuItemTemplate {
  id?: string;
  label?: string;
  sublabel?: string;
  type?: 'normal' | 'separator' | 'submenu' | 'checkbox' | 'radio';
  role?: Electron.MenuItemConstructorOptions['role'];
  enabled?: boolean;
  visible?: boolean;
  checked?: boolean;
  accelerator?: string;
  registerAccelerator?: boolean;
  toolTip?: string;
  submenu?: MenuItemTemplate[];
}

export interface MenuPosition {
  x?: number;
  y?: number;
}

/** A file shown in its own window with JavaScript and Node off (raw source, logs, clipped mail). */
export interface StaticPageOptions {
  filePath: string;
  title: string;
  width?: number;
  height?: number;
}

/** print.html loads the messages from this file next to it. */
export const PrintMessagesScriptName = 'print.messages.js';

export interface PrintWindowOptions {
  /** The window title and the PDF's default file name. */
  subject: string;
  /** print.html, which main writes with `messagesScript` into a private temp directory. */
  html: string;
  /** The contents of print.messages.js. */
  messagesScript: string;
}

export interface QuickPreviewDisplay {
  filePath: string;
  /** None for a type no strategy renders: the window opens empty. */
  strategy?: string;
}

export interface QuickPreviewCapture {
  filePath: string;
  previewPath: string;
  strategy: string;
}

export interface UpdaterState {
  state: string;
  releaseVersion?: string;
  releaseNotes?: string;
}

/** What main knows about the app that windows read synchronously. Constant for the run. */
export interface AppFacts {
  isPackaged: boolean;
  locale: string;
}

interface Channel<Args extends unknown[], Result> {
  args: Args;
  result: Result;
}

export interface AppApiInvoke {
  'app-api:dialog:show-message-box': Channel<
    [options: MessageBoxOptions, parent: DialogParent],
    Electron.MessageBoxReturnValue
  >;
  'app-api:dialog:show-open-dialog': Channel<
    [options: Electron.OpenDialogOptions, parent: DialogParent],
    Electron.OpenDialogReturnValue
  >;
  'app-api:dialog:show-save-dialog': Channel<
    [options: Electron.SaveDialogOptions, parent: DialogParent],
    Electron.SaveDialogReturnValue
  >;
  'app-api:dialog:show-error-box': Channel<[title: string, content: string], void>;
  'app-api:app:get-path': Channel<[name: Parameters<Electron.App['getPath']>[0]], string>;
  'app-api:app:get-login-item-settings': Channel<
    [options?: Electron.LoginItemSettingsOptions],
    Electron.LoginItemSettings
  >;
  'app-api:app:set-login-item-settings': Channel<[settings: Electron.Settings], void>;
  'app-api:app:is-default-protocol-client': Channel<[scheme: string], boolean>;
  'app-api:app:set-as-default-protocol-client': Channel<[scheme: string], boolean>;
  'app-api:app:remove-as-default-protocol-client': Channel<[scheme: string], boolean>;
  'app-api:app:is-in-applications-folder': Channel<[], boolean>;
  'app-api:app:is-quitting': Channel<[], boolean>;
  /** Windows: the registry entries for the mail client, made default where Windows allows. */
  'app-api:app:register-default-mail-client': Channel<
    [],
    { error: string | null; didMakeDefault: boolean }
  >;
  'app-api:updater:get-state': Channel<[], UpdaterState>;
  /** Resolves with the clicked item's id, or null once the menu closes without a click. */
  'app-api:menu:popup': Channel<
    [template: MenuItemTemplate[], position: MenuPosition],
    string | null
  >;
  'app-api:menu:popup-application-menu': Channel<[], void>;
  /** The print preview of the given page. */
  'app-api:print:open': Channel<[options: PrintWindowOptions], void>;
  'app-api:spellchecker:get-available-languages': Channel<[], string[]>;
  'app-api:spellchecker:set-languages': Channel<[languages: string[]], void>;
  'app-api:quickpreview:display': Channel<[options: QuickPreviewDisplay], void>;
  /** Resolves true once the thumbnail is written to `previewPath`. */
  'app-api:quickpreview:capture': Channel<[options: QuickPreviewCapture], boolean>;
}

export interface AppApiSend {
  'app-api:window:command': [command: WindowCommand];
  'app-api:window:edit': [command: EditCommand];
  'app-api:window:open-static-page': [options: StaticPageOptions];
  'app-api:app:quit': [];
  'app-api:app:relaunch': [];
  /** Emits the app's `will-exit`, then exits the main process with the code. */
  'app-api:app:exit': [code: number];
  'app-api:app:crash-main-process': [];
  'app-api:app:open-url': [url: string];
  'app-api:spellchecker:add-word': [word: string];
  'app-api:spellchecker:remove-word': [word: string];
  'app-api:file-list-cache:set-image-data': [imageData: string];
  /** The clipboard is main-only in Electron 44; windows write text through this. */
  'app-api:clipboard:write-text': [text: string];
}

export interface AppApiSync {
  'app-api:sync:facts': Channel<[], AppFacts>;
  'app-api:sync:window-state': Channel<[], WindowState>;
  'app-api:sync:primary-display': Channel<[], Electron.Display>;
  'app-api:sync:display-matching': Channel<[rect: Electron.Rectangle], Electron.Display>;
  /** Linux: main's environment, which windows don't inherit (electron/electron#3306). */
  'app-api:sync:main-env': Channel<[], { [key: string]: string | undefined }>;
  'app-api:sync:file-list-cache-image-data': Channel<[], string>;
  'app-api:sync:should-use-dark-colors': Channel<[], boolean>;
  /** Read in beforeunload callbacks, which answer synchronously. */
  'app-api:sync:open-window-count': Channel<[], number>;
}

/** Main to the window. */
export const AppApiEvent = {
  /** 'begin' or 'end' of a trackpad scroll gesture, from the webContents' input events. */
  GestureScroll: 'app-api:event:gesture-scroll',
} as const;

export type InvokeArgs<K extends keyof AppApiInvoke> = AppApiInvoke[K]['args'];
export type InvokeResult<K extends keyof AppApiInvoke> = AppApiInvoke[K]['result'];
export type SyncArgs<K extends keyof AppApiSync> = AppApiSync[K]['args'];
export type SyncResult<K extends keyof AppApiSync> = AppApiSync[K]['result'];
