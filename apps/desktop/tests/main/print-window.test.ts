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
  let closeWindow: () => void;
  const page = { subject: 'Hello', html: '<p>Hi</p>', messagesScript: 'window.printMessages = []' };
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
      once: ((event: string, fn: () => void) => {
        if (event === 'closed') closeWindow = fn;
      }) as any,
    };
    host = {
      makePrivateDir: () => '/tmp/tern-print-abc',
      removeDir: jasmine.createSpy('removeDir'),
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
      writePrivateFile: jasmine.createSpy('writePrivateFile'),
    };
    registerPrintWindowIPC(ipcMain as Electron.IpcMain, host, (contents) => contents === trusted);
  });

  it("answers Tern's windows only", async () => {
    let error: Error = null;
    try {
      await handlers['app-api:print:open']({ sender: {} }, page);
    } catch (err) {
      error = err;
    }
    expect(error && error.message).toContain('only available to Tern windows');
    expect(windowOptions).toBeUndefined();
    expect(host.writePrivateFile).not.toHaveBeenCalled();
  });

  it('writes the page into its own temp directory, loads it with the print preload, and removes it on close', async () => {
    await handlers['app-api:print:open']({ sender: trusted }, page);
    expect(host.writePrivateFile).toHaveBeenCalledWith('/tmp/tern-print-abc/print.html', page.html);
    expect(host.writePrivateFile).toHaveBeenCalledWith(
      '/tmp/tern-print-abc/print.messages.js',
      page.messagesScript
    );
    expect(windowOptions.title).toBe('Print - Hello');
    expect(windowOptions.webPreferences.preload).toMatch(
      /features[/\\]print[/\\]static[/\\]preload\.js$/
    );
    expect(windowOptions.webPreferences.contextIsolation).toBe(true);
    expect(windowOptions.webPreferences.nodeIntegration).toBe(false);
    expect(win.removeMenu).toHaveBeenCalled();
    expect((win.loadURL as jasmine.Spy).mostRecentCall.args[0]).toBe(
      'file:///tmp/tern-print-abc/print.html'
    );
    expect(host.removeDir).not.toHaveBeenCalled();
    closeWindow();
    expect(host.removeDir).toHaveBeenCalledWith('/tmp/tern-print-abc');
  });

  it('saves a PDF where the user picks, and nothing when they cancel', async () => {
    await handlers['app-api:print:open']({ sender: trusted }, page);
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
    await handlers['app-api:print:open']({ sender: trusted }, page);
    await printToPDFRequest();
    expect(host.writeFile).not.toHaveBeenCalled();
    const options = (host.showMessageBox as jasmine.Spy).mostRecentCall.args[0];
    expect(options.type).toBe('warning');
    expect(options.message).toBe('Save as PDF Failed');
    expect(options.detail).toContain('Error: boom');
  });
});
