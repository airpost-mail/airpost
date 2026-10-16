import { describe, expect, it } from 'vitest';
import {
  activeOperator,
  dateRange,
  inDateRange,
  parseDateValue,
  parseSearchQuery,
  serializeSearchQuery,
  toEngineQuery,
  toggleToken,
  withToken,
  withoutTerm,
  withoutToken,
} from './query';

const NOW = new Date(2026, 9, 16, 15, 30);
const secs = (d: Date) => Math.floor(d.getTime() / 1000);

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

  it('builds an inclusive-after, exclusive-before range', () => {
    const range = dateRange(parseSearchQuery('after:2026-10-01 before:2026-10-02'), NOW);
    expect(range).toEqual({
      after: secs(new Date(2026, 9, 1)),
      before: secs(new Date(2026, 9, 2)),
    });
    expect(inDateRange(secs(new Date(2026, 9, 1, 0, 0)), range)).toBe(true);
    expect(inDateRange(secs(new Date(2026, 9, 1, 23, 59)), range)).toBe(true);
    expect(inDateRange(secs(new Date(2026, 9, 2)), range)).toBe(false);
    expect(inDateRange(secs(new Date(2026, 8, 30)), range)).toBe(false);
  });

  it('ignores unreadable date bounds', () => {
    expect(dateRange(parseSearchQuery('before:whenever'), NOW)).toEqual({});
  });
});

describe('toEngineQuery', () => {
  it('drops date bounds and keeps everything else', () => {
    expect(toEngineQuery(parseSearchQuery('invoice after:7d from:"Ana Lima" before:today'))).toBe(
      'invoice from:"Ana Lima"',
    );
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
