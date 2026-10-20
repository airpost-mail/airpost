import React from 'react';
import ReactDOM from 'react-dom';
import MailspringStore from 'mailspring-store';
import * as Actions from '../actions';
import _ from 'underscore';
import { UIProvider } from '../../components/kit/provider';
import { Popover, PopoverAnchor, PopoverArrow, PopoverContent } from '../../components/kit/popover';

const CONTAINER_ID = 'mailspring-popover-container';

type Direction = 'up' | 'down' | 'left' | 'right';
type OriginRect = {
  top?: number;
  left?: number;
  bottom?: number;
  right?: number;
  width?: number;
  height?: number;
};

type StorePopoverProps = {
  originRect: OriginRect;
  direction?: Direction;
  fallbackDirection?: Direction;
  closeOnAppBlur?: boolean;
  children?: React.ReactNode;
};

const SIDES = { up: 'top', down: 'bottom', left: 'left', right: 'right' } as const;

const FOCUSABLE_SELECTOR = 'input, textarea, [contenteditable], [tabIndex]';

/* What took focus when a popover opened before the kit (AutoFocuses): an [autofocus] element,
   then the lowest positive tabIndex, then the first input, then any other focusable. */
function legacyFocusTarget(root: HTMLElement): HTMLElement | null {
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

/* The popover `Actions.openPopover` opens: the UI kit's Popover anchored to `originRect`.
   `direction` is the side it opens on. Radix flips to the opposite side when that one doesn't
   fit and keeps it inside the window, so a `fallbackDirection` other than the opposite side
   isn't honoured. It closes on a click outside, Escape, focus moving elsewhere, a window resize,
   and when the app loses focus unless `closeOnAppBlur` is false. */
function StorePopover(props: StorePopoverProps) {
  const { originRect, direction = 'down', closeOnAppBlur = true, children } = props;
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
            const target = legacyFocusTarget(root);
            if (target) {
              event.preventDefault();
              target.focus();
            }
          }}
          // Focus isn't put back where it was, as before: an action in the popover may have
          // moved it on purpose.
          onCloseAutoFocus={(event) => event.preventDefault()}
        >
          {children}
          <PopoverArrow />
        </PopoverContent>
      </Popover>
    </>
  );
}

function createContainer(id) {
  const element = document.createElement(id);
  document.body.insertBefore(element, document.body.children[0]);
  return element;
}

class PopoverStore extends MailspringStore {
  isOpen = false;
  container: HTMLElement;

  constructor(containerId = CONTAINER_ID) {
    super();
    this.container = createContainer(containerId);
    ReactDOM.render(
      <UIProvider>
        <span />
      </UIProvider>,
      this.container
    );

    this.listenTo(Actions.openPopover, this.openPopover);
    this.listenTo(Actions.closePopover, this.closePopover);
  }

  renderPopover = (child, props, callback) => {
    const popover = (
      <UIProvider>
        <StorePopover {...props}>{child}</StorePopover>
      </UIProvider>
    );

    ReactDOM.render(popover, this.container, () => {
      this.isOpen = true;
      this.trigger();
      callback();
    });
  };

  openPopover = (
    element,
    { originRect, direction, fallbackDirection, closeOnAppBlur, callback = () => {} }
  ) => {
    const props = {
      direction,
      originRect,
      fallbackDirection,
      closeOnAppBlur,
    };

    if (this.isOpen) {
      this.closePopover(() => {
        this.renderPopover(element, props, callback);
      });
    } else {
      this.renderPopover(element, props, callback);
    }
  };

  closePopover = (callback = () => {}) => {
    const empty = (
      <UIProvider>
        <span />
      </UIProvider>
    );
    ReactDOM.render(empty, this.container, () => {
      this.isOpen = false;
      this.trigger();
      callback();
    });
  };
}

export default new PopoverStore();
