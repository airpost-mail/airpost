// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup, act } from '@testing-library/react';
import { IconButton } from '../../../src/components/kit/icon-button';
import { getPortalContainer } from '../../../src/components/kit/portal';
import { pressRealKey, wait, watchPlainKeyCommands } from './kit-spec-helpers';

describe('kit IconButton', function () {
  beforeEach(() => jasmine.unspy(window, 'setTimeout'));
  afterEach(() => cleanup());

  it('renders a labelled button with a mask image', () => {
    const { getByRole } = render(
      <IconButton icon="icon-composer-trash.png" label="Delete" tooltip={false} />
    );
    const button = getByRole('button');
    expect(button.getAttribute('type')).toBe('button');
    expect(button.getAttribute('data-slot')).toBe('icon-button');
    expect(button.getAttribute('data-variant')).toBe('ghost');
    expect(button.getAttribute('data-size')).toBe('default');
    expect(button.getAttribute('aria-label')).toBe('Delete');
    expect(button.hasAttribute('aria-pressed')).toBe(false);
    expect(button.hasAttribute('title')).toBe(false);
    const img = button.querySelector('img');
    expect(img.classList.contains('content-mask')).toBe(true);
  });

  it('renders the legacy classes: btn-icon for ghost, btn btn-toolbar for toolbar', () => {
    const { getAllByRole } = render(
      <>
        <IconButton icon="x.png" label="A" tooltip={false} className="mine" />
        <IconButton icon="x.png" label="B" tooltip={false} variant="toolbar" />
        <IconButton icon="x.png" label="C" tooltip={false} variant="outline" />
      </>
    );
    const [ghost, toolbar, outline] = getAllByRole('button');
    expect(ghost.className).toBe('btn-icon mine');
    expect(toolbar.className).toBe('btn btn-toolbar');
    expect(outline.className).toBe('');
  });

  it('is a toggle button with pressed', () => {
    const { getAllByRole } = render(
      <>
        <IconButton icon="x.png" label="On" pressed tooltip={false} />
        <IconButton icon="x.png" label="Off" pressed={false} tooltip={false} />
      </>
    );
    const [on, off] = getAllByRole('button');
    expect(on.getAttribute('aria-pressed')).toBe('true');
    expect(on.getAttribute('data-state')).toBe('on');
    expect(off.getAttribute('aria-pressed')).toBe('false');
    expect(off.getAttribute('data-state')).toBe('off');
  });

  it('draws the icon in the button text colour', () => {
    const { getByRole } = render(<IconButton icon="x.png" label="A" tooltip={false} />);
    const button = getByRole('button');
    const img = button.querySelector('img');
    expect(getComputedStyle(img).backgroundColor).toBe(getComputedStyle(button).color);
  });

  it('clicks and forwards its ref', () => {
    const ref = React.createRef<HTMLButtonElement>();
    const onClick = jasmine.createSpy('onClick');
    const { getByRole } = render(<IconButton icon="x.png" label="A" ref={ref} onClick={onClick} />);
    const button = getByRole('button');
    expect(ref.current).toBe(button as HTMLButtonElement);
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalled();
  });

  it('leaves Enter and Space to the keymap, like Button', async () => {
    const commands = watchPlainKeyCommands();
    try {
      const { getByRole } = render(<IconButton icon="x.png" label="A" tooltip={false} />);
      getByRole('button').focus();
      await pressRealKey('Enter');
      expect(commands.counts['core:focus-item']).toBe(1);
    } finally {
      commands.dispose();
    }
  });

  it('shows the label as a tooltip in the portal container on focus, keeping data-state', async () => {
    const { getByRole } = render(<IconButton icon="x.png" label="Archive" pressed />);
    const button = getByRole('button');
    act(() => {
      button.focus();
      fireEvent.focusIn(button);
    });
    await wait(20);
    const content = getPortalContainer().querySelector('[data-slot="tooltip-content"]');
    expect(content).not.toBe(null);
    expect(content.textContent).toContain('Archive');
    // The tooltip trigger's own data-state doesn't replace the toggle state.
    expect(button.getAttribute('data-state')).toBe('on');
  });

  it('keeps aria-label and markup in RTL', () => {
    const { getByRole } = render(
      <div dir="rtl">
        <IconButton icon="x.png" label="Next" tooltip={false} />
      </div>
    );
    const button = getByRole('button');
    expect(getComputedStyle(button).direction).toBe('rtl');
    expect(button.getAttribute('aria-label')).toBe('Next');
  });
});
