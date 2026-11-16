// WsEngineClient's reconnect contract against a fake socket.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EngineUnavailableError } from './client';
import { RpcError } from './rpc';
import {
  backoffDelay,
  BACKOFF_MS,
  CloseCode,
  WsEngineClient,
  type HealthOutcome,
  type WsEngineClientOptions,
} from './ws';

interface Sent {
  id?: number;
  method: string;
  params?: Record<string, unknown>;
}

class FakeSocket {
  readyState = 0;
  readonly sent: Sent[] = [];
  onopen: ((ev: unknown) => void) | null = null;
  onclose: ((ev: { code: number }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: string }) => void) | null = null;
  closedWith: number | null = null;

  constructor(readonly url: string) {}

  send(data: string) {
    this.sent.push(JSON.parse(data) as Sent);
  }

  close(code = 1005) {
    if (this.readyState === 3) return;
    this.closedWith = code;
    this.readyState = 3;
    this.onclose?.({ code });
  }

  // Test side.
  accept() {
    this.readyState = 1;
    this.onopen?.({});
  }
  reply(id: number | undefined, result: unknown) {
    this.onmessage?.({ data: JSON.stringify({ jsonrpc: '2.0', id, result }) });
  }
  drop(code: number) {
    this.readyState = 3;
    this.onclose?.({ code });
  }
  last(method: string) {
    return [...this.sent].reverse().find((m) => m.method === method);
  }
}

const HELLO = { protocol: 1, engine: '0.1.0', pid: 7, accounts: [], features: ['query'] };
const flush = () => vi.advanceTimersByTimeAsync(0);

function setup(opts: Partial<WsEngineClientOptions> = {}) {
  const sockets: FakeSocket[] = [];
  const health = vi.fn(async (): Promise<HealthOutcome> => ({ ok: true, engineUp: true }));
  const onUnpaired = vi.fn();
  const client = new WsEngineClient({
    url: 'wss://airpost.example/gateway/v1/ws',
    health,
    onUnpaired,
    classes: ['Thread'],
    random: () => 0.5, // no jitter
    createSocket: (url) => {
      const s = new FakeSocket(url);
      sockets.push(s);
      return s as unknown as WebSocket;
    },
    ...opts,
  });
  const current = () => sockets[sockets.length - 1];
  /** Accepts the newest socket and answers hello and subscribe. */
  async function handshake() {
    await flush();
    const s = current();
    s.accept();
    await flush();
    s.reply(s.last('hello')!.id, HELLO);
    await flush();
    s.reply(s.last('subscribe')!.id, {});
    await flush();
    return s;
  }
  return { client, sockets, current, health, onUnpaired, handshake };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('backoffDelay', () => {
  it('follows the schedule, capped, with at most 20% jitter', () => {
    expect(BACKOFF_MS).toEqual([500, 1000, 2000, 4000, 8000, 15_000, 30_000]);
    expect(backoffDelay(0, () => 0.5)).toBe(500);
    expect(backoffDelay(6, () => 0.5)).toBe(30_000);
    expect(backoffDelay(20, () => 0.5)).toBe(30_000);
    expect(backoffDelay(0, () => 0)).toBe(400);
    expect(backoffDelay(0, () => 1)).toBe(600);
  });
});

describe('WsEngineClient', () => {
  it('checks health, then says hello without a token and subscribes', async () => {
    const t = setup();
    const connected = t.client.connect();
    expect(t.client.state).toBe('connecting');
    const s = await t.handshake();
    await expect(connected).resolves.toEqual(HELLO);

    expect(t.health).toHaveBeenCalledTimes(1);
    expect(s.url).toBe('wss://airpost.example/gateway/v1/ws');
    expect(s.sent[0]).toMatchObject({
      method: 'hello',
      params: { protocol: 1, client: 'airpost-web/0.1.0' },
    });
    expect(s.sent[0].params).not.toHaveProperty('token');
    expect(s.sent[1]).toMatchObject({
      method: 'subscribe',
      params: { accounts: null, classes: ['Thread'] },
    });
    expect(t.client.state).toBe('open');
    expect(t.client.problem).toBeNull();
    expect([...(t.client.features ?? [])]).toEqual(['query']);
  });

  it('reconnects on the backoff schedule and resets it after 30 s connected', async () => {
    const t = setup();
    void t.client.connect().catch(() => undefined);
    await t.handshake();

    // Drops before the handshake count as failures and back off further each time.
    t.current().drop(CloseCode.Abnormal);
    await flush();
    expect(t.client.state).toBe('reconnecting');
    for (const delay of [500, 1000, 2000, 4000]) {
      const before = t.sockets.length;
      await vi.advanceTimersByTimeAsync(delay - 1);
      expect(t.sockets.length).toBe(before);
      await vi.advanceTimersByTimeAsync(1);
      expect(t.sockets.length).toBe(before + 1);
      t.current().drop(CloseCode.Abnormal);
      await flush();
    }

    // The next attempt (8 s) gets through and stays up for 30 s.
    await vi.advanceTimersByTimeAsync(8000);
    await t.handshake();
    await vi.advanceTimersByTimeAsync(30_000);
    t.current().drop(CloseCode.GoingAway);
    await flush();
    const before = t.sockets.length;
    await vi.advanceTimersByTimeAsync(500);
    expect(t.sockets.length).toBe(before + 1);
  });

  it('re-sends hello and subscribe after a reconnect and reports open again', async () => {
    const t = setup();
    const states: string[] = [];
    t.client.onStateChange((s) => states.push(s));
    void t.client.connect();
    await t.handshake();
    t.current().drop(CloseCode.ServiceRestart);
    await flush();
    await vi.advanceTimersByTimeAsync(500);
    const s = await t.handshake();
    expect(s.sent.map((m) => m.method)).toEqual(['hello', 'subscribe']);
    expect(states.filter((x) => x === 'open')).toHaveLength(2);
    expect(t.health).toHaveBeenCalledTimes(2);
  });

  it('keeps the last features while reconnecting', async () => {
    const t = setup();
    expect(t.client.features).toBeNull();
    void t.client.connect();
    await t.handshake();
    t.current().drop(CloseCode.ServiceRestart);
    await flush();
    expect(t.client.state).not.toBe('open');
    expect(t.client.features?.has('query')).toBe(true);
  });

  it('stops and reports unpaired on close 4401', async () => {
    const t = setup();
    void t.client.connect();
    await t.handshake();
    t.current().drop(CloseCode.Unpaired);
    await flush();
    expect(t.client.state).toBe('closed');
    expect(t.client.problem).toBe('unpaired');
    expect(t.onUnpaired).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.sockets).toHaveLength(1);
  });

  it.each([CloseCode.Protocol, CloseCode.Forbidden])('stops for good on close %i', async (code) => {
    const t = setup();
    vi.spyOn(console, 'error').mockImplementation(() => {});
    void t.client.connect();
    await t.handshake();
    t.current().drop(code);
    await flush();
    expect(t.client.state).toBe('closed');
    expect(t.client.problem).toBe('rejected');
    expect(t.onUnpaired).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.sockets).toHaveLength(1);
  });

  it('waits at least 5 s after close 4429', async () => {
    const t = setup();
    void t.client.connect();
    await t.handshake();
    t.current().drop(CloseCode.RateLimited);
    await flush();
    await vi.advanceTimersByTimeAsync(4999);
    expect(t.sockets).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(t.sockets).toHaveLength(2);
  });

  it('backs off after 4008 instead of retrying at once', async () => {
    const t = setup();
    void t.client.connect();
    await t.handshake();
    t.current().drop(CloseCode.FellBehind);
    await flush();
    expect(t.sockets).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(500);
    expect(t.sockets).toHaveLength(2);
  });

  it('names an unreachable engine on 4502 and keeps trying', async () => {
    const t = setup();
    void t.client.connect();
    await t.handshake();
    t.current().drop(CloseCode.EngineUnreachable);
    await flush();
    expect(t.client.state).toBe('reconnecting');
    expect(t.client.problem).toBe('engine-unreachable');
    await vi.advanceTimersByTimeAsync(500);
    await t.handshake();
    expect(t.client.problem).toBeNull();
  });

  it("doesn't open a socket while the gateway says the engine is down", async () => {
    const t = setup();
    t.health.mockResolvedValueOnce({ ok: true, engineUp: false });
    t.health.mockResolvedValueOnce({ ok: false, reason: 'unreachable' });
    void t.client.connect().catch(() => undefined);
    await flush();
    expect(t.sockets).toHaveLength(0);
    expect(t.client.problem).toBe('engine-down');
    await vi.advanceTimersByTimeAsync(500);
    expect(t.sockets).toHaveLength(0);
    expect(t.client.problem).toBe('server-unreachable');
    await vi.advanceTimersByTimeAsync(1000);
    await t.handshake();
    expect(t.client.state).toBe('open');
  });

  it('stops when the health check says unpaired', async () => {
    const t = setup();
    t.health.mockResolvedValueOnce({ ok: false, reason: 'unpaired' });
    const connected = expect(t.client.connect()).rejects.toBeInstanceOf(EngineUnavailableError);
    await flush();
    await connected;
    expect(t.onUnpaired).toHaveBeenCalled();
    expect(t.sockets).toHaveLength(0);
  });

  it('holds a call made while reconnecting and sends it once connected', async () => {
    const t = setup();
    void t.client.connect();
    await t.handshake();
    t.current().drop(CloseCode.Abnormal);
    await flush();

    const result = t.client.call('engine.status');
    await vi.advanceTimersByTimeAsync(500);
    const s = await t.handshake();
    const req = s.last('engine.status')!;
    s.reply(req.id, { protocol: 1 });
    await expect(result).resolves.toEqual({ protocol: 1 });
  });

  it('rejects a held call after 10 s', async () => {
    const t = setup();
    t.health.mockResolvedValue({ ok: false, reason: 'unreachable' });
    void t.client.connect().catch(() => undefined);
    await flush();
    const result = t.client.call('engine.status');
    const settled = expect(result).rejects.toThrow("Couldn't reach the server.");
    await vi.advanceTimersByTimeAsync(10_000);
    await settled;
    await expect(result).rejects.toBeInstanceOf(EngineUnavailableError);
  });

  it('rejects calls in flight when the socket drops and does not resend them', async () => {
    const t = setup();
    void t.client.connect();
    await t.handshake();
    const task = t.client.call('task.queue', {
      accountId: 'a1',
      task: { __cls: 'ChangeStarredTask' } as never,
    });
    t.current().drop(CloseCode.Abnormal);
    await expect(task).rejects.toBeInstanceOf(EngineUnavailableError);

    await vi.advanceTimersByTimeAsync(500);
    const s = await t.handshake();
    expect(s.sent.map((m) => m.method)).toEqual(['hello', 'subscribe']);
  });

  it('passes engine errors through as RpcError', async () => {
    const t = setup();
    void t.client.connect();
    const s = await t.handshake();
    const result = t.client.call('engine.status');
    await flush();
    s.onmessage?.({
      data: JSON.stringify({
        jsonrpc: '2.0',
        id: s.last('engine.status')!.id,
        error: { code: -32008, message: 'not allowed on a relayed connection' },
      }),
    });
    await expect(result).rejects.toBeInstanceOf(RpcError);
  });

  it('closes cleanly on close() and starts again on connect()', async () => {
    const t = setup();
    void t.client.connect();
    const s = await t.handshake();
    t.client.close();
    expect(s.closedWith).toBe(CloseCode.Normal);
    expect(t.client.state).toBe('closed');
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.sockets).toHaveLength(1);

    const again = t.client.connect();
    await t.handshake();
    await expect(again).resolves.toEqual(HELLO);
  });

  it('retryNow skips the rest of the backoff', async () => {
    const t = setup();
    void t.client.connect();
    await t.handshake();
    t.current().drop(CloseCode.Abnormal);
    await flush();
    // Fail a few times so the delay is long.
    for (const d of [500, 1000, 2000]) {
      await vi.advanceTimersByTimeAsync(d);
      t.current().drop(CloseCode.Abnormal);
      await flush();
    }
    const before = t.sockets.length;
    t.client.retryNow();
    await flush();
    expect(t.sockets.length).toBe(before + 1);
  });
});

describe('WsEngineClient files', () => {
  const file = {
    id: 'f/1',
    aid: 'acc 1',
    v: 1,
    __cls: 'File' as const,
    messageId: 'm1',
    contentId: null,
    contentType: 'application/pdf',
    filename: 'a.pdf',
    size: 3,
  };

  async function connected(features: string[], fetchImpl: typeof fetch = vi.fn()) {
    const onUnpaired = vi.fn();
    const t = setup({ fetch: fetchImpl, onUnpaired });
    void t.client.connect();
    await flush();
    const s = t.current();
    s.accept();
    await flush();
    s.reply(s.last('hello')!.id, { ...HELLO, features });
    await flush();
    s.reply(s.last('subscribe')!.id, {});
    await flush();
    return { client: t.client, onUnpaired };
  }

  it('hands out the file routes once connected', async () => {
    const before = setup();
    expect(before.client.fileUrl(file)).toBeNull();
    const { client } = await connected(['query', 'files']);
    expect(client.fileUrl(file)).toBe('/gateway/v1/files/acc%201/f%2F1');
    expect(client.fileUrl(file, { inline: true })).toBe('/gateway/v1/files/acc%201/f%2F1?inline=1');
  });

  it('uploads by POST and returns the engine File', async () => {
    const stored = { ...file, id: 'engine-id', messageId: '' };
    const fetchImpl = vi.fn(
      async () => new Response(JSON.stringify({ fileId: 'engine-id', file: stored })),
    );
    const { client } = await connected(['files'], fetchImpl as unknown as typeof fetch);
    const blob = new Blob(['abc'], { type: 'text/plain' });
    await expect(client.uploadFile('acc 1', blob, 'a b.txt')).resolves.toEqual(stored);
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('/gateway/v1/files/acc%201?name=a%20b.txt');
    expect(init.method).toBe('POST');
    expect(init.body).toBe(blob);
    expect(init.headers).toEqual({ 'Content-Type': 'text/plain' });
  });

  it("reports the gateway's message for a refused upload, and unpaired on 401", async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ error: 'too-large', message: 'At most 25 MB.' }), {
          status: 413,
        }),
      )
      .mockResolvedValueOnce(new Response('', { status: 401 }));
    const { client, onUnpaired } = await connected(['files'], fetchImpl);
    await expect(client.uploadFile('a', new Blob(['x']), 'x')).rejects.toThrow('At most 25 MB.');
    await expect(client.uploadFile('a', new Blob(['x']), 'x')).rejects.toThrow();
    expect(onUnpaired).toHaveBeenCalledTimes(1);
  });

  it('prepares a file with HEAD on its download route', async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(new Response(null, { status: 200 }))
      .mockResolvedValueOnce(new Response(null, { status: 503 }));
    const { client } = await connected(['files'], fetchImpl);
    await client.prepareFile(file);
    expect(fetchImpl).toHaveBeenCalledWith('/gateway/v1/files/acc%201/f%2F1', { method: 'HEAD' });
    await expect(client.prepareFile(file)).rejects.toThrow("a.pdf isn't available.");
    const old = await connected(['query'], fetchImpl);
    await expect(old.client.prepareFile(file)).rejects.toThrow();
  });
});
