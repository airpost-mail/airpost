// Checkbox selection and the keyboard cursor of the thread list. Pure functions over ids
// so the rules (Gmail-style ranges, pruning on removal) are testable without React.
import { useCallback, useMemo, useState } from 'react';

export interface Selection {
  selected: ReadonlySet<string>;
  /** Where the last plain toggle happened; shift-click extends from here. */
  anchor: string | null;
}

export const emptySelection: Selection = { selected: new Set(), anchor: null };

export function toggle(s: Selection, id: string): Selection {
  const selected = new Set(s.selected);
  if (selected.has(id)) selected.delete(id);
  else selected.add(id);
  return { selected, anchor: id };
}

/** Adds the range between the anchor and `id` (inclusive) to the selection. */
export function extendTo(s: Selection, ids: readonly string[], id: string): Selection {
  const to = ids.indexOf(id);
  if (to === -1) return s;
  const from = s.anchor ? ids.indexOf(s.anchor) : -1;
  if (from === -1) return toggle(s, id);
  const [lo, hi] = from < to ? [from, to] : [to, from];
  const selected = new Set(s.selected);
  for (let i = lo; i <= hi; i++) selected.add(ids[i]);
  return { selected, anchor: s.anchor };
}

export function selectAll(ids: readonly string[]): Selection {
  return { selected: new Set(ids), anchor: ids[0] ?? null };
}

/** Drops ids that left the list (archived, moved), keeping the reference when nothing did. */
export function prune(s: Selection, ids: readonly string[]): Selection {
  const present = new Set(ids);
  if ([...s.selected].every((id) => present.has(id))) return s;
  return {
    selected: new Set([...s.selected].filter((id) => present.has(id))),
    anchor: s.anchor && present.has(s.anchor) ? s.anchor : null,
  };
}

/** The id `delta` rows from `cursor`, clamped to the list. No cursor starts at an end. */
export function moveCursor(
  ids: readonly string[],
  cursor: string | null,
  delta: number,
): string | null {
  if (ids.length === 0) return null;
  const at = cursor ? ids.indexOf(cursor) : -1;
  if (at === -1) return delta > 0 ? ids[0] : ids[ids.length - 1];
  return ids[Math.min(ids.length - 1, Math.max(0, at + delta))];
}

/** Where the cursor goes when `removed` leave the list: the next survivor, else the previous. */
export function nextAfterRemoval(
  ids: readonly string[],
  removed: ReadonlySet<string>,
  current: string | null,
): string | null {
  const at = current ? ids.indexOf(current) : -1;
  if (at === -1) return null;
  for (let i = at + 1; i < ids.length; i++) if (!removed.has(ids[i])) return ids[i];
  for (let i = at - 1; i >= 0; i--) if (!removed.has(ids[i])) return ids[i];
  return null;
}

export function useListSelection(ids: readonly string[]) {
  const [raw, setRaw] = useState<Selection>(emptySelection);
  const selection = useMemo(() => prune(raw, ids), [raw, ids]);

  const update = useCallback(
    (fn: (s: Selection) => Selection) => setRaw((s) => fn(prune(s, ids))),
    [ids],
  );

  return {
    selected: selection.selected,
    anchor: selection.anchor,
    toggle: useCallback((id: string) => update((s) => toggle(s, id)), [update]),
    extendTo: useCallback((id: string) => update((s) => extendTo(s, ids, id)), [update, ids]),
    selectAll: useCallback(() => setRaw(selectAll(ids)), [ids]),
    clear: useCallback(() => setRaw(emptySelection), []),
    setAnchor: useCallback((id: string) => update((s) => ({ ...s, anchor: id })), [update]),
  };
}
