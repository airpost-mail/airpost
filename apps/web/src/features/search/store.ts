import { useSyncExternalStore } from 'react';

const listeners = new Set<() => void>();
let open = false;

export function setCommandPaletteOpen(next: boolean) {
  open = next;
  for (const l of listeners) l();
}

export function openCommandPalette() {
  setCommandPaletteOpen(true);
}

export function useCommandPaletteOpen(): boolean {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => open,
  );
}

type FocusHandler = (insert?: string) => void;
let focusBox: FocusHandler | null = null;

/** Lets the mounted <SearchBox/> receive focus requests. */
export function registerSearchBox(handler: FocusHandler): () => void {
  focusBox = handler;
  return () => {
    if (focusBox === handler) focusBox = null;
  };
}

/**
 * Focuses the top bar search box, optionally appending text (e.g. `from:`). Falls back to
 * the command palette when no box is mounted.
 */
export function focusSearchBox(insert?: string) {
  if (focusBox) focusBox(insert);
  else openCommandPalette();
}

export { openShortcutsSheet as showShortcutsSheet } from '@/components/AppShell';
