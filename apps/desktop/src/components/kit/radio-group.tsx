// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import * as RadioGroupPrimitive from '@radix-ui/react-radio-group';
import { ownKeys } from './keys';

const RADIO_KEYS = [' ', 'Enter', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End'];

export type RadioGroupProps = React.ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Root>;
export type RadioGroupItemProps = React.ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Item>;

/* Public: A group of radio buttons with string values. The group is the tab stop; arrow keys move
   the choice and mirror in RTL. Children are `RadioGroupItem`s. */
export const RadioGroup = React.forwardRef<HTMLDivElement, RadioGroupProps>(function RadioGroup(
  { onKeyDown, ...props },
  ref
) {
  return (
    <RadioGroupPrimitive.Root
      data-slot="radio-group"
      ref={ref}
      onKeyDown={ownKeys(RADIO_KEYS, onKeyDown)}
      {...props}
    />
  );
});
RadioGroup.displayName = 'RadioGroup';

/* Public: One choice in a `RadioGroup`. Without children it draws the round radio; with children
   (a theme tile, a colour swatch) the children are the visual and the indicator follows them. */
export const RadioGroupItem = React.forwardRef<HTMLButtonElement, RadioGroupItemProps>(
  function RadioGroupItem({ children, ...props }, ref) {
    return (
      <RadioGroupPrimitive.Item data-slot="radio-group-item" ref={ref} {...props}>
        {children}
        <RadioGroupPrimitive.Indicator data-slot="radio-group-indicator" />
      </RadioGroupPrimitive.Item>
    );
  }
);
RadioGroupItem.displayName = 'RadioGroupItem';
