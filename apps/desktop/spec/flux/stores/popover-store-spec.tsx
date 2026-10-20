// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { fireEvent } from '@testing-library/react';
import PopoverStore from '../../../src/data/stores/popover';
import { getPortalContainer } from '../../../src/components/kit/portal';
import { wait, pressRealKey } from '../../components/kit/kit-spec-helpers';
import { trackCommands } from '../../components/kit/layer-spec-helpers';

const content = () =>
  getPortalContainer().querySelector('[data-slot="popover-content"]') as HTMLElement;

const RECT = { top: 200, left: 200, width: 40, height: 20, bottom: 220, right: 240 };

const open = (body: React.ReactElement, options: any = {}) =>
  PopoverStore.openPopover(body, { originRect: RECT, direction: 'down', ...options });

describe('PopoverStore on the UI kit', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => {
    PopoverStore.closePopover();
  });

  it('opens the kit popover with the legacy classes, on the side asked for', () => {
    open(<div className="body">Body</div>, { direction: 'left' });
    expect(PopoverStore.isOpen).toBe(true);
    const el = content();
    expect(el.className).toBe('fixed-popover');
    expect(el.getAttribute('data-variant')).toBe('bare');
    expect(el.getAttribute('data-side')).toBe('left');
    expect(el.querySelector('.body').textContent).toBe('Body');
    expect(el.querySelector('.fixed-popover-pointer')).not.toBe(null);
    const trap = document.querySelector<HTMLElement>('.fixed-popover-blur-trap');
    expect([trap.style.top, trap.style.left, trap.style.width]).toEqual(['200px', '200px', '40px']);
  });

  it('maps up to the top side and replaces an open popover', () => {
    open(<div className="first" />);
    open(<div className="second" />, { direction: 'up' });
    expect(getPortalContainer().querySelector('.first')).toBe(null);
    expect(content().getAttribute('data-side')).toBe('top');
  });

  it('focuses the first field, as before the kit', async () => {
    open(
      <div>
        <button>Plain</button>
        <input className="field" />
      </div>
    );
    await wait(20);
    expect(document.activeElement).toBe(content().querySelector('.field'));
  });

  describe('closing', function () {
    const commands = trackCommands();

    it('closes on Escape without popping the sheet', async () => {
      open(<input className="field" />);
      await wait(20);
      await pressRealKey('Escape');
      expect(PopoverStore.isOpen).toBe(false);
      expect(commands.count('core:pop-sheet')).toBe(0);
    });

    it('closes on a window resize', () => {
      open(<div />);
      window.dispatchEvent(new Event('resize'));
      expect(PopoverStore.isOpen).toBe(false);
    });

    it('closes when the app loses focus, unless closeOnAppBlur is false', () => {
      open(<div />, { closeOnAppBlur: false });
      window.dispatchEvent(new Event('blur'));
      expect(PopoverStore.isOpen).toBe(true);
      open(<div />);
      window.dispatchEvent(new Event('blur'));
      expect(PopoverStore.isOpen).toBe(false);
    });

    it('closes on a click outside', async () => {
      open(<div />);
      // Radix starts listening for outside clicks a tick after opening.
      await wait(10);
      fireEvent.pointerDown(document.body);
      fireEvent.click(document.body);
      expect(PopoverStore.isOpen).toBe(false);
    });
  });
});
