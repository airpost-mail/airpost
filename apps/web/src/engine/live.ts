// The TanStack adapter over `@airpost/client` live queries: a
// query key whose data is a live query's window. The first read subscribes and resolves with
// the engine's first answer; each `query.changed` after that writes the new rows into the
// cache, so views update without refetching and nothing here decides membership or order.
// The subscription ends when TanStack drops the query (no observers for `LIVE_GC_MS`).
import { hashKey, queryOptions, type QueryClient, type QueryKey } from '@tanstack/react-query';
import type { LiveQuery, QueryWindow } from '@airpost/client';
import type { EngineClient } from './client';
import { ensureConnected } from './connect';
import type { QueryParams } from './rpc';

/** A live query's window as the cache holds it. */
export interface LiveSnapshot<Row> {
  rows: Row[];
  /** Rows the whole query has. */
  total: number;
  /** Where `rows[0]` is in the query's order. */
  offset: number;
}

/** How long a live query outlives its last observer. */
export const LIVE_GC_MS = 15_000;

interface Entry {
  engine: EngineClient;
  live: LiveQuery<{ id: string }>;
  off: () => void;
}

const registries = new WeakMap<QueryClient, Map<string, Entry>>();

function registry(qc: QueryClient): Map<string, Entry> {
  let entries = registries.get(qc);
  if (!entries) {
    const map = new Map<string, Entry>();
    entries = map;
    registries.set(qc, map);
    qc.getQueryCache().subscribe((event) => {
      if (event.type !== 'removed') return;
      const entry = map.get(event.query.queryHash);
      if (!entry) return;
      map.delete(event.query.queryHash);
      entry.off();
      entry.live.close();
    });
  }
  return entries;
}

function snapshot<Row>(live: LiveQuery<{ id: string }>): LiveSnapshot<Row> {
  return { rows: live.rows as unknown as Row[], total: live.total, offset: live.offset };
}

function entryFor(
  qc: QueryClient,
  engine: EngineClient,
  key: QueryKey,
  query: () => QueryParams,
  window: QueryWindow | null,
  select: (snap: LiveSnapshot<never>) => unknown,
): Entry {
  const entries = registry(qc);
  const hash = hashKey(key);
  const existing = entries.get(hash);
  if (existing && existing.engine === engine && existing.live.state !== 'failed') return existing;
  if (existing) {
    existing.off();
    existing.live.close();
  }
  const live = engine.live.query<{ id: string }>(query(), window);
  const off = live.subscribe((q) => qc.setQueryData(key, select(snapshot(q))));
  const entry: Entry = { engine, live, off };
  entries.set(hash, entry);
  return entry;
}

/** The first answer for the window asked for, or the engine's refusal. */
function whenReady(live: LiveQuery<{ id: string }>): Promise<void> {
  if (live.ready) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const done = () => {
      offChange();
      offError();
    };
    const offChange = live.subscribe((q) => {
      if (!q.ready) return;
      done();
      resolve();
    });
    const offError = live.onError((err) => {
      done();
      reject(err);
    });
  });
}

export interface LiveOptions<Row, Data> {
  key: QueryKey;
  /** Called when the subscription starts; params a later read would differ in (a clock) stay. */
  query: () => QueryParams;
  /** Null for every row. */
  window?: QueryWindow | null;
  select: (snap: LiveSnapshot<Row>) => Data;
}

/**
 * Query options whose data is a live query's window, mapped by `select`. Use with `useQuery`,
 * `useQueries` or `ensureQueryData`; `setLiveWindow` moves the window.
 */
export function liveQueryOptions<Row, Data>(engine: EngineClient, opts: LiveOptions<Row, Data>) {
  const { key, query, window = null, select } = opts;
  return queryOptions({
    queryKey: key,
    queryFn: async ({ client: qc }): Promise<Data> => {
      await ensureConnected(engine);
      const entry = entryFor(qc, engine, key, query, window, select as never);
      await whenReady(entry.live);
      return select(snapshot<Row>(entry.live));
    },
    staleTime: Infinity,
    gcTime: LIVE_GC_MS,
  });
}

/** Moves the window of the live query under `key`, if it has one. */
export function setLiveWindow(qc: QueryClient, key: QueryKey, window: QueryWindow | null) {
  registry(qc).get(hashKey(key))?.live.setWindow(window);
}

/**
 * Subscribes the live queries under `queryKey` again and reads them: for a change the engine
 * sends no deltas for (an account reset or restart). Other queries under it are refetched.
 */
export function rereadLive(qc: QueryClient, queryKey: QueryKey) {
  const entries = registry(qc);
  for (const query of qc.getQueryCache().findAll({ queryKey })) {
    const entry = entries.get(query.queryHash);
    if (!entry) continue;
    entries.delete(query.queryHash);
    entry.off();
    entry.live.close();
  }
  return qc.invalidateQueries({ queryKey });
}
