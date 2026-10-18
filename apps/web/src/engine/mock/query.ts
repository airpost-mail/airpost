// The mock's `query` (DCP 3.2): the engine's compiler (crates/ms-store/src/model_query.rs)
// over the in-memory store. Attribute names are the desktop's model keys from the engine's
// allowlist (crates/ms-store/src/model_query_allowlist.json), for the classes the mock holds.
import {
  RpcError,
  RpcErrorCode,
  type MatcherJSON,
  type QueryParams,
  type QueryResult,
} from '../rpc';
import type { ModelJSON, ThreadJSON } from '../types';
import { grammarMatcher } from './search';
import type { MockStore } from './store';

type Value = string | number | boolean | null | undefined;
/** A scalar attribute, or a collection's values. */
type Attr = { get: (m: never) => Value } | { values: (m: never) => string[] };

interface ClassDef {
  rows: (s: MockStore) => ModelJSON[];
  attrs: Record<string, Attr>;
  natural?: { attr: string; dir: 'asc' | 'desc' };
  /** The text `search` matches by word prefix. */
  searchText?: (m: never, s: MockStore) => string;
}

const base = {
  id: { get: (m: ModelJSON) => m.id },
  accountId: { get: (m: ModelJSON) => m.aid },
};

const metadata = {
  values: (m: { metadata?: { pluginId: string }[] }) => (m.metadata ?? []).map((x) => x.pluginId),
};

function icsField(ics: string, name: string): string {
  const unfolded = ics.replace(/\r?\n[ \t]/g, '');
  const m = new RegExp(`^${name}(?:;[^:\\r\\n]*)?:(.*)$`, 'm').exec(unfolded);
  return m ? m[1].trim() : '';
}

const CLASSES: Record<string, ClassDef> = {
  Thread: {
    rows: (s) => [...s.threads.values()],
    attrs: {
      ...base,
      subject: { get: (t: { subject: string }) => t.subject },
      unread: { get: (t: { unread: number }) => t.unread > 0 },
      starred: { get: (t: { starred: number }) => t.starred > 0 },
      inAllMail: { get: (t: { inAllMail: boolean }) => t.inAllMail },
      version: { get: (t: ModelJSON) => t.v },
      firstMessageTimestamp: { get: (t: { fmt: number }) => t.fmt },
      lastMessageReceivedTimestamp: { get: (t: { lmrt: number }) => t.lmrt },
      lastMessageSentTimestamp: { get: (t: { lmst: number }) => t.lmst },
      categories: {
        values: (t: { folders: CategoryRef[]; labels: CategoryRef[] }) =>
          [...t.folders, ...t.labels].filter((c) => c._refs > 0).map((c) => c.id),
      },
      pluginMetadata: metadata,
    },
    natural: { attr: 'lastMessageReceivedTimestamp', dir: 'desc' },
    searchText: (
      t: { id: string; subject: string; participants: { email: string; name?: string }[] },
      s,
    ) =>
      [
        t.subject,
        ...t.participants.flatMap((p) => [p.email, p.name ?? '']),
        ...s.threadMessages(t.id).map((m) => m.snippet ?? ''),
      ].join(' '),
  },
  Message: {
    rows: (s) => [...s.messages.values()].map((m) => s.wire(m)),
    attrs: {
      ...base,
      date: { get: (m: { date: number }) => m.date },
      draft: { get: (m: { draft: boolean }) => m.draft },
      unread: { get: (m: { unread: boolean }) => m.unread },
      starred: { get: (m: { starred: boolean }) => m.starred },
      threadId: { get: (m: { threadId: string }) => m.threadId },
      headerMessageId: { get: (m: { hMsgId: string }) => m.hMsgId },
      version: { get: (m: ModelJSON) => m.v },
      pluginMetadata: metadata,
    },
    natural: { attr: 'date', dir: 'asc' },
  },
  Folder: {
    rows: (s) => [...s.categories.values()].filter((c) => c.__cls === 'Folder'),
    attrs: categoryAttrs(),
  },
  Label: {
    rows: (s) => [...s.categories.values()].filter((c) => c.__cls === 'Label'),
    attrs: categoryAttrs(),
  },
  Contact: {
    rows: (s) => [...s.contacts.values()],
    attrs: {
      ...base,
      email: { get: (c: { email: string }) => c.email },
      hidden: { get: (c: { h?: boolean }) => !!c.h },
      refs: { get: (c: { refs: number }) => c.refs },
      source: { get: (c: { s?: string }) => c.s ?? 'mail' },
      contactGroups: { values: (c: { gis: string[] }) => c.gis },
    },
    natural: { attr: 'id', dir: 'desc' },
    searchText: (c: { name: string; email: string }) => `${c.name} ${c.email}`,
  },
  Calendar: {
    rows: (s) => [...s.calendars.values()],
    attrs: { ...base },
  },
  Event: {
    rows: (s) => [...s.events.values()],
    attrs: {
      ...base,
      calendarId: { get: (e: { cid: string }) => e.cid },
      icsuid: { get: (e: { icsuid: string }) => e.icsuid },
      recurrenceId: { get: (e: { rid?: string }) => e.rid ?? '' },
      recurrenceStart: { get: (e: { rs: number }) => e.rs },
      recurrenceEnd: { get: (e: { re: number }) => e.re },
    },
    natural: { attr: 'id', dir: 'desc' },
    searchText: (e: { ics: string }) =>
      ['SUMMARY', 'DESCRIPTION', 'LOCATION'].map((f) => icsField(e.ics, f)).join(' '),
  },
  Task: {
    rows: (s) => [...s.tasks.values()],
    attrs: { ...base, status: { get: (t: { status: string }) => t.status } },
  },
  // The mock keeps no rows of these.
  ContactGroup: { rows: () => [], attrs: { ...base }, natural: { attr: 'id', dir: 'asc' } },
  ContactBook: { rows: () => [], attrs: { ...base }, natural: { attr: 'id', dir: 'asc' } },
  File: { rows: () => [], attrs: { ...base } },
};

interface CategoryRef {
  id: string;
  _refs: number;
}

function categoryAttrs(): Record<string, Attr> {
  return {
    ...base,
    path: { get: (c: { path: string }) => c.path },
    role: { get: (c: { role: string }) => c.role },
  };
}

const KEYS = new Set([
  'class',
  'accounts',
  'where',
  'search',
  'order',
  'limit',
  'offset',
  'cursor',
  'distinct',
  'include',
  'idsOnly',
  'count',
]);

function invalid(why: string): never {
  throw new RpcError({ code: RpcErrorCode.InvalidParams, message: `invalid params: ${why}` });
}

type Pred = (m: ModelJSON) => boolean;

function compareValues(a: Value, b: Value): number {
  if (a === b) return 0;
  // SQLite sorts NULL first.
  if (a === null || a === undefined) return -1;
  if (b === null || b === undefined) return 1;
  const x = typeof a === 'boolean' ? Number(a) : a;
  const y = typeof b === 'boolean' ? Number(b) : b;
  return x < y ? -1 : x > y ? 1 : 0;
}

function scalarEq(v: Value, want: unknown): boolean {
  if (typeof v === 'boolean') return v === !!want;
  return v === want;
}

function leaf(
  def: ClassDef,
  cls: string,
  node: { attr: string; op: string; value: unknown },
): Pred {
  const attr = def.attrs[node.attr];
  if (!attr) invalid(`${cls} has no queryable attribute "${node.attr}"`);
  const { op, value } = node;
  if ('values' in attr) {
    const values = attr.values as (m: ModelJSON) => string[];
    if (op === 'contains') return (m) => values(m).includes(value as string);
    if (op === 'containsAny') {
      if (!Array.isArray(value)) invalid('containsAny takes a list');
      return (m) => values(m).some((v) => value.includes(v));
    }
    invalid(`"${node.attr}" is a collection: use contains or containsAny`);
  }
  const get = attr.get as (m: ModelJSON) => Value;
  const list = () => {
    if (!Array.isArray(value)) invalid(`${op} takes a list`);
    return value as unknown[];
  };
  switch (op) {
    case '=':
      return (m) => scalarEq(get(m), value);
    case '!=':
      return (m) => !scalarEq(get(m), value);
    case '<':
      return (m) => compareValues(get(m), value as Value) < 0;
    case '>':
      return (m) => compareValues(get(m), value as Value) > 0;
    case '<=':
      return (m) => compareValues(get(m), value as Value) <= 0;
    case '>=':
      return (m) => compareValues(get(m), value as Value) >= 0;
    case 'in': {
      const l = list();
      return (m) => l.some((x) => scalarEq(get(m), x));
    }
    case 'not in': {
      const l = list();
      return (m) => !l.some((x) => scalarEq(get(m), x));
    }
    case 'startsWith':
      return (m) => String(get(m) ?? '').startsWith(String(value));
    case 'like':
      return (m) =>
        String(get(m) ?? '')
          .toLowerCase()
          .includes(String(value).toLowerCase());
    case 'contains':
    case 'containsAny':
      return invalid(`"${node.attr}" is not a collection`);
    default:
      return invalid(`unknown op "${op}"`);
  }
}

function matcher(def: ClassDef, cls: string, node: MatcherJSON): Pred {
  if (typeof node !== 'object' || node === null) invalid('a matcher is an object');
  if ('and' in node) {
    const ps = node.and.map((n) => matcher(def, cls, n));
    return (m) => ps.every((p) => p(m));
  }
  if ('or' in node) {
    const ps = node.or.map((n) => matcher(def, cls, n));
    return (m) => ps.some((p) => p(m));
  }
  if ('not' in node) {
    const ps = node.not.map((n) => matcher(def, cls, n));
    return (m) => !ps.every((p) => p(m));
  }
  return leaf(def, cls, node);
}

/** Word-prefix match of every term, like FTS5's `"text"*` on the indexed columns. */
function searchPred(def: ClassDef, cls: string, s: MockStore, text: string): Pred {
  if (!def.searchText) invalid(`${cls} has no search index`);
  const terms = text.toLowerCase().replace(/["']/g, ' ').split(/\s+/).filter(Boolean);
  const indexed = def.searchText as (m: ModelJSON, s: MockStore) => string;
  return (m) => {
    const words = indexed(m, s)
      .toLowerCase()
      .split(/[^\p{L}\p{N}]+/u);
    return terms.every((t) => words.some((w) => w.startsWith(t)));
  };
}

export function runQuery(
  s: MockStore,
  p: QueryParams,
  checkAccount: (id: string) => void,
  opts: { grammar: boolean },
): QueryResult {
  if (typeof p !== 'object' || p === null) invalid('params must be an object');
  for (const k of Object.keys(p)) if (!KEYS.has(k)) invalid(`unknown field \`${k}\``);
  const def = CLASSES[p.class];
  if (!def) invalid(`no class "${p.class}"`);
  // An engine without the grammar refuses `grammar: true` as one before stage 9 did.
  if (p.search?.grammar && !opts.grammar) {
    throw new RpcError({
      code: RpcErrorCode.InvalidParams,
      message: 'unsupported: search.grammar is not supported by this engine yet',
      data: { unsupported: 'search-grammar' },
    });
  }
  if (p.search?.grammar && p.class !== 'Thread') {
    invalid(`the search grammar finds threads; search ${p.class} with grammar: false`);
  }
  for (const a of p.accounts ?? []) checkAccount(a);

  const preds: Pred[] = [];
  if (p.accounts) {
    const accounts = new Set(p.accounts);
    preds.push((m) => accounts.has(m.aid));
  }
  const where = p.where ?? [];
  for (const n of Array.isArray(where) ? where : [where]) preds.push(matcher(def, p.class, n));
  if (p.search?.grammar) {
    const match = grammarMatcher(p.search.text, p.search.tz, p.search.now);
    preds.push((m) => match(s.searchable(m as ThreadJSON)));
  } else if (p.search) {
    const search = p.search.text.trim();
    if (!search) invalid('search.text has nothing to match');
    preds.push(searchPred(def, p.class, s, search));
  }

  let rows = def.rows(s).filter((m) => preds.every((f) => f(m)));
  if (p.count) return { rows: [], count: rows.length };

  const order = p.order?.length ? p.order : def.natural ? [def.natural] : [];
  for (const o of order) {
    const a = def.attrs[o.attr];
    if (!a || !('get' in a) || !['asc', 'desc'].includes(o.dir)) {
      invalid(`${p.class} can't be ordered by "${o.attr}"`);
    }
  }
  const cursor = p.cursor ?? null;
  // A cursor query breaks ties on the id, in the direction of the last order.
  const keys =
    cursor !== null
      ? [...order.filter((o) => o.attr !== 'id'), { attr: 'id', dir: order.at(-1)?.dir ?? 'asc' }]
      : order;
  const keyOf = (m: ModelJSON) =>
    keys.map((k) => (def.attrs[k.attr] as { get: (m: ModelJSON) => Value }).get(m));
  const compare = (a: Value[], b: Value[]) => {
    for (let i = 0; i < keys.length; i++) {
      const c = compareValues(a[i], b[i]);
      if (c) return keys[i].dir === 'desc' ? -c : c;
    }
    return 0;
  };
  rows = rows.sort((x, y) => compare(keyOf(x), keyOf(y)));
  if (cursor) {
    let after: Value[];
    try {
      after = JSON.parse(cursor) as Value[];
    } catch {
      invalid('cursor is not one this engine wrote');
    }
    rows = rows.filter((m) => compare(keyOf(m), after) > 0);
  }
  rows = rows.slice(p.offset ?? 0);
  const limit = p.limit ?? null;
  const more = limit !== null && rows.length > limit;
  if (limit !== null) rows = rows.slice(0, limit);
  if (p.idsOnly) return { rows: [], ids: rows.map((m) => m.id) };

  const include = p.include ?? [];
  for (const i of include) {
    if (p.class !== 'Message' || i !== 'body') invalid(`${p.class} has no joined data "${i}"`);
  }
  const out = rows.map((m) => {
    const copy = structuredClone(m) as ModelJSON & Record<string, unknown>;
    if (include.includes('body')) copy.body = s.body(m.id);
    return copy;
  });
  const result: QueryResult = { rows: out };
  if (cursor !== null) {
    result.nextCursor = more ? JSON.stringify(keyOf(rows[rows.length - 1])) : null;
  }
  return result;
}
