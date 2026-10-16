// Public API other features use to open the composer, plus the window list the host renders.
import { useSyncExternalStore } from 'react';
import type { ContactRef } from '@/engine';

/** `replyAll` is accepted as an alias of `reply-all`. */
export type ComposeMode = 'new' | 'reply' | 'reply-all' | 'replyAll' | 'forward' | 'edit-draft';

export interface ComposeRequest {
  mode: ComposeMode;
  accountId?: string;
  /** Thread being replied to or forwarded. */
  threadId?: string;
  /** Message being replied to or forwarded; defaults to the thread's last message. */
  messageId?: string;
  /** Existing draft to reopen (`edit-draft`). */
  draftId?: string;
  to?: ContactRef[];
  cc?: ContactRef[];
  bcc?: ContactRef[];
  subject?: string;
  /** Plain text. */
  body?: string;
}

export type ComposerView = 'normal' | 'minimised' | 'maximised' | 'fullscreen';

export interface ComposerWindow extends ComposeRequest {
  mode: Exclude<ComposeMode, 'replyAll'>;
  key: string;
  view: ComposerView;
  /** Hidden while a send can still be undone; the window stays mounted to keep its state. */
  hidden: boolean;
  /** Bumped to ask the window to take focus. */
  focusSeq: number;
  /** The id of the draft the window is editing, once its session has one. */
  editingId?: string;
}

/** Docked windows that stay open side by side; older ones minimise beyond this. */
const MAX_OPEN = 2;

const listeners = new Set<() => void>();
let windows: ComposerWindow[] = [];
let focusedKey: string | null = null;

function emit() {
  for (const l of listeners) l();
}

function set(next: ComposerWindow[]) {
  windows = next;
  if (focusedKey && !windows.some((w) => w.key === focusedKey && !w.hidden)) focusedKey = null;
  emit();
}

function patch(key: string, change: Partial<ComposerWindow>) {
  set(windows.map((w) => (w.key === key ? { ...w, ...change } : w)));
}

/** Keeps at most MAX_OPEN docked windows expanded, minimising the oldest others. */
function limitOpen(list: ComposerWindow[], keep: string): ComposerWindow[] {
  const open = list.filter((w) => !w.hidden && w.view !== 'minimised' && w.key !== keep);
  const excess = open.length - (MAX_OPEN - 1);
  if (excess <= 0) return list;
  const toMinimise = new Set(open.slice(0, excess).map((w) => w.key));
  return list.map((w) => (toMinimise.has(w.key) ? { ...w, view: 'minimised' } : w));
}

function sameTarget(w: ComposerWindow, req: ComposerWindow): boolean {
  // Whatever mode opened it: two editors on one draft would overwrite each other's saves.
  if (req.mode === 'edit-draft') {
    return (w.mode === 'edit-draft' && w.draftId === req.draftId) || w.editingId === req.draftId;
  }
  if (req.mode === 'new' || !req.threadId) return false;
  return w.mode === req.mode && w.threadId === req.threadId && w.messageId === req.messageId;
}

export function openComposer(req: ComposeRequest = { mode: 'new' }): string {
  const mode = req.mode === 'replyAll' ? 'reply-all' : req.mode;
  const next: ComposerWindow = {
    ...req,
    mode,
    key: crypto.randomUUID(),
    view: 'normal',
    hidden: false,
    focusSeq: 1,
  };
  const existing = windows.find((w) => !w.hidden && sameTarget(w, next));
  if (existing) {
    focusComposer(existing.key);
    return existing.key;
  }
  const others = windows.map((w) =>
    w.view === 'fullscreen' ? { ...w, view: 'normal' as const } : w,
  );
  set(limitOpen([...others, next], next.key));
  return next.key;
}

export function closeComposer(key: string) {
  set(windows.filter((w) => w.key !== key));
}

/** Restores a window (if minimised or hidden) and asks it to take focus. */
export function focusComposer(key: string) {
  const list = windows.map((w) =>
    w.key === key
      ? {
          ...w,
          hidden: false,
          view: w.view === 'minimised' ? ('normal' as const) : w.view,
          focusSeq: w.focusSeq + 1,
        }
      : w,
  );
  set(limitOpen(list, key));
}

export function setComposerView(key: string, view: ComposerView) {
  if (view === 'fullscreen') {
    set(
      windows.map((w) =>
        w.key === key ? { ...w, view } : w.view === 'fullscreen' ? { ...w, view: 'normal' } : w,
      ),
    );
    return;
  }
  if (view === 'minimised') {
    patch(key, { view });
    return;
  }
  set(
    limitOpen(
      windows.map((w) => (w.key === key ? { ...w, view } : w)),
      key,
    ),
  );
}

export function setComposerEditingId(key: string, id: string) {
  if (windows.some((w) => w.key === key && w.editingId !== id)) patch(key, { editingId: id });
}

export function setComposerHidden(key: string, hidden: boolean) {
  patch(key, { hidden });
}

export function setFocusedComposer(key: string | null) {
  if (focusedKey === key) return;
  focusedKey = key;
  emit();
}

export function getFocusedComposer(): string | null {
  return focusedKey;
}

export function getComposerWindows(): ComposerWindow[] {
  return windows;
}

function subscribe(cb: () => void) {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

export function useComposerWindows(): ComposerWindow[] {
  return useSyncExternalStore(subscribe, () => windows);
}

export function useFocusedComposer(): string | null {
  return useSyncExternalStore(subscribe, () => focusedKey);
}

export function useComposer() {
  const list = useComposerWindows();
  return { windows: list, open: openComposer, close: closeComposer, focus: focusComposer };
}

/** For tests. */
export function resetComposers() {
  windows = [];
  focusedKey = null;
  emit();
}
