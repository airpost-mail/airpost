// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import * as TooltipPrimitive from '@radix-ui/react-tooltip';
import { getPortalContainer } from './portal';

/* Public: A hover/focus tooltip on Radix Tooltip (docs/desktop/ui-kit.md 6.12). Each Tooltip
   carries its own Provider, so it works in any React root. It doesn't take focus and doesn't
   consume Escape.

   ```
   <Tooltip>
     <TooltipTrigger asChild><button>?</button></TooltipTrigger>
     <TooltipContent>Help</TooltipContent>
   </Tooltip>
   ```
*/

export type TooltipProps = React.ComponentProps<typeof TooltipPrimitive.Root>;

export function Tooltip({ delayDuration = 700, ...props }: TooltipProps) {
  return (
    <TooltipPrimitive.Provider delayDuration={delayDuration}>
      <TooltipPrimitive.Root delayDuration={delayDuration} {...props} />
    </TooltipPrimitive.Provider>
  );
}
Tooltip.displayName = 'Tooltip';

export const TooltipTrigger = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Trigger>
>(function TooltipTrigger(props, ref) {
  return <TooltipPrimitive.Trigger ref={ref} data-slot="tooltip-trigger" {...props} />;
});

export type TooltipContentProps = React.ComponentPropsWithoutRef<
  typeof TooltipPrimitive.Content
> & {
  container?: HTMLElement | null;
};

export const TooltipContent = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Content>,
  TooltipContentProps
>(function TooltipContent({ container, side = 'top', sideOffset = 4, ...props }, ref) {
  return (
    <TooltipPrimitive.Portal container={container || getPortalContainer()}>
      <TooltipPrimitive.Content
        ref={ref}
        data-slot="tooltip-content"
        side={side}
        sideOffset={sideOffset}
        {...props}
      />
    </TooltipPrimitive.Portal>
  );
});

TooltipTrigger.displayName = 'TooltipTrigger';
TooltipContent.displayName = 'TooltipContent';
