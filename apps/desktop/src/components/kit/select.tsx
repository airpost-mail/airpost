// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import * as SelectPrimitive from '@radix-ui/react-select';
import { getPortalContainer } from './portal';
import { consumeEscape, layerProps } from './layer';
import { ownKeysAndTypeahead } from './keys';

/* Public: A select on Radix Select with shadcn's parts (docs/desktop/ui-kit.md 6.6). Values are
   strings, and an item can't have value="" (Radix uses it for "cleared"): map an empty option to
   a sentinel and back.

   ```
   <Select value={layout} onValueChange={setLayout}>
     <SelectTrigger aria-label="Layout"><SelectValue /></SelectTrigger>
     <SelectContent>
       <SelectItem value="list">List</SelectItem>
       <SelectItem value="split">Split</SelectItem>
     </SelectContent>
   </Select>
   ```
*/

export function Select(props: React.ComponentProps<typeof SelectPrimitive.Root>) {
  return <SelectPrimitive.Root {...props} />;
}
Select.displayName = 'Select';

const ChevronDown = (props: { className?: string }) => (
  <svg
    viewBox="0 0 16 16"
    width="12"
    height="12"
    aria-hidden="true"
    focusable="false"
    className={props.className}
  >
    <path
      d="M4 6l4 4 4-4"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      fill="none"
    />
  </svg>
);

const CheckGlyph = () => (
  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true" focusable="false">
    <path
      d="M3.5 8.5l3 3 6-7"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      fill="none"
    />
  </svg>
);

export const SelectGroup = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Group>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Group>
>(function SelectGroup(props, ref) {
  return <SelectPrimitive.Group ref={ref} data-slot="select-group" {...props} />;
});

export const SelectValue = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Value>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Value>
>(function SelectValue(props, ref) {
  return <SelectPrimitive.Value ref={ref} data-slot="select-value" {...props} />;
});

const TRIGGER_KEYS = [' ', 'Enter', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'];

export type SelectTriggerProps = React.ComponentPropsWithoutRef<typeof SelectPrimitive.Trigger> & {
  size?: 'sm' | 'default';
};

export const SelectTrigger = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Trigger>,
  SelectTriggerProps
>(function SelectTrigger({ size = 'default', children, onKeyDown, ...props }, ref) {
  // The keys a closed trigger acts on (Space, Enter, the arrows, typeahead) don't also act on
  // the thread list, as on the native <select> it replaces. Escape still pops the sheet.
  return (
    <SelectPrimitive.Trigger
      ref={ref}
      data-slot="select-trigger"
      data-size={size}
      onKeyDown={ownKeysAndTypeahead(TRIGGER_KEYS, onKeyDown)}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <span data-slot="select-icon">
          <ChevronDown />
        </span>
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
});

export const SelectScrollUpButton = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.ScrollUpButton>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.ScrollUpButton>
>(function SelectScrollUpButton({ children, ...props }, ref) {
  return (
    <SelectPrimitive.ScrollUpButton ref={ref} data-slot="select-scroll-up-button" {...props}>
      {children ?? <ChevronDown className="flip" />}
    </SelectPrimitive.ScrollUpButton>
  );
});

export const SelectScrollDownButton = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.ScrollDownButton>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.ScrollDownButton>
>(function SelectScrollDownButton({ children, ...props }, ref) {
  return (
    <SelectPrimitive.ScrollDownButton ref={ref} data-slot="select-scroll-down-button" {...props}>
      {children ?? <ChevronDown />}
    </SelectPrimitive.ScrollDownButton>
  );
});

export type SelectContentProps = React.ComponentPropsWithoutRef<typeof SelectPrimitive.Content> & {
  container?: HTMLElement | null;
};

export const SelectContent = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Content>,
  SelectContentProps
>(function SelectContent(
  { children, container, position = 'item-aligned', sideOffset = 4, onEscapeKeyDown, ...props },
  ref
) {
  return (
    <SelectPrimitive.Portal container={container || getPortalContainer()}>
      <SelectPrimitive.Content
        ref={ref}
        data-slot="select-content"
        {...layerProps}
        position={position}
        sideOffset={sideOffset}
        onEscapeKeyDown={consumeEscape(onEscapeKeyDown)}
        {...props}
      >
        <SelectScrollUpButton />
        <SelectPrimitive.Viewport data-slot="select-viewport">{children}</SelectPrimitive.Viewport>
        <SelectScrollDownButton />
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
});

export const SelectLabel = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Label>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Label>
>(function SelectLabel(props, ref) {
  return <SelectPrimitive.Label ref={ref} data-slot="select-label" {...props} />;
});

export const SelectItem = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Item>
>(function SelectItem({ children, ...props }, ref) {
  return (
    <SelectPrimitive.Item ref={ref} data-slot="select-item" {...props}>
      <span data-slot="select-item-indicator">
        <SelectPrimitive.ItemIndicator>
          <CheckGlyph />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText data-slot="select-item-text">{children}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  );
});

export const SelectSeparator = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Separator>
>(function SelectSeparator(props, ref) {
  return <SelectPrimitive.Separator ref={ref} data-slot="select-separator" {...props} />;
});

SelectGroup.displayName = 'SelectGroup';
SelectValue.displayName = 'SelectValue';
SelectTrigger.displayName = 'SelectTrigger';
SelectScrollUpButton.displayName = 'SelectScrollUpButton';
SelectScrollDownButton.displayName = 'SelectScrollDownButton';
SelectContent.displayName = 'SelectContent';
SelectLabel.displayName = 'SelectLabel';
SelectItem.displayName = 'SelectItem';
SelectSeparator.displayName = 'SelectSeparator';
