// A tiny localStorage-backed store for composer settings and templates.
import { useSyncExternalStore } from 'react';

export interface LocalStore<T> {
  get(): T;
  set(next: T): void;
  use(): T;
}

export function localStore<T>(key: string, fallback: T): LocalStore<T> {
  const listeners = new Set<() => void>();
  let value: T = read();

  function read(): T {
    try {
      const raw = localStorage.getItem(key);
      return raw ? (JSON.parse(raw) as T) : fallback;
    } catch {
      return fallback;
    }
  }

  const subscribe = (cb: () => void) => {
    listeners.add(cb);
    return () => {
      listeners.delete(cb);
    };
  };

  return {
    get: () => value,
    set(next) {
      value = next;
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        // Private mode or quota: keep the in-memory value.
      }
      for (const l of listeners) l();
    },
    use: () => useSyncExternalStore(subscribe, () => value),
  };
}
