// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// Newline-delimited JSON on the engine's Unix socket (Node `net`): the desktop's windows, later
// engine extensions. Only the `@airpost/client/node` entry exports it.
import net from 'net';
import { MAX_LINE_BYTES } from '../protocol';
import type { TransportFactory } from '../rpc';

export interface UnixTransportOptions {
  /** A line longer than this ends the connection. */
  maxLineBytes?: number;
}

export function unixTransport(
  socketPath: string,
  opts: UnixTransportOptions = {},
): TransportFactory {
  const maxLineBytes = opts.maxLineBytes ?? MAX_LINE_BYTES;
  return (events) => {
    const socket = net.createConnection(socketPath);
    socket.setEncoding('utf8');
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      socket.removeAllListeners();
      socket.on('error', () => {});
      socket.destroy();
      events.close({ code: null });
    };

    let buffer = '';
    socket.on('connect', () => events.open());
    socket.on('data', (chunk: string) => {
      buffer += chunk;
      if (chunk.indexOf('\n') === -1) {
        if (buffer.length > maxLineBytes) {
          console.warn('The engine sent a line over the size limit');
          close();
        }
        return;
      }
      const lines = buffer.split('\n');
      buffer = lines.pop() ?? '';
      events.messages(lines);
    });
    socket.on('error', (err: Error) => {
      console.warn(`The engine socket failed: ${err.message}`);
      close();
    });
    socket.on('close', close);

    return {
      send(text) {
        if (!closed) socket.write(`${text}\n`, 'utf8');
      },
      close() {
        if (closed) return;
        closed = true;
        socket.removeAllListeners();
        socket.on('error', () => {});
        socket.destroy();
      },
    };
  };
}
