// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Button } from '../../../src/components/kit/button';
import { pressRealKey, watchPlainKeyCommands } from './kit-spec-helpers';

describe('kit Button', function () {
  afterEach(() => cleanup());

  it('renders a type="button" with data-slot, variant and size', () => {
    const { getByRole } = render(<Button>Save</Button>);
    const button = getByRole('button');
    expect(button.tagName).toBe('BUTTON');
    expect(button.getAttribute('type')).toBe('button');
    expect(button.getAttribute('data-slot')).toBe('button');
    expect(button.getAttribute('data-variant')).toBe('default');
    expect(button.getAttribute('data-size')).toBe('default');
    expect(button.textContent).toBe('Save');
  });

  it('keeps an explicit type and passes props and ref through', () => {
    const ref = React.createRef<HTMLButtonElement>();
    const onClick = jasmine.createSpy('onClick');
    const { getByRole } = render(
      <Button type="submit" ref={ref} id="go" title="Go" onClick={onClick}>
        Go
      </Button>
    );
    const button = getByRole('button');
    expect(button.getAttribute('type')).toBe('submit');
    expect(button.id).toBe('go');
    expect(ref.current).toBe(button as HTMLButtonElement);
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalled();
  });

  it('renders the legacy classes for each variant and size, then the caller className', () => {
    const cases: [any, any, string][] = [
      ['default', 'default', 'btn btn-emphasis'],
      ['secondary', 'default', 'btn'],
      ['outline', 'default', 'btn'],
      ['destructive', 'default', 'btn btn-danger'],
      ['toolbar', 'default', 'btn btn-toolbar'],
      ['ghost', 'default', ''],
      ['link', 'default', ''],
      ['secondary', 'sm', 'btn btn-small'],
      ['secondary', 'lg', 'btn btn-large'],
    ];
    for (const [variant, size, classes] of cases) {
      const { getByRole, unmount } = render(
        <Button variant={variant} size={size} className="mine">
          x
        </Button>
      );
      const button = getByRole('button');
      expect(button.className).toBe(classes ? `${classes} mine` : 'mine');
      expect(button.getAttribute('data-variant')).toBe(variant);
      expect(button.getAttribute('data-size')).toBe(size);
      unmount();
    }
  });

  it('adds btn-disabled next to the native disabled attribute', () => {
    const onClick = jasmine.createSpy('onClick');
    const { getByRole } = render(
      <Button variant="secondary" disabled onClick={onClick}>
        x
      </Button>
    );
    const button = getByRole('button') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.classList.contains('btn-disabled')).toBe(true);
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('renders its child with asChild', () => {
    const { container } = render(
      <Button asChild variant="link">
        <a href="#docs">Docs</a>
      </Button>
    );
    const a = container.querySelector('a');
    expect(container.querySelector('button')).toBe(null);
    expect(a.getAttribute('data-slot')).toBe('button');
    expect(a.getAttribute('data-variant')).toBe('link');
    expect(a.hasAttribute('type')).toBe(false);
  });

  it('leaves Enter and Space to the keymap, like a legacy <button>', async () => {
    // The keymap's handler for enter (core:focus-item) and space (core:messages-page-down)
    // prevents the default, which cancels a focused button's native click. Legacy buttons behave
    // the same; changing that for buttons is a separate decision.
    const commands = watchPlainKeyCommands();
    try {
      const onClick = jasmine.createSpy('onClick');
      const { getByRole } = render(<Button onClick={onClick}>x</Button>);
      getByRole('button').focus();
      await pressRealKey('Enter');
      await pressRealKey('Space');
      expect(onClick.callCount).toBe(0);
      expect(commands.counts['core:focus-item']).toBe(1);
      expect(commands.counts['core:messages-page-down']).toBe(1);
    } finally {
      commands.dispose();
    }
  });

  it('is styled by the kit sheet, not the legacy .btn rules', () => {
    const { getByRole } = render(<Button variant="outline">x</Button>);
    const style = getComputedStyle(getByRole('button'));
    // Kit: 26px high with a 1px border. Legacy .btn is 1.9em with box-shadow hairlines.
    expect(style.height).toBe('26px');
    expect(style.borderTopWidth).toBe('1px');
    expect(style.boxShadow).toBe('none');
  });

  it('uses logical padding, so it mirrors in RTL', () => {
    const { getByRole } = render(
      <div dir="rtl">
        <Button>x</Button>
      </div>
    );
    const button = getByRole('button');
    const style = getComputedStyle(button);
    expect(style.direction).toBe('rtl');
    expect(style.paddingInlineStart).toBe(style.paddingInlineEnd);
  });
});
