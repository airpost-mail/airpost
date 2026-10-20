// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import * as CheckboxPrimitive from '@radix-ui/react-checkbox';
import { ownKeys } from './keys';

export type CheckboxProps = React.ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root>;
type CheckedState = CheckboxPrimitive.CheckedState;

/* Public: A checkbox with an indeterminate state (`checked="indeterminate"`). Renders a
   `<button role="checkbox">`, which a `<label htmlFor>` can point at. */
export const Checkbox = React.forwardRef<HTMLButtonElement, CheckboxProps>(function Checkbox(
  { checked, defaultChecked, onCheckedChange, onKeyDown, ...props },
  ref
) {
  // Mirrors Radix's state to pick the glyph; Indicator only says whether it is shown.
  const [uncontrolled, setUncontrolled] = React.useState<CheckedState>(defaultChecked ?? false);
  const state = checked ?? uncontrolled;

  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      checked={checked}
      defaultChecked={defaultChecked}
      onCheckedChange={(next) => {
        if (checked === undefined) setUncontrolled(next);
        onCheckedChange?.(next);
      }}
      // Radix ignores Enter on a checkbox, as a native one does; neither fires core:focus-item.
      onKeyDown={ownKeys([' ', 'Enter'], onKeyDown)}
      ref={ref}
      {...props}
    >
      <CheckboxPrimitive.Indicator data-slot="checkbox-indicator">
        <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden focusable="false">
          {state === 'indeterminate' ? (
            <path
              d="M4 8h8"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            />
          ) : (
            <path
              d="M3.5 8.5l3 3 6-7"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          )}
        </svg>
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
});
Checkbox.displayName = 'Checkbox';
