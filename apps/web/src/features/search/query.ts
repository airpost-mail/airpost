// The search bar's view of the query: free text plus `field:value` operators, for chips,
// filters and suggestions. The engine parses the full grammar (AND/OR/NOT, parentheses)
// itself when it has `search-grammar`; values with spaces are quoted.
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

type Scanned =
  | { kind: 'term'; text: string; start: number; end: number; quoted: boolean }
  | { kind: 'token'; token: SearchToken; start: number; end: number; quoted: boolean }
  /** A bare `field:` while typing: not a filter yet. */
  | { kind: 'skip'; start: number; end: number }
  | { kind: 'paren'; open: boolean; start: number; end: number };

// Parentheses outside quotes are the grammar's, glued to a word or not, as in the engine.
const SCAN = /([A-Za-z]+):(?:"([^"]*)"?|([^\s()]*))|"([^"]*)"?|([()])|([^\s()]+)/g;

function scan(input: string): Scanned[] {
  const out: Scanned[] = [];
  for (const m of input.matchAll(SCAN)) {
    const [whole, name, quotedValue, value, phrase, paren, word] = m;
    const start = m.index;
    const end = start + whole.length;
    const field = name ? toField(name) : null;
    if (field) {
      const v = (quotedValue ?? value ?? '').trim();
      out.push(
        v
          ? {
              kind: 'token',
              token: { field, value: normalizeValue(field, v) },
              start,
              end,
              quoted: quotedValue !== undefined,
            }
          : { kind: 'skip', start, end },
      );
    } else if (name) {
      out.push({ kind: 'term', text: whole, start, end, quoted: false });
    } else if (phrase !== undefined) {
      out.push(
        phrase.trim()
          ? { kind: 'term', text: phrase.trim(), start, end, quoted: true }
          : { kind: 'skip', start, end },
      );
    } else if (paren) {
      out.push({ kind: 'paren', open: paren === '(', start, end });
    } else if (word) {
      out.push({ kind: 'term', text: word, start, end, quoted: false });
    }
  }
  return out;
}

export function parseSearchQuery(input: string): ParsedSearch {
  const out: ParsedSearch = { terms: [], tokens: [] };
  for (const s of scan(input)) {
    if (s.kind === 'term') out.terms.push(s.text);
    else if (s.kind === 'token') out.tokens.push(s.token);
  }
  return out;
}

const OPERATORS = new Set(['and', 'or', 'not']);

function isOperatorWord(value: string): boolean {
  return OPERATORS.has(value.toLowerCase());
}

/** Values the engine's grammar would read as syntax unless quoted. */
function quote(value: string): string {
  return /[\s()"]/.test(value) || isOperatorWord(value) ? `"${value.replace(/"/g, '')}"` : value;
}

export function serializeSearchQuery(q: ParsedSearch): string {
  return [...q.terms.map(quote), ...q.tokens.map((t) => `${t.field}:${quote(t.value)}`)].join(' ');
}

// -- Editing the query text in place, for the engine's grammar --------------------------------
//
// With the grammar, the order of the text and its AND/OR/NOT and parentheses carry meaning,
// so the filter controls edit the text the person typed rather than rebuilding it from the
// parsed terms and filters (which would move a leading NOT or split an OR).

interface Placed {
  start: number;
  end: number;
  /** Removing it from the text leaves the rest of the query meaning what it did. */
  removable: boolean;
  /** An unquoted AND/OR/NOT: grammar, not a term. */
  operator: boolean;
}

/** Where each of `parseSearchQuery(input)`'s terms and tokens sits in `input`. */
export interface SearchLayout {
  terms: Placed[];
  tokens: Placed[];
  /** The text has an unquoted `NOT` outside parentheses, which takes all that follows it. */
  topLevelNot: boolean;
}

function isOperator(s: Scanned | undefined): boolean {
  return s?.kind === 'term' && !s.quoted && isOperatorWord(s.text);
}

export function searchLayout(input: string): SearchLayout {
  const items = scan(input);
  const out: SearchLayout = { terms: [], tokens: [], topLevelNot: false };
  let depth = 0;
  items.forEach((s, i) => {
    if (s.kind === 'paren') {
      depth = Math.max(0, depth + (s.open ? 1 : -1));
      return;
    }
    const operator = isOperator(s);
    const before = items[i - 1];
    const after = items[i + 1];
    // `w x NOT y` without x is `w NOT y`, still w and not y; `x NOT y` without x is not.
    const notAfter = isOperator(after) && after?.kind === 'term' && /^not$/i.test(after.text);
    const removable =
      depth === 0 &&
      !out.topLevelNot &&
      !operator &&
      !isOperator(before) &&
      (!isOperator(after) || (notAfter && i > 0));
    if (operator && depth === 0 && s.kind === 'term' && /^not$/i.test(s.text)) {
      out.topLevelNot = true;
    }
    const placed = { start: s.start, end: s.end, removable, operator };
    if (s.kind === 'term') out.terms.push(placed);
    else if (s.kind === 'token') out.tokens.push(placed);
  });
  return out;
}

/**
 * `input` with the terms and tokens `parsed` (`parseSearchQuery(input)`) has and `next`
 * doesn't removed, and the tokens `next` adds appended, ANDed with the rest. `next` comes from
 * the helpers below, which keep the token objects they don't touch. `null` when an item to
 * remove can't be taken out without changing what the rest means.
 */
export function editSearchQuery(
  input: string,
  parsed: ParsedSearch,
  next: ParsedSearch,
): string | null {
  const layout = searchLayout(input);
  const cut: Placed[] = [];
  parsed.tokens.forEach((t, i) => {
    if (!next.tokens.includes(t)) cut.push(layout.tokens[i]);
  });
  // `next.terms` is `parsed.terms` with some left out, in order.
  let j = 0;
  parsed.terms.forEach((t, i) => {
    if (next.terms[j] === t) j++;
    else cut.push(layout.terms[i]);
  });
  if (cut.some((p) => !p?.removable)) return null;
  let text = input;
  for (const p of [...cut].sort((a, b) => b.start - a.start)) {
    text = text.slice(0, p.start) + text.slice(p.end);
  }
  text = text.replace(/\s+/g, ' ').trim();
  const added = next.tokens
    .filter((t) => !parsed.tokens.includes(t))
    .map((t) => `${t.field}:${quote(t.value)}`);
  if (added.length === 0) return text;
  if (!text) return added.join(' ');
  const base = searchLayout(text).topLevelNot ? `(${text})` : text;
  return [base, ...added].join(' ');
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
 * The text for the engine's `search`. An engine with the search grammar takes the query as
 * typed, dates included. An older one reads plain words, so it gets the words and the
 * operators without the dates, which the client then applies (`dateRange`).
 */
export function toEngineQuery(input: string, grammar: boolean): string {
  if (grammar) return input.trim();
  const q = parseSearchQuery(input);
  return serializeSearchQuery({
    terms: q.terms,
    tokens: q.tokens.filter((t) => t.field !== 'before' && t.field !== 'after'),
  });
}

/**
 * Text someone is still typing, which the grammar would refuse: an open quote or
 * parenthesis, a bare `from:`, or a trailing AND/OR/NOT.
 */
export function isUnfinishedSearch(input: string): boolean {
  if ((input.match(/"/g)?.length ?? 0) % 2 === 1) return true;
  const items = scan(input);
  let depth = 0;
  for (const s of items) if (s.kind === 'paren') depth += s.open ? 1 : -1;
  const last = items.at(-1);
  return depth > 0 || last?.kind === 'skip' || isOperator(last);
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
