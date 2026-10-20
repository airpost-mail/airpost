// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import classNames from 'classnames';
import { RetinaImg } from '../retina-img';
import { ButtonProps, legacyButtonClasses } from './button';
import { Tooltip, TooltipContent, TooltipTrigger } from './tooltip';

export type IconButtonProps = Omit<ButtonProps, 'children' | 'asChild' | 'variant'> & {
  /** A RetinaImg image name. */
  icon?: string;
  /** A RetinaImg url, used instead of `icon`. */
  iconUrl?: string;
  /** A RetinaImg.Mode; defaults to ContentIsMask, so the icon takes the button's text colour. */
  mode?: string;
  /** The accessible name, and the tooltip unless `tooltip={false}`. */
  label: string;
  tooltip?: boolean;
  /** Makes it a toggle button (`aria-pressed`, `data-state="on|off"`). */
  pressed?: boolean;
  variant?: 'ghost' | 'outline' | 'secondary' | 'toolbar';
};

/* Public: A button that shows only an icon. `label` is required: it is the `aria-label` and the
   tooltip. */
export const IconButton = React.forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  {
    icon,
    iconUrl,
    mode = RetinaImg.Mode.ContentIsMask,
    label,
    tooltip = true,
    pressed,
    variant = 'ghost',
    size = 'default',
    type = 'button',
    className,
    ...props
  },
  ref
) {
  const legacy =
    variant === 'ghost'
      ? 'btn-icon'
      : variant === 'toolbar'
        ? legacyButtonClasses('toolbar', size, props.disabled)
        : '';
  const button = (
    <button
      data-slot="icon-button"
      data-variant={variant}
      data-size={size}
      type={type}
      aria-label={label}
      aria-pressed={pressed === undefined ? undefined : pressed}
      data-state={pressed === undefined ? undefined : pressed ? 'on' : 'off'}
      className={classNames(legacy, className)}
      ref={ref}
      {...props}
    >
      <RetinaImg name={icon} url={iconUrl} mode={mode} />
    </button>
  );
  if (!tooltip) return button;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
});
IconButton.displayName = 'IconButton';
