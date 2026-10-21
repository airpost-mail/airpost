// Recent searches, per browser.
// Recent searches stay in this browser in v1.
import { useSyncExternalStore } from 'react';

const STORAGE_KEY = 'tern.search.recent';
const MAX = 8;
const listeners = new Set<() => void>();
let recent: string[] = load();

function load(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

function save(next: string[]) {
  recent = next;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Private mode or quota: keep the in-memory list.
  }
  for (const l of listeners) l();
}

export function addRecentSearch(query: string) {
  const q = query.trim();
  if (!q) return;
  save([q, ...recent.filter((r) => r !== q)].slice(0, MAX));
}

export function clearRecentSearches() {
  save([]);
}

export function useRecentSearches(): string[] {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => recent,
  );
}
