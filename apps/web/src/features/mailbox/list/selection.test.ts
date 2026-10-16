import { describe, expect, it } from 'vitest';
import {
  emptySelection,
  extendTo,
  moveCursor,
  nextAfterRemoval,
  prune,
  selectAll,
  toggle,
} from './selection';

const ids = ['a', 'b', 'c', 'd', 'e'];
const sorted = (s: ReadonlySet<string>) => [...s].sort();

describe('selection', () => {
  it('toggles and sets the anchor', () => {
    const s = toggle(emptySelection, 'b');
    expect(sorted(s.selected)).toEqual(['b']);
    expect(s.anchor).toBe('b');
    expect(toggle(s, 'b').selected.size).toBe(0);
  });

  it('extends a range from the anchor in either direction, keeping earlier picks', () => {
    let s = toggle(emptySelection, 'a');
    s = toggle(s, 'c');
    expect(sorted(extendTo(s, ids, 'e').selected)).toEqual(['a', 'c', 'd', 'e']);
    s = toggle(emptySelection, 'd');
    expect(sorted(extendTo(s, ids, 'b').selected)).toEqual(['b', 'c', 'd']);
  });

  it('treats shift without an anchor as a toggle', () => {
    expect(sorted(extendTo(emptySelection, ids, 'c').selected)).toEqual(['c']);
  });

  it('prunes ids that left the list and keeps the reference otherwise', () => {
    const s = selectAll(ids);
    expect(prune(s, ids)).toBe(s);
    const p = prune(s, ['a', 'e']);
    expect(sorted(p.selected)).toEqual(['a', 'e']);
    expect(p.anchor).toBe('a');
  });

  it('moves the cursor within bounds and starts at the ends', () => {
    expect(moveCursor(ids, null, 1)).toBe('a');
    expect(moveCursor(ids, null, -1)).toBe('e');
    expect(moveCursor(ids, 'a', -1)).toBe('a');
    expect(moveCursor(ids, 'c', 1)).toBe('d');
    expect(moveCursor(ids, 'e', 1)).toBe('e');
    expect(moveCursor([], 'a', 1)).toBeNull();
  });

  it('advances past removed rows, falling back to the previous one', () => {
    expect(nextAfterRemoval(ids, new Set(['b', 'c']), 'b')).toBe('d');
    expect(nextAfterRemoval(ids, new Set(['e']), 'e')).toBe('d');
    expect(nextAfterRemoval(['a'], new Set(['a']), 'a')).toBeNull();
  });
});
