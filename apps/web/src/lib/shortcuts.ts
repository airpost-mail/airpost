// Global keyboard shortcut registry. Features register into it; the shell installs one
// keydown listener and renders the `?` sheet from `useShortcutList()`.
//
// Key syntax: chords are `+`-joined (`mod+k`, `shift+j`), sequences are space-separated
// (`g i`). `mod` is Cmd on macOS and Ctrl elsewhere. Punctuation that needs Shift on most
// layouts (`#`, `?`, `!`) is matched by the character, so write `#`, not `shift+3`.
import { useEffect, useRef, useSyncExternalStore } from 'react';

export type ShortcutGroup =
  'Navigation' | 'Thread' | 'Compose' | 'Calendar' | 'Search' | 'Application';

export interface ShortcutDef {
  /** One binding or several alternatives, e.g. `['j', 'down']`. */
  keys: string | string[];
  description: string;
  group?: ShortcutGroup;
  handler: (e: KeyboardEvent) => void;
  /** Checked at key time; return false to let the key through. */
  enabled?: () => boolean;
  /** Fire while focus is in an input, textarea or contenteditable. */
  allowInInputs?: boolean;
  /**
   * Only fires while this scope is on top of the scope stack. Default `global`, which is
   * active when no scope is pushed. `*` fires in every scope.
   */
  scope?: string;
  /** Leave out of the `?` sheet. */
  hidden?: boolean;
}

interface Registered extends ShortcutDef {
  id: number;
  sequences: string[][];
}

const SEQUENCE_TIMEOUT_MS = 1000;

export const isMac =
  typeof navigator !== 'undefined' &&
  /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

const KEY_ALIASES: Record<string, string> = {
  arrowup: 'up',
  arrowdown: 'down',
  arrowleft: 'left',
  arrowright: 'right',
  esc: 'escape',
  ' ': 'space',
  spacebar: 'space',
  del: 'delete',
  return: 'enter',
};

const MODIFIER_ORDER = ['ctrl', 'alt', 'shift', 'meta'] as const;

function normalizeKey(key: string): string {
  const k = key.toLowerCase();
  return KEY_ALIASES[k] ?? k;
}

/** Turns `Mod+Shift+K` into a canonical chord such as `shift+meta+k`. */
export function normalizeChord(chord: string): string {
  const parts = chord.split('+').filter(Boolean);
  const key = normalizeKey(parts.pop() ?? '');
  const mods = new Set(
    parts.map((p) => {
      const m = p.toLowerCase();
      if (m === 'mod') return isMac ? 'meta' : 'ctrl';
      if (m === 'cmd' || m === 'command') return 'meta';
      if (m === 'control') return 'ctrl';
      if (m === 'option') return 'alt';
      return m;
    }),
  );
  return [...MODIFIER_ORDER.filter((m) => mods.has(m)), key].join('+');
}

function isLetterOrDigit(key: string) {
  return /^[a-z0-9]$/.test(key);
}

/** The canonical chord for a keydown event. */
export function chordFromEvent(e: KeyboardEvent): string {
  const key = normalizeKey(e.key);
  const mods: string[] = [];
  if (e.ctrlKey) mods.push('ctrl');
  if (e.altKey) mods.push('alt');
  // Shift is part of the character for punctuation (`?`, `#`), so it is not a modifier there.
  if (e.shiftKey && (isLetterOrDigit(key) || key.length > 1)) mods.push('shift');
  if (e.metaKey) mods.push('meta');
  return [...mods, key].join('+');
}

function parseSequences(keys: string | string[]): string[][] {
  return (Array.isArray(keys) ? keys : [keys]).map((k) =>
    k.trim().split(/\s+/).map(normalizeChord),
  );
}

let nextId = 1;
let registry: Registered[] = [];
let snapshot: ShortcutDef[] = [];
const listeners = new Set<() => void>();
const scopeStack: string[] = [];

function changed() {
  snapshot = registry.filter((r) => !r.hidden);
  for (const l of listeners) l();
}

export function registerShortcut(def: ShortcutDef): () => void {
  const entry: Registered = { ...def, id: nextId++, sequences: parseSequences(def.keys) };
  registry = [...registry, entry];
  changed();
  return () => {
    registry = registry.filter((r) => r.id !== entry.id);
    changed();
  };
}

export function registerShortcuts(defs: ShortcutDef[]): () => void {
  const offs = defs.map(registerShortcut);
  return () => offs.forEach((off) => off());
}

/** Makes `scope` the active scope until the returned function is called. */
export function pushShortcutScope(scope: string): () => void {
  scopeStack.push(scope);
  return () => {
    const i = scopeStack.lastIndexOf(scope);
    if (i !== -1) scopeStack.splice(i, 1);
  };
}

export function activeShortcutScope(): string {
  return scopeStack[scopeStack.length - 1] ?? 'global';
}

function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (tag === 'INPUT') {
    const type = (target as HTMLInputElement).type;
    return !['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color'].includes(type);
  }
  return false;
}

const ACTIVATABLE =
  'button, a[href], summary, select, [role=button], [role=link], [role=option], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=checkbox], [role=radio], [role=switch], [role=tab], [role=combobox]';

/** Enter and Space on a focused control belong to that control, not to a shortcut. */
function activatesControl(chord: string, target: EventTarget | null): boolean {
  if (chord !== 'enter' && chord !== 'space') return false;
  return target instanceof Element && !!target.closest(ACTIVATABLE);
}

/** Menus, popovers and sheets that don't push a scope of their own. */
const OVERLAY = '[role=dialog], [role=alertdialog], [role=menu], [role=listbox]';

/**
 * Global shortcuts act on the page behind any overlay, so keys pressed inside one (Radix
 * menu typeahead, a popover's buttons) must not reach them.
 */
function inOverlay(scope: string, target: EventTarget | null): boolean {
  return scope === 'global' && target instanceof Element && !!target.closest(OVERLAY);
}

function endsWith(buffer: string[], seq: string[]) {
  if (seq.length > buffer.length) return false;
  const offset = buffer.length - seq.length;
  return seq.every((c, i) => buffer[offset + i] === c);
}

function isPrefixOfAny(buffer: string[], candidates: Registered[]) {
  return candidates.some((r) =>
    r.sequences.some((s) => s.length > buffer.length && buffer.every((c, i) => s[i] === c)),
  );
}

/** Installs the single keydown listener. Returns an uninstall function. */
export function installShortcuts(target: Window | Document = window): () => void {
  let buffer: string[] = [];
  let lastAt = 0;

  const onKeyDown = (event: Event) => {
    const e = event as KeyboardEvent;
    if (e.defaultPrevented || e.isComposing || e.key === 'Unidentified') return;
    if (['Shift', 'Control', 'Alt', 'Meta'].includes(e.key)) return;

    const now = Date.now();
    if (now - lastAt > SEQUENCE_TIMEOUT_MS) buffer = [];
    lastAt = now;
    const chord = chordFromEvent(e);
    buffer = [...buffer, chord].slice(-4);

    const scope = activeShortcutScope();
    const editable = isEditable(e.target) || activatesControl(chord, e.target);
    const overlay = inOverlay(scope, e.target);
    const candidates = registry.filter((r) =>
      r.scope === '*'
        ? !editable || r.allowInInputs
        : (r.scope ?? 'global') === scope && !overlay && (!editable || r.allowInInputs),
    );

    // Newest registration wins, longest sequence first.
    let best: Registered | null = null;
    let bestLen = 0;
    for (let i = candidates.length - 1; i >= 0; i--) {
      const r = candidates[i];
      if (r.enabled && !r.enabled()) continue;
      for (const seq of r.sequences) {
        if (seq.length > bestLen && endsWith(buffer, seq)) {
          best = r;
          bestLen = seq.length;
        }
      }
    }

    if (best) {
      buffer = [];
      e.preventDefault();
      best.handler(e);
      return;
    }
    if (!isPrefixOfAny(buffer, candidates)) buffer = buffer.slice(-1);
  };

  target.addEventListener('keydown', onKeyDown);
  return () => target.removeEventListener('keydown', onKeyDown);
}

/** Registers a shortcut for the component's lifetime. The handler may change every render. */
export function useShortcut(
  keys: string | string[],
  handler: (e: KeyboardEvent) => void,
  opts: Omit<ShortcutDef, 'keys' | 'handler'> & { disabled?: boolean } = { description: '' },
) {
  const handlerRef = useRef(handler);
  const enabledRef = useRef(opts.enabled);
  useEffect(() => {
    handlerRef.current = handler;
    enabledRef.current = opts.enabled;
  });
  const keyId = Array.isArray(keys) ? keys.join('|') : keys;
  const { description, group, allowInInputs, scope, hidden, disabled } = opts;
  useEffect(() => {
    if (disabled) return;
    return registerShortcut({
      keys: keyId.split('|'),
      description,
      group,
      allowInInputs,
      scope,
      hidden,
      handler: (e) => handlerRef.current(e),
      enabled: () => enabledRef.current?.() ?? true,
    });
  }, [keyId, description, group, allowInInputs, scope, hidden, disabled]);
}

/** Pushes a scope while `active` is true (e.g. while a dialog is open). */
export function useShortcutScope(scope: string, active = true) {
  useEffect(() => (active ? pushShortcutScope(scope) : undefined), [scope, active]);
}

/** Registered, visible shortcuts, for the `?` sheet. */
export function useShortcutList(): ShortcutDef[] {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => snapshot,
  );
}

const DISPLAY: Record<string, string> = isMac
  ? {
      meta: '⌘',
      ctrl: '⌃',
      alt: '⌥',
      shift: '⇧',
      enter: '↵',
      escape: 'Esc',
      up: '↑',
      down: '↓',
      left: '←',
      right: '→',
      backspace: '⌫',
      delete: '⌦',
      space: 'Space',
      tab: '⇥',
    }
  : {
      meta: 'Win',
      ctrl: 'Ctrl',
      alt: 'Alt',
      shift: 'Shift',
      enter: 'Enter',
      escape: 'Esc',
      up: '↑',
      down: '↓',
      left: '←',
      right: '→',
      backspace: 'Backspace',
      delete: 'Del',
      space: 'Space',
      tab: 'Tab',
    };

/** Display tokens for one binding: `mod+k` -> `['⌘', 'K']`, `g i` -> `['G', 'then', 'I']`. */
export function formatShortcut(keys: string): string[] {
  const out: string[] = [];
  keys
    .trim()
    .split(/\s+/)
    .forEach((chord, i) => {
      if (i > 0) out.push('then');
      for (const part of normalizeChord(chord).split('+')) {
        out.push(DISPLAY[part] ?? (part.length === 1 ? part.toUpperCase() : part));
      }
    });
  return out;
}

/** For tests. */
export function resetShortcuts() {
  registry = [];
  scopeStack.length = 0;
  changed();
}
