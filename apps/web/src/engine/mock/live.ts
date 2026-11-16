// The mock's live queries (`query.subscribe|setWindow|unsubscribe`, `query.changed`;
// apps/engine/src/live.rs): after each flush every subscription whose class it touched reads
// its window again, and sends the whole window as `reset` when it changed. The engine sends
// steps; a reset is what it sends for a big change, and every client applies both.
import type { QueryChangedParams, QueryWindow } from '@airpost/client';
import {
  RpcError,
  RpcErrorCode,
  type QueryParams,
  type QueryResult,
  type RpcMethods,
} from '../rpc';

interface Sub {
  query: QueryParams;
  window: QueryWindow | null;
  key: string | null;
  version: number;
  total: number;
  /** The last rows sent, as JSON, to tell a change from a re-read that found none. */
  sent: string;
}

type Params<M extends 'query.subscribe' | 'query.setWindow' | 'query.unsubscribe'> =
  RpcMethods[M]['params'];

/** The `query` keys a subscription's window replaces. */
const WINDOW_KEYS = ['limit', 'offset', 'cursor', 'idsOnly', 'count'] as const;

export class MockLiveQueries {
  private readonly subs = new Map<string, Sub>();
  private next = 0;

  constructor(
    private readonly run: (q: QueryParams) => QueryResult,
    private readonly send: (p: QueryChangedParams) => void,
  ) {}

  subscribe(p: Params<'query.subscribe'>) {
    const query = { ...(p.query as QueryParams) };
    for (const k of WINDOW_KEYS) delete query[k];
    const window = p.window ?? null;
    const { rows, total } = this.read(query, window);
    if (p.key) {
      for (const [id, s] of this.subs) if (s.key === p.key) this.subs.delete(id);
    }
    const subId = `mock-sub-${++this.next}`;
    this.subs.set(subId, {
      query,
      window,
      key: p.key ?? null,
      version: 0,
      total,
      sent: JSON.stringify(rows),
    });
    return { subId, rows, total, version: 0 };
  }

  setWindow(p: Params<'query.setWindow'>) {
    const sub = this.subs.get(p.subId);
    if (!sub)
      throw new RpcError({ code: RpcErrorCode.NotFound, message: `no subscription ${p.subId}` });
    sub.window = p.window;
    const { rows, total } = this.read(sub.query, sub.window);
    sub.version += 1;
    sub.total = total;
    sub.sent = JSON.stringify(rows);
    return { rows, total, version: sub.version };
  }

  unsubscribe(p: Params<'query.unsubscribe'>) {
    this.subs.delete(p.subId);
    return {};
  }

  /** A flush changed these classes: re-read the windows that follow one of them. */
  changed(classes: ReadonlySet<string>) {
    for (const [subId, sub] of this.subs) {
      if (!follows(sub.query, classes)) continue;
      let rows: unknown[];
      let total: number;
      try {
        ({ rows, total } = this.read(sub.query, sub.window));
      } catch {
        continue;
      }
      const sent = JSON.stringify(rows);
      if (sent === sub.sent && total === sub.total) continue;
      sub.version += 1;
      sub.sent = sent;
      sub.total = total;
      this.send({ subId, version: sub.version, total, reset: rows as never });
    }
  }

  /** The connection closed: its subscriptions are gone. */
  clear() {
    this.subs.clear();
  }

  private read(query: QueryParams, window: QueryWindow | null) {
    const { rows } = this.run({
      ...query,
      offset: window?.offset ?? 0,
      limit: window?.limit ?? undefined,
    });
    const { count } = this.run({ ...query, include: undefined, count: true });
    return { rows, total: count ?? rows.length };
  }
}

/** A Thread query with `summary` also follows its threads' messages. */
function follows(query: QueryParams, classes: ReadonlySet<string>): boolean {
  if (classes.has(query.class)) return true;
  return query.class === 'Thread' && !!query.include?.includes('summary') && classes.has('Message');
}
