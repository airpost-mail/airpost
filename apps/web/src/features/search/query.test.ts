import { describe, expect, it } from 'vitest';
import {
  activeOperator,
  editSearchQuery,
  parseDateValue,
  parseSearchQuery,
  searchLayout,
  serializeSearchQuery,
  toEngineQuery,
  toggleToken,
  withToken,
  withoutTerm,
  withoutToken,
  type ParsedSearch,
} from './query';

const NOW = new Date(2026, 9, 16, 15, 30);

describe('parseSearchQuery', () => {
  it('splits free text and operators', () => {
    expect(parseSearchQuery('quarterly from:ana@tern.dev is:unread')).toEqual({
      terms: ['quarterly'],
      tokens: [
        { field: 'from', value: 'ana@tern.dev' },
        { field: 'is', value: 'unread' },
      ],
    });
  });

  it('handles every supported operator', () => {
    const q = parseSearchQuery(
      'to:bo subject:invoice has:attachment is:starred in:inbox before:2026-01-01 after:2025-06-01',
    );
    expect(q.terms).toEqual([]);
    expect(q.tokens.map((t) => t.field)).toEqual([
      'to',
      'subject',
      'has',
      'is',
      'in',
      'before',
      'after',
    ]);
  });

  it('keeps quoted phrases and quoted operator values whole', () => {
    expect(parseSearchQuery('"board meeting" from:"Ana Lima" in:"Project X"')).toEqual({
      terms: ['board meeting'],
      tokens: [
        { field: 'from', value: 'Ana Lima' },
        { field: 'in', value: 'Project X' },
      ],
    });
  });

  it('tolerates an unterminated quote while typing', () => {
    expect(parseSearchQuery('subject:"weekly sync').tokens).toEqual([
      { field: 'subject', value: 'weekly sync' },
    ]);
    expect(parseSearchQuery('"half open').terms).toEqual(['half open']);
  });

  it('is case-insensitive for fields and flag values, but keeps other values as typed', () => {
    expect(parseSearchQuery('FROM:Ana IS:Unread Has:Attachments').tokens).toEqual([
      { field: 'from', value: 'Ana' },
      { field: 'is', value: 'unread' },
      { field: 'has', value: 'attachment' },
    ]);
  });

  it('treats since: as after:', () => {
    expect(parseSearchQuery('since:2026-01-01').tokens).toEqual([
      { field: 'after', value: '2026-01-01' },
    ]);
  });

  it('leaves unknown fields and urls in the text', () => {
    expect(parseSearchQuery('label:work https://x.example/a').terms).toEqual([
      'label:work',
      'https://x.example/a',
    ]);
  });

  it('ignores an operator with no value yet', () => {
    expect(parseSearchQuery('report from:')).toEqual({ terms: ['report'], tokens: [] });
  });

  it('returns nothing for blank input', () => {
    expect(parseSearchQuery('   ')).toEqual({ terms: [], tokens: [] });
  });
});

describe('serializeSearchQuery', () => {
  it('round-trips, quoting values with spaces', () => {
    const input = '"board meeting" budget from:"Ana Lima" is:unread';
    expect(serializeSearchQuery(parseSearchQuery(input))).toBe(input);
  });

  it('puts terms before operators', () => {
    expect(serializeSearchQuery(parseSearchQuery('is:unread hello'))).toBe('hello is:unread');
  });

  it('keeps quotes on terms the grammar would read as syntax', () => {
    expect(serializeSearchQuery(parseSearchQuery('"or" "a (b" x'))).toBe('"or" "a (b" x');
  });
});

describe('editSearchQuery (the grammar: edits the text in place)', () => {
  const edit = (input: string, change: (q: ParsedSearch) => ParsedSearch) => {
    const q = parseSearchQuery(input);
    return editSearchQuery(input, q, change(q));
  };

  it('keeps a NOT and an OR when a filter is toggled on', () => {
    // Rebuilt, these were 'NOT budget in:trash is:unread' and 'OR from:ann from:bob is:unread'.
    expect(edit('in:trash NOT budget', (q) => toggleToken(q, 'is', 'unread'))).toBe(
      '(in:trash NOT budget) is:unread',
    );
    expect(edit('from:ann OR from:bob', (q) => toggleToken(q, 'is', 'unread'))).toBe(
      'from:ann OR from:bob is:unread',
    );
    expect(edit('"or" x', (q) => toggleToken(q, 'is', 'starred'))).toBe('"or" x is:starred');
  });

  it('removes a filter where it stands', () => {
    expect(edit('is:unread budget NOT in:trash', (q) => toggleToken(q, 'is', 'unread'))).toBe(
      'budget NOT in:trash',
    );
    expect(edit('a b NOT c', (q) => withoutTerm(q, 1))).toBe('a NOT c');
    expect(edit('(a OR b) c', (q) => withoutTerm(q, 3))).toBe('(a OR b)');
    expect(parseSearchQuery('(in:trash OR a)b').terms).toEqual(['OR', 'a', 'b']);
  });

  it("won't remove what an OR, NOT or parentheses hold", () => {
    for (const [input, term] of [
      ['from:ann OR budget', 0],
      ['budget OR x', 0],
      ['a NOT budget', 2],
      ['budget NOT a', 0],
      ['(budget a) c', 0],
      ['a AND budget', 2],
    ] as const) {
      expect(
        edit(input, (q) => withoutTerm(q, term)),
        input,
      ).toBeNull();
    }
    expect(edit('from:ann OR from:bob', (q) => withoutToken(q, 1))).toBeNull();
    expect(searchLayout('a OR b c').terms.map((t) => t.removable)).toEqual([
      false,
      false,
      false,
      true,
    ]);
    expect(searchLayout('a OR b').terms.map((t) => t.operator)).toEqual([false, true, false]);
  });

  it('replaces a date bound in place of the old one', () => {
    expect(
      edit('budget after:2026-01-01', (q) => withToken(q, { field: 'after', value: '7d' })),
    ).toBe('budget after:7d');
  });
});

describe('token helpers', () => {
  const q = parseSearchQuery('hello world from:ana is:unread');

  it('removes a token or a term by index', () => {
    expect(serializeSearchQuery(withoutToken(q, 0))).toBe('hello world is:unread');
    expect(serializeSearchQuery(withoutTerm(q, 1))).toBe('hello from:ana is:unread');
  });

  it('does not add a duplicate token', () => {
    expect(withToken(q, { field: 'is', value: 'UNREAD' })).toBe(q);
  });

  it('keeps only one before: and one after:', () => {
    const a = withToken(parseSearchQuery('after:2026-01-01'), { field: 'after', value: '7d' });
    expect(a.tokens).toEqual([{ field: 'after', value: '7d' }]);
  });

  it('toggles a flag on and off', () => {
    const on = toggleToken(q, 'is', 'starred');
    expect(serializeSearchQuery(on)).toBe('hello world from:ana is:unread is:starred');
    expect(serializeSearchQuery(toggleToken(on, 'is', 'starred'))).toBe(
      'hello world from:ana is:unread',
    );
  });
});

describe('dates', () => {
  it('reads absolute dates as local midnight', () => {
    expect(parseDateValue('2026-03-01', NOW)).toEqual(new Date(2026, 2, 1));
    expect(parseDateValue('2026/03/01', NOW)).toEqual(new Date(2026, 2, 1));
    expect(parseDateValue('2026-03', NOW)).toEqual(new Date(2026, 2, 1));
  });

  it('reads today, yesterday and relative ages', () => {
    expect(parseDateValue('today', NOW)).toEqual(new Date(2026, 9, 16));
    expect(parseDateValue('yesterday', NOW)).toEqual(new Date(2026, 9, 15));
    expect(parseDateValue('3d', NOW)).toEqual(new Date(2026, 9, 13));
    expect(parseDateValue('2w', NOW)).toEqual(new Date(2026, 9, 2));
    expect(parseDateValue('1m', NOW)).toEqual(new Date(2026, 8, 16));
    expect(parseDateValue('1y', NOW)).toEqual(new Date(2025, 9, 16));
  });

  it('rejects nonsense', () => {
    expect(parseDateValue('soon', NOW)).toBeNull();
    expect(parseDateValue('2026-13-45', NOW)).toBeNull();
  });
});

describe('toEngineQuery', () => {
  const q = ' (invoice OR receipt) after:7d from:"Ana Lima" before:today ';

  it('sends the query as typed, dates included', () => {
    expect(toEngineQuery(q)).toBe(q.trim());
  });
});

describe('activeOperator', () => {
  it('finds the operator under the caret', () => {
    expect(activeOperator('report from:an')).toEqual({ field: 'from', value: 'an', start: 7 });
    expect(activeOperator('in:')).toEqual({ field: 'in', value: '', start: 0 });
    expect(activeOperator('from:"Ana')).toEqual({ field: 'from', value: 'Ana', start: 0 });
  });

  it('returns null outside an operator', () => {
    expect(activeOperator('report from:ana ')).toBeNull();
    expect(activeOperator('hello')).toBeNull();
    expect(activeOperator('label:x')).toBeNull();
  });
});
