// JSON-RPC 2.0 over WebSocket to the engine, through apps/gateway on this page's origin
// (docs/web-gateway-plan.md 2.4). The gateway authorises the upgrade with the pairing cookie,
// so nothing here carries a token. Reconnects follow the plan's contract (2.6).
import {
  EngineUnavailableError,
  type ConnectionProblem,
  type ConnectionState,
  type EngineClient,
  type Unsubscribe,
} from './client';
import {
  EngineFeature,
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
import { GATEWAY_BASE } from './mode';
import type { FileJSON } from './types';

/** What `GET /gateway/v1/session` said before a connect. */
export type HealthOutcome =
  { ok: true; engineUp: boolean } | { ok: false; reason: 'unpaired' | 'unreachable' };

/** Close codes the gateway sends (plan 2.6), plus the standard ones the client acts on. */
export const CloseCode = {
  Normal: 1000,
  GoingAway: 1001,
  Abnormal: 1006,
  InternalError: 1011,
  ServiceRestart: 1012,
  FellBehind: 4008,
  Protocol: 4400,
  Unpaired: 4401,
  Forbidden: 4403,
  RateLimited: 4429,
  EngineUnreachable: 4502,
} as const;

/** Delay before each reconnect attempt; the last one repeats. */
export const BACKOFF_MS = [500, 1000, 2000, 4000, 8000, 15_000, 30_000] as const;
/** A connection that lasted this long resets the backoff. */
export const STABLE_MS = 30_000;
/** First delay after the gateway says too many connections or frames. */
export const RATE_LIMITED_MS = 5000;
const JITTER = 0.2;

export function backoffDelay(attempt: number, random: () => number = Math.random): number {
  const base = BACKOFF_MS[Math.min(attempt, BACKOFF_MS.length - 1)];
  return Math.round(base * (1 + (random() * 2 - 1) * JITTER));
}

interface Pending {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

interface Waiter {
  resolve: (hello: HelloResult) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

interface SessionEnd {
  code: number;
  /** The handshake finished, so the connection counted as open. */
  opened: boolean;
}

export interface WsEngineClientOptions {
  url: string;
  /** Per-call timeout. */
  timeoutMs?: number;
  /** How long a call or `connect()` waits for a connection that isn't up yet. */
  waitMs?: number;
  /** Runs before every connect attempt. Without it the client connects straight away. */
  health?: () => Promise<HealthOutcome>;
  /** The gateway no longer knows this browser (401 or close 4401). */
  onUnpaired?: () => void;
  /** Model classes to subscribe to; null for all. */
  classes?: string[] | null;
  createSocket?: (url: string) => WebSocket;
  random?: () => number;
  /** The gateway's file routes (plan 2.5). */
  filesBase?: string;
  fetch?: typeof fetch;
}

/** A file route answered with an error; `status` is the HTTP status. */
export class FileRequestError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = 'FileRequestError';
  }
}

async function fileError(res: Response, fallback: string): Promise<FileRequestError> {
  let message = fallback;
  try {
    const body = (await res.json()) as { message?: unknown };
    if (typeof body.message === 'string') message = body.message;
  } catch {
    // Not the gateway's JSON error.
  }
  return new FileRequestError(res.status, message);
}

const CLIENT_NAME = 'tern-web/0.1.0';

/**
 * Calls that wait on a model: the engine gives a provider up to 120 s (docs/ai-plan.md 2),
 * after a wait for a free AI worker.
 */
const LONG_CALL_MS = 150_000;
const LONG_CALLS: ReadonlySet<string> = new Set([
  'ai.models',
  'ai.test',
  'ai.summarize',
  'ai.suggestReplies',
  'ai.draftReply',
]);

export class WsEngineClient implements EngineClient {
  readonly kind = 'ws' as const;
  private ws: WebSocket | null = null;
  private _state: ConnectionState = 'idle';
  private _problem: ConnectionProblem | null = null;
  private nextId = 1;
  private readonly pending = new Map<RpcId, Pending>();
  private readonly waiters = new Set<Waiter>();
  private readonly notificationListeners = new Set<(n: EngineNotification) => void>();
  private readonly stateListeners = new Set<(s: ConnectionState) => void>();
  private hello: HelloResult | null = null;
  private _features: ReadonlySet<string> | null = null;
  /** Bumped by close(); a loop from an older generation stops at its next step. */
  private generation = 0;
  private looping = false;
  private attempt = 0;
  private minDelay = 0;
  private openedAt = 0;
  private wake: (() => void) | null = null;
  private endSession: ((end: SessionEnd) => void) | null = null;
  private readonly opts: Required<
    Pick<
      WsEngineClientOptions,
      'url' | 'timeoutMs' | 'waitMs' | 'createSocket' | 'random' | 'filesBase' | 'fetch'
    >
  > &
    WsEngineClientOptions;

  constructor(opts: WsEngineClientOptions) {
    this.opts = {
      timeoutMs: 30_000,
      waitMs: 10_000,
      createSocket: (url) => new WebSocket(url),
      random: Math.random,
      filesBase: `${GATEWAY_BASE}/files`,
      fetch: (input, init) => fetch(input, init),
      ...opts,
    };
  }

  get state() {
    return this._state;
  }

  get problem() {
    return this._problem;
  }

  get features() {
    return this._features;
  }

  connect(): Promise<HelloResult> {
    if (this._state === 'open' && this.hello) return Promise.resolve(this.hello);
    if (!this.looping) void this.run();
    return this.waitReady();
  }

  close() {
    this.generation += 1;
    this.looping = false;
    const ws = this.ws;
    this.ws = null;
    this.endSession?.({ code: CloseCode.Normal, opened: false });
    this.endSession = null;
    if (ws) {
      ws.onopen = ws.onclose = ws.onerror = ws.onmessage = null;
      ws.close(CloseCode.Normal, 'client closed');
    }
    this.wake?.();
    this.hello = null;
    this.attempt = 0;
    this.rejectWaiters(new EngineUnavailableError('The connection was closed.'));
    this.failPending(new EngineUnavailableError('The connection was closed.'));
    this.setProblem(null);
    this.setState('closed');
  }

  retryNow() {
    this.wake?.();
  }

  async call<M extends RpcMethodName>(method: M, ...args: RpcParamsArg<M>): Promise<RpcResult<M>> {
    if (!this.isOpen()) {
      if (!this.looping) throw new EngineUnavailableError();
      await this.waitReady();
    }
    return this.send(method, args[0]) as Promise<RpcResult<M>>;
  }

  onNotification(listener: (n: EngineNotification) => void): Unsubscribe {
    this.notificationListeners.add(listener);
    return () => this.notificationListeners.delete(listener);
  }

  onStateChange(listener: (s: ConnectionState) => void): Unsubscribe {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  }

  /** Null until a handshake shows the engine serves files. */
  fileUrl(file: FileJSON, opts: { inline?: boolean } = {}): string | null {
    if (!this._features?.has(EngineFeature.Files)) return null;
    const url = `${this.opts.filesBase}/${encodeURIComponent(file.aid)}/${encodeURIComponent(file.id)}`;
    return opts.inline ? `${url}?inline=1` : url;
  }

  async uploadFile(accountId: string, file: Blob, name: string): Promise<FileJSON> {
    const url = `${this.opts.filesBase}/${encodeURIComponent(accountId)}?name=${encodeURIComponent(name)}`;
    const res = await this.opts.fetch(url, {
      method: 'POST',
      body: file,
      headers: { 'Content-Type': file.type || 'application/octet-stream' },
    });
    if (res.status === 401) this.opts.onUnpaired?.();
    if (!res.ok) throw await fileError(res, `The upload failed (${res.status}).`);
    const body = (await res.json()) as { fileId: string; file: FileJSON };
    return body.file;
  }

  async prepareFile(file: FileJSON): Promise<void> {
    const url = this.fileUrl(file);
    if (!url) throw new FileRequestError(0, "This engine can't serve files.");
    // HEAD runs files.get, which downloads the body when the file isn't on disk yet.
    const res = await this.opts.fetch(url, { method: 'HEAD' });
    if (res.status === 401) this.opts.onUnpaired?.();
    if (!res.ok) throw new FileRequestError(res.status, `${file.filename} isn't available.`);
  }

  private isOpen(): boolean {
    return this._state === 'open' && this.ws?.readyState === WebSocket.OPEN;
  }

  private async run() {
    const gen = this.generation;
    const live = () => gen === this.generation;
    this.looping = true;
    let first = true;
    while (live()) {
      if (first) {
        this.setState('connecting');
      } else {
        this.setState('reconnecting');
        await this.sleep(this.nextDelay());
        if (!live()) return;
      }
      first = false;

      if (this.opts.health) {
        let h: HealthOutcome;
        try {
          h = await this.opts.health();
        } catch {
          h = { ok: false, reason: 'unreachable' };
        }
        if (!live()) return;
        if (!h.ok && h.reason === 'unpaired') return this.terminate('unpaired');
        if (!h.ok) {
          this.setProblem('server-unreachable');
          continue;
        }
        if (!h.engineUp) {
          this.setProblem('engine-down');
          continue;
        }
      }

      const end = await this.session();
      if (!live()) return;
      if (end.opened && Date.now() - this.openedAt >= STABLE_MS) this.attempt = 0;
      switch (end.code) {
        case CloseCode.Unpaired:
          return this.terminate('unpaired');
        case CloseCode.Protocol:
        case CloseCode.Forbidden:
          console.error(`the gateway refused the connection (close ${end.code})`);
          return this.terminate('rejected');
        case CloseCode.RateLimited:
          this.minDelay = RATE_LIMITED_MS;
          this.setProblem(null);
          break;
        case CloseCode.EngineUnreachable:
          this.setProblem('engine-unreachable');
          break;
        default:
          // A drop after a good connection is usually the network or a gateway restart;
          // one that never got through the handshake means the engine side failed.
          this.setProblem(end.opened ? null : 'engine-unreachable');
      }
    }
  }

  /** One socket from open to close. Resolves when it closes, whoever closed it. */
  private session(): Promise<SessionEnd> {
    return new Promise((resolve) => {
      let ws: WebSocket;
      try {
        ws = this.opts.createSocket(this.opts.url);
      } catch {
        resolve({ code: CloseCode.Abnormal, opened: false });
        return;
      }
      this.ws = ws;
      let opened = false;
      let handshakeFailed = false;
      this.endSession = resolve;
      ws.onopen = () => {
        if (this.ws !== ws) return;
        this.handshake().then(
          (hello) => {
            if (this.ws !== ws) return;
            opened = true;
            this.hello = hello;
            this._features = new Set(hello.features ?? []);
            this.openedAt = Date.now();
            this.setProblem(null);
            this.setState('open');
            for (const w of this.waiters) {
              clearTimeout(w.timer);
              w.resolve(hello);
            }
            this.waiters.clear();
          },
          (err: unknown) => {
            if (this.ws !== ws) return;
            console.warn('engine handshake failed', err);
            handshakeFailed = true;
            ws.close();
          },
        );
      };
      // A close event always follows an error; that is where the outcome is decided.
      ws.onerror = () => {};
      ws.onclose = (ev) => {
        if (this.ws !== ws) return;
        this.ws = null;
        this.endSession = null;
        this.hello = null;
        this.failPending(new EngineUnavailableError('The connection to the engine dropped.'));
        resolve({ code: handshakeFailed ? CloseCode.EngineUnreachable : ev.code, opened });
      };
      ws.onmessage = (ev) => {
        if (this.ws === ws) this.onMessage(ev.data);
      };
    });
  }

  private async handshake(): Promise<HelloResult> {
    const hello = (await this.send('hello', {
      protocol: PROTOCOL_VERSION,
      client: CLIENT_NAME,
    })) as HelloResult;
    await this.send('subscribe', { accounts: null, classes: this.opts.classes ?? null });
    return hello;
  }

  private send(method: string, params: unknown): Promise<unknown> {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return Promise.reject(new EngineUnavailableError());
    }
    const id = this.nextId++;
    const req: RpcRequest = { jsonrpc: '2.0', id, method };
    if (params !== undefined) req.params = params;
    const timeoutMs = LONG_CALLS.has(method)
      ? Math.max(this.opts.timeoutMs, LONG_CALL_MS)
      : this.opts.timeoutMs;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new RpcError({ code: RpcErrorCode.InternalError, message: `${method} timed out` }));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      ws.send(JSON.stringify(req));
    });
  }

  private waitReady(): Promise<HelloResult> {
    if (this.isOpen() && this.hello) return Promise.resolve(this.hello);
    return new Promise((resolve, reject) => {
      const w: Waiter = {
        resolve,
        reject,
        timer: setTimeout(() => {
          this.waiters.delete(w);
          reject(new EngineUnavailableError(problemMessage(this._problem)));
        }, this.opts.waitMs),
      };
      this.waiters.add(w);
    });
  }

  private nextDelay(): number {
    const ms = Math.max(backoffDelay(this.attempt, this.opts.random), this.minDelay);
    this.attempt += 1;
    this.minDelay = 0;
    return ms;
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.wake = null;
        resolve();
      }, ms);
      this.wake = () => {
        clearTimeout(timer);
        this.wake = null;
        resolve();
      };
    });
  }

  /** Stops for good: only a new `connect()` (after a new pairing) starts again. */
  private terminate(problem: 'unpaired' | 'rejected') {
    this.generation += 1;
    this.looping = false;
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      ws.onopen = ws.onclose = ws.onerror = ws.onmessage = null;
      ws.close(CloseCode.Normal);
    }
    const err = new EngineUnavailableError(problemMessage(problem));
    this.rejectWaiters(err);
    this.failPending(err);
    this.setProblem(problem);
    this.setState('closed');
    if (problem === 'unpaired') this.opts.onUnpaired?.();
  }

  private onMessage(data: unknown) {
    if (typeof data !== 'string') return;
    let msg: unknown;
    try {
      msg = JSON.parse(data);
    } catch {
      console.warn('engine sent invalid JSON');
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

  private rejectWaiters(err: Error) {
    for (const w of this.waiters) {
      clearTimeout(w.timer);
      w.reject(err);
    }
    this.waiters.clear();
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
    this.notify();
  }

  private setProblem(p: ConnectionProblem | null) {
    if (this._problem === p) return;
    this._problem = p;
    this.notify();
  }

  private notify() {
    for (const l of this.stateListeners) l(this._state);
  }
}

export function problemMessage(p: ConnectionProblem | null): string {
  switch (p) {
    case 'server-unreachable':
      return "Couldn't reach the server.";
    case 'engine-down':
      return "The engine isn't running.";
    case 'unpaired':
      return 'This browser was unpaired.';
    case 'rejected':
      return 'The server refused the connection.';
    default:
      return "Couldn't reach the engine.";
  }
}
