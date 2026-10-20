// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import classNames from 'classnames';
import { Slot } from '@radix-ui/react-slot';

export type ButtonVariant =
  | 'default'
  | 'secondary'
  | 'outline'
  | 'ghost'
  | 'destructive'
  | 'link'
  | 'toolbar';
export type ButtonSize = 'sm' | 'default' | 'lg';

export type ButtonProps = React.ComponentPropsWithoutRef<'button'> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  asChild?: boolean;
};

// Rendered next to data-slot so community themes that restyle .btn keep reaching kit buttons
// (docs/desktop/ui-kit.md section 7).
const LEGACY_VARIANT_CLASSES: { [V in ButtonVariant]: string } = {
  default: 'btn btn-emphasis',
  secondary: 'btn',
  outline: 'btn',
  destructive: 'btn btn-danger',
  toolbar: 'btn btn-toolbar',
  ghost: '',
  link: '',
};

const LEGACY_SIZE_CLASSES: { [S in ButtonSize]: string } = {
  sm: 'btn-small',
  default: '',
  lg: 'btn-large',
};

export function legacyButtonClasses(variant: ButtonVariant, size: ButtonSize, disabled?: boolean) {
  return classNames(LEGACY_VARIANT_CLASSES[variant], LEGACY_SIZE_CLASSES[size], {
    'btn-disabled': disabled,
  });
}

/* Public: A button. `variant` and `size` set `data-variant` and `data-size`; `asChild` renders
   the single child element instead of a `<button>`, with the button's props merged into it. */
export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = 'default', size = 'default', asChild = false, className, type, ...props },
  ref
) {
  const Comp: any = asChild ? Slot : 'button';
  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      type={asChild ? type : type || 'button'}
      className={classNames(legacyButtonClasses(variant, size, props.disabled), className)}
      ref={ref}
      {...props}
    />
  );
});
Button.displayName = 'Button';
