// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Live queries (docs/thin-clients-plan.md 4.1) and the raw feed's resume point.

A LiveQuery is one `query.subscribe`: the rows of its window in the engine's order, the
query's `total`, and the `query.changed` steps applied to them as they come. The engine
decides membership and order; nothing here evaluates a matcher or a sort. LiveQueries holds a
connection's live queries: it routes `query.changed` by `subId`, subscribes each query again
after a reconnect (one `query.subscribe` each, no delta resume needed), and shares one
subscription between `watch()` callers that ask for the same query and window.

Ordering. The engine sends a subscription's answers and its `query.changed` in order, each
change one `version` past the last. A client may see them out of order (a desktop window
handles answers after the notifications read with them), so a change for a `subId` not known
yet is held until the answer names it, a change past the next version waits for the answer
in flight, and a gap with no answer in flight reads the window again (`query.setWindow` with
the same window).

FeedCursor keeps the last `delta.seq` a client applied, so the `subscribe` after a reconnect
can ask for what it missed (`since`); `resumed: false` in the answer means it must read again
what it shows.
*/
import {
  Method,
  type AnyModelJson,
  type DeltaCursor,
  type HelloResult,
  type LiveOp,
  type QueryChangedParams,
  type QueryParams,
  type QuerySubscribeResult,
  type QueryWindow,
  type QueryWindowResult,
  type SubscribeResult,
} from './protocol';
import { RpcError, type IncomingNotification } from './rpc';

/** What live queries need of a connection; a Session is one. */
export interface LiveSource {
  request(method: string, params?: unknown): Promise<unknown>;
  onNotification(listener: (n: IncomingNotification) => void): () => unknown;
  /** After each hello: the connection is new, and its subscriptions are gone. */
  onHello(listener: (hello: HelloResult) => void): () => unknown;
  onDisconnect(listener: () => void): () => unknown;
  /** The current connection's hello; null while there is none. */
  readonly hello: HelloResult | null;
}

/** A row of a live query: model JSON, which always has an id. */
export type LiveRow = { id: string } & Record<string, unknown>;

/** What changed in a LiveQuery: the whole window (`reset`), or the steps applied. */
export interface LiveChange {
  reset: boolean;
  ops?: LiveOp[];
}

export type LiveState = 'idle' | 'subscribing' | 'live' | 'failed' | 'closed';

/** A `query.subscribe`'s query: the keys that page or shape a `query` are the window's. */
export function liveQueryParams(query: QueryParams): QueryParams {
  const q: QueryParams = { ...query };
  delete q.limit;
  delete q.offset;
  delete q.cursor;
  delete q.idsOnly;
  delete q.count;
  return q;
}

/** `rows` with `ops` applied in order, as `query.changed` describes them; a new array. */
export function applyLiveOps<Row extends { id: string }>(rows: readonly Row[], ops: LiveOp[]): Row[] {
  const out = [...rows];
  const indexOf = (id: string) => out.findIndex((r) => r.id === id);
  for (const op of ops) {
    switch (op.op) {
      case 'remove': {
        const i = indexOf(op.id);
        if (i !== -1) out.splice(i, 1);
        break;
      }
      case 'insert':
        out.splice(Math.min(op.index, out.length), 0, op.row as unknown as Row);
        break;
      case 'move': {
        const i = indexOf(op.id);
        if (i === -1) break;
        const [row] = out.splice(i, 1);
        out.splice(Math.min(op.index, out.length), 0, row);
        break;
      }
      case 'update': {
        const row = op.row as unknown as Row;
        const i = indexOf(row.id);
        if (i !== -1) out[i] = row;
        break;
      }
    }
  }
  return out;
}

function sameWindow(a: QueryWindow | null, b: QueryWindow | null) {
  return (a?.offset ?? 0) === (b?.offset ?? 0) && (a?.limit ?? null) === (b?.limit ?? null);
}

// Changes held for a subId no answer has named yet: this many subIds, this many changes each.
const MAX_ORPHAN_SUBS = 32;
const MAX_ORPHAN_CHANGES = 64;

/**
 * One live query. Rows keep their object identity until a step replaces them, so a client can
 * cache what it builds from a row (its model) by the row.
 */
export class LiveQuery<Row extends { id: string } = LiveRow> {
  /** The `key` it subscribes with, so a new subscribe on the connection replaces the old. */
  readonly key: string;
  private _query: QueryParams;
  /** The window asked for. */
  private _window: QueryWindow | null;
  /** The window `rows` are for. */
  private _rowsWindow: QueryWindow | null = null;
  private _rows: Row[] = [];
  private _total = 0;
  private _version = 0;
  private _state: LiveState = 'idle';
  private _error: unknown = null;
  private _subId: string | null = null;
  /** Bumped by each subscribe and by a disconnect; an older answer is dropped. */
  private _gen = 0;
  /** A subscribe or setWindow is out. */
  private _inFlight = false;
  /** Changes past the next version, waiting for the answer in flight. */
  private _ahead = new Map<number, QueryChangedParams>();
  private readonly _listeners = new Set<(q: LiveQuery<Row>, change: LiveChange) => void>();
  private readonly _errorListeners = new Set<(err: unknown) => void>();

  constructor(
    private readonly _owner: LiveQueries,
    key: string,
    query: QueryParams,
    window: QueryWindow | null,
  ) {
    this.key = key;
    this._query = liveQueryParams(query);
    this._window = window;
  }

  get query(): QueryParams {
    return this._query;
  }
  /** The window asked for; `offset` is where `rows` start once the engine answered it. */
  get window(): QueryWindow | null {
    return this._window;
  }
  /** The window `rows` are for; it lags `window` while a move is out. */
  get rowsWindow(): QueryWindow | null {
    return this._rowsWindow;
  }
  /** The offset of `rows[0]` in the query's order. */
  get offset(): number {
    return this._rowsWindow?.offset ?? 0;
  }
  get rows(): readonly Row[] {
    return this._rows;
  }
  get total(): number {
    return this._total;
  }
  get version(): number {
    return this._version;
  }
  get state(): LiveState {
    return this._state;
  }
  /** Why the engine refused the subscription, while `state` is `failed`. */
  get error(): unknown {
    return this._error;
  }
  /** The engine answered at least once since the query or window last changed. */
  get ready(): boolean {
    return this._state === 'live' && sameWindow(this._rowsWindow, this._window);
  }
  get subId(): string | null {
    return this._subId;
  }

  /** Called after each change of rows or total. */
  subscribe(listener: (q: LiveQuery<Row>, change: LiveChange) => void): () => void {
    this._listeners.add(listener);
    return () => this._listeners.delete(listener);
  }

  /** Called when the engine refuses the subscription (an RpcError). */
  onError(listener: (err: unknown) => void): () => void {
    this._errorListeners.add(listener);
    return () => this._errorListeners.delete(listener);
  }

  /** Moves the window; the rows change when the engine answers. */
  setWindow(window: QueryWindow | null) {
    if (this._state === 'closed' || sameWindow(window, this._window)) return;
    this._window = window;
    if (this._state === 'live' && !this._inFlight) this._sendWindow();
  }

  /** Subscribes to another query under the same key, which ends the old subscription. */
  setQuery(query: QueryParams, window: QueryWindow | null = this._window) {
    if (this._state === 'closed') return;
    this._query = liveQueryParams(query);
    this._window = window;
    this._subscribe();
  }

  /** Ends the subscription. */
  close() {
    if (this._state === 'closed') return;
    const subId = this._subId;
    this._state = 'closed';
    this._gen += 1;
    this._subId = null;
    this._ahead.clear();
    this._listeners.clear();
    this._errorListeners.clear();
    this._owner._closed(this, subId);
  }

  // LiveQueries' side.

  /** (Re)subscribes on the current connection, or waits for the next hello. */
  _subscribe() {
    if (this._state === 'closed') return;
    const gen = ++this._gen;
    const old = this._subId;
    this._subId = null;
    this._ahead.clear();
    if (old) this._owner._forget(old);
    if (!this._owner.source.hello) {
      this._state = 'idle';
      this._inFlight = false;
      return;
    }
    this._state = 'subscribing';
    this._inFlight = true;
    const window = this._window;
    this._owner.source
      .request(Method.QuerySubscribe, { query: this._query, window, key: this.key })
      .then(
        (result) => {
          if (gen !== this._gen) {
            // Replaced or closed while it was out: the engine keeps it until told.
            const late = (result as QuerySubscribeResult | null)?.subId;
            if (late && this._state === 'closed') this._owner._unsubscribe(late);
            return;
          }
          const r = result as QuerySubscribeResult;
          this._inFlight = false;
          this._state = 'live';
          this._error = null;
          this._subId = r.subId;
          this._rowsWindow = window;
          this._rows = r.rows as unknown as Row[];
          this._total = r.total;
          this._version = r.version;
          this._owner._claim(this, r.subId);
          this._emit({ reset: true });
          this._afterAnswer();
        },
        (err: unknown) => {
          if (gen !== this._gen) return;
          this._inFlight = false;
          if (err instanceof RpcError) {
            this._state = 'failed';
            this._error = err;
            for (const l of [...this._errorListeners]) l(err);
          } else {
            // No answer: the next hello subscribes again.
            this._state = 'idle';
          }
        },
      );
  }

  _disconnected() {
    if (this._state === 'closed') return;
    this._gen += 1;
    this._subId = null;
    this._inFlight = false;
    this._ahead.clear();
    this._state = 'idle';
  }

  _changed(p: QueryChangedParams) {
    if (this._state !== 'live' || p.subId !== this._subId) return;
    if (p.version <= this._version) return;
    if (p.version > this._version + 1) {
      if (this._ahead.size < MAX_ORPHAN_CHANGES) this._ahead.set(p.version, p);
      // Only an answer still out can fill the gap; without one, read the window again.
      if (!this._inFlight) this._sendWindow(true);
      return;
    }
    this._apply(p);
    this._drainAhead();
  }

  private _apply(p: QueryChangedParams) {
    this._version = p.version;
    this._total = p.total;
    if (p.reset) {
      this._rows = p.reset as unknown as Row[];
      this._emit({ reset: true });
    } else {
      const ops = p.ops ?? [];
      this._rows = applyLiveOps(this._rows, ops);
      this._emit({ reset: false, ops });
    }
  }

  private _drainAhead() {
    for (;;) {
      const next = this._ahead.get(this._version + 1);
      if (!next) break;
      this._ahead.delete(next.version);
      this._apply(next);
    }
    for (const v of [...this._ahead.keys()]) if (v <= this._version) this._ahead.delete(v);
  }

  /** `query.setWindow` with the asked-for window; `force` when it is the same one. */
  private _sendWindow(force = false) {
    const subId = this._subId;
    if (!subId || (!force && sameWindow(this._window, this._rowsWindow))) return;
    const gen = this._gen;
    const window = this._window;
    this._inFlight = true;
    this._owner.source
      .request(Method.QuerySetWindow, { subId, window: window ?? {} })
      .then(
        (result) => {
          if (gen !== this._gen) return;
          const r = result as QueryWindowResult;
          this._inFlight = false;
          this._rowsWindow = window;
          this._rows = r.rows as unknown as Row[];
          this._total = r.total;
          this._version = r.version;
          this._emit({ reset: true });
          this._drainAhead();
          this._afterAnswer();
        },
        (err: unknown) => {
          if (gen !== this._gen) return;
          this._inFlight = false;
          // The engine lost the subscription (NOT_FOUND) or refused the window: start over.
          if (err instanceof RpcError) this._subscribe();
        },
      );
  }

  /** After an answer: the window may have moved again meanwhile. */
  private _afterAnswer() {
    if (this._state === 'live' && !this._inFlight && !sameWindow(this._window, this._rowsWindow)) {
      this._sendWindow();
    }
  }

  private _emit(change: LiveChange) {
    for (const l of [...this._listeners]) {
      try {
        l(this, change);
      } catch (err) {
        console.error('A live query listener failed', err);
      }
    }
  }
}

interface Shared {
  query: LiveQuery<LiveRow>;
  refs: number;
}

/** The live queries of one connection (a Session, or a client with the same methods). */
export class LiveQueries {
  private _next = 0;
  private readonly _queries = new Set<LiveQuery<any>>();
  private readonly _bySubId = new Map<string, LiveQuery<any>>();
  /** `query.changed` for a subId no answer named yet. */
  private readonly _orphans = new Map<string, QueryChangedParams[]>();
  private readonly _shared = new Map<string, Shared>();
  private readonly _off: Array<() => unknown>;

  constructor(
    readonly source: LiveSource,
    private readonly _keyPrefix = 'live-',
  ) {
    this._off = [
      source.onNotification((n) => {
        if (n.method === Method.QueryChanged && n.params) this._onChanged(n.params);
      }),
      source.onHello(() => {
        this._orphans.clear();
        for (const q of [...this._queries]) q._subscribe();
      }),
      source.onDisconnect(() => {
        this._bySubId.clear();
        this._orphans.clear();
        for (const q of this._queries) q._disconnected();
      }),
    ];
  }

  /** A live query of its own; it subscribes now, or after the next hello. */
  query<Row extends { id: string } = LiveRow>(
    query: QueryParams,
    window: QueryWindow | null = null,
  ): LiveQuery<Row> {
    const q = new LiveQuery<Row>(this, `${this._keyPrefix}${++this._next}`, query, window);
    this._queries.add(q);
    q._subscribe();
    return q;
  }

  /**
   * Follows `query` in `window`, sharing one subscription with the other watchers of the same
   * query and window. `listener` gets the current rows at once when there are any.
   */
  watch(
    query: QueryParams,
    window: QueryWindow | null,
    listener: (q: LiveQuery<LiveRow>, change: LiveChange) => void,
  ): () => void {
    const key = JSON.stringify([liveQueryParams(query), window]);
    let shared = this._shared.get(key);
    if (!shared) {
      shared = { query: this.query(query, window), refs: 0 };
      this._shared.set(key, shared);
    }
    const entry = shared;
    entry.refs += 1;
    const off = entry.query.subscribe(listener);
    if (entry.query.state === 'live') listener(entry.query, { reset: true });
    let done = false;
    return () => {
      if (done) return;
      done = true;
      off();
      entry.refs -= 1;
      if (entry.refs === 0) {
        this._shared.delete(key);
        entry.query.close();
      }
    };
  }

  /** How many live queries are open. */
  get size(): number {
    return this._queries.size;
  }

  /** Ends every live query and stops listening to the source. */
  close() {
    for (const q of [...this._queries]) q.close();
    for (const off of this._off) off();
  }

  // LiveQuery's side.

  _claim(q: LiveQuery<any>, subId: string) {
    this._bySubId.set(subId, q);
    const held = this._orphans.get(subId);
    if (!held) return;
    this._orphans.delete(subId);
    for (const p of held) q._changed(p);
  }

  _forget(subId: string) {
    this._bySubId.delete(subId);
    this._orphans.delete(subId);
  }

  _closed(q: LiveQuery<any>, subId: string | null) {
    this._queries.delete(q);
    if (subId) {
      this._forget(subId);
      this._unsubscribe(subId);
    }
  }

  _unsubscribe(subId: string) {
    if (!this.source.hello) return;
    this.source.request(Method.QueryUnsubscribe, { subId }).catch(() => {
      // Gone with its connection, or already ended.
    });
  }

  private _onChanged(p: QueryChangedParams) {
    const q = this._bySubId.get(p.subId);
    if (q) {
      q._changed(p);
      return;
    }
    let held = this._orphans.get(p.subId);
    if (!held) {
      if (this._orphans.size >= MAX_ORPHAN_SUBS) {
        const oldest = this._orphans.keys().next().value;
        if (oldest !== undefined) this._orphans.delete(oldest);
      }
      held = [];
      this._orphans.set(p.subId, held);
    }
    if (held.length < MAX_ORPHAN_CHANGES) held.push(p);
  }
}

/** Model JSON rows, for callers that don't narrow the row type. */
export type LiveModelQuery = LiveQuery<LiveRow & AnyModelJson>;

const SEQ_AT_START = /^\{"accountId":"(?:[^"\\]|\\.)*","seq":(\d+)/;

/**
 * The `seq` of a `delta` notification's params given as JSON text, as the engine writes them
 * (`accountId` first, then `seq`), without parsing the rest; null when it has none.
 */
export function deltaSeqOf(paramsText: string): number | null {
  const m = SEQ_AT_START.exec(paramsText);
  return m ? Number(m[1]) : null;
}

/**
 * The raw feed's resume point: the engine's `feed` and the last `delta.seq` applied. The
 * `subscribe` after a reconnect sends `since()`; `subscribed()` reads its answer.
 */
export class FeedCursor {
  private _feed: string | null = null;
  private _seq: number | null = null;

  get feed(): string | null {
    return this._feed;
  }
  get seq(): number | null {
    return this._seq;
  }

  /** Where to resume; null before the first subscribe answered. */
  since(): DeltaCursor | null {
    return this._feed !== null && this._seq !== null ? { feed: this._feed, seq: this._seq } : null;
  }

  /** A delta was applied. */
  delta(seq: number | null | undefined) {
    if (typeof seq !== 'number') return;
    if (this._seq === null || seq > this._seq) this._seq = seq;
  }

  /**
   * A `subscribe` answered. Returns `resumed` when it was sent with `since`: true when every
   * delta since then follows, false when the client must read again what it shows.
   */
  subscribed(result: SubscribeResult | null | undefined, sentSince: DeltaCursor | null): boolean | null {
    if (!result || typeof result.feed !== 'string') return null;
    const resumed = sentSince ? result.resumed === true && result.feed === sentSince.feed : null;
    // Resumed: the deltas after our seq follow and move it on. Otherwise the feed starts over
    // after the answer's seq.
    if (resumed !== true || this._feed !== result.feed) this._seq = result.seq;
    this._feed = result.feed;
    return resumed;
  }

  reset() {
    this._feed = null;
    this._seq = null;
  }
}
