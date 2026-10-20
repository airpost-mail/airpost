// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { ButtonDropdown } from '../../src/components/button-dropdown';
import { wait, pressRealKey } from '../support/kit-helpers';
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
