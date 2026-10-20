// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  DialogClose,
} from '../../src/components/kit/dialog';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from '../support/kit-helpers';
import { withRTLPortal, trackCommands } from '../support/kit-layer-helpers';

const Controlled = (props: {
  onEscapeKeyDown?: (e: KeyboardEvent) => void;
  onOpenChange?: (open: boolean) => void;
  children?: React.ReactNode;
}) => {
  const [open, setOpen] = React.useState(true);
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        props.onOpenChange?.(o);
        setOpen(o);
      }}
    >
      <DialogTrigger>Open</DialogTrigger>
      <DialogContent
        className="custom"
        aria-describedby={undefined}
        onEscapeKeyDown={props.onEscapeKeyDown}
      >
        <DialogHeader>
          <DialogTitle>Add account</DialogTitle>
        </DialogHeader>
        <input aria-label="Email" />
        {props.children}
        <DialogFooter>
          <DialogClose>Done</DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

describe('kit Dialog', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('portals into the UI portal root with the kit markup and legacy classes', () => {
    const errors = spyOn(console, 'error').andCallThrough();
    render(<Controlled />);
    const portal = getPortalContainer();
    const content = portal.querySelector('[data-slot="dialog-content"]');
    expect(content).not.toBe(null);
    expect(content.getAttribute('role')).toBe('dialog');
    expect(content.getAttribute('data-state')).toBe('open');
    expect(content.hasAttribute('data-ui-layer')).toBe(true);
    expect(content.className).toBe('modal custom');
    const overlay = portal.querySelector('[data-slot="dialog-overlay"]');
    expect(overlay.className).toBe('modal-container');
    expect(overlay.getAttribute('data-state')).toBe('open');
    const title = portal.querySelector('[data-slot="dialog-title"]');
    expect(title.tagName).toBe('H2');
    expect(content.getAttribute('aria-labelledby')).toBe(title.id);
    expect(portal.querySelector('[data-slot="dialog-header"]').contains(title)).toBe(true);
    expect(portal.querySelector('[data-slot="dialog-footer"]')).not.toBe(null);
    const close = portal.querySelector('button.modal-close');
    expect(close.getAttribute('data-slot')).toBe('dialog-close');
    expect(close.getAttribute('aria-label')).toBe('Close');
    expect(errors).not.toHaveBeenCalled();
  });

  it('renders a visually hidden title and a description', () => {
    const errors = spyOn(console, 'error').andCallThrough();
    render(
      <Dialog defaultOpen>
        <DialogContent hiddenTitle="Preferences" showCloseButton={false}>
          <DialogDescription>Choose a theme</DialogDescription>
        </DialogContent>
      </Dialog>
    );
    const portal = getPortalContainer();
    const content = portal.querySelector('[data-slot="dialog-content"]');
    const title = document.getElementById(content.getAttribute('aria-labelledby'));
    expect(title.textContent).toBe('Preferences');
    expect(getComputedStyle(title.parentElement).position).toBe('absolute');
    const description = portal.querySelector('[data-slot="dialog-description"]');
    expect(description.tagName).toBe('P');
    expect(content.getAttribute('aria-describedby')).toBe(description.id);
    expect(portal.querySelector('[data-slot="dialog-close"]')).toBe(null);
    expect(errors).not.toHaveBeenCalled();
  });

  it('closes from its close button and returns focus to the trigger', async () => {
    const onOpenChange = jasmine.createSpy('onOpenChange');
    const { container } = render(
      <Dialog onOpenChange={onOpenChange}>
        <DialogTrigger>Open</DialogTrigger>
        <DialogContent hiddenTitle="Dialog" aria-describedby={undefined}>
          Body
        </DialogContent>
      </Dialog>
    );
    const trigger = container.querySelector('[data-slot="dialog-trigger"]') as HTMLElement;
    expect(trigger.getAttribute('data-state')).toBe('closed');
    trigger.focus();
    fireEvent.click(trigger);
    expect(trigger.getAttribute('data-state')).toBe('open');
    await wait(20);
    const portal = getPortalContainer();
    expect(portal.contains(document.activeElement)).toBe(true);
    fireEvent.click(portal.querySelector('button.modal-close'));
    expect(onOpenChange.calls.map((c) => c.args[0])).toEqual([true, false]);
    await wait(300);
    expect(portal.querySelector('[data-slot="dialog-content"]')).toBe(null);
    expect(document.activeElement).toBe(trigger);
  });

  describe('Escape', function () {
    const commands = trackCommands();

    it('closes the dialog without popping the sheet; the next Escape pops it', async () => {
      await wait(250);
      const onOpenChange = jasmine.createSpy('onOpenChange');
      render(<Controlled onOpenChange={onOpenChange} />);
      await wait(50);
      expect(getPortalContainer().contains(document.activeElement)).toBe(true);
      await pressRealKey('Escape');
      expect(onOpenChange).toHaveBeenCalledWith(false);
      expect(commands.count('core:pop-sheet')).toBe(0);
      await wait(300);
      expect(getPortalContainer().querySelector('[data-slot="dialog-content"]')).toBe(null);
      await pressRealKey('Escape');
      expect(commands.count('core:pop-sheet')).toBeGreaterThan(0);
    });

    it('stays open without popping the sheet when the caller prevents the dismiss', async () => {
      await wait(250);
      const onEscape = jasmine.createSpy('onEscape').andCallFake((e) => e.preventDefault());
      render(<Controlled onEscapeKeyDown={onEscape} />);
      await wait(50);
      await pressRealKey('Escape');
      expect(onEscape).toHaveBeenCalled();
      expect(commands.count('core:pop-sheet')).toBe(0);
      const content = getPortalContainer().querySelector('[data-slot="dialog-content"]');
      expect(content.getAttribute('data-state')).toBe('open');
    });

    it('keeps plain keys pressed inside it from firing global commands', async () => {
      await wait(250);
      render(<Controlled />);
      await wait(50);
      (getPortalContainer().querySelector('[data-slot="dialog-close"]') as HTMLElement).focus();
      await pressRealKey('Down');
      await pressRealKey('Backspace');
      expect(commands.count('core:next-item')).toBe(0);
      expect(commands.count('core:remove-from-view')).toBe(0);
    });
  });

  it('takes its CSS direction from the portal container in RTL', () => {
    withRTLPortal(() => {
      render(<Controlled />);
      const content = getPortalContainer().querySelector(
        '[data-slot="dialog-content"]'
      ) as HTMLElement;
      expect(getPortalContainer().getAttribute('dir')).toBe('rtl');
      expect(content.hasAttribute('dir')).toBe(false);
      expect(getComputedStyle(content).direction).toBe('rtl');
      // The close button sits at the inline end, which is the left in RTL.
      const close = content.querySelector('[data-slot="dialog-close"]') as HTMLElement;
      const box = content.getBoundingClientRect();
      const btn = close.getBoundingClientRect();
      expect(btn.left - box.left).toBeLessThan(box.right - btn.right);
    });
  });
});
