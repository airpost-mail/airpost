// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { DirectionProvider } from '@radix-ui/react-direction';
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
} from '../../src/components/kit/select';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from '../support/kit-helpers';
import { withRTLPortal, trackCommands } from '../support/kit-layer-helpers';

const q = (selector: string) => getPortalContainer().querySelector(selector) as HTMLElement;
const qa = (selector: string) =>
  Array.from(getPortalContainer().querySelectorAll(selector)) as HTMLElement[];

const Layout = (props: {
  value?: string;
  onValueChange?: (v: string) => void;
  position?: 'item-aligned' | 'popper';
  size?: 'sm' | 'default';
  defaultOpen?: boolean;
}) => (
  <Select value={props.value} onValueChange={props.onValueChange} defaultOpen={props.defaultOpen}>
    <SelectTrigger aria-label="Layout" size={props.size} className="custom">
      <SelectValue placeholder="Choose a layout" />
    </SelectTrigger>
    <SelectContent position={props.position}>
      <SelectGroup>
        <SelectLabel>Layouts</SelectLabel>
        <SelectItem value="list">List</SelectItem>
        <SelectItem value="split">Split</SelectItem>
        <SelectItem value="wide" disabled>
          Wide
        </SelectItem>
      </SelectGroup>
      <SelectSeparator />
      <SelectItem value="auto">Automatic</SelectItem>
    </SelectContent>
  </Select>
);

describe('kit Select', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  it('renders the trigger markup, with a placeholder when no value is set', () => {
    const { container, rerender } = render(<Layout value="" size="sm" />);
    const trigger = container.querySelector('[data-slot="select-trigger"]');
    expect(trigger.tagName).toBe('BUTTON');
    expect(trigger.getAttribute('type')).toBe('button');
    expect(trigger.getAttribute('role')).toBe('combobox');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(trigger.getAttribute('data-state')).toBe('closed');
    expect(trigger.getAttribute('data-size')).toBe('sm');
    expect(trigger.className).toBe('custom');
    expect(trigger.hasAttribute('data-placeholder')).toBe(true);
    expect(trigger.querySelector('[data-slot="select-value"]').textContent).toBe('Choose a layout');
    expect(trigger.querySelector('[data-slot="select-icon"]').getAttribute('aria-hidden')).toBe(
      'true'
    );
    rerender(<Layout value="split" />);
    expect(trigger.hasAttribute('data-placeholder')).toBe(false);
    expect(trigger.getAttribute('data-size')).toBe('default');
    expect(trigger.querySelector('[data-slot="select-value"]').textContent).toBe('Split');
  });

  it('opens into the portal root with the listbox markup', () => {
    const { container } = render(<Layout value="split" />);
    const trigger = container.querySelector('[data-slot="select-trigger"]');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(trigger.getAttribute('data-state')).toBe('open');
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    const content = q('[data-slot="select-content"]');
    expect(content.getAttribute('role')).toBe('listbox');
    expect(content.getAttribute('data-state')).toBe('open');
    expect(content.hasAttribute('data-ui-layer')).toBe(true);
    expect(trigger.getAttribute('aria-controls')).toBe(content.id);
    const items = qa('[data-slot="select-item"]');
    expect(items.map((i) => i.getAttribute('role'))).toEqual([
      'option',
      'option',
      'option',
      'option',
    ]);
    expect(items.map((i) => i.getAttribute('data-state'))).toEqual([
      'unchecked',
      'checked',
      'unchecked',
      'unchecked',
    ]);
    expect(items[1].getAttribute('aria-selected')).toBe('true');
    expect(items[2].hasAttribute('data-disabled')).toBe(true);
    expect(items[1].querySelector('[data-slot="select-item-text"]').textContent).toBe('Split');
    expect(items[1].querySelector('[data-slot="select-item-indicator"] svg')).not.toBe(null);
    expect(items[0].querySelector('[data-slot="select-item-indicator"] svg')).toBe(null);
    expect(q('[data-slot="select-label"]').textContent).toBe('Layouts');
    expect(q('[data-slot="select-separator"]')).not.toBe(null);
    // Item-aligned (the macOS popup's placement) is the default, so there's no data-side.
    expect(content.hasAttribute('data-side')).toBe(false);
  });

  it('positions as a popper when asked', () => {
    render(<Layout value="list" position="popper" defaultOpen />);
    const content = q('[data-slot="select-content"]');
    expect(content.getAttribute('data-side')).toBe('bottom');
    expect(content.getAttribute('data-align')).toBe('start');
  });

  it('picks a value with the arrow keys and Enter, skipping disabled items', async () => {
    const onValueChange = jasmine.createSpy('onValueChange');
    const { container } = render(<Layout value="split" onValueChange={onValueChange} />);
    const trigger = container.querySelector('[data-slot="select-trigger"]');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    await wait(50);
    const items = qa('[data-slot="select-item"]');
    // Focus starts on the chosen item.
    expect(document.activeElement).toBe(items[1]);
    expect(items[1].hasAttribute('data-highlighted')).toBe(true);
    fireEvent.keyDown(items[1], { key: 'ArrowDown' });
    await wait(20);
    expect(document.activeElement).toBe(items[3]);
    expect(items[3].hasAttribute('data-highlighted')).toBe(true);
    fireEvent.keyDown(items[3], { key: 'Enter' });
    expect(onValueChange).toHaveBeenCalledWith('auto');
    expect(trigger.getAttribute('data-state')).toBe('closed');
  });

  describe('keys and the keymap', function () {
    const commands = trackCommands();

    it('closes on Escape without popping the sheet, and keeps plain keys local', async () => {
      await wait(250);
      const onValueChange = jasmine.createSpy('onValueChange');
      render(<Layout value="list" defaultOpen onValueChange={onValueChange} />);
      await wait(50);
      expect(getPortalContainer().contains(document.activeElement)).toBe(true);
      await pressRealKey('Down');
      await pressRealKey('Backspace');
      expect(commands.count('core:next-item')).toBe(0);
      expect(commands.count('core:remove-from-view')).toBe(0);
      await pressRealKey('Escape');
      expect(commands.count('core:pop-sheet')).toBe(0);
      expect(onValueChange).not.toHaveBeenCalled();
      await wait(300);
      expect(q('[data-slot="select-content"]')).toBe(null);
      await pressRealKey('Escape');
      expect(commands.count('core:pop-sheet')).toBeGreaterThan(0);
    });

    it('keeps the keys a closed trigger acts on, as a native select did', async () => {
      await wait(250);
      const { container } = render(<Layout value="list" />);
      const trigger = container.querySelector<HTMLElement>('[data-slot="select-trigger"]');
      trigger.focus();
      await pressRealKey('Down');
      expect(commands.count('core:next-item')).toBe(0);
      expect(trigger.getAttribute('data-state')).toBe('open');
      await pressRealKey('Escape');
      await wait(300);
      expect(document.activeElement).toBe(trigger);
      await pressRealKey('Space');
      expect(commands.count('core:messages-page-down')).toBe(0);
      expect(trigger.getAttribute('data-state')).toBe('open');
    });
  });

  it('takes the direction from the provider in RTL', () => {
    withRTLPortal(() => {
      render(
        <DirectionProvider dir="rtl">
          <Layout value="list" defaultOpen />
        </DirectionProvider>
      );
      const content = q('[data-slot="select-content"]');
      expect(content.getAttribute('dir')).toBe('rtl');
      expect(getComputedStyle(content).direction).toBe('rtl');
    });
  });
});
