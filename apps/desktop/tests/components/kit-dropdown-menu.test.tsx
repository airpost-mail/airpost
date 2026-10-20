// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuCheckboxItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubTrigger,
  DropdownMenuSubContent,
} from '../../src/components/kit/dropdown-menu';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from '../support/kit-helpers';
import { withRTLPortal, trackCommands } from '../support/kit-layer-helpers';

const q = (selector: string) => getPortalContainer().querySelector(selector) as HTMLElement;
const qa = (selector: string) =>
  Array.from(getPortalContainer().querySelectorAll(selector)) as HTMLElement[];

const Menu = (props: {
  onArchive?: () => void;
  onOpenChange?: (open: boolean) => void;
  defaultOpen?: boolean;
}) => {
  const [starred, setStarred] = React.useState(true);
  const [sort, setSort] = React.useState('date');
  return (
    <DropdownMenu defaultOpen={props.defaultOpen} onOpenChange={props.onOpenChange}>
      <DropdownMenuTrigger>More</DropdownMenuTrigger>
      <DropdownMenuContent className="custom">
        <DropdownMenuLabel>Thread</DropdownMenuLabel>
        <DropdownMenuGroup>
          <DropdownMenuItem onSelect={props.onArchive}>
            Archive<DropdownMenuShortcut>E</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem variant="destructive" inset>
            Delete
          </DropdownMenuItem>
          <DropdownMenuItem disabled>Unavailable</DropdownMenuItem>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuCheckboxItem checked={starred} onCheckedChange={setStarred}>
          Starred
        </DropdownMenuCheckboxItem>
        <DropdownMenuRadioGroup value={sort} onValueChange={setSort}>
          <DropdownMenuRadioItem value="date">Date</DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="sender">Sender</DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>Move to</DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            <DropdownMenuItem>Inbox</DropdownMenuItem>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
      </DropdownMenuContent>
    </DropdownMenu>
  );
};

describe('kit DropdownMenu', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('renders the kit markup, state attributes and legacy classes', () => {
    const { container } = render(<Menu />);
    const trigger = container.querySelector('[data-slot="dropdown-menu-trigger"]');
    expect(trigger.getAttribute('aria-haspopup')).toBe('menu');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(trigger.getAttribute('data-state')).toBe('open');

    const content = q('[data-slot="dropdown-menu-content"]');
    expect(content.getAttribute('role')).toBe('menu');
    expect(content.getAttribute('aria-orientation')).toBe('vertical');
    expect(content.getAttribute('data-state')).toBe('open');
    expect(content.getAttribute('data-side')).toBe('bottom');
    expect(content.hasAttribute('data-ui-layer')).toBe(true);
    expect(content.className).toBe('menu custom');

    const items = qa('[data-slot="dropdown-menu-item"]');
    expect(items.map((i) => i.getAttribute('role'))).toEqual(['menuitem', 'menuitem', 'menuitem']);
    expect(items.every((i) => i.classList.contains('item'))).toBe(true);
    expect(items[0].getAttribute('data-variant')).toBe('default');
    expect(items[1].getAttribute('data-variant')).toBe('destructive');
    expect(items[1].hasAttribute('data-inset')).toBe(true);
    expect(items[0].hasAttribute('data-inset')).toBe(false);
    expect(items[2].hasAttribute('data-disabled')).toBe(true);
    expect(items[0].querySelector('[data-slot="dropdown-menu-shortcut"]').textContent).toBe('E');

    expect(q('[data-slot="dropdown-menu-label"]').textContent).toBe('Thread');
    expect(q('[data-slot="dropdown-menu-group"]').getAttribute('role')).toBe('group');
    const separator = q('[data-slot="dropdown-menu-separator"]');
    expect(separator.getAttribute('role')).toBe('separator');
    expect(separator.className).toBe('item divider');

    const checkbox = q('[data-slot="dropdown-menu-checkbox-item"]');
    expect(checkbox.getAttribute('role')).toBe('menuitemcheckbox');
    expect(checkbox.getAttribute('data-state')).toBe('checked');
    expect(checkbox.querySelector('[data-slot="dropdown-menu-item-indicator"] svg')).not.toBe(null);
    const radios = qa('[data-slot="dropdown-menu-radio-item"]');
    expect(radios.map((r) => r.getAttribute('role'))).toEqual(['menuitemradio', 'menuitemradio']);
    expect(radios.map((r) => r.getAttribute('data-state'))).toEqual(['checked', 'unchecked']);
    const sub = q('[data-slot="dropdown-menu-sub-trigger"]');
    expect(sub.getAttribute('data-state')).toBe('closed');
    expect(sub.getAttribute('aria-haspopup')).toBe('menu');
  });

  it('moves the highlight with the arrow keys and tracks the legacy selected class', async () => {
    const { container } = render(<Menu />);
    fireEvent.keyDown(container.querySelector('[data-slot="dropdown-menu-trigger"]'), {
      key: 'Enter',
    });
    await wait(20);
    const [archive, del] = qa('[data-slot="dropdown-menu-item"]');
    // Opening from the keyboard highlights the first item.
    expect(document.activeElement).toBe(archive);
    expect(archive.hasAttribute('data-highlighted')).toBe(true);
    expect(archive.classList.contains('selected')).toBe(true);
    fireEvent.keyDown(archive, { key: 'ArrowDown' });
    await wait(20);
    expect(document.activeElement).toBe(del);
    expect(del.hasAttribute('data-highlighted')).toBe(true);
    expect(del.classList.contains('selected')).toBe(true);
    expect(archive.hasAttribute('data-highlighted')).toBe(false);
    expect(archive.classList.contains('selected')).toBe(false);
  });

  it('selects an item and closes; checkbox and radio items update their state', async () => {
    const onArchive = jasmine.createSpy('onArchive');
    const { container } = render(<Menu onArchive={onArchive} />);
    const trigger = container.querySelector('[data-slot="dropdown-menu-trigger"]');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(q('[data-slot="dropdown-menu-checkbox-item"]'));
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(q('[data-slot="dropdown-menu-checkbox-item"]').getAttribute('data-state')).toBe(
      'unchecked'
    );
    fireEvent.click(qa('[data-slot="dropdown-menu-radio-item"]')[1]);
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const radios = qa('[data-slot="dropdown-menu-radio-item"]');
    expect(radios.map((r) => r.getAttribute('data-state'))).toEqual(['unchecked', 'checked']);
    fireEvent.click(qa('[data-slot="dropdown-menu-item"]')[0]);
    expect(onArchive).toHaveBeenCalled();
    expect(trigger.getAttribute('data-state')).toBe('closed');
  });

  it('opens a submenu with ArrowRight into its own layer', async () => {
    const { container } = render(<Menu />);
    fireEvent.keyDown(container.querySelector('[data-slot="dropdown-menu-trigger"]'), {
      key: 'Enter',
    });
    await wait(20);
    const sub = q('[data-slot="dropdown-menu-sub-trigger"]');
    sub.focus();
    fireEvent.keyDown(sub, { key: 'ArrowRight' });
    await wait(20);
    expect(sub.getAttribute('data-state')).toBe('open');
    const subContent = q('[data-slot="dropdown-menu-sub-content"]');
    expect(subContent.className).toBe('menu');
    expect(subContent.hasAttribute('data-ui-layer')).toBe(true);
    expect(subContent.getAttribute('data-side')).toBe('right');
  });

  describe('keys and the keymap', function () {
    const commands = trackCommands();

    it('keeps plain keys pressed in the open menu from firing global commands', async () => {
      await wait(250);
      render(<Menu defaultOpen />);
      await wait(50);
      expect(getPortalContainer().contains(document.activeElement)).toBe(true);
      await pressRealKey('Down');
      expect(document.activeElement.textContent).toBe('ArchiveE');
      await pressRealKey('Down');
      await pressRealKey('Backspace');
      expect(commands.count('core:next-item')).toBe(0);
      expect(commands.count('core:remove-from-view')).toBe(0);
    });

    it('closes on Escape without popping the sheet; the next Escape pops it', async () => {
      await wait(250);
      const onOpenChange = jasmine.createSpy('onOpenChange');
      render(<Menu defaultOpen onOpenChange={onOpenChange} />);
      await wait(50);
      await pressRealKey('Escape');
      expect(onOpenChange).toHaveBeenCalledWith(false);
      expect(commands.count('core:pop-sheet')).toBe(0);
      await wait(300);
      expect(q('[data-slot="dropdown-menu-content"]')).toBe(null);
      await pressRealKey('Escape');
      expect(commands.count('core:pop-sheet')).toBeGreaterThan(0);
    });
  });

  it('mirrors in RTL: the dir comes from the provider and submenus open to the left', async () => {
    const { DirectionProvider } = require('@radix-ui/react-direction');
    withRTLPortal(() => {
      render(
        <DirectionProvider dir="rtl">
          <Menu defaultOpen />
        </DirectionProvider>
      );
      const content = q('[data-slot="dropdown-menu-content"]');
      expect(content.getAttribute('dir')).toBe('rtl');
      expect(getComputedStyle(content).direction).toBe('rtl');
      const sub = q('[data-slot="dropdown-menu-sub-trigger"]');
      fireEvent.keyDown(sub, { key: 'ArrowLeft' });
      expect(sub.getAttribute('data-state')).toBe('open');
      expect(q('[data-slot="dropdown-menu-sub-content"]').getAttribute('data-side')).toBe('left');
    });
  });
});
