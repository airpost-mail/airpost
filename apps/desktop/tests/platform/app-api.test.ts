// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import {
  AppApiTransport,
  CurrentWindow,
  Dialogs,
  FileListCache,
  Menus,
  serializeMenu,
} from '../../src/platform/app-api';

// The windows' client against a stand-in transport: no IPC reaches main.
describe('the app-api client', function () {
  let invoked: any[][];
  let sent: any[][];
  let invokeResult: any;
  let syncResult: any;

  beforeEach(() => {
    invoked = [];
    sent = [];
    invokeResult = undefined;
    syncResult = undefined;
    spyOn(AppApiTransport, 'invoke').andCallFake((...args: any[]) => {
      invoked.push(args);
      return Promise.resolve(invokeResult);
    });
    spyOn(AppApiTransport, 'send').andCallFake((...args: any[]) => {
      sent.push(args);
    });
    spyOn(AppApiTransport, 'sendSync').andCallFake(() => syncResult);
  });

  it('asks with the given parent and resolves with the button', async () => {
    invokeResult = { response: 2, checkboxChecked: false };
    const options = { type: 'warning' as const, message: 'Delete?', buttons: ['A', 'B', 'C'] };
    expect(await Dialogs.ask(options, 'none')).toBe(2);
    expect(invoked).toEqual([['app-api:dialog:show-message-box', options, 'none']]);
  });

  it('reads the window from one snapshot and sends commands', () => {
    syncResult = {
      bounds: { x: 5, y: 6, width: 700, height: 500 },
      minimumSize: [400, 250],
      isVisible: true,
      isMaximized: true,
      isFullScreen: false,
    };
    expect(CurrentWindow.getSize()).toEqual([700, 500]);
    expect(CurrentWindow.getPosition()).toEqual([5, 6]);
    expect(CurrentWindow.isMaximized()).toBe(true);
    expect(AppApiTransport.sendSync).toHaveBeenCalledWith('app-api:sync:window-state');

    CurrentWindow.setTitle('Tern');
    CurrentWindow.setSize(800, 600);
    CurrentWindow.edit('paste');
    expect(sent).toEqual([
      ['app-api:window:command', { op: 'setTitle', title: 'Tern' }],
      ['app-api:window:command', { op: 'setSize', width: 800, height: 600 }],
      ['app-api:window:edit', 'paste'],
    ]);
  });

  it("is AppEnv's current window", () => {
    expect(AppEnv.getCurrentWindow()).toBe(CurrentWindow);
  });

  it('reads and writes the shared file list cache', () => {
    syncResult = '{"x":1}';
    expect(FileListCache.imageData).toBe('{"x":1}');
    FileListCache.imageData = '{}';
    expect(sent).toEqual([['app-api:file-list-cache:set-image-data', '{}']]);
  });

  describe('menus', () => {
    it('gives ids to clickable items only, through submenus', () => {
      const noop = () => {};
      const { template, handlers } = serializeMenu([
        { label: 'Reply', click: noop },
        { type: 'separator' },
        { label: 'Move', submenu: [{ label: 'Inbox', type: 'radio', checked: true, click: noop }] },
        { label: 'Copy', role: 'copy' },
      ]);
      expect(template).toEqual([
        { label: 'Reply', id: '0' },
        { type: 'separator' },
        { label: 'Move', submenu: [{ label: 'Inbox', type: 'radio', checked: true, id: '1' }] },
        { label: 'Copy', role: 'copy' },
      ]);
      expect([...handlers.keys()]).toEqual(['0', '1']);
    });

    it("runs the clicked item's handler, and none when dismissed", async () => {
      const reply = jasmine.createSpy('reply');
      const forward = jasmine.createSpy('forward');
      const template = [
        { label: 'Reply', click: reply },
        { label: 'Forward', click: forward },
      ];

      invokeResult = '1';
      await Menus.popup(template, { x: 3, y: 4 });
      expect(invoked[0][0]).toBe('app-api:menu:popup');
      expect(invoked[0][2]).toEqual({ x: 3, y: 4 });
      expect(forward).toHaveBeenCalled();
      expect(reply).not.toHaveBeenCalled();

      invokeResult = null;
      await Menus.popup(template);
      expect(reply).not.toHaveBeenCalled();
      expect(forward.callCount).toBe(1);
    });
  });
});
