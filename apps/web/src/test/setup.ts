import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// jsdom has no layout. Radix Switch/Select and react-resizable-panels measure with
// ResizeObserver; cmdk and lists scroll the active row into view.
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;
Element.prototype.scrollIntoView ??= () => {};
window.scrollTo = () => {};

afterEach(() => {
  cleanup();
  localStorage.clear();
});
