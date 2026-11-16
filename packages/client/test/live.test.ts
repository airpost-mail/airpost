// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import { describe, expect, it } from 'vitest';
import {
  EngineUnavailableError,
  FeedCursor,
  LiveQueries,
  RpcError,
  applyLiveOps,
  deltaSeqOf,
  liveQueryParams,
  type HelloResult,
  type IncomingNotification,
  type LiveSource,
  type QueryParams,
} from '../src';

interface Call {
  method: string;
  params: any;
  resolve: (v: unknown) => void;
  reject: (e: unknown) => void;
}

/** A connection the spec answers by hand. */
class FakeSource implements LiveSource {
  hello: HelloResult | null = null;
  calls: Call[] = [];
  private notes = new Set<(n: IncomingNotification) => void>();
  private hellos = new Set<(h: HelloResult) => void>();
  private drops = new Set<() => void>();

  request(method: string, params?: unknown) {
    return new Promise((resolve, reject) => this.calls.push({ method, params, resolve, reject }));
  }
  onNotification(l: (n: IncomingNotification) => void) {
    this.notes.add(l);
    return () => this.notes.delete(l);
  }
  onHello(l: (h: HelloResult) => void) {
    this.hellos.add(l);
    return () => this.hellos.delete(l);
  }
  onDisconnect(l: () => void) {
    this.drops.add(l);
    return () => this.drops.delete(l);
  }

  connect() {
    this.hello = { protocol: 1, engine: 't', pid: 1, accounts: [], features: [] } as never;
    for (const l of [...this.hellos]) l(this.hello!);
  }
  disconnect() {
    this.hello = null;
    const out = this.calls;
    this.calls = [];
    for (const c of out) c.reject(new EngineUnavailableError('closed'));
    for (const l of [...this.drops]) l();
  }
  notify(method: string, params: unknown) {
    for (const l of [...this.notes]) l({ method, params });
  }
  /** Takes the oldest call of `method`. */
  take(method: string): Call {
    const i = this.calls.findIndex((c) => c.method === method);
    expect(i, `a ${method} call`).not.toBe(-1);
    return this.calls.splice(i, 1)[0];
  }
}

const tick = () => new Promise((r) => setTimeout(r, 0));
const row = (id: string, extra: Record<string, unknown> = {}) => ({ id, ...extra });
const ids = (rows: readonly { id: string }[]) => rows.map((r) => r.id);
const Q: QueryParams = { class: 'Thread', limit: 10, offset: 0, count: false } as QueryParams;

describe('applyLiveOps', () => {
  it('applies steps in order, each index counting the rows after the steps before it', () => {
    const rows = ['a', 'b', 'c', 'd'].map((id) => row(id));
    const out = applyLiveOps(rows, [
      { op: 'remove', id: 'b' },
      { op: 'insert', index: 0, row: row('x') as never },
      { op: 'move', id: 'd', index: 1 },
      { op: 'update', row: row('c', { v: 2 }) as never },
    ]);
    expect(ids(out)).toEqual(['x', 'd', 'a', 'c']);
    expect(out[3]).toEqual({ id: 'c', v: 2 });
    expect(out[2]).toBe(rows[0]);
    expect(ids(rows)).toEqual(['a', 'b', 'c', 'd']);
  });
});

describe('liveQueryParams', () => {
  it('leaves the window keys out', () => {
    expect(liveQueryParams({ ...Q, cursor: '', idsOnly: false })).toEqual({ class: 'Thread' });
  });
});

describe('LiveQuery', () => {
  async function live(window = { offset: 0, limit: 3 }) {
    const source = new FakeSource();
    const queries = new LiveQueries(source);
    const q = queries.query(Q, window);
    expect(q.state).toBe('idle');
    source.connect();
    const sub = source.take('query.subscribe');
    expect(sub.params).toEqual({ query: { class: 'Thread' }, window, key: q.key });
    const changes: boolean[] = [];
    q.subscribe((_q, c) => changes.push(c.reset));
    sub.resolve({ subId: 'q1', rows: [row('a'), row('b'), row('c')], total: 7, version: 0 });
    await tick();
    return { source, queries, q, changes };
  }

  it('subscribes after hello and applies changes in version order', async () => {
    const { source, q, changes } = await live();
    expect(q.state).toBe('live');
    expect(ids(q.rows)).toEqual(['a', 'b', 'c']);
    source.notify('query.changed', {
      subId: 'q1',
      version: 1,
      total: 8,
      ops: [{ op: 'insert', index: 0, row: row('z') }],
    });
    // A repeat and an older one are dropped.
    source.notify('query.changed', { subId: 'q1', version: 1, total: 0, reset: [] });
    expect(ids(q.rows)).toEqual(['z', 'a', 'b', 'c']);
    expect(q.total).toBe(8);
    source.notify('query.changed', { subId: 'q1', version: 2, total: 8, reset: [row('y')] });
    expect(ids(q.rows)).toEqual(['y']);
    expect(changes).toEqual([true, false, true]);
  });

  it('holds changes that arrive before the answer that names their subId', async () => {
    const source = new FakeSource();
    source.connect();
    const q = new LiveQueries(source).query(Q, { limit: 2 });
    const sub = source.take('query.subscribe');
    source.notify('query.changed', {
      subId: 'q9',
      version: 1,
      total: 2,
      ops: [{ op: 'remove', id: 'a' }],
    });
    sub.resolve({ subId: 'q9', rows: [row('a'), row('b')], total: 3, version: 0 });
    await tick();
    expect(ids(q.rows)).toEqual(['b']);
    expect(q.version).toBe(1);
  });

  it('moves the window, applying only what builds on its answer', async () => {
    const { source, q } = await live();
    q.setWindow({ offset: 3, limit: 3 });
    q.setWindow({ offset: 6, limit: 3 });
    const w = source.take('query.setWindow');
    expect(w.params).toEqual({ subId: 'q1', window: { offset: 3, limit: 3 } });
    // One for the old window, then one past the answer's version, before the answer.
    source.notify('query.changed', { subId: 'q1', version: 1, total: 7, reset: [row('old')] });
    source.notify('query.changed', {
      subId: 'q1',
      version: 3,
      total: 7,
      ops: [{ op: 'remove', id: 'e' }],
    });
    w.resolve({ rows: [row('d'), row('e'), row('f')], total: 7, version: 2 });
    await tick();
    expect(ids(q.rows)).toEqual(['d', 'f']);
    expect(q.version).toBe(3);
    // The window moved again while it was out.
    const w2 = source.take('query.setWindow');
    expect(w2.params.window).toEqual({ offset: 6, limit: 3 });
    expect(q.ready).toBe(false);
    w2.resolve({ rows: [row('g')], total: 7, version: 4 });
    await tick();
    expect(q.offset).toBe(6);
    expect(q.ready).toBe(true);
  });

  it('reads the window again after a gap with no answer out', async () => {
    const { source, q } = await live();
    source.notify('query.changed', { subId: 'q1', version: 2, total: 7, reset: [row('x')] });
    expect(ids(q.rows)).toEqual(['a', 'b', 'c']);
    const w = source.take('query.setWindow');
    expect(w.params).toEqual({ subId: 'q1', window: { offset: 0, limit: 3 } });
    w.resolve({ rows: [row('x'), row('y')], total: 7, version: 3 });
    await tick();
    expect(ids(q.rows)).toEqual(['x', 'y']);
  });

  it('subscribes again after a reconnect and drops what the old connection answers', async () => {
    const { source, q } = await live();
    q.setWindow({ offset: 0, limit: 5 });
    source.disconnect();
    expect(q.state).toBe('idle');
    source.connect();
    const sub = source.take('query.subscribe');
    expect(sub.params.window).toEqual({ offset: 0, limit: 5 });
    expect(sub.params.key).toBe(q.key);
    sub.resolve({ subId: 'q2', rows: [row('n')], total: 1, version: 0 });
    await tick();
    source.notify('query.changed', { subId: 'q1', version: 1, total: 0, reset: [] });
    expect(ids(q.rows)).toEqual(['n']);
  });

  it('reports a refused subscribe and unsubscribes on close', async () => {
    const source = new FakeSource();
    source.connect();
    const queries = new LiveQueries(source);
    const q = queries.query(Q);
    const errors: unknown[] = [];
    q.onError((e) => errors.push(e));
    source.take('query.subscribe').reject(new RpcError({ code: -32602, message: 'bad' }));
    await tick();
    expect(q.state).toBe('failed');
    expect(errors).toHaveLength(1);

    const q2 = queries.query(Q);
    source.take('query.subscribe').resolve({ subId: 'q5', rows: [], total: 0, version: 0 });
    await tick();
    q2.close();
    expect(source.take('query.unsubscribe').params).toEqual({ subId: 'q5' });
    expect(queries.size).toBe(1);
  });

  it('unsubscribes one closed while its subscribe was out', async () => {
    const source = new FakeSource();
    source.connect();
    const q = new LiveQueries(source).query(Q);
    const sub = source.take('query.subscribe');
    q.close();
    sub.resolve({ subId: 'q7', rows: [], total: 0, version: 0 });
    await tick();
    expect(source.take('query.unsubscribe').params).toEqual({ subId: 'q7' });
  });

  it('shares one subscription between watchers of the same query and window', async () => {
    const source = new FakeSource();
    source.connect();
    const queries = new LiveQueries(source);
    const seen: string[] = [];
    const off1 = queries.watch(Q, null, (q) => seen.push(`1:${ids(q.rows)}`));
    const off2 = queries.watch({ ...Q, limit: 99 }, null, (q) => seen.push(`2:${ids(q.rows)}`));
    expect(source.calls).toHaveLength(1);
    source.take('query.subscribe').resolve({ subId: 'q1', rows: [row('a')], total: 1, version: 0 });
    await tick();
    const off3 = queries.watch(Q, null, (q) => seen.push(`3:${ids(q.rows)}`));
    expect(seen).toEqual(['1:a', '2:a', '3:a']);
    off1();
    off2();
    expect(source.calls).toHaveLength(0);
    off3();
    expect(source.take('query.unsubscribe').params).toEqual({ subId: 'q1' });
  });
});

describe('FeedCursor', () => {
  it('reads seq from delta params text', () => {
    expect(deltaSeqOf('{"accountId":"a\\"b","seq":42,"modelClass":"Thread"}')).toBe(42);
    expect(deltaSeqOf('{"accountId":"a","modelClass":"Thread","seq":3}')).toBeNull();
  });

  it('resumes from the last delta, or starts over when the engine could not', () => {
    const cursor = new FeedCursor();
    expect(cursor.since()).toBeNull();
    expect(cursor.subscribed({ feed: 'f1', seq: 10 }, null)).toBeNull();
    cursor.delta(11);
    cursor.delta(9);
    expect(cursor.since()).toEqual({ feed: 'f1', seq: 11 });
    expect(cursor.subscribed({ feed: 'f1', seq: 20, resumed: true }, cursor.since())).toBe(true);
    expect(cursor.seq).toBe(11);
    expect(cursor.subscribed({ feed: 'f2', seq: 5, resumed: false }, cursor.since())).toBe(false);
    expect(cursor.since()).toEqual({ feed: 'f2', seq: 5 });
  });
});
