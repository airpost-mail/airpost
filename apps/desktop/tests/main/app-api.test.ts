// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import {
  AppApiHost,
  MenuDismissGraceMs,
  editableContextMenuTemplate,
  menuFromTemplate,
  registerAppApiIPC,
} from '../../src/main/ipc/app-api';
import { registerQuickPreviewWindowsIPC } from '../../src/main/windows/quick-preview';

// ipcMain with the handlers kept, so a spec can call them as a window would.
function fakeIpcMain() {
  const handlers: { [channel: string]: (...args: any[]) => any } = {};
  const listeners: { [channel: string]: (...args: any[]) => void } = {};
  const ipcMain: any = {
    handle: (channel: string, fn: any) => (handlers[channel] = fn),
    on: (channel: string, fn: any) => (listeners[channel] = fn),
  };
  return {
    ipcMain: ipcMain as Electron.IpcMain,
    invoke: (sender: any, channel: string, ...args: any[]) =>
      handlers[channel]({ sender }, ...args),
    send: (sender: any, channel: string, ...args: any[]) => listeners[channel]({ sender }, ...args),
    sendSync: (sender: any, channel: string, ...args: any[]) => {
      const event: any = { sender };
      listeners[channel](event, ...args);
      return event.returnValue;
    },
    handlers,
    listeners,
  };
}

function fakeWindow(): any {
  const win: any = {
    webContents: {
      isDevToolsOpened: () => false,
      showDefinitionForSelection: jasmine.createSpy('showDefinitionForSelection'),
    },
  };
  for (const method of [
    'close',
    'show',
    'hide',
    'focus',
    'center',
    'maximize',
    'unmaximize',
    'minimize',
    'setFullScreen',
    'setTitle',
    'setSize',
    'setPosition',
    'setBounds',
    'setMinimumSize',
    'setResizable',
    'setMenuBarVisibility',
    'setSheetOffset',
    'previewFile',
  ]) {
    win[method] = jasmine.createSpy(method);
  }
  win.getBounds = () => ({ x: 10, y: 20, width: 800, height: 600 });
  win.getMinimumSize = () => [400, 250];
  win.isVisible = () => true;
  win.isMaximized = () => false;
  win.isMinimized = () => false;
  win.isFullScreen = () => true;
  win.isFocused = () => true;
  win.isResizable = () => true;
  return win;
}

describe('the app-api main handlers', function () {
  let ipc: ReturnType<typeof fakeIpcMain>;
  let host: AppApiHost;
  let trusted: any;
  let untrusted: any;
  let senderWindow: any;
  let mainWindow: any;
  let popups: { template: Electron.MenuItemConstructorOptions[]; options: any }[];

  beforeEach(() => {
    ipc = fakeIpcMain();
    trusted = {
      copy: jasmine.createSpy('copy'),
      reload: jasmine.createSpy('reload'),
      session: {
        availableSpellCheckerLanguages: ['en-US', 'de'],
        setSpellCheckerLanguages: jasmine.createSpy('setSpellCheckerLanguages'),
        addWordToSpellCheckerDictionary: jasmine.createSpy('addWord'),
        removeWordFromSpellCheckerDictionary: jasmine.createSpy('removeWord'),
      },
    };
    untrusted = {};
    senderWindow = fakeWindow();
    mainWindow = fakeWindow();
    popups = [];
    const fileListCache = { imageData: '{}' };
    host = {
      dialog: {
        showMessageBox: jasmine
          .createSpy('showMessageBox')
          .andReturn(Promise.resolve({ response: 1, checkboxChecked: false })),
        showMessageBoxSync: jasmine.createSpy('showMessageBoxSync').andReturn(2),
        showOpenDialog: jasmine
          .createSpy('showOpenDialog')
          .andReturn(Promise.resolve({ canceled: false, filePaths: ['/a'] })),
        showSaveDialog: jasmine
          .createSpy('showSaveDialog')
          .andReturn(Promise.resolve({ canceled: false, filePath: '/b' })),
        showErrorBox: jasmine.createSpy('showErrorBox'),
      },
      app: {
        isPackaged: true,
        getLocale: () => 'pt-BR',
        getPath: jasmine.createSpy('getPath').andReturn('/tmp'),
        quit: jasmine.createSpy('quit'),
        relaunch: jasmine.createSpy('relaunch'),
        emit: jasmine.createSpy('emit'),
        getLoginItemSettings: jasmine.createSpy('getLoginItemSettings'),
        setLoginItemSettings: jasmine.createSpy('setLoginItemSettings'),
        isDefaultProtocolClient: jasmine.createSpy('isDefaultProtocolClient').andReturn(true),
        setAsDefaultProtocolClient: jasmine.createSpy('setAsDefaultProtocolClient'),
        removeAsDefaultProtocolClient: jasmine.createSpy('removeAsDefaultProtocolClient'),
        isInApplicationsFolder: jasmine.createSpy('isInApplicationsFolder').andReturn(false),
      },
      screen: {
        getPrimaryDisplay: () => ({ id: 1 }) as any,
        getDisplayMatching: (rect: Electron.Rectangle) => ({ id: rect.x }) as any,
      },
      nativeTheme: { shouldUseDarkColors: true },
      clipboard: { writeText: jasmine.createSpy('writeText').andReturn(Promise.resolve()) },
      buildMenu: (template) => ({
        popup: (options: any) => popups.push({ template, options }),
      }),
      isTrustedSender: (contents) => contents === trusted,
      windowFor: (contents) => (contents === trusted ? senderWindow : null),
      mainWindow: () => mainWindow,
      applicationMenu: () => null,
      isQuitting: () => false,
      openUrl: jasmine.createSpy('openUrl'),
      openWindowCount: () => 3,
      fileListCache,
      updaterState: () => ({ state: 'idle' }),
      registerDefaultMailClient: () => Promise.resolve({ error: null, didMakeDefault: true }),
      openStaticPage: jasmine.createSpy('openStaticPage'),
      exitProcess: jasmine.createSpy('exitProcess'),
      crashProcess: jasmine.createSpy('crashProcess'),
    } as AppApiHost;
    registerAppApiIPC(ipc.ipcMain, host);
  });

  it("answers Tern's windows only", async () => {
    let error: Error = null;
    try {
      await ipc.invoke(untrusted, 'app-api:app:get-path', 'temp');
    } catch (err) {
      error = err;
    }
    expect(error && error.message).toContain('only available to Tern windows');
    ipc.send(untrusted, 'app-api:app:quit');
    expect(host.app.quit).not.toHaveBeenCalled();
    expect(ipc.sendSync(untrusted, 'app-api:sync:facts')).toBe(null);

    expect(await ipc.invoke(trusted, 'app-api:app:get-path', 'temp')).toBe('/tmp');
    expect(ipc.sendSync(trusted, 'app-api:sync:facts')).toEqual({
      isPackaged: true,
      locale: 'pt-BR',
    });
  });

  it('attaches a dialog to the sender, the main window or nothing', async () => {
    const options = { message: 'Hi', buttons: ['OK'] };
    const result = await ipc.invoke(trusted, 'app-api:dialog:show-message-box', options, 'window');
    expect(result).toEqual({ response: 1, checkboxChecked: false });
    await ipc.invoke(trusted, 'app-api:dialog:show-message-box', options, 'main');
    await ipc.invoke(trusted, 'app-api:dialog:show-message-box', options, 'none');
    const calls = (host.dialog.showMessageBox as jasmine.Spy).calls.map((c) => c.args);
    expect(calls[0]).toEqual([senderWindow, options]);
    expect(calls[1]).toEqual([mainWindow, options]);
    expect(calls[2]).toEqual([options]);

    expect(
      await ipc.invoke(trusted, 'app-api:dialog:show-open-dialog', { title: 'Open' }, 'window')
    ).toEqual({ canceled: false, filePaths: ['/a'] });
    expect((host.dialog.showOpenDialog as jasmine.Spy).mostRecentCall.args).toEqual([
      senderWindow,
      { title: 'Open' },
    ]);
  });

  it('shows a blocking message box for a sync sender', () => {
    const options = { message: 'Sure?', buttons: ['Send', 'Cancel'] };
    expect(ipc.sendSync(trusted, 'app-api:sync:dialog:show-message-box', options, 'window')).toBe(
      2
    );
    ipc.sendSync(trusted, 'app-api:sync:dialog:show-message-box', options, 'none');
    const calls = (host.dialog.showMessageBoxSync as jasmine.Spy).calls.map((c) => c.args);
    expect(calls).toEqual([[senderWindow, options], [options]]);
    expect(ipc.sendSync(untrusted, 'app-api:sync:dialog:show-message-box', options, 'none')).toBe(
      null
    );
  });

  it("runs window commands on the sender's window", () => {
    ipc.send(trusted, 'app-api:window:command', { op: 'setTitle', title: 'Tern · Hi' });
    ipc.send(trusted, 'app-api:window:command', { op: 'setMinimumSize', width: 875, height: 250 });
    ipc.send(trusted, 'app-api:window:command', { op: 'setFullScreen', flag: true });
    ipc.send(trusted, 'app-api:window:command', { op: 'showDefinitionForSelection' });
    expect(senderWindow.setTitle).toHaveBeenCalledWith('Tern · Hi');
    expect(senderWindow.setMinimumSize).toHaveBeenCalledWith(875, 250);
    expect(senderWindow.setFullScreen).toHaveBeenCalledWith(true);
    expect(senderWindow.webContents.showDefinitionForSelection).toHaveBeenCalled();
    expect(mainWindow.setTitle).not.toHaveBeenCalled();
  });

  it('reads the window in one synchronous snapshot', () => {
    expect(ipc.sendSync(trusted, 'app-api:sync:window-state')).toEqual({
      bounds: { x: 10, y: 20, width: 800, height: 600 },
      minimumSize: [400, 250],
      isVisible: true,
      isMaximized: false,
      isMinimized: false,
      isFullScreen: true,
      isFocused: true,
      isDevToolsOpened: false,
      resizable: true,
    });
    // A failing read still answers, or the window would block forever.
    host.windowFor = () => null;
    spyOn(console, 'error');
    expect(ipc.sendSync(trusted, 'app-api:sync:window-state')).toBe(null);
  });

  it('runs only the edit commands', () => {
    spyOn(console, 'error');
    ipc.send(trusted, 'app-api:window:edit', 'copy');
    ipc.send(trusted, 'app-api:window:edit', 'reload');
    expect(trusted.copy).toHaveBeenCalled();
    expect(trusted.reload).not.toHaveBeenCalled();
  });

  it('emits will-exit before exiting main', () => {
    ipc.send(trusted, 'app-api:app:exit', 3);
    expect(host.app.emit).toHaveBeenCalledWith('will-exit');
    expect(host.exitProcess).toHaveBeenCalledWith(3);
  });

  it('shares the file list cache between windows', () => {
    ipc.send(trusted, 'app-api:file-list-cache:set-image-data', '{"a":1}');
    expect(ipc.sendSync(trusted, 'app-api:sync:file-list-cache-image-data')).toBe('{"a":1}');
  });

  it("writes text to main's clipboard", () => {
    ipc.send(trusted, 'app-api:clipboard:write-text', 'alice@tern.test');
    ipc.send(untrusted, 'app-api:clipboard:write-text', 'nope');
    expect(host.clipboard.writeText).toHaveBeenCalledWith('alice@tern.test');
    expect((host.clipboard.writeText as jasmine.Spy).calls.length).toBe(1);
  });

  it("drives the sender's session spellchecker", async () => {
    expect(await ipc.invoke(trusted, 'app-api:spellchecker:get-available-languages')).toEqual([
      'en-US',
      'de',
    ]);
    await ipc.invoke(trusted, 'app-api:spellchecker:set-languages', ['de']);
    ipc.send(trusted, 'app-api:spellchecker:add-word', 'Tern');
    expect(trusted.session.setSpellCheckerLanguages).toHaveBeenCalledWith(['de']);
    expect(trusted.session.addWordToSpellCheckerDictionary).toHaveBeenCalledWith('Tern');
  });

  describe('context menus', () => {
    const template = [
      { id: '0', label: 'Reply' },
      { type: 'separator' as const },
      {
        label: 'Move',
        submenu: [{ id: '1', label: 'Inbox', checked: true, type: 'radio' as const }],
      },
      { label: 'Copy', role: 'copy' as const },
    ];

    it('builds the template and resolves with the clicked id', async () => {
      const clicked = ipc.invoke(trusted, 'app-api:menu:popup', template, { x: 10.4, y: 20 });
      expect(popups.length).toBe(1);
      const { template: built, options } = popups[0];
      expect(options.window).toBe(senderWindow);
      expect(options.x).toBe(10);
      expect(options.y).toBe(20);
      expect(built[1]).toEqual({ type: 'separator' });
      expect(built[3]).toEqual({ label: 'Copy', role: 'copy' });
      const submenu = built[2].submenu as Electron.MenuItemConstructorOptions[];
      expect(submenu[0].label).toBe('Inbox');
      expect(submenu[0].checked).toBe(true);

      submenu[0].click(null, null, null);
      options.callback();
      expect(await clicked).toBe('1');
    });

    it('resolves with null once a menu closes without a click', async () => {
      let result: string | null | undefined;
      ipc.invoke(trusted, 'app-api:menu:popup', template, {}).then((r: string) => (result = r));
      popups[0].options.callback();
      await Promise.resolve();
      expect(result).toBeUndefined();
      // On macOS the click can follow the close callback.
      advanceClock(MenuDismissGraceMs);
      await Promise.resolve();
      expect(result).toBe(null);
    });

    it('takes a click that arrives after the close callback', async () => {
      const clicked = ipc.invoke(trusted, 'app-api:menu:popup', template, {});
      popups[0].options.callback();
      popups[0].template[0].click(null, null, null);
      advanceClock(MenuDismissGraceMs);
      expect(await clicked).toBe('0');
    });

    it('leaves items without an id to Electron', () => {
      const built = menuFromTemplate([{ label: 'Paste', role: 'paste' }], () => {});
      expect(built[0].click).toBeUndefined();
    });
  });

  describe('the editable area menu', () => {
    const contents: any = {
      replaceMisspelling: jasmine.createSpy('replaceMisspelling'),
      cut: jasmine.createSpy('cut'),
      copy: jasmine.createSpy('copy'),
      paste: jasmine.createSpy('paste'),
      pasteAndMatchStyle: jasmine.createSpy('pasteAndMatchStyle'),
      session: { addWordToSpellCheckerDictionary: jasmine.createSpy('learn') },
    };
    const editFlags: any = { canCut: false, canCopy: true, canPaste: true };

    it('offers suggestions and Learn Spelling for a misspelled word', () => {
      const template = editableContextMenuTemplate(contents, {
        misspelledWord: 'teh',
        dictionarySuggestions: ['the', 'ten'],
        editFlags,
      });
      expect(template.map((i) => i.label || i.type)).toEqual([
        'the',
        'ten',
        'separator',
        'Learn Spelling',
        'separator',
        'Cut',
        'Copy',
        'Paste',
        'Paste and Match Style',
      ]);
      template[0].click(null, null, null);
      expect(contents.replaceMisspelling).toHaveBeenCalledWith('the');
      template[3].click(null, null, null);
      expect(contents.session.addWordToSpellCheckerDictionary).toHaveBeenCalledWith('teh');
      expect(template[5].enabled).toBe(false);
      expect(template[6].enabled).toBe(true);
    });

    it('says so when there are no guesses, and has only the edit items otherwise', () => {
      const noGuesses = editableContextMenuTemplate(contents, {
        misspelledWord: 'qqq',
        dictionarySuggestions: [],
        editFlags,
      });
      expect(noGuesses[0]).toEqual({ label: 'No Guesses Found', enabled: false });
      const plain = editableContextMenuTemplate(contents, {
        misspelledWord: '',
        dictionarySuggestions: [],
        editFlags,
      });
      expect(plain.map((i) => i.label)).toEqual(['Cut', 'Copy', 'Paste', 'Paste and Match Style']);
    });
  });

  it('refuses quick preview strategies it does not know', async () => {
    const qp = fakeIpcMain();
    registerQuickPreviewWindowsIPC(
      qp.ipcMain,
      { configDirPath: '/tmp', autoloadImages: () => false },
      (contents) => contents === trusted
    );
    let error: Error = null;
    try {
      await qp.invoke(trusted, 'app-api:quickpreview:capture', {
        filePath: '/tmp/files/a.md',
        previewPath: '/tmp/files/a.md.png',
        strategy: 'javascript:alert(1)',
      });
    } catch (err) {
      error = err;
    }
    expect(error && error.message).toContain('Unknown quick preview strategy');
  });
});
