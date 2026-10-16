import { useSyncExternalStore } from 'react';

let open = false;
const listeners = new Set<() => void>();

export function setShortcutsSheetOpen(next: boolean) {
  if (open === next) return;
  open = next;
  for (const l of listeners) l();
}

/** Opens the `?` sheet from anywhere, e.g. the command palette. */
export function openShortcutsSheet() {
  setShortcutsSheetOpen(true);
}

export function useShortcutsSheetOpen(): boolean {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => open,
  );
}
