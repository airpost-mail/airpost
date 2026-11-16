// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// JSON-RPC over a WebSocket, one message per text frame: the web through the gateway, later
// mobile.
import type { TransportFactory } from '../rpc';

/**
 * The part of the browser's WebSocket the transport uses. The handlers take `any` so the DOM's
 * WebSocket fits; the transport reads `code` and `reason` of a close and `data` of a message.
 */
export interface WebSocketLike {
  readonly readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  /* eslint-disable @typescript-eslint/no-explicit-any */
  onopen: ((ev: any) => void) | null;
  onclose: ((ev: any) => void) | null;
  onerror: ((ev: any) => void) | null;
  onmessage: ((ev: any) => void) | null;
  /* eslint-enable @typescript-eslint/no-explicit-any */
}

const OPEN = 1;

export interface WebSocketTransportOptions {
  /** Defaults to the global WebSocket. */
  createSocket?: (url: string) => WebSocketLike;
}

function globalSocket(url: string): WebSocketLike {
  const WS = (globalThis as { WebSocket?: new (url: string) => WebSocketLike }).WebSocket;
  if (!WS) throw new Error('No WebSocket here; pass createSocket.');
  return new WS(url);
}

export function webSocketTransport(
  url: string,
  opts: WebSocketTransportOptions = {},
): TransportFactory {
  return (events) => {
    const ws = (opts.createSocket ?? globalSocket)(url);
    ws.onopen = () => events.open();
    // A close event always follows an error; that is where the outcome is decided.
    ws.onerror = () => {};
    ws.onclose = (ev: { code: number; reason?: string }) =>
      events.close({ code: ev.code, reason: ev.reason });
    ws.onmessage = (ev: { data: unknown }) => {
      if (typeof ev.data === 'string') events.messages([ev.data]);
    };
    return {
      send(text) {
        if (ws.readyState === OPEN) ws.send(text);
      },
      close(code, reason) {
        ws.onopen = ws.onclose = ws.onerror = ws.onmessage = null;
        ws.close(code, reason);
      },
    };
  };
}
