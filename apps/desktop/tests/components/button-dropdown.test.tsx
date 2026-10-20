// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { ButtonDropdown } from '../../src/components/button-dropdown';
import { Menu } from '../../src/components/menu';
import { wait, pressRealKey, watchPlainKeyCommands } from '../support/kit-helpers';
import { trackCommands } from '../support/kit-layer-helpers';

const MenuBody = (props: { onPick?: () => void }) => (
  <div className="menu">
    <div className="item" onMouseDown={props.onPick}>
      Later
    </div>
  </div>
);

describe('ButtonDropdown on the UI kit', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  const split = (props: any = {}) =>
    render(
      <ButtonDropdown
        primaryItem={<span>Send</span>}
        primaryTitle="Send"
        primaryClick={props.primaryClick || (() => {})}
        menu={<MenuBody onPick={props.onPick} />}
        {...props}
      />
    ).container;

  it('opens the menu as kit popover content inside the component, right-aligned', () => {
    const container = split();
    const root = container.querySelector('.button-dropdown');
    expect(root.querySelector('.secondary-items')).toBe(null);
    fireEvent.click(root.querySelector('.secondary-picker'));
    expect(root.className).toContain('open open-down');
    const menu = root.querySelector<HTMLElement>('.secondary-items');
    expect(menu.getAttribute('data-slot')).toBe('popover-content');
    expect(menu.getAttribute('role')).toBe('menu');
    expect(menu.getAttribute('data-variant')).toBe('plain');
    expect(menu.getAttribute('data-align')).toBe('end');
    expect(menu.querySelector('.menu .item').textContent).toBe('Later');
    expect(root.querySelector('.secondary-picker').getAttribute('aria-expanded')).toBe('true');
  });

  it('toggles from the picker and leaves the primary item alone', async () => {
    const primaryClick = jasmine.createSpy('primaryClick');
    const container = split({ primaryClick });
    const picker = container.querySelector('.secondary-picker');
    fireEvent.click(picker);
    await wait(10);
    fireEvent.pointerDown(picker);
    fireEvent.click(picker);
    expect(container.querySelector('.secondary-items')).toBe(null);
    fireEvent.click(container.querySelector('.primary-item'));
    expect(primaryClick).toHaveBeenCalled();
    expect(container.querySelector('.secondary-items')).toBe(null);
  });

  it('closes on a pick when closeOnMenuClick is set, and stays open otherwise', () => {
    const onPick = jasmine.createSpy('onPick');
    let container = split({ onPick });
    fireEvent.click(container.querySelector('.secondary-picker'));
    fireEvent.mouseDown(container.querySelector('.secondary-items .item'));
    expect(onPick).toHaveBeenCalled();
    expect(container.querySelector('.secondary-items')).not.toBe(null);
    cleanup();

    container = split({ onPick, closeOnMenuClick: true });
    fireEvent.click(container.querySelector('.secondary-picker'));
    fireEvent.mouseDown(container.querySelector('.secondary-items .item'));
    expect(container.querySelector('.secondary-items')).toBe(null);
  });

  it('closes on a click outside', async () => {
    const container = split();
    fireEvent.click(container.querySelector('.secondary-picker'));
    await wait(10);
    fireEvent.pointerDown(document.body);
    fireEvent.click(document.body);
    expect(container.querySelector('.secondary-items')).toBe(null);
  });

  it('aligns the one-item variant to the start, with the attachment class', () => {
    const { container } = render(
      <ButtonDropdown primaryItem={<span>Signature</span>} menu={<MenuBody />} />
    );
    fireEvent.click(container.querySelector('.only-item'));
    const menu = container.querySelector('.secondary-items');
    expect(menu.className).toContain('secondary-items left');
    expect(menu.getAttribute('data-align')).toBe('start');
  });

  describe('keys on the picker while the menu is open', function () {
    const withMenu = (onSelect: (item: string) => void, closeOnMenuClick = true) =>
      render(
        <ButtonDropdown
          primaryItem={<span>Send</span>}
          primaryTitle="Send"
          primaryClick={() => {}}
          closeOnMenuClick={closeOnMenuClick}
          menu={
            <Menu
              items={['Later', 'Archive', 'Snooze']}
              itemKey={(item) => item}
              itemContent={(item) => item}
              onSelect={onSelect}
            />
          }
        />
      ).container;
    const selected = (container: Element) =>
      container.querySelector('.secondary-items .item.selected')?.textContent;

    it('move the highlight with the arrows, outside the keymap, and pick with Enter', async () => {
      const commands = watchPlainKeyCommands();
      try {
        const onSelect = jasmine.createSpy('onSelect');
        const container = withMenu(onSelect);
        const picker = container.querySelector<HTMLElement>('.secondary-picker');
        picker.focus();
        fireEvent.click(picker);
        await wait(10);
        expect(selected(container)).toBe('Later');
        await pressRealKey('Down');
        expect(selected(container)).toBe('Archive');
        await pressRealKey('Down');
        await pressRealKey('Up');
        expect(selected(container)).toBe('Archive');
        expect(commands.counts['core:next-item']).toBe(0);
        expect(commands.counts['core:previous-item']).toBe(0);
        expect(document.activeElement).toBe(picker);
        await pressRealKey('Enter');
        expect(onSelect).toHaveBeenCalledWith('Archive');
        expect(container.querySelector('.secondary-items')).toBe(null);
      } finally {
        commands.dispose();
      }
    });

    it('leave the menu open after Enter without closeOnMenuClick, and open it when closed', async () => {
      const onSelect = jasmine.createSpy('onSelect');
      const container = withMenu(onSelect, false);
      const picker = container.querySelector<HTMLElement>('.secondary-picker');
      picker.focus();
      await pressRealKey('Enter');
      await wait(10);
      expect(selected(container)).toBe('Later');
      expect(onSelect).not.toHaveBeenCalled();
      await pressRealKey('Enter');
      expect(onSelect).toHaveBeenCalledWith('Later');
      expect(container.querySelector('.secondary-items')).not.toBe(null);
    });
  });

  describe('Escape', function () {
    const commands = trackCommands();

    it('closes the menu without popping the sheet', async () => {
      const container = split();
      const picker = container.querySelector<HTMLElement>('.secondary-picker');
      picker.focus();
      fireEvent.click(picker);
      await wait(10);
      await pressRealKey('Escape');
      expect(container.querySelector('.secondary-items')).toBe(null);
      expect(commands.count('core:pop-sheet')).toBe(0);
    });
  });
});
