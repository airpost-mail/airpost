// Client-side preferences, persisted in localStorage. Theme lives in next-themes.
// These stay in this browser in v1.
import { useSyncExternalStore } from 'react';

export type Density = 'compact' | 'comfortable';
export type ReadingPanePosition = 'right' | 'bottom' | 'off';

export interface Preferences {
  density: Density;
  /** Load remote images in message bodies without asking. */
  loadRemoteImages: boolean;
  /** Mark a thread read when it is opened. False when `markReadDelayMs` is -1. */
  markReadOnOpen: boolean;
  /** Where the reading pane sits; `off` opens threads in place of the list. */
  readingPane: ReadingPanePosition;
  /** Delay before an opened thread is marked read; -1 means manually. */
  markReadDelayMs: number;
  /** How long a sent message can be undone; 0 sends at once. */
  undoSendMs: number;
}

const STORAGE_KEY = 'airpost.preferences';
// Delay defaults follow the desktop (core.reading.markAsReadDelay, core.sending.undoSend).
const DEFAULTS: Preferences = {
  density: 'compact',
  loadRemoteImages: false,
  markReadOnOpen: true,
  readingPane: 'right',
  markReadDelayMs: 500,
  undoSendMs: 5000,
};

const listeners = new Set<() => void>();
let current: Preferences = load();

function load(): Preferences {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? { ...DEFAULTS, ...(JSON.parse(raw) as Partial<Preferences>) } : DEFAULTS;
  } catch {
    return DEFAULTS;
  }
}

function apply(p: Preferences) {
  if (typeof document !== 'undefined') document.documentElement.dataset.density = p.density;
}
apply(current);

export function getPreferences(): Preferences {
  return current;
}

export function setPreferences(patch: Partial<Preferences>) {
  current = { ...current, ...patch };
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(current));
  } catch {
    // Private mode or quota: keep the in-memory value.
  }
  apply(current);
  for (const l of listeners) l();
}

export function usePreferences(): Preferences {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => current,
  );
}
