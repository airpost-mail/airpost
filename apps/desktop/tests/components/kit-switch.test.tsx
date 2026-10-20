// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Switch } from '../../src/components/kit/switch';
import { pressRealKey, wait, watchPlainKeyCommands } from '../support/kit-helpers';

describe('kit Switch', function () {
  afterEach(() => cleanup());

  it('renders a switch button with data-slot, data-state and the legacy classes', () => {
    const { getByRole } = render(<Switch label="Wrap" className="mine" />);
    const root = getByRole('switch');
    expect(root.tagName).toBe('BUTTON');
    expect(root.getAttribute('type')).toBe('button');
    expect(root.getAttribute('data-slot')).toBe('switch');
    expect(root.getAttribute('data-size')).toBe('default');
    expect(root.getAttribute('data-state')).toBe('unchecked');
    expect(root.getAttribute('aria-checked')).toBe('false');
    expect(root.getAttribute('aria-label')).toBe('Wrap');
    expect(root.className).toBe('slide-switch mine');
    const thumb = root.querySelector('[data-slot="switch-thumb"]');
    expect(thumb.className).toBe('handle');
    expect(thumb.getAttribute('data-state')).toBe('unchecked');
  });

  it('toggles uncontrolled, with onCheckedChange and the active class', () => {
    const onCheckedChange = jasmine.createSpy('onCheckedChange');
    const { getByRole } = render(<Switch label="A" onCheckedChange={onCheckedChange} />);
    const root = getByRole('switch');
    fireEvent.click(root);
    expect(onCheckedChange).toHaveBeenCalledWith(true);
    expect(root.getAttribute('data-state')).toBe('checked');
    expect(root.classList.contains('active')).toBe(true);
    fireEvent.click(root);
    expect(root.classList.contains('active')).toBe(false);
  });

  it('works like the legacy Switch: checked plus onChange(event), labelledBy', () => {
    const Legacy = () => {
      const [on, setOn] = React.useState(true);
      return <Switch checked={on} onChange={() => setOn(!on)} labelledBy="lbl" />;
    };
    const { getByRole } = render(<Legacy />);
    const root = getByRole('switch');
    expect(root.getAttribute('aria-labelledby')).toBe('lbl');
    expect(root.classList.contains('active')).toBe(true);
    fireEvent.click(root);
    expect(root.getAttribute('aria-checked')).toBe('false');
    expect(root.classList.contains('active')).toBe(false);
  });

  it('calls onChange with the event on Space and Enter, which stay out of the keymap', async () => {
    const commands = watchPlainKeyCommands();
    try {
      const onChange = jasmine.createSpy('onChange');
      const { getByRole } = render(<Switch checked={false} onChange={onChange} label="A" />);
      const root = getByRole('switch');
      root.focus();
      await pressRealKey('Space');
      await pressRealKey('Enter');
      expect(onChange.callCount).toBe(2);
      expect(onChange.argsForCall[0][0].type).toBe('click');
      expect(commands.counts['core:messages-page-down']).toBe(0);
      expect(commands.counts['core:focus-item']).toBe(0);
    } finally {
      commands.dispose();
    }
  });

  it('toggles uncontrolled from the keyboard', async () => {
    const { getByRole } = render(<Switch label="A" />);
    const root = getByRole('switch');
    root.focus();
    await pressRealKey('Space');
    expect(root.getAttribute('data-state')).toBe('checked');
    expect(root.classList.contains('active')).toBe(true);
  });

  it('does nothing when disabled', () => {
    const onChange = jasmine.createSpy('onChange');
    const { getByRole } = render(<Switch disabled onChange={onChange} label="A" />);
    const root = getByRole('switch');
    expect(root.hasAttribute('data-disabled')).toBe(true);
    fireEvent.click(root);
    expect(onChange).not.toHaveBeenCalled();
    expect(root.getAttribute('data-state')).toBe('unchecked');
  });

  it('is styled by the kit sheet, not .slide-switch', () => {
    const { getAllByRole } = render(
      <>
        <Switch label="A" />
        <Switch label="B" size="sm" />
      </>
    );
    const [normal, small] = getAllByRole('switch');
    expect(getComputedStyle(normal).width).toBe('32px');
    expect(getComputedStyle(normal).height).toBe('18px');
    expect(getComputedStyle(small).width).toBe('26px');
    expect(getComputedStyle(small).height).toBe('14px');
  });

  it('moves the thumb towards the end edge, mirrored in RTL', async () => {
    const { container } = render(
      <>
        <div dir="ltr">
          <Switch label="A" defaultChecked />
        </div>
        <div dir="rtl">
          <Switch label="B" defaultChecked />
        </div>
      </>
    );
    await wait(200);
    const [ltr, rtl] = Array.from(container.querySelectorAll('[data-slot="switch-thumb"]'));
    expect(getComputedStyle(ltr).transform).toBe('matrix(1, 0, 0, 1, 14, 0)');
    expect(getComputedStyle(rtl).transform).toBe('matrix(1, 0, 0, 1, -14, 0)');
  });
});
