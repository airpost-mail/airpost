// JSON-RPC 2.0 over WebSocket to a hosted engine at VITE_ENGINE_URL.
// TODO(engine): the engine has no HTTP/WebSocket server yet (the protocol plan only has a
// Unix socket, S1). This client follows the plan's envelope and methods so it should work
// unchanged once one exists; what it cannot know yet is marked below.
import type { ConnectionState, EngineClient, Unsubscribe } from './client';
import {
  PROTOCOL_VERSION,
  RpcError,
  RpcErrorCode,
  isNotification,
  isResponse,
  type EngineNotification,
  type HelloResult,
  type RpcId,
  type RpcMethodName,
  type RpcParamsArg,
  type RpcRequest,
  type RpcResult,
} from './rpc';
import type { FileJSON } from './types';

interface Pending {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export interface WsEngineClientOptions {
  url: string;
  /** Per-call timeout. */
  timeoutMs?: number;
  getToken?: () => string | null;
}

export class WsEngineClient implements EngineClient {
  readonly kind = 'ws' as const;
  private ws: WebSocket | null = null;
  /** Rejects the `open()` in progress, if any. */
  private abortOpen: ((err: Error) => void) | null = null;
  private _state: ConnectionState = 'idle';
  private nextId = 1;
  private readonly pending = new Map<RpcId, Pending>();
  private readonly notificationListeners = new Set<(n: EngineNotification) => void>();
  private readonly stateListeners = new Set<(s: ConnectionState) => void>();
  private readonly opts: Required<Omit<WsEngineClientOptions, 'getToken'>> &
    Pick<WsEngineClientOptions, 'getToken'>;

  constructor(opts: WsEngineClientOptions) {
    this.opts = { timeoutMs: 30_000, ...opts };
  }

  get state() {
    return this._state;
  }

  async connect(): Promise<HelloResult> {
    this.setState('connecting');
    // TODO(engine): auth. Decide between a bearer token in `hello`, a cookie set by the
    // backend's login, or a WebSocket subprotocol; the Unix socket relied on peer uid (S6).
    await this.open();
    const token = this.opts.getToken?.() ?? undefined;
    const hello = await this.call('hello', {
      protocol: PROTOCOL_VERSION,
      client: 'tern-web/0.1.0',
      token,
    });
    await this.call('subscribe', { accounts: null, classes: null });
    return hello;
  }

  close() {
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      // Its close event arrives after a new socket may have opened; it must not touch that one.
      ws.onopen = ws.onclose = ws.onerror = ws.onmessage = null;
      ws.close(1000, 'client closed');
    }
    this.abortOpen?.(new Error('connection closed'));
    this.abortOpen = null;
    this.failPending(new Error('connection closed'));
    this.setState('closed');
  }

  call<M extends RpcMethodName>(method: M, ...args: RpcParamsArg<M>): Promise<RpcResult<M>> {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return Promise.reject(
        new RpcError({ code: RpcErrorCode.InternalError, message: 'engine is not connected' }),
      );
    }
    const id = this.nextId++;
    const req: RpcRequest = { jsonrpc: '2.0', id, method };
    if (args[0] !== undefined) req.params = args[0];
    return new Promise<RpcResult<M>>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new RpcError({ code: RpcErrorCode.InternalError, message: `${method} timed out` }));
      }, this.opts.timeoutMs);
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, timer });
      ws.send(JSON.stringify(req));
    });
  }

  onNotification(listener: (n: EngineNotification) => void): Unsubscribe {
    this.notificationListeners.add(listener);
    return () => this.notificationListeners.delete(listener);
  }

  onStateChange(listener: (s: ConnectionState) => void): Unsubscribe {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  }

  fileUrl(_file: FileJSON): string | null {
    // TODO(engine): needs an attachment download route on the hosted engine.
    return null;
  }

  private open(): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(this.opts.url);
      this.ws = ws;
      this.abortOpen = reject;
      ws.onopen = () => {
        if (this.ws !== ws) return;
        this.abortOpen = null;
        this.setState('open');
        resolve();
      };
      ws.onerror = () => {
        if (this.ws !== ws) return;
        // TODO(engine): server availability. Surface "engine unreachable" distinctly from
        // auth failures once the server reports them (close codes or an HTTP health route).
        reject(new Error(`cannot reach the engine at ${this.opts.url}`));
      };
      ws.onclose = () => {
        if (this.ws !== ws) return;
        this.ws = null;
        this.failPending(new Error('connection closed'));
        // TODO(engine): reconnect with backoff, re-send hello + subscribe, then invalidate
        // every query (plan 1.4: a dropped subscriber re-reads what it shows).
        this.setState('closed');
      };
      ws.onmessage = (ev) => {
        if (this.ws === ws) this.onMessage(ev.data);
      };
    });
  }

  private onMessage(data: unknown) {
    if (typeof data !== 'string') return;
    let msg: unknown;
    try {
      msg = JSON.parse(data);
    } catch {
      console.warn('engine sent invalid JSON', data);
      return;
    }
    if (isResponse(msg)) {
      const p = msg.id === null ? undefined : this.pending.get(msg.id);
      if (!p) return;
      this.pending.delete(msg.id!);
      clearTimeout(p.timer);
      if ('error' in msg) p.reject(new RpcError(msg.error));
      else p.resolve(msg.result);
    } else if (isNotification(msg)) {
      for (const l of this.notificationListeners) l(msg as EngineNotification);
    }
  }

  private failPending(err: Error) {
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(err);
    }
    this.pending.clear();
  }

  private setState(s: ConnectionState) {
    if (this._state === s) return;
    this._state = s;
    for (const l of this.stateListeners) l(s);
  }
}
