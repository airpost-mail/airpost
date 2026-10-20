// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import classNames from 'classnames';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import { getPortalContainer } from './portal';
import { consumeEscape, layerProps } from './layer';

/* Public: A non-modal popover on Radix Popover (docs/desktop/ui-kit.md 6.10). Anchor it to a
   trigger, to another element with PopoverAnchor, or to a rect with
   `<PopoverAnchor virtualRef={{ current: { getBoundingClientRect: () => rect } }} />`.
*/

export function Popover(props: React.ComponentProps<typeof PopoverPrimitive.Root>) {
  return <PopoverPrimitive.Root {...props} />;
}
Popover.displayName = 'Popover';

export const PopoverTrigger = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Trigger>
>(function PopoverTrigger(props, ref) {
  return <PopoverPrimitive.Trigger ref={ref} data-slot="popover-trigger" {...props} />;
});

export const PopoverAnchor = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Anchor>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Anchor>
>(function PopoverAnchor(props, ref) {
  return <PopoverPrimitive.Anchor ref={ref} data-slot="popover-anchor" {...props} />;
});

export type PopoverContentProps = React.ComponentPropsWithoutRef<
  typeof PopoverPrimitive.Content
> & {
  container?: HTMLElement | null;
};

export const PopoverContent = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Content>,
  PopoverContentProps
>(function PopoverContent(
  { className, container, sideOffset = 4, collisionPadding = 8, onEscapeKeyDown, ...props },
  ref
) {
  return (
    <PopoverPrimitive.Portal container={container || getPortalContainer()}>
      <PopoverPrimitive.Content
        ref={ref}
        data-slot="popover-content"
        {...layerProps}
        className={classNames('fixed-popover', className)}
        sideOffset={sideOffset}
        collisionPadding={collisionPadding}
        onEscapeKeyDown={consumeEscape(onEscapeKeyDown)}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
});

export const PopoverArrow = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Arrow>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Arrow>
>(function PopoverArrow({ className, width = 14, height = 7, ...props }, ref) {
  return (
    <PopoverPrimitive.Arrow
      ref={ref}
      data-slot="popover-arrow"
      className={classNames('fixed-popover-pointer', className)}
      width={width}
      height={height}
      {...props}
    />
  );
});

PopoverTrigger.displayName = 'PopoverTrigger';
PopoverAnchor.displayName = 'PopoverAnchor';
PopoverContent.displayName = 'PopoverContent';
PopoverArrow.displayName = 'PopoverArrow';
