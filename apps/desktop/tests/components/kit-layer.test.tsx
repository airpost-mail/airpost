// SPDX-License-Identifier: GPL-3.0-only
import mousetrap from 'mousetrap';
import { consumeEscape, layerProps } from '../../src/components/kit/layer';

describe('kit layers', function () {
  describe('consumeEscape', function () {
    it('calls the handler and stops propagation, even when the handler prevents the dismiss', () => {
      const handler = jasmine.createSpy('handler').andCallFake((e) => e.preventDefault());
      const event = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true });
      consumeEscape(handler)(event);
      expect(handler).toHaveBeenCalledWith(event);
      expect(event.defaultPrevented).toBe(true);
      expect((event as any).isPropagationStopped()).toBe(true);
    });

    it('works without a handler', () => {
      const event = new KeyboardEvent('keydown', { key: 'Escape' });
      consumeEscape()(event);
      expect((event as any).isPropagationStopped()).toBe(true);
    });
  });

  describe('the keymap inside a layer', function () {
    let layer: HTMLElement;
    let inside: HTMLElement;
    let outside: HTMLElement;

    beforeEach(() => {
      layer = document.createElement('div');
      Object.entries(layerProps).forEach(([k, v]) => layer.setAttribute(k, v));
      inside = document.createElement('div');
      layer.appendChild(inside);
      outside = document.createElement('div');
      document.body.append(layer, outside);
    });

    afterEach(() => {
      layer.remove();
      outside.remove();
    });

    // stopCallback reads the event's target, so dispatch it and ask from a listener.
    const stops = (element: HTMLElement, combo: string) => {
      let result: boolean = null;
      const listener = (e: KeyboardEvent) => {
        result = mousetrap.prototype.stopCallback(e, element, combo);
      };
      element.addEventListener('keydown', listener);
      element.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true }));
      element.removeEventListener('keydown', listener);
      return result;
    };

    it('blocks plain keys and the reserved text-editing shortcuts', () => {
      for (const combo of ['down', 'up', 'enter', 'space', 'backspace', 'escape', 'e']) {
        expect(stops(inside, combo)).toBe(true);
      }
      expect(stops(inside, 'mod+a')).toBe(true);
      expect(stops(inside, 'mod+c')).toBe(true);
    });

    it('lets other mod+ shortcuts through', () => {
      expect(stops(inside, 'mod+,')).toBe(false);
      expect(stops(inside, 'mod+shift+z')).toBe(false);
    });

    it('leaves elements outside layers alone', () => {
      expect(stops(outside, 'down')).toBe(false);
      expect(stops(outside, 'backspace')).toBe(false);
    });
  });
});
