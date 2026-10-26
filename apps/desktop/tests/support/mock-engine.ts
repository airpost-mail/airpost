// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
An engine for specs that answers `query` from fixture models, standing in for the window's
connection to `serve`. It reads Query.toJSON() the way
crates/tern-store/src/model_query.rs does for what specs use: matchers run as the desktop's own
Matcher on the fixtures, then the order, offset and limit, `count`, `idsOnly` and `include`.
`search` keeps the models whose JSON contains the text. Answers go through JSON, as on the wire.

install() connects it to the window's EngineEvents and turns `core.engine.reads` to "engine";
kill() drops the connection (requests out fail as a closed socket fails them) and start()
connects again, which is how the reconnect specs drive the engine.

With Feature.LiveQueries (`MockEngine.live()`) it also answers `query.subscribe`,
`query.setWindow` and `query.unsubscribe`: add() and remove() read every subscription again
on the next turn and send `query.changed` with the whole window (`reset`) when it changed, as
the engine sends its steps.
*/
import EngineEvents, { EngineEvents as EngineEventsType } from '../../src/engine/client/events';
import {
  EngineUnavailableError,
  ErrorCode,
  Feature,
  Method,
  QueryResult,
  RpcError,
} from '../../src/engine/protocol';
import EngineReads from '../../src/data/database/engine-reads';
import { Matcher } from '../../src/data/attributes/matcher';
import { AttributeJoinedData } from '../../src/data/attributes/joined-data';
import DatabaseObjectRegistry from '../../src/data/database-object-registry';
import type ModelQuery from '../../src/data/query/query';
import type { ModelQueryJSON } from '../../src/data/query/query';
import DatabaseStore from '../../src/data/database/store';
import QuerySubscriptionPool from '../../src/data/query/subscription-pool';
import type { MatcherJSON } from '../../src/data/attributes/matcher';
import { Model } from '../../src/data/models/model';

type Pending = { reject: (err: Error) => void };

interface LiveSub {
  key: string | null;
  query: any;
  window: { offset?: number; limit?: number } | null;
  version: number;
  last: string;
}

export class MockEngine {
  events: EngineEventsType;
  features: string[];
  // Fixtures by class name; joined data (Message bodies) by model id.
  models = new Map<string, Model[]>();
  joined = new Map<string, { [modelKey: string]: string | null }>();
  requests: Array<{ method: string; params: any }> = [];
  running = false;
  _pending = new Set<Pending>();
  live = new Map<string, LiveSub>();
  _nextSub = 0;
  _flushQueued = false;

  // An engine with live queries, as the bundled one is.
  static live() {
    return new MockEngine({ features: [Feature.Query, Feature.Counts, Feature.LiveQueries] });
  }

  constructor({
    events = EngineEvents,
    features = [Feature.Query, Feature.Counts],
  }: { events?: EngineEventsType; features?: string[] } = {}) {
    this.events = events;
    this.features = features;
  }

  install() {
    spyOn(EngineReads, '_flag').andReturn('engine');
    this.start();
    return this;
  }

  uninstall() {
    this.kill();
    if (this.features.includes(Feature.LiveQueries)) leaveLiveQueries();
  }

  start() {
    this.running = true;
    this.events.connected(this.features, (method, params) => this._request(method, params));
  }

  kill() {
    if (!this.running) return;
    this.running = false;
    this.live.clear();
    const gone = new EngineUnavailableError('The sync engine connection closed');
    for (const pending of [...this._pending]) pending.reject(gone);
    this._pending.clear();
    this.events.gone();
  }

  add(...models: Model[]) {
    for (const model of models) {
      const name = this._tableOf(model.constructor as typeof Model);
      const list = (this.models.get(name) || []).filter((m) => m.id !== model.id);
      list.push(model);
      this.models.set(name, list);
      const klass = model.constructor as typeof Model;
      for (const key of Object.keys(klass.attributes)) {
        if (!(klass.attributes[key] instanceof AttributeJoinedData)) continue;
        const value = model[key];
        if (value === undefined) continue;
        this.joined.set(model.id, { ...(this.joined.get(model.id) || {}), [key]: value });
      }
    }
    this._queueFlush();
  }

  remove(klass: typeof Model, id: string) {
    const name = this._tableOf(klass);
    this.models.set(
      name,
      (this.models.get(name) || []).filter((m) => m.id !== id)
    );
    this._queueFlush();
  }

  /** The live queries' params by subId, as `query.subscribe` gave them. */
  subscriptions() {
    return [...this.live.values()].map((sub) => sub.query);
  }

  /** Reads every live query again and sends `query.changed` for the ones that changed. */
  flush() {
    this._flushQueued = false;
    if (!this.running) return;
    for (const [subId, sub] of this.live) {
      const { rows, total } = this._liveRead(sub.query, sub.window);
      const now = JSON.stringify({ rows, total });
      if (now === sub.last) continue;
      sub.last = now;
      sub.version += 1;
      this.events.notification(Method.QueryChanged, {
        subId,
        version: sub.version,
        total,
        reset: JSON.parse(JSON.stringify(rows)),
      });
    }
  }

  _queueFlush() {
    if (this._flushQueued || this.live.size === 0) return;
    this._flushQueued = true;
    Promise.resolve().then(() => this.flush());
  }

  _liveRead(query: any, window: LiveSub['window']) {
    const json = {
      ...query,
      include: query.include || [],
      order: query.order || [],
      offset: window?.offset ?? null,
      limit: window?.limit ?? null,
      idsOnly: false,
      count: false,
    };
    const { rows } = this.answer(json);
    const { count } = this.answer({ ...json, offset: null, limit: null, count: true });
    return { rows, total: count };
  }

  _subscribe(params: any) {
    const key = params.key ?? null;
    if (key !== null) {
      for (const [id, sub] of this.live) if (sub.key === key) this.live.delete(id);
    }
    const window = params.window ?? null;
    const { rows, total } = this._liveRead(params.query, window);
    const subId = `q${++this._nextSub}`;
    this.live.set(subId, {
      key,
      query: params.query,
      window,
      version: 0,
      last: JSON.stringify({ rows, total }),
    });
    return { subId, rows, total, version: 0 };
  }

  _setWindow(params: any) {
    const sub = this.live.get(params.subId);
    if (!sub) {
      throw new RpcError({ code: ErrorCode.NotFound, message: `no subscription ${params.subId}` });
    }
    sub.window = params.window;
    const { rows, total } = this._liveRead(sub.query, sub.window);
    sub.last = JSON.stringify({ rows, total });
    sub.version += 1;
    return { rows, total, version: sub.version };
  }

  queries(className?: string): ModelQueryJSON[] {
    return this.requests
      .filter((r) => r.method === Method.Query && (!className || r.params.class === className))
      .map((r) => r.params);
  }

  answer(json: ModelQueryJSON): QueryResult {
    const klass = DatabaseObjectRegistry.get(json.class) as unknown as typeof Model;
    if (!klass) throw rpcError(`invalid params: unknown class ${json.class}`);
    let rows = [...(this.models.get(json.class) || [])];
    if (json.accounts) rows = rows.filter((m) => json.accounts.includes(m.accountId));
    if (json.where) {
      const matcher = matcherFor(klass, json.where);
      rows = rows.filter((m) => matcher.evaluate(m));
    }
    if (json.search) {
      const text = json.search.text.toLowerCase();
      rows = rows.filter((m) => JSON.stringify(m.toJSON()).toLowerCase().includes(text));
    }
    if (json.count) return { rows: [], count: rows.length };
    for (const { attr, dir } of [...(json.order || [])].reverse()) {
      if (!klass.attributes[attr]) throw rpcError(`invalid params: unknown attribute ${attr}`);
      const sign = dir === 'desc' ? -1 : 1;
      rows.sort((a, b) => sign * compare(a[attr], b[attr]));
    }
    const offset = json.offset || 0;
    rows = rows.slice(offset, json.limit === null ? undefined : offset + json.limit);
    if (json.idsOnly) return { rows: [], ids: rows.map((m) => m.id) };
    const summary = json.class === 'Thread' && json.include.includes('summary');
    return {
      rows: rows.map((m) => {
        const row = m.toJSON();
        if (summary) row.summary = this.summaryOf(m.id);
        for (const key of Object.keys(klass.attributes)) {
          const attr = klass.attributes[key];
          if (!(attr instanceof AttributeJoinedData)) continue;
          delete row[attr.jsonKey || key];
          if (json.include.includes(key)) {
            const value = (this.joined.get(m.id) || {})[key];
            row[key] = value === undefined ? null : value;
          }
        }
        return row;
      }),
    };
  }

  /** The thread's `summary` (ThreadSummaryJson) from the fixture messages, as summary.rs makes it. */
  summaryOf(threadId: string) {
    const messages = (this.models.get('Message') || [])
      .filter((m: any) => m.threadId === threadId && !m.isHidden())
      .sort((a: any, b: any) => a.date - b.date) as any[];
    const sent = messages.filter((m) => !m.draft);
    const field = sent.every((m) => m.isFromMe()) ? 'to' : 'from';
    const senders: Array<{ email: string; name?: string; me: boolean; unread: boolean }> = [];
    for (const m of sent) {
      for (const c of m[field] || []) {
        const last = senders[senders.length - 1];
        if (last && last.email.toLowerCase() === c.email.toLowerCase() && last.name === c.name) {
          last.unread = last.unread || !!m.unread;
        } else {
          senders.push({ email: c.email, name: c.name, me: c.isMe(), unread: !!m.unread });
        }
      }
    }
    const last = sent[sent.length - 1];
    const lastReply =
      sent.length > 1 && last.from[0] && last.from[0].isMe()
        ? last.isForwarded()
          ? 'forwarded'
          : 'replied'
        : undefined;
    const snippets = messages.map((m) => m.snippet).filter(Boolean);
    return {
      messageCount: messages.length,
      senders,
      hasDraft: messages.some((m) => m.draft),
      sending: false,
      hasAttachment: messages.some((m) => (m.files || []).some((f: any) => !f.contentId)),
      snippet: snippets.length ? snippets[snippets.length - 1] : null,
      ...(lastReply ? { lastReply } : {}),
    };
  }

  _tableOf(klass: typeof Model): string {
    return ((klass as any).SubclassesUseModelTable || klass).name;
  }

  _request(method: string, params: any): Promise<unknown> {
    this.requests.push({ method, params });
    return new Promise((resolve, reject) => {
      const pending = { reject };
      this._pending.add(pending);
      // Answered after the caller's current turn, as a socket reply is.
      Promise.resolve().then(() => {
        if (!this._pending.has(pending)) return;
        this._pending.delete(pending);
        try {
          resolve(JSON.parse(JSON.stringify(this._handle(method, params))));
        } catch (err) {
          reject(err);
        }
      });
    });
  }

  _handle(method: string, params: any): unknown {
    switch (method) {
      case Method.Query:
        return this.answer(params);
      case Method.CountsGet:
        return { counts: {} };
      case Method.QuerySubscribe:
        return this._subscribe(params);
      case Method.QuerySetWindow:
        return this._setWindow(params);
      case Method.QueryUnsubscribe:
        this.live.delete(params.subId);
        return {};
      default:
        throw new RpcError({ code: ErrorCode.MethodNotFound, message: `unknown method ${method}` });
    }
  }
}

// The pool's subscriptions that went live on this engine (module-level ones too: TaskQueue's,
// the category lists) read the way the next spec's engine says, as after a restart.
function leaveLiveQueries() {
  for (const subscription of Object.values(QuerySubscriptionPool._subscriptions)) {
    if (subscription._mode !== 'live') continue;
    subscription._closeLive();
    subscription._mode = null;
  }
}

function rpcError(message: string) {
  return new RpcError({ code: ErrorCode.InvalidParams, message });
}

function compare(a: any, b: any) {
  const value = (v: any) => (v instanceof Date ? v.getTime() : v);
  const x = value(a);
  const y = value(b);
  if (x === y) return 0;
  if (x === undefined || x === null) return -1;
  if (y === undefined || y === null) return 1;
  return x < y ? -1 : 1;
}

function matcherFor(klass: typeof Model, json: MatcherJSON): Matcher {
  if ('and' in json) return new Matcher.And(json.and.map((m) => matcherFor(klass, m)));
  if ('or' in json) return new Matcher.Or(json.or.map((m) => matcherFor(klass, m)));
  if ('not' in json) return new Matcher.Not(json.not.map((m) => matcherFor(klass, m)));
  const attr = klass.attributes[json.attr];
  if (!attr || !attr.queryable) {
    throw rpcError(`invalid params: ${json.attr} is not queryable on ${klass.name}`);
  }
  return new Matcher(attr, json.op, json.value);
}

// Waits out the mock's replies and what the callers do with them, React's passive effects too.
export async function settle(turns = 20) {
  for (let i = 0; i < turns; i++) await Promise.resolve();
  await new Promise((resolve) => setImmediate(resolve));
}

export type Transport = 'sqlite' | 'engine';

/*
Fixtures for DatabaseStore reads on either transport, so one spec runs on both. "engine"
installs the MockEngine; "sqlite" sets `core.engine.reads` to "sqlite" and answers the SQLite
path's `_query` with the rows the same fixtures give, in better-sqlite3's shape (`data`, joined
columns, `id`, `count`), so `Query.inflateResult` runs as it does on a real database.
*/
export class FixtureReads {
  transport: Transport;
  engine = new MockEngine();

  constructor(transport: Transport) {
    this.transport = transport;
  }

  install() {
    if (this.transport === 'engine') {
      this.engine.install();
      return this;
    }
    spyOn(EngineReads, '_flag').andReturn('sqlite');
    // _inflated calls _query synchronously on the SQLite path, so the query is the last one
    // _inflated was given; other SQL (transactions) gets no rows.
    let current: ModelQuery<any> | null = null;
    const inflated = DatabaseStore._inflated;
    spyOn(DatabaseStore, '_inflated').andCallFake((query: ModelQuery<any>) => {
      current = query;
      return inflated.call(DatabaseStore, query);
    });
    jasmine.unspy(DatabaseStore, '_query');
    spyOn(DatabaseStore, '_query').andCallFake(() => {
      const query = current;
      current = null;
      if (!query) return Promise.resolve([]);
      const json = query.toJSON();
      this.engine.requests.push({ method: Method.Query, params: json });
      return Promise.resolve().then(() => sqliteRows(query, this.engine.answer(json)));
    });
    return this;
  }

  uninstall() {
    if (this.transport === 'engine') this.engine.uninstall();
  }

  add(...models: Model[]) {
    this.engine.add(...models);
  }

  /** The reads sent, as Query.toJSON(), on either transport. */
  queries(className?: string): ModelQueryJSON[] {
    return this.engine.queries(className);
  }
}

function sqliteRows(query: ModelQuery<any>, result: QueryResult) {
  const q = query as any;
  if (q._count) return [{ count: result.count }];
  if (q._returnIds) return (result.ids || []).map((id) => ({ id }));
  return (result.rows || []).map((json) => {
    const data = { ...json };
    const row: Record<string, unknown> = {};
    for (const attr of q._includeJoinedData as AttributeJoinedData[]) {
      const value = json[attr.modelKey];
      delete data[attr.modelKey];
      row[attr.tableColumn] =
        value === null || value === undefined ? AttributeJoinedData.NullPlaceholder : value;
    }
    row.data = JSON.stringify(data);
    return row;
  });
}

/**
 * Runs `body` once per transport, each in its own describe, with a FixtureReads installed.
 * Specs that assert on SQL text keep a sqlite-only branch (`reads().transport`).
 */
export function forEachTransport(body: (reads: () => FixtureReads) => void) {
  for (const transport of ['sqlite', 'engine'] as Transport[]) {
    describe(`on ${transport}`, function () {
      let reads: FixtureReads = null;
      beforeEach(function () {
        reads = new FixtureReads(transport).install();
      });
      afterEach(function () {
        reads.uninstall();
        reads = null;
      });
      body(() => reads);
    });
  }
}
