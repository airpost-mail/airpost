// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';

export type TextFieldProps = Omit<React.ComponentPropsWithoutRef<'input'>, 'size'> & {
  size?: 'sm' | 'default';
  invalid?: boolean;
};

/* Public: A single-line text input. `type` defaults to "text"; `invalid` sets `aria-invalid`. */
export const TextField = React.forwardRef<HTMLInputElement, TextFieldProps>(function TextField(
  { size = 'default', invalid, type = 'text', ...props },
  ref
) {
  return (
    <input
      data-slot="input"
      data-size={size}
      type={type}
      aria-invalid={invalid || undefined}
      ref={ref}
      {...props}
    />
  );
});
TextField.displayName = 'TextField';
