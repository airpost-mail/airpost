// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  EngineUnavailableError,
  ErrorCode,
  RpcError,
  Session,
  type SessionOptions,
  type TransportEvents,
} from '../src';

interface Sent {
  id?: number;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: { code: number; message: string };
}

/** One fake connection: what the client wrote, and the engine's side. */
class FakeConn {
  readonly sent: Sent[] = [];
  closed: { code?: number; reason?: string } | null = null;
  constructor(readonly events: TransportEvents) {}
  open() {
    this.events.open();
  }
  push(...msgs: unknown[]) {
    this.events.messages(msgs.map((m) => JSON.stringify(m)));
  }
  answer(method: string, result: unknown) {
    const req = [...this.sent].reverse().find((m) => m.method === method);
    this.push({ jsonrpc: '2.0', id: req!.id, result });
  }
  refuse(method: string, code = -32000) {
    const req = [...this.sent].reverse().find((m) => m.method === method);
    this.push({ jsonrpc: '2.0', id: req!.id, error: { code, message: 'no' } });
  }
  drop() {
    this.events.close({ code: null });
  }
  methods() {
    return this.sent.map((m) => m.method);
  }
}

const HELLO = { protocol: 1, engine: 't', pid: 1, accounts: [], features: ['query'] };
const flush = () => vi.advanceTimersByTimeAsync(0);

function setup(opts: Partial<SessionOptions<string>> = {}, key: unknown = 1) {
  const conns: FakeConn[] = [];
  const endpoint = (k: unknown) => ({
    key: k,
    transport: (events: TransportEvents) => {
      const c = new FakeConn(events);
      conns.push(c);
      return {
        send: (text: string) => c.sent.push(JSON.parse(text) as Sent),
        close: (code?: number, reason?: string) => (c.closed = { code, reason }),
      };
    },
  });
  const session = new Session({
    client: 'spec',
    endpoint: endpoint(key),
    subscribe: () => ({ accounts: null, classes: null }),
    backoff: (n) => 100 * 2 ** n,
    ...opts,
  });
  const current = () => conns[conns.length - 1];
  async function handshake() {
    await flush();
    const c = current();
    c.open();
    c.answer('hello', HELLO);
    c.answer('subscribe', {});
    await flush();
    return c;
  }
  return { session, conns, current, handshake, endpoint };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('Session', () => {
  it('writes hello and subscribe once the transport opens, then opens', async () => {
    const t = setup();
    const ready = t.session.connect();
    await flush();
    expect(t.current().sent).toEqual([]);
    t.current().open();
    expect(t.current().sent.map((m) => [m.method, m.params])).toEqual([
      ['hello', { protocol: 1, client: 'spec' }],
      ['subscribe', { accounts: null, classes: null }],
    ]);
    t.current().answer('hello', HELLO);
    expect(t.session.state).toBe('connecting');
    t.current().answer('subscribe', {});
    expect(t.session.state).toBe('open');
    await expect(ready).resolves.toEqual(HELLO);
    expect([...t.session.features!]).toEqual(['query']);
  });

  it('tells hello listeners before the rest of the batch is handled', async () => {
    const t = setup();
    const seen: string[] = [];
    t.session.onHello(() => seen.push('hello'));
    t.session.onNotification((n) => seen.push(n.method));
    t.session.onBatch(() => seen.push('batch'));
    void t.session.connect();
    await flush();
    const c = t.current();
    c.open();
    const [hello, subscribe] = c.sent;
    c.push(
      { jsonrpc: '2.0', id: hello.id, result: HELLO },
      { jsonrpc: '2.0', id: subscribe.id, result: {} },
      { jsonrpc: '2.0', method: 'delta', params: {} },
    );
    expect(seen).toEqual(['hello', 'delta', 'batch']);
  });

  it('matches answers that come out of order', async () => {
    const t = setup();
    void t.session.connect();
    const c = await t.handshake();
    const a = t.session.request('a');
    const b = t.session.request('b');
    c.answer('b', 'B');
    c.answer('a', 'A');
    await expect(Promise.all([a, b])).resolves.toEqual(['A', 'B']);
  });

  it('rejects calls out when the connection drops, then reconnects on the backoff', async () => {
    const t = setup({ droppedMessage: 'gone' });
    void t.session.connect();
    const c = await t.handshake();
    const out = t.session.request('a');
    c.drop();
    await expect(out).rejects.toThrow('gone');
    await expect(out).rejects.toBeInstanceOf(EngineUnavailableError);
    expect(t.session.state).toBe('reconnecting');
    await vi.advanceTimersByTimeAsync(99);
    expect(t.conns).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(t.conns).toHaveLength(2);
    await t.handshake();
    expect(t.current().methods()).toEqual(['hello', 'subscribe']);
  });

  it('passes engine errors through as RpcError with their code', async () => {
    const t = setup();
    void t.session.connect();
    const c = await t.handshake();
    const out = t.session.call('engine.status');
    await flush();
    c.refuse('engine.status', ErrorCode.NotFound);
    await expect(out).rejects.toMatchObject({ code: ErrorCode.NotFound });
    await expect(out).rejects.toBeInstanceOf(RpcError);
  });

  it('times calls out by method', async () => {
    const t = setup({ timeoutMs: 1000, timeoutFor: (m) => (m === 'slow' ? 5000 : undefined) });
    void t.session.connect();
    await t.handshake();
    const fast = t.session.request('fast');
    const slow = t.session.request('slow');
    const fastFailed = expect(fast).rejects.toThrow('fast timed out');
    await vi.advanceTimersByTimeAsync(1000);
    await fastFailed;
    const slowFailed = expect(slow).rejects.toThrow('slow timed out');
    await vi.advanceTimersByTimeAsync(4000);
    await slowFailed;
  });

  it('drops the connection after the batch that says disconnect', async () => {
    const t = setup();
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const got: string[] = [];
    t.session.onNotification((n) => got.push(n.method));
    void t.session.connect();
    const c = await t.handshake();
    c.push(
      { jsonrpc: '2.0', method: 'disconnect', params: { reason: 'slow consumer' } },
      { jsonrpc: '2.0', method: 'delta', params: {} },
    );
    expect(got).toEqual(['delta']);
    expect(c.closed).not.toBeNull();
    await flush();
    expect(t.session.state).toBe('reconnecting');
  });

  it('answers the engine requests it has a handler for, and MethodNotFound otherwise', async () => {
    const t = setup();
    t.session.handle('hook.call', async (p: { n: number }) => ({ n: p.n + 1 }));
    t.session.handle('hook.fail', () => {
      throw new RpcError({ code: -32050, message: 'nope', data: { key: 'x' } });
    });
    void t.session.connect();
    const c = await t.handshake();
    c.push(
      { jsonrpc: '2.0', id: 's1', method: 'hook.call', params: { n: 1 } },
      { jsonrpc: '2.0', id: 's2', method: 'hook.fail', params: {} },
      { jsonrpc: '2.0', id: 's3', method: 'hook.other', params: {} },
    );
    await flush();
    const replies = c.sent
      .filter((m) => typeof m.id === 'string')
      .sort((a, b) => String(a.id).localeCompare(String(b.id)));
    expect(replies).toEqual([
      { jsonrpc: '2.0', id: 's1', result: { n: 2 } },
      { jsonrpc: '2.0', id: 's2', error: { code: -32050, message: 'nope', data: { key: 'x' } } },
      {
        jsonrpc: '2.0',
        id: 's3',
        error: { code: ErrorCode.MethodNotFound, message: 'no handler for hook.other' },
      },
    ]);
  });

  it('keeps the capabilities current from capabilities.changed', async () => {
    const t = setup();
    void t.session.connect();
    const c = await t.handshake();
    expect(t.session.capabilities).toBeNull();
    c.push({ jsonrpc: '2.0', method: 'capabilities.changed', params: { ai: {} } });
    expect(t.session.capabilities).toEqual({ ai: {} });
  });

  it('opens when a subscribe is refused unless it is required', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const t = setup();
    void t.session.connect();
    await flush();
    t.current().open();
    t.current().answer('hello', HELLO);
    t.current().refuse('subscribe');
    expect(t.session.state).toBe('open');

    const ends: boolean[] = [];
    const r = setup({
      subscribeRequired: true,
      onEnd: (end) => {
        ends.push(end.handshakeFailed);
        return { next: 'retry' };
      },
    });
    void r.session.connect().catch(() => undefined);
    await flush();
    r.current().open();
    r.current().answer('hello', HELLO);
    r.current().refuse('subscribe');
    await flush();
    expect(r.session.state).toBe('reconnecting');
    expect(ends).toEqual([true]);
  });

  it('sends subscribe again only when its params changed', async () => {
    let accounts = ['a1'];
    const t = setup({ subscribe: () => ({ accounts, classes: null }) });
    void t.session.connect();
    const c = await t.handshake();
    t.session.resubscribe();
    expect(c.methods()).toEqual(['hello', 'subscribe']);
    accounts = ['a1', 'a2'];
    t.session.resubscribe();
    expect(c.sent[2]).toMatchObject({
      method: 'subscribe',
      params: { accounts: ['a1', 'a2'], classes: null },
    });
  });

  it('adds since to the handshake subscribe only, and reports each answer', async () => {
    let since: { feed: string; seq: number } | null = null;
    let accounts = ['a1'];
    const t = setup({ subscribe: () => ({ accounts, classes: null }), since: () => since });
    const answers: unknown[] = [];
    t.session.onSubscribed((a) => answers.push(a));
    void t.session.connect();
    let c = await t.handshake();
    expect(c.sent[1].params).toEqual({ accounts: ['a1'], classes: null });
    since = { feed: 'f', seq: 4 };
    accounts = ['a2'];
    t.session.resubscribe();
    expect(c.sent[2].params).toEqual({ accounts: ['a2'], classes: null });
    c.answer('subscribe', { feed: 'f', seq: 9 });
    c.drop();
    await vi.advanceTimersByTimeAsync(100);
    c = t.current();
    c.open();
    expect(c.sent[1].params).toEqual({ accounts: ['a2'], classes: null, since });
    c.answer('hello', HELLO);
    c.answer('subscribe', { feed: 'f', seq: 12, resumed: true });
    expect(answers).toEqual([
      { result: {}, since: null, handshake: true },
      { result: { feed: 'f', seq: 9 }, since: null, handshake: false },
      { result: { feed: 'f', seq: 12, resumed: true }, since, handshake: true },
    ]);
  });

  it('connects to a new endpoint key at once, keeps the same key, and waits on null', async () => {
    const t = setup({ backoff: () => 10_000 });
    void t.session.connect();
    await t.handshake();
    t.session.setEndpoint(t.endpoint(1));
    await flush();
    expect(t.conns).toHaveLength(1);

    t.session.setEndpoint(t.endpoint(2));
    await flush();
    expect(t.conns).toHaveLength(2);
    expect(t.conns[0].closed).not.toBeNull();
    await t.handshake();
    expect(t.session.state).toBe('open');

    t.session.setEndpoint(null);
    expect(t.session.state).toBe('idle');
    await expect(t.session.request('a')).rejects.toBeInstanceOf(EngineUnavailableError);
  });

  it('waits for a new endpoint when onEnd says so', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const t = setup({
      onEnd: (end) => (end.handshakeFailed ? { next: 'wait' } : { next: 'retry' }),
    });
    void t.session.connect().catch(() => undefined);
    await flush();
    t.current().open();
    t.current().refuse('hello');
    await flush();
    expect(t.session.state).toBe('idle');
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.conns).toHaveLength(1);
    t.session.setEndpoint(t.endpoint(2));
    await flush();
    expect(t.conns).toHaveLength(2);
  });

  it('stops for good on a stop step and tells onStop', async () => {
    const onStop = vi.fn();
    const t = setup({
      onEnd: () => ({ next: 'stop', problem: 'unpaired' }),
      onStop,
      unavailableMessage: (p) => `problem: ${p}`,
    });
    void t.session.connect();
    const c = await t.handshake();
    c.drop();
    await flush();
    expect(t.session.state).toBe('closed');
    expect(t.session.problem).toBe('unpaired');
    expect(onStop).toHaveBeenCalledWith('unpaired');
    await expect(t.session.call('engine.status')).rejects.toThrow('problem: unpaired');
  });

  it('runs the preflight before each attempt and holds a call until open', async () => {
    const preflight = vi
      .fn()
      .mockResolvedValueOnce({ ok: false, problem: 'down' })
      .mockResolvedValue({ ok: true });
    const t = setup({ preflight });
    void t.session.connect();
    const call = t.session.call('engine.status');
    await flush();
    expect(t.conns).toHaveLength(0);
    expect(t.session.problem).toBe('down');
    await vi.advanceTimersByTimeAsync(100);
    const c = await t.handshake();
    expect(t.session.problem).toBeNull();
    c.answer('engine.status', { ok: 1 });
    await expect(call).resolves.toEqual({ ok: 1 });
  });

  it('closes, failing what waits, and connects again on connect()', async () => {
    const t = setup();
    void t.session.connect();
    const c = await t.handshake();
    const out = t.session.request('a');
    t.session.close();
    await expect(out).rejects.toThrow('The connection was closed.');
    expect(c.closed).toEqual({ code: 1000, reason: 'client closed' });
    expect(t.session.state).toBe('closed');
    const again = t.session.connect();
    await t.handshake();
    await expect(again).resolves.toEqual(HELLO);
  });

  it('sends a request made before the handshake after it', async () => {
    const t = setup();
    void t.session.connect();
    await flush();
    expect(t.session.hasConnection).toBe(true);
    const early = t.session.request('task.queue', { n: 1 });
    t.current().open();
    expect(t.current().methods()).toEqual(['hello', 'subscribe', 'task.queue']);
    t.current().answer('task.queue', 'queued');
    await expect(early).resolves.toBe('queued');

    // One whose connection ends first fails with it.
    const u = setup({ droppedMessage: 'gone' });
    void u.session.connect().catch(() => undefined);
    await flush();
    const lost = u.session.request('task.queue', {});
    u.current().drop();
    await expect(lost).rejects.toThrow('gone');
  });

  it('sends what afterHandshake writes after hello and subscribe', async () => {
    let session: Session | null = null;
    const t = setup({
      afterHandshake: () => void session!.request('engine.status').catch(() => {}),
    });
    session = t.session;
    void t.session.connect();
    await flush();
    t.current().open();
    expect(t.current().methods()).toEqual(['hello', 'subscribe', 'engine.status']);
  });
});
