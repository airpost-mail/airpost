// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
One listener on a store for any number of subscribers. BaseStore warns past 250 listeners
(data/stores/base.ts), and the thread list mounts a few components per visible row that each
follow ComponentRegistry or AccountStore, so a tall window passed it.
*/
type Listenable = { listen(callback: (...args: any[]) => void): () => void };

export function sharedListener(store: Listenable) {
  const callbacks = new Set<() => void>();
  let unlisten: (() => void) | null = null;
  return (callback: () => void) => {
    callbacks.add(callback);
    if (!unlisten) {
      unlisten = store.listen(() => {
        for (const cb of [...callbacks]) {
          // One that unsubscribed while an earlier one ran isn't called.
          if (callbacks.has(cb)) cb();
        }
      });
    }
    return () => {
      callbacks.delete(callback);
      if (callbacks.size === 0 && unlisten) {
        unlisten();
        unlisten = null;
      }
    };
  };
}
