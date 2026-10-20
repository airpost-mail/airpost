// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Tooltip, TooltipTrigger, TooltipContent } from '../../src/components/kit/tooltip';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from '../support/kit-helpers';
import { withRTLPortal, trackCommands } from '../support/kit-layer-helpers';

const content = () =>
  getPortalContainer().querySelector('[data-slot="tooltip-content"]') as HTMLElement;

describe('kit Tooltip', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('renders without an outer provider, into the portal root, above its trigger', () => {
    const { container } = render(
      <Tooltip open>
        <TooltipTrigger>?</TooltipTrigger>
        <TooltipContent className="custom">Help</TooltipContent>
      </Tooltip>
    );
    const el = content();
    expect(el).not.toBe(null);
    expect(el.getAttribute('data-state')).toMatch(/^(delayed|instant)-open$/);
    expect(el.getAttribute('data-side')).toBe('top');
    expect(el.className).toBe('custom');
    expect(el.hasAttribute('data-ui-layer')).toBe(false);
    // Radix keeps a visually hidden role="tooltip" copy for screen readers.
    const hidden = getPortalContainer().querySelector('[role="tooltip"]');
    expect(hidden.textContent).toBe('Help');
    const trigger = container.querySelector('[data-slot="tooltip-trigger"]');
    expect(trigger.getAttribute('aria-describedby')).toBe(hidden.id);
  });

  it('opens on keyboard focus and closes on blur', async () => {
    const { container } = render(
      <Tooltip>
        <TooltipTrigger>?</TooltipTrigger>
        <TooltipContent>Help</TooltipContent>
      </Tooltip>
    );
    const trigger = container.querySelector('[data-slot="tooltip-trigger"]') as HTMLElement;
    expect(content()).toBe(null);
    fireEvent.focus(trigger);
    expect(content().getAttribute('data-state')).toBe('instant-open');
    fireEvent.blur(trigger);
    await wait(250);
    expect(content()).toBe(null);
  });

  it('waits 700 ms on hover by default', async () => {
    const { container } = render(
      <Tooltip>
        <TooltipTrigger>?</TooltipTrigger>
        <TooltipContent>Help</TooltipContent>
      </Tooltip>
    );
    const trigger = container.querySelector('[data-slot="tooltip-trigger"]') as HTMLElement;
    fireEvent.pointerMove(trigger, { pointerType: 'mouse' });
    await wait(400);
    expect(content()).toBe(null);
    await wait(450);
    expect(content().getAttribute('data-state')).toBe('delayed-open');
  });

  describe('Escape', function () {
    const commands = trackCommands();

    it("closes the tooltip and still pops the sheet (it doesn't consume Escape)", async () => {
      await wait(250);
      const onOpenChange = jasmine.createSpy('onOpenChange');
      render(
        <Tooltip defaultOpen onOpenChange={onOpenChange}>
          <TooltipTrigger>?</TooltipTrigger>
          <TooltipContent>Help</TooltipContent>
        </Tooltip>
      );
      await wait(50);
      await pressRealKey('Escape');
      expect(onOpenChange).toHaveBeenCalledWith(false);
      expect(commands.count('core:pop-sheet')).toBeGreaterThan(0);
    });
  });

  it('takes its CSS direction from the portal container in RTL', () => {
    withRTLPortal(() => {
      render(
        <Tooltip open>
          <TooltipTrigger>?</TooltipTrigger>
          <TooltipContent>Help</TooltipContent>
        </Tooltip>
      );
      expect(getComputedStyle(content()).direction).toBe('rtl');
    });
  });
});
