// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import classNames from 'classnames';
import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import { getPortalContainer } from './portal';
import { consumeEscape, layerProps } from './layer';
import { CheckGlyph, DotGlyph } from './glyphs';

/* Public: A menu opened from a button, on Radix DropdownMenu with shadcn's parts
   (docs/desktop/ui-kit.md 6.11). Context menus stay native Electron menus. Plugins get the root
   as `DropdownMenuRoot` from mailspring-component-kit, where `DropdownMenu` is the legacy menu.

   ```
   <DropdownMenu>
     <DropdownMenuTrigger asChild><button>More</button></DropdownMenuTrigger>
     <DropdownMenuContent align="end">
       <DropdownMenuItem onSelect={onArchive}>Archive</DropdownMenuItem>
       <DropdownMenuSeparator />
       <DropdownMenuCheckboxItem checked={starred} onCheckedChange={setStarred}>Starred</DropdownMenuCheckboxItem>
     </DropdownMenuContent>
   </DropdownMenu>
   ```
*/

export function DropdownMenu(props: React.ComponentProps<typeof DropdownMenuPrimitive.Root>) {
  return <DropdownMenuPrimitive.Root {...props} />;
}
DropdownMenu.displayName = 'DropdownMenu';

export function DropdownMenuSub(props: React.ComponentProps<typeof DropdownMenuPrimitive.Sub>) {
  return <DropdownMenuPrimitive.Sub {...props} />;
}
DropdownMenuSub.displayName = 'DropdownMenuSub';

// Legacy themes style `.menu .item.selected`. Radix highlights the focused item, so the
// `selected` class follows focus.
function useLegacySelected<E extends HTMLElement>(
  onFocus?: React.FocusEventHandler<E>,
  onBlur?: React.FocusEventHandler<E>
) {
  const [selected, setSelected] = React.useState(false);
  return {
    selected,
    onFocus: (e: React.FocusEvent<E>) => {
      onFocus?.(e);
      setSelected(true);
    },
    onBlur: (e: React.FocusEvent<E>) => {
      onBlur?.(e);
      setSelected(false);
    },
  };
}

const ChevronGlyph = () => (
  <svg
    viewBox="0 0 16 16"
    width="12"
    height="12"
    aria-hidden="true"
    focusable="false"
    data-slot="dropdown-menu-sub-chevron"
  >
    <path
      d="M6 3.5l4.5 4.5L6 12.5"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      fill="none"
    />
  </svg>
);

export const DropdownMenuTrigger = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Trigger>
>(function DropdownMenuTrigger(props, ref) {
  return <DropdownMenuPrimitive.Trigger ref={ref} data-slot="dropdown-menu-trigger" {...props} />;
});

export type DropdownMenuContentProps = React.ComponentPropsWithoutRef<
  typeof DropdownMenuPrimitive.Content
> & { container?: HTMLElement | null };

export const DropdownMenuContent = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.Content>,
  DropdownMenuContentProps
>(function DropdownMenuContent(
  { className, container, sideOffset = 4, onEscapeKeyDown, ...props },
  ref
) {
  return (
    <DropdownMenuPrimitive.Portal container={container || getPortalContainer()}>
      <DropdownMenuPrimitive.Content
        ref={ref}
        data-slot="dropdown-menu-content"
        {...layerProps}
        className={classNames('menu', className)}
        sideOffset={sideOffset}
        onEscapeKeyDown={consumeEscape(onEscapeKeyDown)}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
});

export const DropdownMenuGroup = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.Group>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Group>
>(function DropdownMenuGroup(props, ref) {
  return <DropdownMenuPrimitive.Group ref={ref} data-slot="dropdown-menu-group" {...props} />;
});

export type DropdownMenuItemProps = React.ComponentPropsWithoutRef<
  typeof DropdownMenuPrimitive.Item
> & { inset?: boolean; variant?: 'default' | 'destructive' };

export const DropdownMenuItem = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.Item>,
  DropdownMenuItemProps
>(function DropdownMenuItem(
  { className, inset, variant = 'default', onFocus, onBlur, ...props },
  ref
) {
  const legacy = useLegacySelected(onFocus, onBlur);
  return (
    <DropdownMenuPrimitive.Item
      ref={ref}
      data-slot="dropdown-menu-item"
      data-variant={variant}
      data-inset={inset ? '' : undefined}
      className={classNames('item', { selected: legacy.selected }, className)}
      onFocus={legacy.onFocus}
      onBlur={legacy.onBlur}
      {...props}
    />
  );
});

export const DropdownMenuCheckboxItem = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.CheckboxItem>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.CheckboxItem>
>(function DropdownMenuCheckboxItem({ className, children, onFocus, onBlur, ...props }, ref) {
  const legacy = useLegacySelected(onFocus, onBlur);
  return (
    <DropdownMenuPrimitive.CheckboxItem
      ref={ref}
      data-slot="dropdown-menu-checkbox-item"
      className={classNames('item', { selected: legacy.selected }, className)}
      onFocus={legacy.onFocus}
      onBlur={legacy.onBlur}
      {...props}
    >
      <span data-slot="dropdown-menu-item-indicator">
        <DropdownMenuPrimitive.ItemIndicator>
          <CheckGlyph />
        </DropdownMenuPrimitive.ItemIndicator>
      </span>
      {children}
    </DropdownMenuPrimitive.CheckboxItem>
  );
});

export const DropdownMenuRadioGroup = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.RadioGroup>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.RadioGroup>
>(function DropdownMenuRadioGroup(props, ref) {
  return (
    <DropdownMenuPrimitive.RadioGroup ref={ref} data-slot="dropdown-menu-radio-group" {...props} />
  );
});

export const DropdownMenuRadioItem = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.RadioItem>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.RadioItem>
>(function DropdownMenuRadioItem({ className, children, onFocus, onBlur, ...props }, ref) {
  const legacy = useLegacySelected(onFocus, onBlur);
  return (
    <DropdownMenuPrimitive.RadioItem
      ref={ref}
      data-slot="dropdown-menu-radio-item"
      className={classNames('item', { selected: legacy.selected }, className)}
      onFocus={legacy.onFocus}
      onBlur={legacy.onBlur}
      {...props}
    >
      <span data-slot="dropdown-menu-item-indicator">
        <DropdownMenuPrimitive.ItemIndicator>
          <DotGlyph />
        </DropdownMenuPrimitive.ItemIndicator>
      </span>
      {children}
    </DropdownMenuPrimitive.RadioItem>
  );
});

export const DropdownMenuLabel = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.Label>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Label> & { inset?: boolean }
>(function DropdownMenuLabel({ inset, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Label
      ref={ref}
      data-slot="dropdown-menu-label"
      data-inset={inset ? '' : undefined}
      {...props}
    />
  );
});

export const DropdownMenuSeparator = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Separator>
>(function DropdownMenuSeparator({ className, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Separator
      ref={ref}
      data-slot="dropdown-menu-separator"
      className={classNames('item', 'divider', className)}
      {...props}
    />
  );
});

export const DropdownMenuShortcut = React.forwardRef<
  HTMLSpanElement,
  React.ComponentPropsWithoutRef<'span'>
>(function DropdownMenuShortcut(props, ref) {
  return <span ref={ref} data-slot="dropdown-menu-shortcut" {...props} />;
});

export const DropdownMenuSubTrigger = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.SubTrigger>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.SubTrigger> & { inset?: boolean }
>(function DropdownMenuSubTrigger({ className, inset, children, onFocus, onBlur, ...props }, ref) {
  const legacy = useLegacySelected(onFocus, onBlur);
  return (
    <DropdownMenuPrimitive.SubTrigger
      ref={ref}
      data-slot="dropdown-menu-sub-trigger"
      data-inset={inset ? '' : undefined}
      className={classNames('item', { selected: legacy.selected }, className)}
      onFocus={legacy.onFocus}
      onBlur={legacy.onBlur}
      {...props}
    >
      {children}
      <ChevronGlyph />
    </DropdownMenuPrimitive.SubTrigger>
  );
});

export type DropdownMenuSubContentProps = React.ComponentPropsWithoutRef<
  typeof DropdownMenuPrimitive.SubContent
> & { container?: HTMLElement | null };

export const DropdownMenuSubContent = React.forwardRef<
  React.ElementRef<typeof DropdownMenuPrimitive.SubContent>,
  DropdownMenuSubContentProps
>(function DropdownMenuSubContent({ className, container, onEscapeKeyDown, ...props }, ref) {
  // Portalled like the root content: inline, it would sit inside the parent's zoom transform.
  return (
    <DropdownMenuPrimitive.Portal container={container || getPortalContainer()}>
      <DropdownMenuPrimitive.SubContent
        ref={ref}
        data-slot="dropdown-menu-sub-content"
        {...layerProps}
        className={classNames('menu', className)}
        onEscapeKeyDown={consumeEscape(onEscapeKeyDown)}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
});

DropdownMenuTrigger.displayName = 'DropdownMenuTrigger';
DropdownMenuContent.displayName = 'DropdownMenuContent';
DropdownMenuGroup.displayName = 'DropdownMenuGroup';
DropdownMenuItem.displayName = 'DropdownMenuItem';
DropdownMenuCheckboxItem.displayName = 'DropdownMenuCheckboxItem';
DropdownMenuRadioGroup.displayName = 'DropdownMenuRadioGroup';
DropdownMenuRadioItem.displayName = 'DropdownMenuRadioItem';
DropdownMenuLabel.displayName = 'DropdownMenuLabel';
DropdownMenuSeparator.displayName = 'DropdownMenuSeparator';
DropdownMenuShortcut.displayName = 'DropdownMenuShortcut';
DropdownMenuSubTrigger.displayName = 'DropdownMenuSubTrigger';
DropdownMenuSubContent.displayName = 'DropdownMenuSubContent';
