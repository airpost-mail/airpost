// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import Switch from '../../src/components/switch';
import { SettingsSwitch } from '../../src/components/settings';

describe('legacy Switch on the UI kit', function () {
  afterEach(() => cleanup());

  it('renders the kit switch with the legacy classes and labels', () => {
    const { container, rerender } = render(
      <Switch className="custom" checked label="All day" onChange={() => {}} />
    );
    const sw = container.querySelector('[role="switch"]');
    expect(sw.tagName).toBe('BUTTON');
    expect(sw.getAttribute('data-slot')).toBe('switch');
    expect(sw.getAttribute('aria-checked')).toBe('true');
    expect(sw.getAttribute('aria-label')).toBe('All day');
    expect(sw.className).toBe('slide-switch active custom');
    expect(sw.querySelector('.handle').getAttribute('data-slot')).toBe('switch-thumb');
    rerender(<Switch checked={false} labelledBy="heading" onChange={() => {}} />);
    expect(sw.className).toBe('slide-switch');
    expect(sw.getAttribute('aria-labelledby')).toBe('heading');
  });

  it('calls onChange with the click and stays as the parent set it', () => {
    const onChange = jasmine.createSpy('onChange');
    const { container } = render(<Switch onChange={onChange} />);
    const sw = container.querySelector('[role="switch"]');
    fireEvent.click(sw);
    expect(onChange).toHaveBeenCalled();
    expect(onChange.calls[0].args[0].type).toBe('click');
    // No `checked`: off, as before the kit, rather than an uncontrolled toggle.
    expect(sw.getAttribute('aria-checked')).toBe('false');
  });
});

describe('SettingsSwitch on the UI kit', function () {
  afterEach(() => cleanup());

  it('reports the flipped value', () => {
    const onChange = jasmine.createSpy('onChange');
    const { container } = render(<SettingsSwitch checked onChange={onChange} label="Sync" />);
    const sw = container.querySelector('[role="switch"]');
    expect(sw.className).toBe('slide-switch active settings-switch');
    fireEvent.click(sw);
    expect(onChange).toHaveBeenCalledWith(false);
  });

  it('stays focusable when disabled and ignores the toggle', () => {
    const onChange = jasmine.createSpy('onChange');
    const { container } = render(
      <SettingsSwitch checked={false} disabled onChange={onChange} label="Sync" />
    );
    const sw = container.querySelector<HTMLButtonElement>('[role="switch"]');
    expect(sw.disabled).toBe(false);
    expect(sw.getAttribute('aria-disabled')).toBe('true');
    expect(sw.classList.contains('disabled')).toBe(true);
    fireEvent.click(sw);
    expect(onChange).not.toHaveBeenCalled();
  });
});
