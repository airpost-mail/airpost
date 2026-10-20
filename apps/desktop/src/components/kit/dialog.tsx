// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import classNames from 'classnames';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { VisuallyHidden } from '@radix-ui/react-visually-hidden';
import { localized } from '../../shared/intl';
import { getPortalContainer } from './portal';
import { consumeEscape, layerProps } from './layer';

/* Public: A modal dialog on Radix Dialog with shadcn's parts (docs/desktop/ui-kit.md 6.9).
   Every dialog needs a title: a visible DialogTitle, or `hiddenTitle` on DialogContent.

   ```
   <Dialog open={open} onOpenChange={setOpen}>
     <DialogContent aria-describedby={undefined}>
       <DialogHeader><DialogTitle>Add account</DialogTitle></DialogHeader>
       …
       <DialogFooter><DialogClose asChild><button>Done</button></DialogClose></DialogFooter>
     </DialogContent>
   </Dialog>
   ```
*/

export function Dialog(props: React.ComponentProps<typeof DialogPrimitive.Root>) {
  return <DialogPrimitive.Root {...props} />;
}
Dialog.displayName = 'Dialog';

export const DialogTrigger = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Trigger>
>(function DialogTrigger(props, ref) {
  return <DialogPrimitive.Trigger ref={ref} data-slot="dialog-trigger" {...props} />;
});

export const DialogClose = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Close>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Close>
>(function DialogClose(props, ref) {
  return <DialogPrimitive.Close ref={ref} data-slot="dialog-close" {...props} />;
});

export const DialogOverlay = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(function DialogOverlay({ className, ...props }, ref) {
  return (
    <DialogPrimitive.Overlay
      ref={ref}
      data-slot="dialog-overlay"
      className={classNames('modal-container', className)}
      {...props}
    />
  );
});

export type DialogContentProps = React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content> & {
  showCloseButton?: boolean;
  hiddenTitle?: string;
  container?: HTMLElement | null;
};

const CloseGlyph = () => (
  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true" focusable="false">
    <path
      d="M4 4l8 8M12 4l-8 8"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      fill="none"
    />
  </svg>
);

export const DialogContent = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Content>,
  DialogContentProps
>(function DialogContent(
  {
    className,
    children,
    showCloseButton = true,
    hiddenTitle,
    container,
    onEscapeKeyDown,
    ...props
  },
  ref
) {
  return (
    <DialogPrimitive.Portal container={container || getPortalContainer()}>
      <DialogOverlay />
      <DialogPrimitive.Content
        ref={ref}
        data-slot="dialog-content"
        {...layerProps}
        className={classNames('modal', className)}
        onEscapeKeyDown={consumeEscape(onEscapeKeyDown)}
        {...props}
      >
        {hiddenTitle ? (
          <VisuallyHidden>
            <DialogPrimitive.Title data-slot="dialog-title">{hiddenTitle}</DialogPrimitive.Title>
          </VisuallyHidden>
        ) : null}
        {children}
        {showCloseButton ? (
          <DialogPrimitive.Close
            data-slot="dialog-close"
            className="modal-close"
            aria-label={localized('Close')}
          >
            <CloseGlyph />
          </DialogPrimitive.Close>
        ) : null}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
});

export const DialogHeader = React.forwardRef<HTMLDivElement, React.ComponentPropsWithoutRef<'div'>>(
  function DialogHeader(props, ref) {
    return <div ref={ref} data-slot="dialog-header" {...props} />;
  }
);

export const DialogFooter = React.forwardRef<HTMLDivElement, React.ComponentPropsWithoutRef<'div'>>(
  function DialogFooter(props, ref) {
    return <div ref={ref} data-slot="dialog-footer" {...props} />;
  }
);

export const DialogTitle = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(function DialogTitle(props, ref) {
  return <DialogPrimitive.Title ref={ref} data-slot="dialog-title" {...props} />;
});

export const DialogDescription = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(function DialogDescription(props, ref) {
  return <DialogPrimitive.Description ref={ref} data-slot="dialog-description" {...props} />;
});

DialogTrigger.displayName = 'DialogTrigger';
DialogClose.displayName = 'DialogClose';
DialogOverlay.displayName = 'DialogOverlay';
DialogContent.displayName = 'DialogContent';
DialogHeader.displayName = 'DialogHeader';
DialogFooter.displayName = 'DialogFooter';
DialogTitle.displayName = 'DialogTitle';
DialogDescription.displayName = 'DialogDescription';
