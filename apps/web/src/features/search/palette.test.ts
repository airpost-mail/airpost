import { describe, expect, it } from 'vitest';
import { filterCommands, matchScore, type PaletteCommand } from './palette';

const cmd = (label: string, extra: Partial<PaletteCommand> = {}): PaletteCommand => ({
  id: label,
  group: 'Actions',
  label,
  run: () => {},
  ...extra,
});

describe('matchScore', () => {
  it('ranks label prefixes over word prefixes over substrings', () => {
    expect(matchScore('set', 'Settings')).toBe(3);
    expect(matchScore('dark', 'Switch to dark theme')).toBe(2);
    expect(matchScore('ark', 'Switch to dark theme')).toBe(1);
    expect(matchScore('zzz', 'Settings')).toBe(0);
  });

  it('matches keywords by word prefix', () => {
    expect(matchScore('work inbox', 'Inbox', ['Work', 'ana@work.example'])).toBe(2);
  });

  it('matches everything for an empty query', () => {
    expect(matchScore('  ', 'Anything')).toBe(1);
  });
});

describe('filterCommands', () => {
  const list = [
    cmd('Compose'),
    cmd('Switch to dark theme'),
    cmd('Receipts', { group: 'Go to', onlyWhenSearching: true }),
    cmd('Settings', { keywords: ['preferences'] }),
  ];

  it('hides search-only commands until the user types', () => {
    expect(filterCommands(list, '').map((c) => c.label)).toEqual([
      'Compose',
      'Switch to dark theme',
      'Settings',
    ]);
    expect(filterCommands(list, 'rec').map((c) => c.label)).toEqual(['Receipts']);
  });

  it('sorts best match first', () => {
    expect(filterCommands(list, 'pref').map((c) => c.label)).toEqual(['Settings']);
    expect(filterCommands(list, 't').map((c) => c.label)).toEqual([
      'Switch to dark theme',
      'Receipts',
      'Settings',
    ]);
  });
});
