// The mock's copy of the engine grammar against the cases of the engine's own tests
// (crates/tern-store/src/search_grammar.rs, search_dates.rs), so the two don't drift.
import { describe, expect, it } from 'vitest';
import { RpcErrorCode } from '../rpc';
import {
  grammarMatcher,
  MAX_MATCH_DEPTH,
  MAX_SEARCH_TOKENS,
  parseSearch,
  resolveSearchDate,
  type SearchExpr,
} from './search';

const generic = (text: string): SearchExpr => ({ type: 'generic', text });
const and = (e1: SearchExpr, e2: SearchExpr): SearchExpr => ({ type: 'and', e1, e2 });
const or = (e1: SearchExpr, e2: SearchExpr): SearchExpr => ({ type: 'or', e1, e2 });
const not = (e1: SearchExpr, e2: SearchExpr): SearchExpr => ({ type: 'not', e1, e2 });

describe('parseSearch', () => {
  it('parses the desktop spec cases', () => {
    expect(parseSearch('"foo bar"')).toEqual(generic('foo bar'));
    expect(parseSearch('to:"Mark mhahnenb@gmail.com"')).toEqual({
      type: 'to',
      text: 'Mark mhahnenb@gmail.com',
    });
    expect(parseSearch('is:read')).toEqual({ type: 'unread', status: false });
    expect(parseSearch('is:unstarred')).toEqual({ type: 'starred', status: false });
    expect(parseSearch('has:attachment')).toEqual({ type: 'hasAttachment' });
    expect(parseSearch('to:from')).toEqual({ type: 'to', text: 'from' });
    const [foo, bar, baz] = [generic('foo'), generic('bar'), generic('baz')];
    expect(parseSearch('foo AND bar AND baz')).toEqual(and(foo, and(bar, baz)));
    expect(parseSearch('foo OR bar AND baz')).toEqual(and(or(foo, bar), baz));
    expect(parseSearch('foo OR bar OR baz')).toEqual(or(foo, or(bar, baz)));
    expect(parseSearch('is:unread foo')).toEqual(and({ type: 'unread', status: true }, foo));
  });

  it('reads NOT, parentheses and the engine deviations', () => {
    expect(parseSearch('foo AND NOT bar')).toEqual(not(generic('foo'), generic('bar')));
    expect(parseSearch('subject:"q3 plan" NOT (in:trash OR in:spam)')).toEqual(
      not(
        { type: 'subject', text: 'q3 plan' },
        or({ type: 'in', text: 'trash' }, { type: 'in', text: 'spam' }),
      ),
    );
    expect(parseSearch('trip to paris')).toEqual(
      and(generic('trip'), and(generic('to'), generic('paris'))),
    );
    expect(parseSearch('"or"')).toEqual(generic('or'));
    expect(parseSearch('http://x.test/a')).toEqual(generic('http://x.test/a'));
    expect(parseSearch('since:yesterday')).toEqual({
      type: 'date',
      text: 'yesterday',
      direction: 'after',
    });
  });

  it('refuses what the engine refuses', () => {
    for (const q of [
      '',
      '"open',
      'from:',
      'to:(bob)',
      'is:weird',
      'has:pdf',
      'foo)',
      '(foo',
      '()',
    ]) {
      expect(() => parseSearch(q), q).toThrow(
        expect.objectContaining({ code: RpcErrorCode.InvalidParams }),
      );
    }
    for (const q of ['foo OR', 'foo AND', 'foo NOT']) expect(() => parseSearch(q), q).toThrow();
    const long = Array(MAX_SEARCH_TOKENS).fill('a').join(' ');
    expect(() => parseSearch(long)).not.toThrow();
    expect(() => parseSearch(`${long} b`)).toThrow(/too long/);
    expect(() => parseSearch('('.repeat(100_000))).toThrow(/too long/);
    for (const q of ['a - b', 'a : b', '""', 'from:""', 'subject:-']) {
      expect(() => parseSearch(q), q).toThrow(/letters or digits|nothing to search/);
    }
    const nots = (n: number) => Array(n).fill('a').join(' NOT ');
    expect(() => parseSearch(nots(MAX_MATCH_DEPTH))).not.toThrow();
    expect(() => parseSearch(nots(MAX_MATCH_DEPTH + 1))).toThrow(/nests too deeply/);
    expect(() => parseSearch(Array(MAX_SEARCH_TOKENS).fill('a').join(' '))).not.toThrow();
    expect(() => parseSearch(`is:unread (${nots(MAX_MATCH_DEPTH + 1)})`)).toThrow(/too deeply/);
    expect(() => parseSearch(`is:unread NOT (${nots(MAX_MATCH_DEPTH)})`)).not.toThrow();
  });

  it('glues a field value to a colon, as a plain word', () => {
    expect(parseSearch('subject:re:hello')).toEqual({ type: 'subject', text: 're:hello' });
  });

  it('checks the zone only for date terms', () => {
    expect(() => grammarMatcher('budget', 'UTC0')).not.toThrow();
    expect(() => grammarMatcher('after:2024-01-01', 'Mars/Base')).toThrow(/IANA/);
  });
});

describe('resolveSearchDate', () => {
  // 2026-10-14 15:00 UTC, a Wednesday.
  const NOW = 1_791_990_000;
  const day = (term: string, tz = 'Europe/Lisbon') =>
    new Date(resolveSearchDate(term, tz, NOW) * 1000).toLocaleDateString('en-CA', {
      timeZone: tz,
    });

  it('reads the documented subset', () => {
    expect(day('2024/3/1')).toBe('2024-03-01');
    expect(day('2024-03')).toBe('2024-03-01');
    expect(day('march 1st, 2024')).toBe('2024-03-01');
    expect(day('yesterday')).toBe('2026-10-13');
    expect(day('mon')).toBe('2026-10-12');
    expect(day('wednesday')).toBe('2026-10-14');
    expect(day('november')).toBe('2025-11-01');
    expect(day('15 oct')).toBe('2025-10-15');
    expect(day('2 weeks ago')).toBe('2026-09-30');
    expect(day('6m')).toBe('2026-04-14');
    expect(day('last year')).toBe('2025-10-14');
  });

  it('resolves in the zone it is given', () => {
    expect(day('today', 'Pacific/Auckland')).toBe('2026-10-15');
    expect(day('today', 'Pacific/Honolulu')).toBe('2026-10-14');
    expect(resolveSearchDate('2024-07-01', 'Europe/Lisbon')).toBe(1_719_788_400);
    expect(resolveSearchDate('2024-07-01', 'America/New_York')).toBe(1_719_806_400);
    expect(resolveSearchDate('2024-09-08', 'America/Santiago')).toBe(1_725_768_000);
  });

  it('refuses other terms, and relative ones without now', () => {
    for (const t of ['someday', 'next week', '3 days', '2024-02-30', 'march 32']) {
      expect(() => resolveSearchDate(t, 'UTC', NOW), t).toThrow();
    }
    expect(resolveSearchDate('2024-03-01')).toBe(1_709_251_200);
    expect(() => resolveSearchDate('yesterday')).toThrow(/search\.now/);
  });
});
