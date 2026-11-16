// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
JSON-RPC 2.0 over a Transport: the envelope, the two errors a call can end with, and RpcPeer,
one connection's request/answer bookkeeping (ids, pending calls, timeouts, answers in any
order, notifications, requests from the engine). Session (session.ts) runs one RpcPeer per
connection; the transports (transports/) frame the messages.
*/
import { ErrorCode } from './protocol';

export type RpcId = number | string;

export interface RpcRequest<P = unknown> {
  jsonrpc: '2.0';
  id: RpcId;
  method: string;
  params?: P;
}

export interface RpcErrorObject {
  code: number;
  message: string;
  data?: unknown;
}

export type RpcResponse<R = unknown> =
  | { jsonrpc: '2.0'; id: RpcId; result: R }
  | { jsonrpc: '2.0'; id: RpcId | null; error: RpcErrorObject };

export interface RpcNotification<P = unknown> {
  jsonrpc: '2.0';
  method: string;
  params?: P;
}

/** A notification as listeners get it. */
export interface IncomingNotification {
  method: string;
  params: any;
}

/** The engine answered with an error. */
export class RpcError extends Error {
  readonly code: number;
  readonly data?: unknown;

  constructor({ code, message, data }: RpcErrorObject) {
    super(message);
    this.name = 'RpcError';
    this.code = code;
    this.data = data;
  }
}

/**
 * The call got no answer: there was no connection, or it closed first. `code` is null so code
 * that only sees `{code, message, data}` can tell it from an RpcError.
 */
export class EngineUnavailableError extends Error {
  readonly code = null;
  readonly data = undefined;

  constructor(message = 'The engine is not connected.') {
    super(message);
    this.name = 'EngineUnavailable';
  }
}

export function isResponse(msg: unknown): msg is RpcResponse {
  return typeof msg === 'object' && msg !== null && 'id' in msg && !('method' in msg);
}

export function isNotification(msg: unknown): msg is RpcNotification {
  return typeof msg === 'object' && msg !== null && 'method' in msg && !('id' in msg);
}

export function isRequest(msg: unknown): msg is RpcRequest {
  return (
    typeof msg === 'object' &&
    msg !== null &&
    'method' in msg &&
    'id' in msg &&
    (msg as { id: unknown }).id !== null
  );
}

/** How a transport ended. `code` is the WebSocket close code; null on other transports. */
export interface TransportClose {
  code: number | null;
  reason?: string;
}

export interface TransportEvents {
  /** It can carry messages now. */
  open(): void;
  /** Messages in arrival order, as JSON text; those read together come as one batch. */
  messages(texts: string[]): void;
  /** Once, whoever closed it, opened or not. */
  close(end: TransportClose): void;
}

/** One connection's framing: NDJSON on a socket, frames on a WebSocket. */
export interface Transport {
  /** One message as JSON text, without the framing. Only called after `open`. */
  send(text: string): void;
  /** `close` follows on the events, unless it already came. */
  close(code?: number, reason?: string): void;
}

/** Opens a transport that reports to `events`. It may throw instead of opening. */
export type TransportFactory = (events: TransportEvents) => Transport;

/** Answers a request the engine sends the client. Throw an RpcError to answer with an error. */
export type RequestHandler = (params: any) => unknown | Promise<unknown>;

export interface RequestOptions {
  /** Overrides the peer's timeout; null for none. */
  timeoutMs?: number | null;
  /**
   * Runs as the answer is read, before the rest of its batch is handled, with the result or
   * the error. The promise settles after it.
   */
  onAnswer?: (result: unknown, error: RpcError | null) => void;
}

export interface RpcPeerOptions {
  /** Per-call timeout; null or absent for none. */
  timeoutMs?: number | null;
  /** A method's own timeout (null for none); undefined for `timeoutMs`. */
  timeoutFor?: (method: string) => number | null | undefined;
  /** Takes a message before it is parsed, returning true when it did (the desktop's deltas). */
  takeRaw?: (text: string) => boolean;
  onNotification?: (n: IncomingNotification) => void;
  /** Requests from the engine by method. Others are answered with MethodNotFound. */
  handlers?: ReadonlyMap<string, RequestHandler>;
  /** After each batch of messages. */
  onBatchEnd?: () => void;
}

interface Pending {
  method: string;
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  onAnswer?: RequestOptions['onAnswer'];
  timer: ReturnType<typeof setTimeout> | undefined;
}

/**
 * The calls on one connection. Writes before the transport opens are kept and sent in order
 * when it does; `fail` ends every call still out.
 */
export class RpcPeer {
  private nextId = 1;
  private readonly pending = new Map<RpcId, Pending>();
  private outbox: string[] | null = [];
  private failed: Error | null = null;

  constructor(
    private readonly transport: () => Transport | null,
    private readonly opts: RpcPeerOptions = {},
  ) {}

  /** The calls waiting for an answer. */
  get inFlight(): number {
    return this.pending.size;
  }

  request(method: string, params?: unknown, opts: RequestOptions = {}): Promise<unknown> {
    if (this.failed) return Promise.reject(this.failed);
    const id = this.nextId++;
    const req: RpcRequest = { jsonrpc: '2.0', id, method };
    if (params !== undefined) req.params = params;
    const own = this.opts.timeoutFor?.(method);
    const timeoutMs =
      opts.timeoutMs !== undefined
        ? opts.timeoutMs
        : own !== undefined
          ? own
          : (this.opts.timeoutMs ?? null);
    return new Promise((resolve, reject) => {
      const timer =
        timeoutMs === null
          ? undefined
          : setTimeout(() => {
              this.pending.delete(id);
              reject(
                new RpcError({ code: ErrorCode.InternalError, message: `${method} timed out` }),
              );
            }, timeoutMs);
      this.pending.set(id, { method, resolve, reject, onAnswer: opts.onAnswer, timer });
      this.write(JSON.stringify(req));
    });
  }

  notify(method: string, params?: unknown) {
    if (this.failed) return;
    const msg: RpcNotification = { jsonrpc: '2.0', method };
    if (params !== undefined) msg.params = params;
    this.write(JSON.stringify(msg));
  }

  /** The transport opened: what was written before goes out. */
  opened() {
    const outbox = this.outbox;
    this.outbox = null;
    for (const text of outbox ?? []) this.transport()?.send(text);
  }

  receive(texts: string[]) {
    for (const text of texts) this.receiveOne(text);
    this.opts.onBatchEnd?.();
  }

  /** The connection ended: every call still out rejects with `err`, and later ones too. */
  fail(err: Error) {
    if (this.failed) return;
    this.failed = err;
    this.outbox = null;
    const pending = [...this.pending.values()];
    this.pending.clear();
    for (const p of pending) {
      clearTimeout(p.timer);
      p.reject(err);
    }
  }

  private write(text: string) {
    if (this.outbox) this.outbox.push(text);
    else this.transport()?.send(text);
  }

  private receiveOne(text: string) {
    if (!text) return;
    if (this.opts.takeRaw?.(text)) return;
    let msg: unknown;
    try {
      msg = JSON.parse(text);
    } catch {
      console.warn('The engine sent a message that is not JSON');
      return;
    }
    if (isResponse(msg)) {
      this.answer(msg);
    } else if (isRequest(msg)) {
      void this.serve(msg);
    } else if (isNotification(msg)) {
      this.opts.onNotification?.({ method: msg.method, params: msg.params });
    }
  }

  private answer(msg: RpcResponse) {
    const p = msg.id === null ? undefined : this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id!);
    clearTimeout(p.timer);
    const error = 'error' in msg ? new RpcError(msg.error) : null;
    const result = 'result' in msg ? msg.result : undefined;
    if (p.onAnswer) {
      try {
        p.onAnswer(result, error);
      } catch (err) {
        console.error(`An answer handler for ${p.method} failed`, err);
      }
    }
    if (error) p.reject(error);
    else p.resolve(result);
  }

  private async serve(req: RpcRequest) {
    const handler = this.opts.handlers?.get(req.method);
    let reply: RpcResponse;
    if (!handler) {
      reply = {
        jsonrpc: '2.0',
        id: req.id,
        error: { code: ErrorCode.MethodNotFound, message: `no handler for ${req.method}` },
      };
    } else {
      try {
        reply = { jsonrpc: '2.0', id: req.id, result: (await handler(req.params)) ?? null };
      } catch (err) {
        const e = err instanceof RpcError ? err : null;
        reply = {
          jsonrpc: '2.0',
          id: req.id,
          error: {
            code: e ? e.code : ErrorCode.InternalError,
            message: err instanceof Error ? err.message : String(err),
            ...(e && e.data !== undefined ? { data: e.data } : {}),
          },
        };
      }
    }
    if (!this.failed) this.write(JSON.stringify(reply));
  }
}
