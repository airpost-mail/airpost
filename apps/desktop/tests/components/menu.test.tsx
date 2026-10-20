// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Menu } from '../../src/components/menu';
import PopoverStore from '../../src/data/stores/popover';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from '../support/kit-helpers';
import { trackCommands } from '../support/kit-layer-helpers';

type Item = { id: string; name?: string; divider?: string | boolean };

const ITEMS: Item[] = [
  { id: 'h', divider: 'Folders' },
  { id: 'inbox', name: 'Inbox' },
  { id: 'archive', name: 'Archive' },
  { id: 'line', divider: true },
  { id: 'trash', name: 'Trash' },
];

const renderMenu = (props: any = {}) =>
  render(
    <Menu
      className="picker"
      items={ITEMS}
      itemKey={(item: Item) => item.id}
      itemContent={(item: Item) =>
        item.divider ? <Menu.Item key={item.id} divider={item.divider} /> : item.name
      }
      itemChecked={(item: Item) => item.id === 'archive'}
      defaultSelectedIndex={1}
      onSelect={() => {}}
      headerComponents={[<input key="search" className="search" />]}
      footerComponents={[<div key="f" className="footer-thing" />]}
      {...props}
    />
  ).container;

describe('Menu on the UI kit', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('renders the kit markup with the legacy classes', () => {
    const container = renderMenu();
    const root = container.querySelector<HTMLElement>('[data-slot="menu"]');
    expect(root.className).toBe('menu picker');
    expect(root.querySelector('[data-slot="menu-header"]').className).toBe('header-container');
    expect(root.querySelector('[data-slot="menu-header"] input.search')).not.toBe(null);
    expect(root.querySelector('[data-slot="menu-content"]').className).toBe('content-container');
    expect(root.querySelector('[data-slot="menu-footer"]').className).toBe('footer-container');
    expect(root.querySelector('.footer-container .footer-thing')).not.toBe(null);

    const items = Array.from(root.querySelectorAll<HTMLElement>('[data-slot="menu-item"]'));
    expect(items.map((i) => i.textContent)).toEqual(['Inbox', 'Archive', 'Trash']);
    expect(items.every((i) => i.classList.contains('item'))).toBe(true);
  });

  it('marks the selected item highlighted and the checked one with an indicator', () => {
    const container = renderMenu();
    const [inbox, archive, trash] = Array.from(
      container.querySelectorAll<HTMLElement>('[data-slot="menu-item"]')
    );
    expect(inbox.className).toBe('item selected');
    expect(inbox.hasAttribute('data-highlighted')).toBe(true);
    expect(archive.hasAttribute('data-highlighted')).toBe(false);
    expect(archive.className).toBe('item checked');
    expect(archive.getAttribute('data-state')).toBe('checked');
    expect(archive.querySelector('[data-slot="menu-item-indicator"] svg')).not.toBe(null);
    expect(trash.hasAttribute('data-state')).toBe(false);
    expect(trash.querySelector('[data-slot="menu-item-indicator"]')).toBe(null);
  });

  it('renders a divider with text as a label and one without as a separator', () => {
    const container = renderMenu();
    const label = container.querySelector('[data-slot="menu-label"]');
    expect(label.className).toBe('item divider');
    expect(label.textContent).toBe('Folders');
    const separator = container.querySelector('[data-slot="menu-separator"]');
    expect(separator.className).toBe('item divider');
    expect(separator.textContent).toBe('');
  });

  it('marks an empty list', () => {
    const container = renderMenu({ items: [] });
    expect(container.querySelector('[data-slot="menu-content"]').className).toBe(
      'content-container empty'
    );
  });

  it('moves the highlight with the arrow keys, skipping dividers, and picks with Enter', () => {
    const onSelect = jasmine.createSpy('onSelect');
    const container = renderMenu({ onSelect });
    const search = container.querySelector('input.search');
    const highlighted = () =>
      container.querySelector('[data-slot="menu-item"][data-highlighted]').textContent;

    fireEvent.keyDown(search, { key: 'ArrowDown' });
    expect(highlighted()).toBe('Archive');
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    expect(highlighted()).toBe('Trash');
    expect(container.querySelector('.item.selected').textContent).toBe('Trash');
    fireEvent.keyDown(search, { key: 'ArrowUp' });
    expect(highlighted()).toBe('Archive');
    fireEvent.keyDown(search, { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledWith(ITEMS[2]);
  });

  it('expands the highlighted item on Tab or ArrowRight when onExpand is given', () => {
    const onExpand = jasmine.createSpy('onExpand');
    const container = renderMenu({ onExpand });
    const search = container.querySelector('input.search');
    fireEvent.keyDown(search, { key: 'ArrowRight' });
    expect(onExpand).toHaveBeenCalledWith(ITEMS[1]);
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    fireEvent.keyDown(search, { key: 'Tab' });
    expect(onExpand.mostRecentCall.args).toEqual([ITEMS[2]]);
  });

  it('picks on mouse down', () => {
    const onSelect = jasmine.createSpy('onSelect');
    const container = renderMenu({ onSelect });
    fireEvent.mouseDown(container.querySelectorAll('[data-slot="menu-item"]')[2]);
    expect(onSelect).toHaveBeenCalledWith(ITEMS[4]);
  });

  describe('Escape', function () {
    const commands = trackCommands();

    afterEach(() => PopoverStore.closePopover());

    it('calls onEscape without popping the sheet', async () => {
      const onEscape = jasmine.createSpy('onEscape');
      const container = renderMenu({ onEscape, headerComponents: null });
      container.querySelector<HTMLElement>('[data-slot="menu"]').focus();
      await pressRealKey('Escape');
      expect(onEscape).toHaveBeenCalled();
      expect(commands.count('core:pop-sheet')).toBe(0);
    });

    it('closes the popover a picker shows it in, without popping the sheet', async () => {
      PopoverStore.openPopover(
        <Menu
          items={ITEMS.slice(1, 3)}
          itemKey={(item: Item) => item.id}
          itemContent={(item: Item) => item.name}
          onSelect={() => {}}
          headerComponents={[<input key="search" className="search" />]}
        />,
        { originRect: { top: 100, left: 100, width: 40, height: 20 }, direction: 'down' } as any
      );
      await wait(20);
      expect(document.activeElement).toBe(getPortalContainer().querySelector('input.search'));
      await pressRealKey('Escape');
      expect(PopoverStore.isOpen).toBe(false);
      expect(commands.count('core:pop-sheet')).toBe(0);
    });
  });
});
