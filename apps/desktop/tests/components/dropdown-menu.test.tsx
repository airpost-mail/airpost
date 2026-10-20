// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import DropdownMenu from '../../src/components/dropdown-menu';
import { wait, pressRealKey } from '../support/kit-helpers';
import { trackCommands } from '../support/kit-layer-helpers';

const ITEMS = [
  { id: 'week', name: 'Last week' },
  { id: 'month', name: 'Last month' },
];

const renderMenu = (props: any = {}) =>
  render(
    <DropdownMenu
      className="timespan"
      intitialSelectionItem={ITEMS[0]}
      items={ITEMS}
      itemKey={(item) => item.id}
      itemContent={(item) => item.name}
      onSelect={() => {}}
      {...props}
    />
  ).container;

describe('legacy DropdownMenu on the UI kit', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('opens its Menu as kit popover content inside the component', () => {
    const container = renderMenu({ attachment: DropdownMenu.Attachment.RightEdge });
    const root = container.querySelector('.btn.dropdown-menu.timespan');
    expect(root.querySelector('.menu')).toBe(null);
    fireEvent.click(root.firstElementChild);
    const layer = root.querySelector<HTMLElement>('[data-slot="popover-content"]');
    expect(layer.getAttribute('data-align')).toBe('end');
    expect(Array.from(layer.querySelectorAll('.menu .item')).map((i) => i.textContent)).toEqual([
      'Last week',
      'Last month',
    ]);
  });

  it('shows the pick, calls onSelect and closes', () => {
    const onSelect = jasmine.createSpy('onSelect');
    const container = renderMenu({ onSelect });
    const root = container.querySelector('.dropdown-menu');
    expect(root.textContent).toBe('Last week');
    fireEvent.click(root.firstElementChild);
    fireEvent.mouseDown(root.querySelectorAll('.menu .item')[1]);
    expect(onSelect).toHaveBeenCalledWith(ITEMS[1]);
    expect(root.querySelector('.menu')).toBe(null);
    expect(root.textContent).toBe('Last month');
  });

  it("focuses the menu's search field when it has one", async () => {
    const container = renderMenu({ headerComponents: [<input key="s" className="search" />] });
    fireEvent.click(container.querySelector('.dropdown-menu').firstElementChild);
    await wait(10);
    expect(document.activeElement).toBe(container.querySelector('input.search'));
  });

  it('closes on a click outside', async () => {
    const container = renderMenu();
    fireEvent.click(container.querySelector('.dropdown-menu').firstElementChild);
    await wait(10);
    fireEvent.pointerDown(document.body);
    fireEvent.click(document.body);
    expect(container.querySelector('.menu')).toBe(null);
  });

  describe('Escape', function () {
    const commands = trackCommands();

    it('closes the menu without popping the sheet', async () => {
      const container = renderMenu({ headerComponents: [<input key="s" className="search" />] });
      fireEvent.click(container.querySelector('.dropdown-menu').firstElementChild);
      await wait(10);
      expect(document.activeElement).toBe(container.querySelector('input.search'));
      await pressRealKey('Escape');
      expect(container.querySelector('.menu')).toBe(null);
      expect(commands.count('core:pop-sheet')).toBe(0);
    });
  });
});
