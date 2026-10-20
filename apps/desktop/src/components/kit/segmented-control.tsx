// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import classNames from 'classnames';

export type SegmentedValue = string | number | boolean;

export type SegmentedControlProps<T extends SegmentedValue> = {
  value: T;
  options: [T, string][];
  onChange: (value: T) => void;
  disabled?: boolean;
  label?: string;
  className?: string;
  size?: 'sm' | 'default';
};

/* Public: A row of mutually exclusive segments (`role="radiogroup"`). The chosen segment is the
   only tab stop; ArrowLeft/ArrowRight move the choice and focus follows once the parent applies
   the new value. Values keep their type (strings, numbers or booleans).

   Hand-written rather than on Radix RadioGroup, which would make the group the tab stop, move
   focus asynchronously and force string values (docs/desktop/ui-kit.md section 6.7). */
function SegmentedControlInner<T extends SegmentedValue>(
  props: SegmentedControlProps<T>,
  ref: React.ForwardedRef<HTMLDivElement>
) {
  const { value, options, onChange, disabled, size = 'default' } = props;
  const root = React.useRef<HTMLDivElement | null>(null);
  const movedByKey = React.useRef(false);

  const setRef = (el: HTMLDivElement | null) => {
    root.current = el;
    if (typeof ref === 'function') ref(el);
    else if (ref) ref.current = el;
  };

  // Focus follows the choice once the parent has applied it.
  React.useEffect(() => {
    if (!movedByKey.current) return;
    movedByKey.current = false;
    root.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
  }, [value]);

  // Arrows don't mirror in RTL, as before the kit; changing that is a behaviour change.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (disabled || options.length === 0) return;
    const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
    if (!step) return;
    e.preventDefault();
    const idx = options.findIndex(([v]) => v === value);
    const from = idx < 0 ? (step > 0 ? -1 : 0) : idx;
    const next = options[(from + step + options.length) % options.length][0];
    if (next === value) return;
    movedByKey.current = true;
    onChange(next);
  };

  return (
    <div
      ref={setRef}
      data-slot="segmented-control"
      data-size={size}
      className={classNames('settings-segmented', props.className)}
      role="radiogroup"
      aria-label={props.label}
      onKeyDown={onKeyDown}
    >
      {options.map(([v, label], i) => {
        const on = v === value;
        // With nothing chosen the first segment takes the tab stop.
        const tabStop = on || (i === 0 && !options.some(([o]) => o === value));
        return (
          <button
            key={String(v)}
            type="button"
            data-slot="segmented-control-item"
            data-state={on ? 'on' : 'off'}
            role="radio"
            aria-checked={on}
            tabIndex={tabStop ? 0 : -1}
            className={classNames({ on })}
            disabled={disabled}
            onClick={() => onChange(v)}
          >
            {label}
          </button>
        );
      })}
    </div>
  );
}

export const SegmentedControl = React.forwardRef(SegmentedControlInner) as (<
  T extends SegmentedValue,
>(
  props: SegmentedControlProps<T> & { ref?: React.Ref<HTMLDivElement> }
) => React.ReactElement) & { displayName?: string };
SegmentedControl.displayName = 'SegmentedControl';
