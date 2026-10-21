// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, cleanup } from '@testing-library/react';
import Webview from '../../src/components/webview';
import { openWebviewPopupExternally } from '../../src/main/windows/webview-guest';

describe('Webview', function () {
  let live: Map<string, Set<unknown>>;

  beforeEach(() => {
    live = new Map();
    const add = EventTarget.prototype.addEventListener;
    const remove = EventTarget.prototype.removeEventListener;
    spyOn(HTMLElement.prototype, 'addEventListener').andCallFake(function (
      this: HTMLElement,
      type: string,
      fn: any,
      opts?: any
    ) {
      if (this.tagName === 'WEBVIEW') {
        if (!live.has(type)) live.set(type, new Set());
        live.get(type).add(fn);
      }
      return add.call(this, type, fn, opts);
    });
    spyOn(HTMLElement.prototype, 'removeEventListener').andCallFake(function (
      this: HTMLElement,
      type: string,
      fn: any,
      opts?: any
    ) {
      if (this.tagName === 'WEBVIEW') live.get(type)?.delete(fn);
      return remove.call(this, type, fn, opts);
    });
  });

  afterEach(() => cleanup());

  it('keeps one did-navigate listener across src changes and removes it on unmount', () => {
    const { rerender, unmount, container } = render(<Webview src="about:blank#1" />);
    rerender(<Webview src="about:blank#2" />);
    rerender(<Webview src="about:blank#3" />);
    expect(live.get('did-navigate').size).toBe(1);
    expect(container.querySelector('webview').hasAttribute('allowpopups')).toBe(true);
    unmount();
    expect(Array.from(live.values()).every((fns) => fns.size === 0)).toBe(true);
  });

  it('opens http(s) popups from the guest in the browser and denies the window', () => {
    const openExternal = jasmine.createSpy('openExternal').andReturn(Promise.resolve());
    expect(openWebviewPopupExternally({ url: 'https://example.com/a' }, openExternal)).toEqual({
      action: 'deny',
    });
    expect(openWebviewPopupExternally({ url: 'file:///etc/passwd' }, openExternal)).toEqual({
      action: 'deny',
    });
    expect(openExternal.calls.map((c: any) => c.args[0])).toEqual(['https://example.com/a']);
  });
});
