// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import classNames from 'classnames';
import * as SwitchPrimitive from '@radix-ui/react-switch';
import { ownKeys } from './keys';

export type SwitchProps = Omit<
  React.ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>,
  'onChange'
> & {
  size?: 'sm' | 'default';
  /** The legacy `Switch` callback: called with the click event (keyboard activation too). */
  onChange?: (event: React.SyntheticEvent) => void;
  label?: string;
  labelledBy?: string;
};

/* Public: An on/off switch. Takes Radix's `checked`/`defaultChecked`/`onCheckedChange` and the
   legacy `Switch` props (`onChange`, `label`, `labelledBy`), so callers that flip `checked`
   themselves from `onChange` keep working. */
export const Switch = React.forwardRef<HTMLButtonElement, SwitchProps>(function Switch(
  {
    size = 'default',
    onChange,
    label,
    labelledBy,
    className,
    checked,
    defaultChecked,
    onCheckedChange,
    onClick,
    onKeyDown,
    ...props
  },
  ref
) {
  // Mirrors Radix's state so the legacy `active` class follows an uncontrolled switch too.
  const [uncontrolled, setUncontrolled] = React.useState(!!defaultChecked);
  const on = checked ?? uncontrolled;

  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      data-size={size}
      aria-label={label}
      aria-labelledby={labelledBy}
      className={classNames('slide-switch', { active: on }, className)}
      checked={checked}
      defaultChecked={defaultChecked}
      onCheckedChange={(next) => {
        if (checked === undefined) setUncontrolled(next);
        onCheckedChange?.(next);
      }}
      onClick={(e) => {
        onClick?.(e);
        onChange?.(e);
      }}
      onKeyDown={ownKeys([' ', 'Enter'], onKeyDown)}
      ref={ref}
      {...props}
    >
      <SwitchPrimitive.Thumb data-slot="switch-thumb" className="handle" />
    </SwitchPrimitive.Root>
  );
});
Switch.displayName = 'Switch';
