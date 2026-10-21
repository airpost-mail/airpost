// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import {
  PrintWindowHost,
  PrintWindowLike,
  registerPrintWindowIPC,
} from '../../src/main/windows/print-window';

describe('the print window', function () {
  let handlers: { [channel: string]: (...args: any[]) => any };
  let host: PrintWindowHost;
  let win: PrintWindowLike;
  let windowOptions: Electron.BrowserWindowConstructorOptions;
  let printToPDFRequest: () => Promise<void>;
  const trusted: any = {};

  beforeEach(() => {
    handlers = {};
    const ipcMain: any = { handle: (channel: string, fn: any) => (handlers[channel] = fn) };
    win = {
      webContents: {
        ipc: {
          on: ((channel: string, fn: () => Promise<void>) => {
            if (channel === 'print-to-pdf') printToPDFRequest = fn;
          }) as any,
        },
        printToPDF: jasmine.createSpy('printToPDF').andReturn(Promise.resolve(Buffer.from('%PDF'))),
      },
      removeMenu: jasmine.createSpy('removeMenu'),
      loadURL: jasmine.createSpy('loadURL').andReturn(Promise.resolve()),
    };
    host = {
      tempPath: () => '/tmp/tern-temp',
      createWindow: (options) => {
        windowOptions = options;
        return win;
      },
      showSaveDialog: jasmine
        .createSpy('showSaveDialog')
        .andReturn(Promise.resolve({ canceled: false, filePath: '/out/Hello.pdf' })),
      showMessageBox: jasmine
        .createSpy('showMessageBox')
        .andReturn(Promise.resolve({ response: 0, checkboxChecked: false })),
      writeFile: jasmine.createSpy('writeFile'),
    };
    registerPrintWindowIPC(ipcMain as Electron.IpcMain, host, (contents) => contents === trusted);
  });

  it("answers Tern's windows only", async () => {
    let error: Error = null;
    try {
      await handlers['app-api:print:open']({ sender: {} }, { subject: 'Hello' });
    } catch (err) {
      error = err;
    }
    expect(error && error.message).toContain('only available to Tern windows');
    expect(windowOptions).toBeUndefined();
  });

  it('loads print.html from the temp directory with the print preload', async () => {
    await handlers['app-api:print:open']({ sender: trusted }, { subject: 'Hello' });
    expect(windowOptions.title).toBe('Print - Hello');
    expect(windowOptions.webPreferences.preload).toMatch(
      /features[/\\]print[/\\]static[/\\]preload\.js$/
    );
    expect(windowOptions.webPreferences.contextIsolation).toBe(true);
    expect(windowOptions.webPreferences.nodeIntegration).toBe(false);
    expect(win.removeMenu).toHaveBeenCalled();
    expect((win.loadURL as jasmine.Spy).mostRecentCall.args[0]).toBe(
      'file:///tmp/tern-temp/print.html'
    );
  });

  it('saves a PDF where the user picks, and nothing when they cancel', async () => {
    await handlers['app-api:print:open']({ sender: trusted }, { subject: 'Hello' });
    await printToPDFRequest();
    expect((host.showSaveDialog as jasmine.Spy).mostRecentCall.args[0]).toEqual({
      defaultPath: 'Hello.pdf',
    });
    expect(host.writeFile).toHaveBeenCalledWith('/out/Hello.pdf', Buffer.from('%PDF'));

    (host.showSaveDialog as jasmine.Spy).andReturn(Promise.resolve({ canceled: true }));
    await printToPDFRequest();
    expect((host.writeFile as jasmine.Spy).callCount).toBe(1);
  });

  it('shows the error dialog when the PDF fails', async () => {
    (win.webContents.printToPDF as jasmine.Spy).andReturn(Promise.reject(new Error('boom')));
    await handlers['app-api:print:open']({ sender: trusted }, { subject: 'Hello' });
    await printToPDFRequest();
    expect(host.writeFile).not.toHaveBeenCalled();
    const options = (host.showMessageBox as jasmine.Spy).mostRecentCall.args[0];
    expect(options.type).toBe('warning');
    expect(options.message).toBe('Save as PDF Failed');
    expect(options.detail).toContain('Error: boom');
  });
});
