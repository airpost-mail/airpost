import _ from 'underscore';
import React from 'react';

import * as Actions from '../data/actions';
import { Popover, PopoverAnchor, PopoverArrow, PopoverContent } from './kit/popover';

enum Direction {
  Up = 'up',
  Down = 'down',
  Left = 'left',
  Right = 'right',
}

type FixedPopoverProps = {
  direction?: Direction | `${Direction}`;
  fallbackDirection?: Direction | `${Direction}`;
  closeOnAppBlur?: boolean;
  originRect?: {
    bottom?: number;
    top?: number;
    right?: number;
    left?: number;
    height?: number;
    width?: number;
  };
  /** Accepted for compatibility and not called: the popover focuses its first field itself. */
  focusElementWithTabIndex?: (...args: any[]) => any;
  children?: React.ReactNode;
};

const SIDES = { up: 'top', down: 'bottom', left: 'left', right: 'right' } as const;

const FOCUSABLE_SELECTOR = 'input, textarea, [contenteditable], [tabIndex]';

/* The element that takes focus when the popover opens: an [autofocus] element, then the lowest
   positive tabIndex, then the first input, then any other focusable. */
function initialFocusTarget(root: HTMLElement): HTMLElement | null {
  const autoFocus = root.querySelector<HTMLElement>('[autofocus]');
  if (autoFocus) return autoFocus;
  const focusable = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
  const sorted = _.sortBy(focusable, (node) => {
    if (node.tabIndex > 0) return node.tabIndex;
    if (node.nodeName === 'INPUT') return 1000000;
    return 1000001;
  });
  return sorted[0] || null;
}

/* Public: A popover next to a rect in the window, as `Actions.openPopover(element, { originRect,
   direction, fallbackDirection, closeOnAppBlur })` shows it. The UI kit's Popover anchored to
   `originRect`, with the legacy `fixed-popover` (content), `fixed-popover-pointer` (arrow) and
   `fixed-popover-blur-trap` classes.

   `direction` is the side it opens on, centred on the rect. Radix flips to the opposite side when
   that one doesn't fit and slides it along the other axis to keep it inside the window, also
   after its content changes size, so a `fallbackDirection` other than the opposite side isn't
   honoured. It closes (`Actions.closePopover()`) on a click outside, Escape, a window resize, and
   when the app loses focus unless `closeOnAppBlur` is false. The Escape that closes it doesn't
   reach the keymap, so it doesn't also pop the sheet. */
function FixedPopover(props: FixedPopoverProps) {
  const { originRect = {}, direction = Direction.Down, closeOnAppBlur = true, children } = props;
  const close = () => Actions.closePopover();

  React.useEffect(() => {
    const onBlur = () => closeOnAppBlur && close();
    window.addEventListener('resize', close);
    window.addEventListener('blur', onBlur);
    return () => {
      window.removeEventListener('resize', close);
      window.removeEventListener('blur', onBlur);
    };
  }, [closeOnAppBlur]);

  const top = originRect.top || 0;
  const left = originRect.left || 0;
  const width = originRect.width ?? (originRect.right ?? left) - left;
  const height = originRect.height ?? (originRect.bottom ?? top) - top;
  const anchor = React.useMemo(
    () => ({ current: { getBoundingClientRect: () => new DOMRect(left, top, width, height) } }),
    [left, top, width, height]
  );

  return (
    <>
      {/* Covers the element that opened the popover, so the click that closes the popover
          doesn't reach it and open the popover again. */}
      <div className="fixed-popover-blur-trap" style={{ top, left, width, height }} />
      <Popover open onOpenChange={(open) => !open && close()}>
        <PopoverAnchor virtualRef={anchor} />
        <PopoverContent
          data-variant="bare"
          side={SIDES[direction] || 'bottom'}
          sticky="always"
          onOpenAutoFocus={(event) => {
            const root = event.currentTarget as HTMLElement;
            if (root.contains(document.activeElement)) return;
            const target = initialFocusTarget(root);
            if (target) {
              event.preventDefault();
              target.focus();
            }
          }}
          // Focus isn't put back where it was: an action in the popover may have moved it on
          // purpose.
          onCloseAutoFocus={(event) => event.preventDefault()}
        >
          {children}
          <PopoverArrow />
        </PopoverContent>
      </Popover>
    </>
  );
}
FixedPopover.displayName = 'FixedPopover';
FixedPopover.Directions = Direction;

export default FixedPopover;
