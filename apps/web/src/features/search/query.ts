// The search bar grammar: free text plus `field:value` operators, as in the desktop's
// SearchQueryParser (without its AND/OR/parentheses). Values with spaces are quoted.
import {
  format,
  isValid,
  parse,
  startOfDay,
  subDays,
  subMonths,
  subWeeks,
  subYears,
} from 'date-fns';

export const SEARCH_FIELDS = [
  'from',
  'to',
  'subject',
  'in',
  'is',
  'has',
  'before',
  'after',
] as const;

export type SearchField = (typeof SEARCH_FIELDS)[number];

export interface SearchToken {
  field: SearchField;
  value: string;
}

export interface ParsedSearch {
  /** Free-text terms; a quoted phrase is one term. */
  terms: string[];
  tokens: SearchToken[];
}

const ALIASES: Record<string, SearchField> = { since: 'after' };

function toField(name: string): SearchField | null {
  const f = name.toLowerCase();
  if ((SEARCH_FIELDS as readonly string[]).includes(f)) return f as SearchField;
  return ALIASES[f] ?? null;
}

function normalizeValue(field: SearchField, value: string): string {
  if (field === 'is' || field === 'has') {
    const v = value.toLowerCase();
    return field === 'has' && v === 'attachments' ? 'attachment' : v;
  }
  return value;
}

export function parseSearchQuery(input: string): ParsedSearch {
  const out: ParsedSearch = { terms: [], tokens: [] };
  const re = /([A-Za-z]+):(?:"([^"]*)"?|(\S*))|"([^"]*)"?|(\S+)/g;
  for (const m of input.matchAll(re)) {
    const [whole, name, quotedValue, value, phrase, word] = m;
    const field = name ? toField(name) : null;
    if (field) {
      const v = (quotedValue ?? value ?? '').trim();
      // A bare `from:` while typing is not a filter yet.
      if (v) out.tokens.push({ field, value: normalizeValue(field, v) });
    } else if (name) {
      out.terms.push(whole);
    } else if (phrase !== undefined) {
      if (phrase.trim()) out.terms.push(phrase.trim());
    } else if (word) {
      out.terms.push(word);
    }
  }
  return out;
}

function quote(value: string): string {
  return /\s/.test(value) ? `"${value.replace(/"/g, '')}"` : value;
}

export function serializeSearchQuery(q: ParsedSearch): string {
  return [...q.terms.map(quote), ...q.tokens.map((t) => `${t.field}:${quote(t.value)}`)].join(' ');
}

export function isEmptySearch(q: ParsedSearch): boolean {
  return q.terms.length === 0 && q.tokens.length === 0;
}

export function hasToken(q: ParsedSearch, field: SearchField, value: string): boolean {
  const v = normalizeValue(field, value);
  return q.tokens.some((t) => t.field === field && t.value === v);
}

/** Adds a filter; `before`/`after` replace any existing one since only one bound makes sense. */
export function withToken(q: ParsedSearch, token: SearchToken): ParsedSearch {
  const t = { field: token.field, value: normalizeValue(token.field, token.value) };
  if (hasToken(q, t.field, t.value)) return q;
  const single = t.field === 'before' || t.field === 'after';
  const tokens = single ? q.tokens.filter((x) => x.field !== t.field) : q.tokens;
  return { ...q, tokens: [...tokens, t] };
}

export function withoutToken(q: ParsedSearch, index: number): ParsedSearch {
  return { ...q, tokens: q.tokens.filter((_, i) => i !== index) };
}

export function withoutTerm(q: ParsedSearch, index: number): ParsedSearch {
  return { ...q, terms: q.terms.filter((_, i) => i !== index) };
}

export function toggleToken(q: ParsedSearch, field: SearchField, value: string): ParsedSearch {
  const v = normalizeValue(field, value);
  const i = q.tokens.findIndex((t) => t.field === field && t.value === v);
  return i === -1 ? withToken(q, { field, value: v }) : withoutToken(q, i);
}

const DATE_FORMATS = ['yyyy-MM-dd', 'yyyy/MM/dd', 'yyyy-MM', 'd MMM yyyy', 'MMM d yyyy'];

/**
 * Reads a `before:`/`after:` value: ISO-ish dates (`2024-03-01`, `2024/03/01`, `2024-03`),
 * `today`, `yesterday`, or a relative age (`3d`, `2w`, `6m`, `1y`). Returns local midnight.
 */
export function parseDateValue(value: string, now = new Date()): Date | null {
  const v = value.trim().toLowerCase();
  if (v === 'today') return startOfDay(now);
  if (v === 'yesterday') return startOfDay(subDays(now, 1));
  const rel = /^(\d+)([dwmy])$/.exec(v);
  if (rel) {
    const n = Number(rel[1]);
    const sub = { d: subDays, w: subWeeks, m: subMonths, y: subYears }[
      rel[2] as 'd' | 'w' | 'm' | 'y'
    ];
    return startOfDay(sub(now, n));
  }
  for (const f of DATE_FORMATS) {
    const d = parse(value.trim(), f, now);
    if (isValid(d)) return startOfDay(d);
  }
  return null;
}

export function formatDateValue(d: Date): string {
  return format(d, 'yyyy-MM-dd');
}

export interface DateRange {
  /** Unix seconds, inclusive. */
  after?: number;
  /** Unix seconds, exclusive. */
  before?: number;
}

export function dateRange(q: ParsedSearch, now = new Date()): DateRange {
  const range: DateRange = {};
  for (const t of q.tokens) {
    if (t.field !== 'before' && t.field !== 'after') continue;
    const d = parseDateValue(t.value, now);
    if (d) range[t.field] = Math.floor(d.getTime() / 1000);
  }
  return range;
}

export function inDateRange(unixSeconds: number, range: DateRange): boolean {
  if (range.after !== undefined && unixSeconds < range.after) return false;
  if (range.before !== undefined && unixSeconds >= range.before) return false;
  return true;
}

/**
 * The query string sent to the engine's `search` method. Date bounds are applied by the
 * client for now.
 */
export function toEngineQuery(q: ParsedSearch): string {
  // TODO(engine): `search` is plain text: ms-store quotes every term for FTS5, so from:,
  // to:, subject:, in:, is: and has: are matched as literal words and before:/after: can't
  // be expressed at all. Only the mock understands the operators sent here. The engine
  // gets the desktop grammar with DCP stage 9.
  return serializeSearchQuery({
    terms: q.terms,
    tokens: q.tokens.filter((t) => t.field !== 'before' && t.field !== 'after'),
  });
}

/** The partial `field:value` operator under the caret, for autocomplete. */
export function activeOperator(
  input: string,
  caret = input.length,
): { field: SearchField; value: string; start: number } | null {
  const before = input.slice(0, caret);
  const m = /(?:^|\s)([A-Za-z]+):("?[^\s"]*)$/.exec(before);
  if (!m) return null;
  const field = toField(m[1]);
  if (!field) return null;
  return {
    field,
    value: m[2].replace(/^"/, ''),
    start: before.length - m[1].length - 1 - m[2].length,
  };
}
