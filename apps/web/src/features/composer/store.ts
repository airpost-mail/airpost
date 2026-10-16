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
  /**
   * Replies and forwards go inline when their thread is open in the reader, else docked.
   * `inline` on an edit-draft asks for the same (a reply draft, not a standalone one).
   */
  placement?: ComposerPlacement;
  /** False opens it without taking focus (a reply draft shown when its thread opens). */
  focus?: boolean;
}

export type ComposerPlacement = 'docked' | 'inline';

export type ComposerView = 'normal' | 'minimised' | 'maximised' | 'fullscreen';

export interface ComposerWindow extends ComposeRequest {
  mode: Exclude<ComposeMode, 'replyAll'>;
  key: string;
  placement: ComposerPlacement;
  view: ComposerView;
  /** Hidden while a send can still be undone; the window stays mounted to keep its state. */
  hidden: boolean;
  /** Bumped to ask the window to take focus; 0 until something asks. */
  focusSeq: number;
  /** The id of the draft the window is editing, once its session has one. */
  editingId?: string;
}

/** Docked windows that stay open side by side; older ones minimise beyond this. */
const MAX_OPEN = 2;

const listeners = new Set<() => void>();
let windows: ComposerWindow[] = [];
let focusedKey: string | null = null;
/** Threads whose reader is on screen, with an inline composer slot (mount count). */
const inlineSlots = new Map<string, number>();
/** Drafts whose composer was closed; their thread shows them as messages until reopened. */
const dismissedInline = new Set<string>();

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
  const open = list.filter(
    (w) => w.placement === 'docked' && !w.hidden && w.view !== 'minimised' && w.key !== keep,
  );
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

/** The accessible name of a composer, by what it is writing. */
export function composerLabel(mode: ComposerWindow['mode']): string {
  return mode === 'new'
    ? 'New message'
    : mode === 'forward'
      ? 'Forward'
      : mode === 'edit-draft'
        ? 'Draft'
        : 'Reply';
}

const isReply = (mode: ComposerWindow['mode']) =>
  mode === 'reply' || mode === 'reply-all' || mode === 'forward';

export function hasInlineSlot(threadId: string | undefined): boolean {
  return !!threadId && (inlineSlots.get(threadId) ?? 0) > 0;
}

function inlineWindowFor(threadId: string): ComposerWindow | undefined {
  return windows.find((w) => w.placement === 'inline' && w.threadId === threadId && !w.hidden);
}

export function openComposer(req: ComposeRequest = { mode: 'new' }): string {
  const mode = req.mode === 'replyAll' ? 'reply-all' : req.mode;
  const next: ComposerWindow = {
    ...req,
    mode,
    key: crypto.randomUUID(),
    placement: 'docked',
    view: 'normal',
    hidden: false,
    focusSeq: req.focus === false ? 0 : 1,
  };
  if (mode === 'edit-draft' && req.draftId) dismissedInline.delete(req.draftId);
  const existing = windows.find((w) => !w.hidden && sameTarget(w, next));
  if (existing) {
    focusComposer(existing.key);
    return existing.key;
  }
  const wantsInline = req.placement === 'inline' || (req.placement === undefined && isReply(mode));
  if (wantsInline && hasInlineSlot(req.threadId)) {
    const current = inlineWindowFor(req.threadId!);
    // One inline reply per thread: asking again focuses it. A second draft of the thread
    // gets a docked window instead.
    if (current && mode !== 'edit-draft') {
      focusComposer(current.key);
      return current.key;
    }
    if (!current) next.placement = 'inline';
  }
  const others = windows.map((w) =>
    w.view === 'fullscreen' ? { ...w, view: 'normal' as const } : w,
  );
  set(next.placement === 'docked' ? limitOpen([...others, next], next.key) : [...others, next]);
  return next.key;
}

/**
 * Called by the reader's inline slot while it is mounted. When the thread is left, its
 * inline composers close (their edits were autosaved and come back as the thread's draft);
 * one whose send can still be undone becomes an edit of its saved draft, so undoing
 * reopens the saved text rather than a fresh reply.
 */
export function registerInlineSlot(threadId: string): () => void {
  inlineSlots.set(threadId, (inlineSlots.get(threadId) ?? 0) + 1);
  return () => {
    const n = (inlineSlots.get(threadId) ?? 1) - 1;
    if (n > 0) {
      inlineSlots.set(threadId, n);
      return;
    }
    inlineSlots.delete(threadId);
    const inThread = (w: ComposerWindow) => w.placement === 'inline' && w.threadId === threadId;
    if (!windows.some(inThread)) return;
    set(
      windows
        .filter((w) => !inThread(w) || w.hidden)
        .map((w) =>
          inThread(w) && w.editingId
            ? { ...w, mode: 'edit-draft' as const, draftId: w.editingId }
            : w,
        ),
    );
  };
}

/** Moves an inline composer to a docked window, optionally reopening it as a saved draft. */
export function dockComposer(key: string, draftId?: string) {
  const list = windows.map((w) =>
    w.key === key
      ? {
          ...w,
          placement: 'docked' as const,
          view: 'normal' as const,
          focusSeq: w.focusSeq + 1,
          ...(draftId ? { mode: 'edit-draft' as const, draftId } : {}),
        }
      : w,
  );
  set(limitOpen(list, key));
}

export function isInlineDraftDismissed(draftId: string): boolean {
  return dismissedInline.has(draftId);
}

export function closeComposer(key: string) {
  // Closed, discarded or sent: its thread must not reopen it inline while the engine's
  // deltas catch up. Asking to edit it again clears this.
  const draftId = windows.find((w) => w.key === key)?.editingId;
  if (draftId) dismissedInline.add(draftId);
  set(windows.filter((w) => w.key !== key));
}

/**
 * Restores a window (if minimised or hidden) and asks it to take focus. An inline window
 * whose thread is no longer on screen (an undone send after leaving it) docks instead.
 */
export function focusComposer(key: string) {
  const list = windows.map((w) =>
    w.key === key
      ? {
          ...w,
          hidden: false,
          placement:
            w.placement === 'inline' && !hasInlineSlot(w.threadId) ? 'docked' : w.placement,
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
  inlineSlots.clear();
  dismissedInline.clear();
  emit();
}
