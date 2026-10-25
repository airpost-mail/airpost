// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { describe, expect, it, vi } from 'vitest';
import { webSocketTransport, type WebSocketLike } from '../src';

class FakeSocket implements WebSocketLike {
  readyState = 0;
  sent: string[] = [];
  closed: [number?, string?] | null = null;
  onopen: WebSocketLike['onopen'] = null;
  onclose: WebSocketLike['onclose'] = null;
  onerror: WebSocketLike['onerror'] = null;
  onmessage: WebSocketLike['onmessage'] = null;
  send(data: string) {
    this.sent.push(data);
  }
  close(code?: number, reason?: string) {
    this.closed = [code, reason];
    this.onclose?.({ code: code ?? 1005 });
  }
}

describe('webSocketTransport', () => {
  it('carries text frames both ways and reports the close code', () => {
    const ws = new FakeSocket();
    const events = { open: vi.fn(), messages: vi.fn(), close: vi.fn() };
    const t = webSocketTransport('wss://x', { createSocket: () => ws })(events);
    ws.readyState = 1;
    ws.onopen?.({});
    expect(events.open).toHaveBeenCalled();
    t.send('{"a":1}');
    expect(ws.sent).toEqual(['{"a":1}']);
    ws.onmessage?.({ data: '{"b":2}' });
    ws.onmessage?.({ data: new ArrayBuffer(1) });
    expect(events.messages).toHaveBeenCalledTimes(1);
    expect(events.messages).toHaveBeenCalledWith(['{"b":2}']);
    ws.onclose?.({ code: 4401, reason: 'unpaired' });
    expect(events.close).toHaveBeenCalledWith({ code: 4401, reason: 'unpaired' });
  });

  it('stops reporting once the client closes it', () => {
    const ws = new FakeSocket();
    const events = { open: vi.fn(), messages: vi.fn(), close: vi.fn() };
    const t = webSocketTransport('wss://x', { createSocket: () => ws })(events);
    t.close(1000, 'bye');
    expect(ws.closed).toEqual([1000, 'bye']);
    expect(events.close).not.toHaveBeenCalled();
  });
});
