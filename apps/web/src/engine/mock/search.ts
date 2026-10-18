// The engine's search grammar (crates/ms-store/src/search_grammar.rs, search_dates.rs) for
// the mock's `search {grammar: true}` and `query.search {grammar: true}`: the same parser,
// errors and date terms, evaluated over the mock's threads instead of SQL. Without
// `grammar`, `search` matches plain words, as ms-store's `fts_query` does.
import { RpcError, RpcErrorCode } from '../rpc';

export type SearchExpr =
  | { type: 'and' | 'or' | 'not'; e1: SearchExpr; e2: SearchExpr }
  | { type: 'from' | 'to' | 'subject' | 'in' | 'generic'; text: string }
  | { type: 'unread' | 'starred'; status: boolean }
  | { type: 'hasAttachment' }
  | { type: 'date'; text: string; direction: 'before' | 'after' };

function fail(message: string): never {
  throw new RpcError({ code: RpcErrorCode.InvalidParams, message: `invalid params: ${message}` });
}

interface Token {
  s: string;
  quoted: boolean;
}

const isSpace = (c: string) => c === ' ' || c === '\t' || c === '\n';
const isSpecial = (c: string) => c === '(' || c === ')' || c === ':';
const is = (t: Token | null, word: string) =>
  !!t && !t.quoted && t.s.toLowerCase() === word.toLowerCase();

function nextToken(text: string): [Token | null, string] {
  let i = 0;
  while (i < text.length && isSpace(text[i])) i++;
  const rest = text.slice(i);
  if (!rest) return [null, rest];
  if (rest[0] === '"') {
    const end = rest.indexOf('"', 1);
    if (end === -1) fail('search: a quoted phrase is missing its closing "');
    return [{ s: rest.slice(1, end), quoted: true }, rest.slice(end + 1)];
  }
  let end = 1;
  if (!isSpecial(rest[0])) {
    while (end < rest.length && !isSpace(rest[end]) && !isSpecial(rest[end])) end++;
  }
  return [{ s: rest.slice(0, end), quoted: false }, rest.slice(end)];
}

function expect(text: string, word: string): string {
  const [t, rest] = nextToken(text);
  if (!is(t, word)) fail(`search: expected '${word}'`);
  return rest;
}

type Parsed = [SearchExpr | null, string];

function fieldValue(text: string, field: string): [string, string] {
  const [t, rest] = nextToken(text);
  if (!t || (!t.quoted && isSpecial(t.s[0]))) {
    fail(`search: ${field}: needs a value, a word or a "quoted phrase"`);
  }
  return [t.s, rest];
}

const FIELDS = ['from', 'to', 'subject', 'in', 'is', 'has', 'before', 'after', 'since'];

function fieldExpr(field: string, value: string): SearchExpr {
  const v = value.toUpperCase();
  switch (field) {
    case 'is':
      if (v === 'READ' || v === 'UNREAD') return { type: 'unread', status: v === 'UNREAD' };
      if (v === 'STARRED' || v === 'UNSTARRED') return { type: 'starred', status: v === 'STARRED' };
      return fail(
        `search: is:${value} isn't a search filter: use is:unread, is:read, is:starred or is:unstarred`,
      );
    case 'has':
      if (v === 'ATTACHMENT' || v === 'ATTACHMENTS') return { type: 'hasAttachment' };
      return fail(`search: has:${value} isn't a search filter: use has:attachment`);
    case 'before':
      return { type: 'date', text: value, direction: 'before' };
    case 'after':
    case 'since':
      return { type: 'date', text: value, direction: 'after' };
    default:
      return { type: field as 'from' | 'to' | 'subject' | 'in', text: value };
  }
}

function parseSimple(text: string): Parsed {
  const [tok, after] = nextToken(text);
  if (!tok) return [null, after];
  if (is(tok, '(')) {
    const [exp, afterExp] = parseAnd(after);
    if (!exp) fail('search: empty parentheses');
    const [close, rest] = nextToken(afterExp);
    if (!is(close, ')')) fail("search: a '(' is never closed");
    return [exp, rest];
  }
  if (is(tok, ')')) fail("search: a ')' has no '(' before it");
  const field = FIELDS.find((f) => is(tok, f));
  if (field && is(nextToken(after)[0], ':')) {
    const [value, rest] = fieldValue(expect(after, ':'), field);
    return [fieldExpr(field, value), rest];
  }
  if (tok.quoted) return [{ type: 'generic', text: tok.s }, after];
  // A word glued to `:` and more (`http://x.test`) stays one word.
  let word = tok.s;
  let rest = after;
  while (rest.startsWith(':')) {
    rest = rest.slice(1);
    let end = 0;
    while (end < rest.length && !isSpace(rest[end]) && !isSpecial(rest[end]) && rest[end] !== '"') {
      end++;
    }
    word += `:${rest.slice(0, end)}`;
    rest = rest.slice(end);
  }
  return [{ type: 'generic', text: word }, rest];
}

function parseOr(text: string): Parsed {
  const [lhs, afterLhs] = parseSimple(text);
  if (!lhs) return [null, afterLhs];
  const [tok, afterOr] = nextToken(afterLhs);
  if (!is(tok, 'OR')) return [lhs, afterLhs];
  const [rhs, afterRhs] = parseOr(afterOr);
  if (!rhs) fail('search: OR needs a search term after it');
  return [{ type: 'or', e1: lhs, e2: rhs }, afterRhs];
}

function parseAnd(text: string): Parsed {
  const [lhs, afterLhs] = parseOr(text);
  if (!lhs) return [null, afterLhs];
  const [first, afterFirst] = nextToken(afterLhs);
  if (!first || is(first, 'OR') || is(first, ')')) return [lhs, afterLhs];
  let tok = first;
  let rhsStart = afterLhs;
  if (is(first, 'AND')) {
    rhsStart = afterFirst;
    const [next] = nextToken(afterFirst);
    if (!next) fail('search: AND needs a search term after it');
    tok = next;
  }
  if (is(tok, 'NOT')) {
    const [rhs, afterRhs] = parseAnd(expect(rhsStart, 'NOT'));
    if (!rhs) fail('search: NOT needs a search term after it');
    return [{ type: 'not', e1: lhs, e2: rhs }, afterRhs];
  }
  const [rhs, afterRhs] = parseAnd(rhsStart);
  if (!rhs) fail('search: AND needs a search term after it');
  return [{ type: 'and', e1: lhs, e2: rhs }, afterRhs];
}

/** Tokens a query may have, as in the engine. */
export const MAX_SEARCH_TOKENS = 200;

export function parseSearch(text: string): SearchExpr {
  let tail = text;
  for (let i = 0; i < MAX_SEARCH_TOKENS; i++) {
    const [t, r] = nextToken(tail);
    if (!t) break;
    tail = r;
  }
  if (nextToken(tail)[0]) {
    fail(
      `search: the search is too long: at most ${MAX_SEARCH_TOKENS} words, operators and parentheses`,
    );
  }
  const [exp, rest] = parseAnd(text);
  if (!exp) fail('search: the search has nothing to look for');
  if (nextToken(rest)[0]) fail("search: a ')' has no '(' before it");
  return exp;
}

// -- Dates ----------------------------------------------------------------------------------

interface Day {
  y: number;
  m: number;
  d: number;
}

const MONTHS = [
  ['january', 'jan'],
  ['february', 'feb'],
  ['march', 'mar'],
  ['april', 'apr'],
  ['may'],
  ['june', 'jun'],
  ['july', 'jul'],
  ['august', 'aug'],
  ['september', 'sep', 'sept'],
  ['october', 'oct'],
  ['november', 'nov'],
  ['december', 'dec'],
];
const WEEKDAYS = [
  ['monday', 'mon'],
  ['tuesday', 'tue', 'tues'],
  ['wednesday', 'wed'],
  ['thursday', 'thu', 'thur', 'thurs'],
  ['friday', 'fri'],
  ['saturday', 'sat'],
  ['sunday', 'sun'],
];

const indexIn = (lists: string[][], w: string) => lists.findIndex((names) => names.includes(w));
const monthOf = (w: string) => indexIn(MONTHS, w) + 1 || null;
const dayOf = (w: string) => {
  const m = /^(\d{1,2})(st|nd|rd|th)?$/.exec(w);
  const d = m ? Number(m[1]) : NaN;
  return d >= 1 && d <= 31 ? d : null;
};
const yearOf = (w: string) => (/^\d{4}$/.test(w) && Number(w) >= 1900 ? Number(w) : null);

function makeDay(y: number, m: number, d: number): Day | null {
  const t = new Date(Date.UTC(y, m - 1, d));
  return t.getUTCFullYear() === y && t.getUTCMonth() === m - 1 && t.getUTCDate() === d
    ? { y, m, d }
    : null;
}

const toUtc = (day: Day) => Date.UTC(day.y, day.m - 1, day.d);
const fromUtc = (ms: number): Day => {
  const t = new Date(ms);
  return { y: t.getUTCFullYear(), m: t.getUTCMonth() + 1, d: t.getUTCDate() };
};

function subMonths(day: Day, n: number): Day {
  const total = day.y * 12 + (day.m - 1) - n;
  const y = Math.floor(total / 12);
  const m = (total % 12) + 1;
  const last = new Date(Date.UTC(y, m, 0)).getUTCDate();
  return { y, m, d: Math.min(day.d, last) };
}

const DAY_MS = 86_400_000;
const UNITS: Record<string, 'd' | 'w' | 'm' | 'y'> = { day: 'd', week: 'w', month: 'm', year: 'y' };

function back(today: Day, n: number, unit: 'd' | 'w' | 'm' | 'y'): Day {
  if (unit === 'd') return fromUtc(toUtc(today) - n * DAY_MS);
  if (unit === 'w') return fromUtc(toUtc(today) - n * 7 * DAY_MS);
  return subMonths(today, unit === 'm' ? n : n * 12);
}

/** The latest date on or before `today` with `month` and `day`. */
function latestMonthDay(today: Day, month: number, day = 1): Day | null {
  for (let years = 0; years < 8; years++) {
    const d = makeDay(today.y - years, month, day);
    if (d && toUtc(d) <= toUtc(today)) return d;
  }
  return null;
}

function parts(ms: number, tz: string): Record<string, number> {
  const out: Record<string, number> = {};
  const fmt = new Intl.DateTimeFormat('en-US', {
    timeZone: tz,
    hourCycle: 'h23',
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: 'numeric',
    minute: 'numeric',
  });
  for (const p of fmt.formatToParts(new Date(ms))) out[p.type] = Number(p.value);
  return out;
}

const dayIn = (ms: number, tz: string): Day => {
  const p = parts(ms, tz);
  return { y: p.year, m: p.month, d: p.day };
};

/** Milliseconds `tz` is ahead of UTC at `ms`. */
function offsetAt(ms: number, tz: string): number {
  const p = parts(ms, tz);
  return Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute) - Math.floor(ms / 60_000) * 60_000;
}

function startOf(day: Day, tz: string): number {
  const wall = toUtc(day);
  let ms = wall - offsetAt(wall, tz);
  ms = wall - offsetAt(ms, tz);
  // Midnight in a DST gap: the day starts when the clocks resume.
  while (dayIn(ms, tz).d !== day.d) ms += 30 * 60_000;
  return Math.floor(ms / 1000);
}

/** The first instant (unix seconds) of the day `term` names in `tz`, counted from `now`. */
export function resolveSearchDate(term: string, tz = 'UTC', now?: number): number {
  const w = term.trim().toLowerCase().replace(/,/g, ' ').split(/\s+/).filter(Boolean);
  const today = () => {
    if (now === undefined) {
      fail(`search: the date "${term}" is relative to now, and the request has no search.now`);
    }
    return dayIn(now * 1000, tz);
  };
  let found: Day | null = null;
  if (w.length === 1) {
    const iso = /^(\d{4})[-/.](\d{1,2})(?:[-/.](\d{1,2}))?$/.exec(w[0]);
    const short = /^(\d+)([dwmy])$/.exec(w[0]);
    const weekday = indexIn(WEEKDAYS, w[0]);
    if (iso) found = makeDay(Number(iso[1]), Number(iso[2]), iso[3] ? Number(iso[3]) : 1);
    else if (w[0] === 'today') found = today();
    else if (w[0] === 'yesterday') found = back(today(), 1, 'd');
    else if (monthOf(w[0])) found = latestMonthDay(today(), monthOf(w[0])!);
    else if (weekday !== -1) {
      const t = today();
      const dow = (new Date(toUtc(t)).getUTCDay() + 6) % 7;
      found = back(t, (7 + dow - weekday) % 7, 'd');
    } else if (short) found = back(today(), Number(short[1]), short[2] as 'd' | 'w' | 'm' | 'y');
  } else if (w.length === 2 && w[0] === 'last' && ['week', 'month', 'year'].includes(w[1])) {
    found = back(today(), 1, UNITS[w[1]]);
  } else if (w.length === 3 && w[2] === 'ago' && /^\d+$/.test(w[0])) {
    const unit = UNITS[w[1].replace(/s$/, '')];
    if (unit) found = back(today(), Number(w[0]), unit);
  } else if (w.length === 2) {
    const [ma, mb] = [monthOf(w[0]), monthOf(w[1])];
    if (ma && yearOf(w[1])) found = makeDay(yearOf(w[1])!, ma, 1);
    else if (ma && dayOf(w[1])) found = latestMonthDay(today(), ma, dayOf(w[1])!);
    else if (mb && dayOf(w[0])) found = latestMonthDay(today(), mb, dayOf(w[0])!);
  } else if (w.length === 3 && yearOf(w[2])) {
    const [ma, mb] = [monthOf(w[0]), monthOf(w[1])];
    if (ma && dayOf(w[1])) found = makeDay(yearOf(w[2])!, ma, dayOf(w[1])!);
    else if (mb && dayOf(w[0])) found = makeDay(yearOf(w[2])!, mb, dayOf(w[0])!);
  }
  if (!found) {
    fail(
      `search: "${term}" isn't a date the search understands: use a date like 2024-03-01 or ` +
        '"march 1 2024", a month or weekday name, today, yesterday, "3 days ago" (or 3d, 2w, ' +
        '6m, 1y) or "last week|month|year"',
    );
  }
  return startOf(found, tz);
}

// -- Matching -------------------------------------------------------------------------------

/** A thread as the search sees it: its `ThreadSearch` columns and flags. */
export interface Searchable {
  columns: { subject: string; from: string; to: string; categories: string; body: string };
  unread: number;
  starred: number;
  attachmentCount: number;
  /** lastMessageReceivedTimestamp */
  lmrt: number;
}

type Match = (t: Searchable) => boolean;

const wordsOf = (s: string) =>
  s
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);

/** FTS5's `"phrase"*`: the phrase's words in a row, the last one as a prefix. */
function phraseIn(column: string, phrase: string): boolean {
  const want = wordsOf(phrase);
  if (want.length === 0) return false;
  const have = wordsOf(column);
  const last = want.length - 1;
  for (let i = 0; i + want.length <= have.length; i++) {
    if (want.every((w, k) => (k === last ? have[i + k].startsWith(w) : have[i + k] === w))) {
      return true;
    }
  }
  return false;
}

/** A predicate for `text` in the grammar, its dates read in `tz` at `now`. */
export function grammarMatcher(text: string, tz?: string, now?: number): Match {
  if (tz !== undefined) {
    try {
      new Intl.DateTimeFormat('en-US', { timeZone: tz });
    } catch {
      fail(`search: search.tz "${tz}" is not an IANA time zone`);
    }
  }
  const build = (e: SearchExpr): Match => {
    switch (e.type) {
      case 'and':
      case 'or':
      case 'not': {
        const [a, b] = [build(e.e1), build(e.e2)];
        if (e.type === 'and') return (t) => a(t) && b(t);
        if (e.type === 'or') return (t) => a(t) || b(t);
        return (t) => a(t) && !b(t);
      }
      case 'generic':
        return (t) => Object.values(t.columns).some((c) => phraseIn(c, e.text));
      case 'from':
      case 'to':
      case 'subject':
      case 'in': {
        const column = e.type === 'in' ? 'categories' : e.type;
        return (t) => phraseIn(t.columns[column], e.text);
      }
      case 'unread':
        return (t) => t.unread > 0 === e.status;
      case 'starred':
        return (t) => t.starred > 0 === e.status;
      case 'hasAttachment':
        return (t) => t.attachmentCount !== 0;
      case 'date': {
        const start = resolveSearchDate(e.text, tz, now);
        return e.direction === 'before' ? (t) => t.lmrt < start : (t) => t.lmrt >= start;
      }
    }
  };
  return build(parseSearch(text));
}

/** Plain text: every word somewhere in the thread, nothing read as syntax. */
export function plainMatcher(text: string): Match {
  const terms = text.split(/\s+/).filter((w) => wordsOf(w).length > 0);
  if (terms.length === 0) return () => false;
  return (t) => terms.every((w) => Object.values(t.columns).some((c) => phraseIn(c, w)));
}
