// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, cleanup } from '@testing-library/react';
import { Textarea } from '../../src/components/kit/textarea';

describe('kit Textarea', function () {
  afterEach(() => cleanup());

  it('renders a textarea with data-slot and aria-invalid', () => {
    const ref = React.createRef<HTMLTextAreaElement>();
    const { getByRole, rerender } = render(<Textarea ref={ref} rows={3} />);
    const textarea = getByRole('textbox') as HTMLTextAreaElement;
    expect(textarea.tagName).toBe('TEXTAREA');
    expect(textarea.getAttribute('data-slot')).toBe('textarea');
    expect(textarea.rows).toBe(3);
    expect(textarea.hasAttribute('aria-invalid')).toBe(false);
    expect(textarea.className).toBe('');
    expect(ref.current).toBe(textarea);
    rerender(<Textarea ref={ref} rows={3} invalid />);
    expect(textarea.getAttribute('aria-invalid')).toBe('true');
  });

  it('follows the direction it is in', () => {
    const { getByRole } = render(
      <div dir="rtl">
        <Textarea />
      </div>
    );
    expect(getComputedStyle(getByRole('textbox')).direction).toBe('rtl');
  });
});
