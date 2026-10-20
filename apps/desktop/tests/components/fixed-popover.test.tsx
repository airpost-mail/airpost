// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import FixedPopover from '../../src/components/fixed-popover';
import * as Actions from '../../src/data/actions';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from '../support/kit-helpers';
import { trackCommands } from '../support/kit-layer-helpers';

const {
  Directions: { Up, Down, Left, Right },
} = FixedPopover;

const content = () =>
  getPortalContainer().querySelector('[data-slot="popover-content"]') as HTMLElement;

const windowSize = () => ({
  width: document.documentElement.clientWidth,
  height: document.documentElement.clientHeight,
});

// Placement resolves asynchronously in floating-ui.
const placed = () => wait(50);

describe('FixedPopover on the UI kit', function fixedPopover() {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
    spyOn(Actions, 'closePopover');
  });

  afterEach(() => cleanup());

  const open = (
    props: any = {},
    body = <div className="body" style={{ width: 120, height: 60 }} />
  ) =>
    render(
      <FixedPopover originRect={{ top: 200, left: 200, width: 40, height: 20 }} {...props}>
        {body}
      </FixedPopover>
    );

  it('renders the content with the legacy classes and a blur trap over the rect', () => {
    open();
    const el = content();
    expect(el.className).toBe('fixed-popover');
    expect(el.getAttribute('data-variant')).toBe('bare');
    expect(el.hasAttribute('data-ui-layer')).toBe(true);
    expect(el.querySelector('.body')).not.toBe(null);
    expect(el.querySelector('.fixed-popover-pointer')).not.toBe(null);
    const trap = document.querySelector<HTMLElement>('.fixed-popover-blur-trap');
    expect([trap.style.top, trap.style.left, trap.style.width, trap.style.height]).toEqual([
      '200px',
      '200px',
      '40px',
      '20px',
    ]);
  });

  it('takes the rect from bottom and right when width and height are missing', () => {
    open({ originRect: { top: 10, left: 20, bottom: 40, right: 70 } });
    const trap = document.querySelector<HTMLElement>('.fixed-popover-blur-trap');
    expect([trap.style.width, trap.style.height]).toEqual(['50px', '30px']);
  });

  [
    [Up, 'top'],
    [Down, 'bottom'],
    [Left, 'left'],
    [Right, 'right'],
  ].forEach(([direction, side]) => {
    it(`opens on the ${side} side when the direction is ${direction}, centred on the rect`, async () => {
      open({ direction });
      await placed();
      const el = content();
      expect(el.getAttribute('data-side')).toBe(side);
      expect(el.getAttribute('data-align')).toBe('center');
      const box = el.getBoundingClientRect();
      if (side === 'top' || side === 'bottom') {
        expect(Math.round(box.left + box.width / 2)).toBe(220);
      } else {
        expect(Math.round(box.top + box.height / 2)).toBe(210);
      }
    });
  });

  it('opens down when no direction is given', async () => {
    open();
    await placed();
    expect(content().getAttribute('data-side')).toBe('bottom');
  });

  // The rect sits against the window edge on the side asked for (body 120x60).
  const edgeRects = () => {
    const { width, height } = windowSize();
    return {
      [Up]: { top: 5, left: 200, width: 40, height: 20 },
      [Down]: { top: height - 25, left: 200, width: 40, height: 20 },
      [Left]: { top: 200, left: 5, width: 40, height: 20 },
      [Right]: { top: 200, left: width - 45, width: 40, height: 20 },
    };
  };

  [
    [Up, 'bottom'],
    [Down, 'top'],
    [Left, 'right'],
    [Right, 'left'],
  ].forEach(([direction, flipped]) => {
    it(`flips to the ${flipped} side when ${direction} has no room`, async () => {
      open({ direction, originRect: edgeRects()[direction] });
      await placed();
      expect(content().getAttribute('data-side')).toBe(flipped);
    });
  });

  // A rect against one window edge, so the body centred on it overflows that edge.
  const rectAgainst = (edge: string) => {
    const { width, height } = windowSize();
    return {
      right: { top: 100, left: width - 30, width: 20, height: 20 },
      left: { top: 200, left: 5, width: 20, height: 20 },
      bottom: { top: height - 30, left: 100, width: 20, height: 20 },
      top: { top: 5, left: 300, width: 20, height: 20 },
    }[edge];
  };

  [
    [Down, 'right'],
    [Up, 'left'],
    [Right, 'bottom'],
    [Left, 'top'],
  ].forEach(([direction, edge]) => {
    it(`keeps the ${direction} side and slides in from the ${edge} edge`, async () => {
      const { width, height } = windowSize();
      open({ direction, originRect: rectAgainst(edge) });
      await placed();
      const el = content();
      expect(el.getAttribute('data-side')).toBe(
        { up: 'top', down: 'bottom', left: 'left', right: 'right' }[direction]
      );
      const box = el.getBoundingClientRect();
      expect(box.left).toBeGreaterThan(-1);
      expect(box.top).toBeGreaterThan(-1);
      expect(box.right).toBeLessThan(width + 1);
      expect(box.bottom).toBeLessThan(height + 1);
    });
  });

  it('keeps its start edge inside the window when it is taller than the room beside the rect', async () => {
    const { height } = windowSize();
    open(
      { direction: Right, originRect: { top: height - 40, left: 20, width: 40, height: 20 } },
      <div style={{ width: 100, height: height + 200 }} />
    );
    await placed();
    expect(content().getBoundingClientRect().top).toBeGreaterThan(-1);
  });

  describe('when the content changes size after placement', () => {
    // The spec window is hidden, and a hidden renderer produces frames irregularly, so a real
    // ResizeObserver may not report within any timeout. This one reports when the spec says so.
    let observers: FakeResizeObserver[];
    class FakeResizeObserver {
      observed = new Set<Element>();
      constructor(public callback: ResizeObserverCallback) {
        observers.push(this);
      }
      observe(el: Element) {
        this.observed.add(el);
      }
      unobserve(el: Element) {
        this.observed.delete(el);
      }
      disconnect() {
        this.observed.clear();
      }
    }
    const reportResizes = () => {
      for (const o of observers) {
        for (const target of o.observed) {
          o.callback([{ target } as ResizeObserverEntry], o as any);
        }
      }
    };

    let RealResizeObserver: typeof ResizeObserver;
    beforeEach(() => {
      observers = [];
      RealResizeObserver = window.ResizeObserver;
      (window as any).ResizeObserver = FakeResizeObserver;
    });
    afterEach(() => {
      window.ResizeObserver = RealResizeObserver;
    });

    it('places it again so it comes back inside the window', async () => {
      const { height } = windowSize();
      open(
        { direction: Right, originRect: { top: height - 60, left: 20, width: 40, height: 20 } },
        <div className="grows" style={{ width: 100, height: 40 }} />
      );
      await placed();
      const before = content().getBoundingClientRect();
      expect(before.bottom).toBeLessThan(height + 1);

      content().querySelector<HTMLElement>('.grows').style.height = `${height - 100}px`;
      reportResizes();
      await placed();
      const after = content().getBoundingClientRect();
      expect(after.top).toBeLessThan(before.top);
      expect(after.top).toBeGreaterThan(-1);
      expect(after.bottom).toBeLessThan(height + 1);
    });
  });

  it('focuses the first field, or an [autofocus] one', async () => {
    open(
      {},
      <div>
        <button>Plain</button>
        <input className="field" />
      </div>
    );
    await wait(20);
    expect(document.activeElement).toBe(content().querySelector('.field'));
    cleanup();

    open(
      {},
      <div>
        <input className="first" />
        <input className="chosen" autoFocus />
      </div>
    );
    await wait(20);
    expect(document.activeElement).toBe(content().querySelector('.chosen'));
  });

  describe('closing', function () {
    const commands = trackCommands();

    it('closes on Escape without popping the sheet', async () => {
      open({}, <input className="field" />);
      await wait(20);
      await pressRealKey('Escape');
      expect(Actions.closePopover).toHaveBeenCalled();
      expect(commands.count('core:pop-sheet')).toBe(0);
    });

    it('closes on a window resize', () => {
      open();
      window.dispatchEvent(new Event('resize'));
      expect(Actions.closePopover).toHaveBeenCalled();
    });

    it('closes when the app loses focus, unless closeOnAppBlur is false', () => {
      open({ closeOnAppBlur: false });
      window.dispatchEvent(new Event('blur'));
      expect(Actions.closePopover).not.toHaveBeenCalled();
      cleanup();
      open();
      window.dispatchEvent(new Event('blur'));
      expect(Actions.closePopover).toHaveBeenCalled();
    });

    it('closes on a click outside, and not on a click inside', async () => {
      open();
      await wait(10);
      fireEvent.pointerDown(content().querySelector('.body'));
      expect(Actions.closePopover).not.toHaveBeenCalled();
      fireEvent.pointerDown(document.body);
      fireEvent.click(document.body);
      expect(Actions.closePopover).toHaveBeenCalled();
    });
  });
});
