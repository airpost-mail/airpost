// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup, act } from '@testing-library/react';
import { DirectionProvider } from '@radix-ui/react-direction';
import { RadioGroup, RadioGroupItem } from '../../../src/components/kit/radio-group';
import { pressRealKey, wait, watchPlainKeyCommands } from './kit-spec-helpers';

function Density(props: { dir?: 'ltr' | 'rtl'; onValueChange?: (v: string) => void }) {
  const [value, setValue] = React.useState('b');
  return (
    <RadioGroup
      aria-label="Density"
      orientation="horizontal"
      loop
      dir={props.dir}
      value={value}
      onValueChange={(v) => {
        setValue(v);
        props.onValueChange?.(v);
      }}
    >
      <RadioGroupItem value="a" aria-label="A" />
      <RadioGroupItem value="b" aria-label="B" />
      <RadioGroupItem value="c" aria-label="C">
        <span className="tile">Tile</span>
      </RadioGroupItem>
    </RadioGroup>
  );
}

describe('kit RadioGroup', function () {
  // Roving focus moves focus in a delay-less setTimeout (docs/desktop/ui-kit.md finding 5).
  beforeEach(() => jasmine.unspy(window, 'setTimeout'));
  afterEach(() => cleanup());

  it('renders the group and items with data-slot, data-state and no legacy classes', () => {
    const { getByRole, getAllByRole } = render(<Density />);
    const group = getByRole('radiogroup');
    expect(group.getAttribute('data-slot')).toBe('radio-group');
    expect(group.getAttribute('aria-orientation')).toBe('horizontal');
    expect(group.className).toBe('');
    const items = getAllByRole('radio');
    expect(items.map((i) => i.getAttribute('data-slot'))).toEqual([
      'radio-group-item',
      'radio-group-item',
      'radio-group-item',
    ]);
    expect(items.map((i) => i.getAttribute('data-state'))).toEqual([
      'unchecked',
      'checked',
      'unchecked',
    ]);
    expect(items[1].getAttribute('aria-checked')).toBe('true');
    expect(items[1].querySelector('[data-slot="radio-group-indicator"]')).not.toBe(null);
    expect(items[0].querySelector('[data-slot="radio-group-indicator"]')).toBe(null);
    expect(items[2].querySelector('.tile')).not.toBe(null);
  });

  it('picks on click', () => {
    const onValueChange = jasmine.createSpy('onValueChange');
    const { getAllByRole } = render(<Density onValueChange={onValueChange} />);
    const items = getAllByRole('radio');
    act(() => {
      fireEvent.mouseDown(items[0]);
      fireEvent.click(items[0]);
    });
    expect(onValueChange).toHaveBeenCalledWith('a');
    expect(items[0].getAttribute('data-state')).toBe('checked');
  });

  it('is one tab stop and moves the choice with the arrow keys, mirrored in RTL', async () => {
    for (const [dir, forward] of [
      ['ltr', 'ArrowRight'],
      ['rtl', 'ArrowLeft'],
    ] as const) {
      const { getByRole, getAllByRole, unmount } = render(
        <DirectionProvider dir={dir}>
          <Density />
        </DirectionProvider>
      );
      const group = getByRole('radiogroup');
      expect(group.getAttribute('dir')).toBe(dir);
      const items = getAllByRole('radio');
      // The group is the tab stop (finding 4); focusing it forwards to the checked item.
      expect(group.tabIndex).toBe(0);
      expect(items.map((i) => i.tabIndex)).toEqual([-1, -1, -1]);

      act(() => items[1].focus());
      fireEvent.keyDown(items[1], { key: forward });
      await wait(30);
      expect(document.activeElement).toBe(items[2]);
      expect(items[2].getAttribute('data-state')).toBe('checked');

      // loop wraps from the last item to the first.
      fireEvent.keyDown(items[2], { key: forward });
      await wait(30);
      expect(items[0].getAttribute('data-state')).toBe('checked');
      unmount();
    }
  });

  it('keeps its keys out of the keymap', async () => {
    const commands = watchPlainKeyCommands();
    try {
      const { getAllByRole } = render(<Density />);
      const items = getAllByRole('radio');
      act(() => items[1].focus());
      await pressRealKey('Right');
      expect(items[2].getAttribute('data-state')).toBe('checked');
      await pressRealKey('Down');
      await pressRealKey('Enter');
      await pressRealKey('Space');
      expect(commands.total()).toBe(0);
    } finally {
      commands.dispose();
    }
  });

  it('takes dir as a prop without a provider', () => {
    const { getByRole } = render(<Density dir="rtl" />);
    expect(getByRole('radiogroup').getAttribute('dir')).toBe('rtl');
  });

  it('skips disabled items', () => {
    const onValueChange = jasmine.createSpy('onValueChange');
    const { getAllByRole } = render(
      <RadioGroup aria-label="X" defaultValue="a" onValueChange={onValueChange}>
        <RadioGroupItem value="a" aria-label="A" />
        <RadioGroupItem value="b" aria-label="B" disabled />
      </RadioGroup>
    );
    const items = getAllByRole('radio');
    expect(items[1].hasAttribute('data-disabled')).toBe(true);
    fireEvent.click(items[1]);
    expect(onValueChange).not.toHaveBeenCalled();
  });
});
