// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { TextField } from '../../src/components/kit/text-field';
import { pressRealKey } from '../support/kit-helpers';

describe('kit TextField', function () {
  afterEach(() => cleanup());

  it('renders a text input with data-slot and data-size, and no legacy classes', () => {
    const { getByRole } = render(<TextField placeholder="Name" className="mine" />);
    const input = getByRole('textbox') as HTMLInputElement;
    expect(input.tagName).toBe('INPUT');
    expect(input.type).toBe('text');
    expect(input.getAttribute('data-slot')).toBe('input');
    expect(input.getAttribute('data-size')).toBe('default');
    expect(input.hasAttribute('aria-invalid')).toBe(false);
    expect(input.className).toBe('mine');
    expect(input.placeholder).toBe('Name');
  });

  it('keeps a given type, sets aria-invalid, and forwards ref and events', () => {
    const ref = React.createRef<HTMLInputElement>();
    const onChange = jasmine.createSpy('onChange');
    const { container } = render(
      <TextField type="email" size="sm" invalid ref={ref} defaultValue="" onChange={onChange} />
    );
    const input = container.querySelector('input');
    expect(ref.current).toBe(input);
    expect(input.type).toBe('email');
    expect(input.getAttribute('data-size')).toBe('sm');
    expect(input.getAttribute('aria-invalid')).toBe('true');
    fireEvent.change(input, { target: { value: 'a@b.c' } });
    expect(onChange).toHaveBeenCalled();
  });

  it('keeps plain keys from the keymap, as a text input', async () => {
    let nextItem = 0;
    const handlers = AppEnv.commands.add(document.body, {
      'core:next-item': () => (nextItem += 1),
    });
    try {
      const { getByRole } = render(<TextField defaultValue="" />);
      const input = getByRole('textbox') as HTMLInputElement;
      input.focus();
      expect(document.activeElement).toBe(input);
      await pressRealKey('Down');
      expect(nextItem).toBe(0);
    } finally {
      handlers.dispose();
    }
  });

  it('is styled by the kit sheet; the legacy input rules are guarded', () => {
    const { getByRole } = render(<TextField />);
    const style = getComputedStyle(getByRole('textbox'));
    expect(style.height).toBe('26px');
    expect(style.paddingInlineStart).toBe(style.paddingInlineEnd);
  });

  it('follows the direction it is in', () => {
    const { getByRole } = render(
      <div dir="rtl">
        <TextField />
      </div>
    );
    expect(getComputedStyle(getByRole('textbox')).direction).toBe('rtl');
  });
});
