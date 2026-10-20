// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup, act } from '@testing-library/react';
import { SegmentedControl } from '../../src/components/kit/segmented-control';

type Kind = 'all' | 'open' | 'click';

const OPTIONS: [Kind, string][] = [
  ['all', 'All'],
  ['open', 'Opens'],
  ['click', 'Clicks'],
];

function Controlled(props: { initial: Kind | null; onChange?: (v: Kind) => void }) {
  const [value, setValue] = React.useState<Kind | null>(props.initial);
  return (
    <SegmentedControl<Kind>
      label="Kind"
      value={value}
      options={OPTIONS}
      onChange={(v) => {
        setValue(v);
        props.onChange?.(v);
      }}
    />
  );
}

describe('kit SegmentedControl', function () {
  afterEach(() => cleanup());

  const segments = (root: HTMLElement) =>
    Array.from(root.querySelectorAll<HTMLButtonElement>('[role="radio"]'));
  const checked = (root: HTMLElement) =>
    segments(root).find((b) => b.getAttribute('aria-checked') === 'true');

  it('renders data-slot, data-state and the legacy classes', () => {
    const { container, getByRole } = render(<Controlled initial="open" />);
    const group = getByRole('radiogroup');
    expect(group.getAttribute('data-slot')).toBe('segmented-control');
    expect(group.getAttribute('data-size')).toBe('default');
    expect(group.getAttribute('aria-label')).toBe('Kind');
    expect(group.className).toBe('settings-segmented');
    const items = segments(container);
    expect(items.map((b) => b.getAttribute('data-slot'))).toEqual([
      'segmented-control-item',
      'segmented-control-item',
      'segmented-control-item',
    ]);
    expect(items.map((b) => b.getAttribute('data-state'))).toEqual(['off', 'on', 'off']);
    expect(items.map((b) => b.className)).toEqual(['', 'on', '']);
    expect(items.map((b) => b.getAttribute('type'))).toEqual(['button', 'button', 'button']);
  });

  it('makes the chosen segment the only tab stop, or the first with nothing chosen', () => {
    const { container, unmount } = render(<Controlled initial="open" />);
    expect(segments(container).map((b) => b.tabIndex)).toEqual([-1, 0, -1]);
    unmount();
    const second = render(<Controlled initial={null} />);
    expect(segments(second.container).map((b) => b.tabIndex)).toEqual([0, -1, -1]);
  });

  it('moves the choice and the focus with the arrow keys, wrapping at the ends', () => {
    const { container } = render(<Controlled initial="open" />);
    act(() => segments(container)[1].focus());

    act(() => {
      fireEvent.keyDown(document.activeElement, { key: 'ArrowRight' });
    });
    expect(checked(container).textContent).toBe('Clicks');
    expect(document.activeElement).toBe(checked(container));
    expect(checked(container).getAttribute('data-state')).toBe('on');

    act(() => {
      fireEvent.keyDown(document.activeElement, { key: 'ArrowRight' });
    });
    expect(checked(container).textContent).toBe('All');

    act(() => {
      fireEvent.keyDown(document.activeElement, { key: 'ArrowLeft' });
    });
    expect(checked(container).textContent).toBe('Clicks');
  });

  it('keeps typed values', () => {
    const onChange = jasmine.createSpy('onChange');
    const { getAllByRole } = render(
      <SegmentedControl<number>
        value={1}
        options={[
          [1, 'One'],
          [2, 'Two'],
        ]}
        onChange={onChange}
      />
    );
    fireEvent.click(getAllByRole('radio')[1]);
    expect(onChange).toHaveBeenCalledWith(2);
  });

  it('ignores keys when disabled', () => {
    const onChange = jasmine.createSpy('onChange');
    const { getByRole } = render(
      <SegmentedControl<Kind> value="all" options={OPTIONS} onChange={onChange} disabled />
    );
    fireEvent.keyDown(getByRole('radiogroup'), { key: 'ArrowRight' });
    expect(onChange).not.toHaveBeenCalled();
  });

  it('takes a className and a size, and forwards its ref', () => {
    const ref = React.createRef<HTMLDivElement>();
    const { getByRole } = render(
      <SegmentedControl<Kind>
        ref={ref}
        className="segmented-control"
        size="sm"
        value="all"
        options={OPTIONS}
        onChange={() => {}}
      />
    );
    const group = getByRole('radiogroup');
    expect(ref.current).toBe(group as HTMLDivElement);
    expect(group.className).toBe('settings-segmented segmented-control');
    expect(group.getAttribute('data-size')).toBe('sm');
  });

  it('does not mirror the arrows in RTL, as before the kit', () => {
    const { container } = render(
      <div dir="rtl">
        <Controlled initial="open" />
      </div>
    );
    act(() => segments(container)[1].focus());
    act(() => {
      fireEvent.keyDown(document.activeElement, { key: 'ArrowRight' });
    });
    expect(checked(container).textContent).toBe('Clicks');
  });
});
