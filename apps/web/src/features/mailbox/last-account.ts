// What the sidebar shows: one account's folders, or `all` (unified views plus every
// account's tree). Chosen in the account switcher, not derived from the URL, so opening
// an account's folder from the unified sidebar doesn't switch the sidebar to that account.
import { useSyncExternalStore } from 'react';

const KEY = 'airpost.mailbox.account';
const listeners = new Set<() => void>();

export function getLastAccount(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export function setLastAccount(id: string) {
  try {
    localStorage.setItem(KEY, id);
  } catch {
    // Not persisted; the first account is a fine fallback.
  }
  listeners.forEach((l) => l());
}

export function useLastAccount(): string | null {
  return useSyncExternalStore((l) => {
    listeners.add(l);
    return () => listeners.delete(l);
  }, getLastAccount);
}
