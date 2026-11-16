// The web side of A7 over the gateway's WebSocket: a streamed call outlives the default
// 30 s call timeout, and only its own `ai.delta` reaches it.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { RpcError, WsEngineClient } from '@/engine';
import { startAiCall } from '../api';

interface Sent {
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

class FakeSocket {
  readyState = 0;
  readonly sent: Sent[] = [];
  onopen: (() => void) | null = null;
  onclose: ((ev: { code: number }) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((ev: { data: string }) => void) | null = null;
  send(data: string) {
    this.sent.push(JSON.parse(data) as Sent);
  }
  close() {
    this.readyState = 3;
  }
  last(method: string) {
    return [...this.sent].reverse().find((m) => m.method === method)!;
  }
  push(msg: object) {
    this.onmessage?.({ data: JSON.stringify({ jsonrpc: '2.0', ...msg }) });
  }
}

const flush = () => vi.advanceTimersByTimeAsync(0);

async function connected() {
  const s = new FakeSocket();
  const client = new WsEngineClient({
    url: 'wss://airpost.example/gateway/v1/ws',
    createSocket: () => s as unknown as WebSocket,
  });
  const ready = client.connect();
  await flush();
  s.readyState = 1;
  s.onopen?.();
  await flush();
  s.push({
    id: s.last('hello').id,
    result: { protocol: 1, engine: 'x', pid: 1, accounts: [], features: ['ai'] },
  });
  await flush();
  s.push({ id: s.last('subscribe').id, result: {} });
  await ready;
  return { client, s };
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('ai calls over the gateway', () => {
  it('wait up to 150 s for a model; other calls keep 30 s', async () => {
    const { client, s } = await connected();
    const call = startAiCall(client, 'ai.summarize', { accountId: 'a', threadId: 't' });
    const status = client.call('engine.status');
    const statusFailed = expect(status).rejects.toBeInstanceOf(RpcError);
    let settled = false;
    call.promise.then(
      () => (settled = true),
      () => (settled = true),
    );
    await vi.advanceTimersByTimeAsync(31_000);
    await statusFailed;
    expect(settled).toBe(false);
    s.push({ id: s.last('ai.summarize').id, result: { text: 'done', model: 'm' } });
    await expect(call.promise).resolves.toMatchObject({ text: 'done' });

    const late = startAiCall(client, 'ai.draftReply', { accountId: 'a', threadId: 't' });
    const lateFailed = expect(late.promise).rejects.toMatchObject({ key: 'unknown' });
    await vi.advanceTimersByTimeAsync(150_001);
    await lateFailed;
  });

  it('collects only its own stream, then cancels by streamId', async () => {
    const { client, s } = await connected();
    const texts: string[] = [];
    const call = startAiCall(client, 'ai.summarize', { accountId: 'a', threadId: 't' }, (t) =>
      texts.push(t),
    );
    await flush();
    const req = s.last('ai.summarize');
    const streamId = req.params!.streamId as string;
    expect(req.params).toEqual({ accountId: 'a', threadId: 't', streamId });
    s.push({ method: 'ai.delta', params: { streamId: 'someone-else', text: 'no' } });
    s.push({ method: 'ai.delta', params: { streamId, text: 'Hel' } });
    s.push({ method: 'ai.delta', params: { streamId, text: 'lo' } });
    expect(texts).toEqual(['Hel', 'lo']);

    call.cancel();
    await flush();
    expect(s.last('ai.cancel').params).toEqual({ streamId });
    s.push({
      id: req.id,
      error: { code: -32011, message: 'AI request failed', data: { key: 'cancelled' } },
    });
    await expect(call.promise).rejects.toMatchObject({ key: 'cancelled' });
    s.push({ method: 'ai.delta', params: { streamId, text: 'late' } });
    expect(texts).toEqual(['Hel', 'lo']);
  });
});
