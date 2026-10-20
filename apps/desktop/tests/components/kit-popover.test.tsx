// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import {
  Popover,
  PopoverTrigger,
  PopoverAnchor,
  PopoverContent,
  PopoverArrow,
} from '../../src/components/kit/popover';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from '../support/kit-helpers';
import { withRTLPortal, trackCommands } from '../support/kit-layer-helpers';

const content = () =>
  getPortalContainer().querySelector('[data-slot="popover-content"]') as HTMLElement;

describe('kit Popover', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('opens from its trigger into the portal root with the kit markup and legacy classes', () => {
    const { container } = render(
      <Popover>
        <PopoverTrigger>Open</PopoverTrigger>
        <PopoverContent className="custom">
          Body
          <PopoverArrow />
        </PopoverContent>
      </Popover>
    );
    const trigger = container.querySelector('[data-slot="popover-trigger"]');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(content()).toBe(null);
    fireEvent.click(trigger);
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    expect(trigger.getAttribute('data-state')).toBe('open');
    const el = content();
    expect(el.getAttribute('role')).toBe('dialog');
    expect(el.getAttribute('data-state')).toBe('open');
    expect(el.getAttribute('data-side')).toBe('bottom');
    expect(el.getAttribute('data-align')).toBe('center');
    expect(el.hasAttribute('data-ui-layer')).toBe(true);
    expect(el.className).toBe('fixed-popover custom');
    expect(el.parentElement.hasAttribute('data-radix-popper-content-wrapper')).toBe(true);
    const arrow = el.querySelector('[data-slot="popover-arrow"]');
    expect(arrow.getAttribute('class')).toBe('fixed-popover-pointer');
  });

  it('anchors to a rect through a virtual ref', async () => {
    const rect = new DOMRect(100, 120, 40, 20);
    render(
      <Popover open>
        <PopoverAnchor virtualRef={{ current: { getBoundingClientRect: () => rect } }} />
        <PopoverContent side="right" sideOffset={6}>
          Body
        </PopoverContent>
      </Popover>
    );
    await wait(50);
    const el = content();
    expect(el.getAttribute('data-side')).toBe('right');
    const box = el.getBoundingClientRect();
    expect(Math.round(box.left)).toBe(146);
  });

  it('is non-modal: an outside click closes it, and is not swallowed', async () => {
    const onOpenChange = jasmine.createSpy('onOpenChange');
    const outsideClick = jasmine.createSpy('outsideClick');
    const { container } = render(
      <div>
        <button onClick={outsideClick}>Outside</button>
        <Popover defaultOpen onOpenChange={onOpenChange}>
          <PopoverTrigger>Open</PopoverTrigger>
          <PopoverContent>Body</PopoverContent>
        </Popover>
      </div>
    );
    await wait(20);
    const outside = container.querySelector('button') as HTMLElement;
    expect(document.body.style.pointerEvents).not.toBe('none');
    fireEvent.pointerDown(outside);
    fireEvent.click(outside);
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(outsideClick).toHaveBeenCalled();
  });

  describe('Escape', function () {
    const commands = trackCommands();

    it('closes the popover without popping the sheet; the next Escape pops it', async () => {
      await wait(250);
      render(
        <Popover defaultOpen>
          <PopoverTrigger>Open</PopoverTrigger>
          <PopoverContent>
            <button>Inside</button>
          </PopoverContent>
        </Popover>
      );
      await wait(50);
      expect(getPortalContainer().contains(document.activeElement)).toBe(true);
      await pressRealKey('Escape');
      expect(commands.count('core:pop-sheet')).toBe(0);
      await wait(300);
      expect(content()).toBe(null);
      await pressRealKey('Escape');
      expect(commands.count('core:pop-sheet')).toBeGreaterThan(0);
    });

    it('consumes the Escape even when focus stayed outside the popover', async () => {
      await wait(250);
      const { container } = render(
        <Popover defaultOpen>
          <PopoverTrigger>Open</PopoverTrigger>
          <PopoverContent onOpenAutoFocus={(e) => e.preventDefault()}>Body</PopoverContent>
        </Popover>
      );
      const trigger = container.querySelector('[data-slot="popover-trigger"]') as HTMLElement;
      trigger.focus();
      await wait(50);
      expect(document.activeElement).toBe(trigger);
      await pressRealKey('Escape');
      expect(commands.count('core:pop-sheet')).toBe(0);
      expect(trigger.getAttribute('data-state')).toBe('closed');
    });

    it('keeps plain keys inside it from firing global commands', async () => {
      await wait(250);
      render(
        <Popover defaultOpen>
          <PopoverTrigger>Open</PopoverTrigger>
          <PopoverContent>
            <button>Inside</button>
          </PopoverContent>
        </Popover>
      );
      await wait(50);
      await pressRealKey('Down');
      await pressRealKey('Enter');
      expect(commands.count('core:next-item')).toBe(0);
      expect(commands.count('core:focus-item')).toBe(0);
    });
  });

  it('takes its CSS direction from the portal container in RTL', () => {
    withRTLPortal(() => {
      render(
        <Popover defaultOpen>
          <PopoverTrigger>Open</PopoverTrigger>
          <PopoverContent>Body</PopoverContent>
        </Popover>
      );
      expect(content().hasAttribute('dir')).toBe(false);
      expect(getComputedStyle(content()).direction).toBe('rtl');
    });
  });
});
