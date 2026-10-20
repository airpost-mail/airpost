// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Checkbox } from '../../../src/components/kit/checkbox';
import { pressRealKey, watchPlainKeyCommands } from './kit-spec-helpers';

describe('kit Checkbox', function () {
  afterEach(() => cleanup());

  it('renders a checkbox button with data-slot and data-state, and no legacy classes', () => {
    const { getByRole } = render(<Checkbox aria-label="All" className="mine" />);
    const root = getByRole('checkbox');
    expect(root.tagName).toBe('BUTTON');
    expect(root.getAttribute('type')).toBe('button');
    expect(root.getAttribute('data-slot')).toBe('checkbox');
    expect(root.getAttribute('data-state')).toBe('unchecked');
    expect(root.getAttribute('aria-checked')).toBe('false');
    expect(root.className).toBe('mine');
    expect(root.querySelector('[data-slot="checkbox-indicator"]')).toBe(null);
  });

  it('toggles on click and shows the check glyph', () => {
    const onCheckedChange = jasmine.createSpy('onCheckedChange');
    const { getByRole } = render(<Checkbox aria-label="A" onCheckedChange={onCheckedChange} />);
    const root = getByRole('checkbox');
    fireEvent.click(root);
    expect(onCheckedChange).toHaveBeenCalledWith(true);
    expect(root.getAttribute('data-state')).toBe('checked');
    const indicator = root.querySelector('[data-slot="checkbox-indicator"]');
    expect(indicator.getAttribute('data-state')).toBe('checked');
    expect(indicator.querySelector('path').getAttribute('d')).toBe('M3.5 8.5l3 3 6-7');
  });

  it('shows the dash glyph when indeterminate', () => {
    const { getByRole } = render(<Checkbox aria-label="A" checked="indeterminate" />);
    const root = getByRole('checkbox');
    expect(root.getAttribute('aria-checked')).toBe('mixed');
    expect(root.getAttribute('data-state')).toBe('indeterminate');
    expect(root.querySelector('path').getAttribute('d')).toBe('M4 8h8');
  });

  it('toggles on Space but not on Enter, and keeps both out of the keymap', async () => {
    const commands = watchPlainKeyCommands();
    try {
      const onCheckedChange = jasmine.createSpy('onCheckedChange');
      const { getByRole } = render(<Checkbox aria-label="A" onCheckedChange={onCheckedChange} />);
      const root = getByRole('checkbox');
      root.focus();
      await pressRealKey('Enter');
      expect(onCheckedChange).not.toHaveBeenCalled();
      await pressRealKey('Space');
      expect(onCheckedChange).toHaveBeenCalledWith(true);
      expect(commands.total()).toBe(0);
    } finally {
      commands.dispose();
    }
  });

  it('is labelled by a <label htmlFor>', () => {
    const onCheckedChange = jasmine.createSpy('onCheckedChange');
    const { getByText, getByLabelText } = render(
      <>
        <Checkbox id="cb-remember" onCheckedChange={onCheckedChange} />
        <label htmlFor="cb-remember">Remember</label>
      </>
    );
    expect(getByLabelText('Remember').getAttribute('data-slot')).toBe('checkbox');
    fireEvent.click(getByText('Remember'));
    expect(onCheckedChange).toHaveBeenCalledWith(true);
  });

  it('does nothing when disabled', () => {
    const onCheckedChange = jasmine.createSpy('onCheckedChange');
    const { getByRole } = render(
      <Checkbox aria-label="A" disabled onCheckedChange={onCheckedChange} />
    );
    const root = getByRole('checkbox');
    expect(root.hasAttribute('data-disabled')).toBe(true);
    fireEvent.click(root);
    expect(onCheckedChange).not.toHaveBeenCalled();
  });

  it('renders the same in RTL', () => {
    const { getByRole } = render(
      <div dir="rtl">
        <Checkbox aria-label="A" defaultChecked />
      </div>
    );
    const root = getByRole('checkbox');
    expect(getComputedStyle(root).direction).toBe('rtl');
    expect(root.getAttribute('data-state')).toBe('checked');
  });
});
