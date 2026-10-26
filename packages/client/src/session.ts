// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
A client's connection to the engine, kept up: it opens a transport, says hello, subscribes,
and opens again with backoff when the connection ends, until it is closed or told to stop.
`features`, `capabilities` and `hello` are what the last handshake said. Calls go on the
current connection; one that is out when the connection ends rejects with
EngineUnavailableError and is not sent again.

What differs between clients is in the options: the endpoint (the web's gateway URL, the
desktop's engine socket, which changes with each engine generation), a preflight before each
attempt (the web's session probe), what an ended connection means (`onEnd`: retry, stop for
good, or wait for a new endpoint), the backoff and how long a call waits for a connection.
*/
import {
  Method,
  PROTOCOL_VERSION,
  type Capabilities,
  type DisconnectParams,
  type HelloResult,
  type RpcMethodName,
  type RpcParamsArg,
  type RpcResult,
  type DeltaCursor,
  type SubscribeParams,
  type SubscribeResult,
} from './protocol';
import {
  EngineUnavailableError,
  RpcError,
  RpcPeer,
  type IncomingNotification,
  type RequestHandler,
  type RequestOptions,
  type Transport,
  type TransportClose,
  type TransportFactory,
} from './rpc';

export type ConnectionState = 'idle' | 'connecting' | 'open' | 'reconnecting' | 'closed';

export type Unsubscribe = () => void;

/** Delay before each reconnect attempt; the last one repeats. */
export const BACKOFF_MS = [500, 1000, 2000, 4000, 8000, 15_000, 30_000] as const;
const JITTER = 0.2;

/** `BACKOFF_MS[attempt]` (the last one past its end), give or take 20%. */
export function backoffDelay(attempt: number, random: () => number = Math.random): number {
  const base = BACKOFF_MS[Math.min(attempt, BACKOFF_MS.length - 1)];
  return Math.round(base * (1 + (random() * 2 - 1) * JITTER));
}

/** Where the session connects. */
export interface Endpoint {
  /**
   * Which server this is. `setEndpoint` with another key connects to it at once with a fresh
   * backoff; with the same key it changes nothing.
   */
  key?: unknown;
  transport: TransportFactory;
}

/** How a connection ended, for `onEnd`. */
export interface SessionEnd extends TransportClose {
  /** The handshake finished. */
  opened: boolean;
  /** `hello`, or a required `subscribe`, failed. */
  handshakeFailed: boolean;
  /** How long it was open; 0 if it never was. */
  openMs: number;
}

/**
 * What to do after a connection ended: connect again after the backoff (at least
 * `minDelayMs`), stop for good (state `closed` until `connect()`), or wait for a new endpoint
 * (state `idle`).
 */
export type NextStep<P> =
  | { next: 'retry'; problem?: P | null; minDelayMs?: number }
  | { next: 'stop'; problem: P }
  | { next: 'wait' };

/** A preflight's answer: go ahead, or not now (`stop`: not again). */
export type Preflight<P> = { ok: true } | { ok: false; problem: P; stop?: boolean };

export interface SessionOptions<P extends string> {
  /** `hello.client`, e.g. `tern-web/0.1.0`. */
  client: string;
  /** Where to connect; `setEndpoint` changes it. */
  endpoint?: Endpoint | null;
  /** The `subscribe` sent after hello on every connection, read each time; null for none. */
  subscribe?: () => SubscribeParams | null;
  /**
   * Added as `since` to the `subscribe` of each new connection (not to `resubscribe()`), so
   * the engine replays the deltas the last one missed (FeedCursor in live.ts).
   */
  since?: () => DeltaCursor | null;
  /** A refused subscribe fails the handshake. Otherwise it is logged and the session opens. */
  subscribeRequired?: boolean;
  /** Per-call timeout; null (the default) for none. */
  timeoutMs?: number | null;
  timeoutFor?: (method: string) => number | null | undefined;
  /** How long `call()` and `connect()` wait for an open connection. */
  waitMs?: number;
  /** Delay before reconnect attempt `attempt` (0 after a good connection). */
  backoff?: (attempt: number) => number;
  /** A connection open this long resets the backoff. */
  stableMs?: number;
  /** Runs before every attempt. Without it the session connects straight away. */
  preflight?: () => Promise<Preflight<P>>;
  /** Decides what follows an ended connection. Without it: retry. */
  onEnd?: (end: SessionEnd) => NextStep<P>;
  /** Called after a stop, with its problem. */
  onStop?: (problem: P) => void;
  /** After hello and subscribe are written; what it sends follows them. */
  afterHandshake?: () => void;
  /** Takes a message before it is parsed (`RpcPeerOptions.takeRaw`). */
  takeRaw?: (text: string) => boolean;
  /** The message of a call or wait that gave up, by the session's problem. */
  unavailableMessage?: (problem: P | null) => string;
  /** The message of a call out when the connection dropped. */
  droppedMessage?: string;
  /** The message of a call out when `close()` or `setEndpoint` ended its connection. */
  closedMessage?: string;
  /** The message of a `request` made with no connection. */
  notConnectedMessage?: (method: string) => string;
}

/** A `subscribe` answer, for `onSubscribed`. */
export interface SubscribeAnswer {
  result: SubscribeResult;
  /** The `since` it was sent with; null without one. */
  since: DeltaCursor | null;
  /** Sent in the handshake of a new connection, as opposed to by `resubscribe()`. */
  handshake: boolean;
}

interface Waiter {
  resolve: (hello: HelloResult) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

interface Connection {
  transport: Transport | null;
  peer: RpcPeer;
  opened: boolean;
  openedAt: number;
  hello: HelloResult | null;
  subscribed: boolean;
  handshakeFailed: boolean;
  /** The engine said `disconnect`: drop after this batch. */
  dropAfterBatch: boolean;
  /** Why calls out fail, when the session ended it. */
  endMessage: string | null;
  /** Calls made before hello was written; they follow the handshake. Null after it. */
  early: Array<() => void> | null;
  end: (close: TransportClose) => void;
}

function emit<T extends unknown[]>(listeners: Set<(...args: T) => void>, ...args: T) {
  for (const l of [...listeners]) {
    try {
      l(...args);
    } catch (err) {
      console.error('An engine session listener failed', err);
    }
  }
}

export class Session<P extends string = string> {
  private _state: ConnectionState = 'idle';
  private _problem: P | null = null;
  private _features: ReadonlySet<string> | null = null;
  private _capabilities: Capabilities | null = null;
  private endpoint: Endpoint | null;
  private conn: Connection | null = null;
  /** Bumped by close(), setEndpoint() and a stop; an older loop ends at its next step. */
  private generation = 0;
  private looping = false;
  private attempt = 0;
  private minDelay = 0;
  private wake: (() => void) | null = null;
  private subscribedKey: string | null = null;
  private readonly waiters = new Set<Waiter>();
  private readonly handlers = new Map<string, RequestHandler>();
  private readonly notificationListeners = new Set<(n: IncomingNotification) => void>();
  private readonly stateListeners = new Set<(s: ConnectionState) => void>();
  private readonly helloListeners = new Set<(hello: HelloResult) => void>();
  private readonly batchListeners = new Set<() => void>();
  private readonly disconnectListeners = new Set<() => void>();
  private readonly subscribedListeners = new Set<(answer: SubscribeAnswer) => void>();

  constructor(private readonly opts: SessionOptions<P>) {
    this.endpoint = opts.endpoint ?? null;
  }

  get state(): ConnectionState {
    return this._state;
  }

  /** Why it isn't connected, when `onEnd` or the preflight said; null while open. */
  get problem(): P | null {
    return this._problem;
  }

  /** `hello.features` of the last handshake, kept across reconnects; null before the first. */
  get features(): ReadonlySet<string> | null {
    return this._features;
  }

  /** From the last hello or `capabilities.changed`. */
  get capabilities(): Capabilities | null {
    return this._capabilities;
  }

  /** The current connection's hello; null when there is none or it hasn't answered. */
  get hello(): HelloResult | null {
    return this.conn?.hello ?? null;
  }

  /** A connection is up or being set up, so `request` sends on it. */
  get hasConnection(): boolean {
    return this.conn !== null;
  }

  /** Starts connecting if it isn't, and resolves with the hello once open. */
  connect(): Promise<HelloResult> {
    const hello = this.hello;
    if (this.conn?.opened && hello) return Promise.resolve(hello);
    if (!this.looping) this.start();
    return this.waitReady();
  }

  /**
   * Connects to `endpoint` instead, at once and with a fresh backoff, unless it has the
   * current one's key. Null disconnects and waits (state `idle`).
   */
  setEndpoint(endpoint: Endpoint | null) {
    if (endpoint && this.endpoint && endpoint.key === this.endpoint.key) {
      this.endpoint = endpoint;
      return;
    }
    this.endpoint = endpoint;
    this.halt(this.opts.closedMessage);
    if (endpoint) this.start();
    else this.setState('idle');
  }

  /** Disconnects for good; `connect()` starts again. */
  close() {
    const message = this.opts.closedMessage ?? 'The connection was closed.';
    this.halt(message, 1000, 'client closed');
    this.rejectWaiters(new EngineUnavailableError(message));
    this.setProblem(null);
    this.setState('closed');
  }

  /** Skips the rest of a reconnect backoff. */
  retryNow() {
    this.wake?.();
  }

  /** A typed call; waits up to `waitMs` for the connection while the session is connecting. */
  async call<M extends RpcMethodName>(method: M, ...args: RpcParamsArg<M>): Promise<RpcResult<M>> {
    if (!this.conn?.opened) {
      if (!this.looping) throw new EngineUnavailableError(this.unavailableMessage());
      await this.waitReady();
    }
    return (await this.request(method, args[0])) as RpcResult<M>;
  }

  /**
   * A call on the current connection as it is, open or still in its handshake (it is sent
   * after hello and subscribe); rejects at once when there is none.
   */
  request<R = unknown>(method: string, params?: unknown, opts?: RequestOptions): Promise<R> {
    const conn = this.conn;
    if (!conn) {
      const message = this.opts.notConnectedMessage?.(method) ?? this.unavailableMessage();
      return Promise.reject(new EngineUnavailableError(message));
    }
    const early = conn.early;
    if (!early) return conn.peer.request(method, params, opts) as Promise<R>;
    return new Promise<R>((resolve, reject) => {
      early.push(() => {
        conn.peer.request(method, params, opts).then(resolve as (v: unknown) => void, reject);
      });
    });
  }

  /** Sends `subscribe` again on the current connection if its params changed. */
  resubscribe() {
    const conn = this.conn;
    // Before the handshake, its own subscribe reads the params.
    if (!conn || conn.early) return;
    const params = this.opts.subscribe?.() ?? null;
    if (!params || JSON.stringify(params) === this.subscribedKey) return;
    this.sendSubscribe(conn, params);
  }

  /** Answers the engine's requests for `method` (extension hooks). */
  handle(method: string, handler: RequestHandler): Unsubscribe {
    this.handlers.set(method, handler);
    return () => {
      if (this.handlers.get(method) === handler) this.handlers.delete(method);
    };
  }

  /** Every notification but `disconnect`, as it is read. */
  onNotification(listener: (n: IncomingNotification) => void): Unsubscribe {
    this.notificationListeners.add(listener);
    return () => this.notificationListeners.delete(listener);
  }

  /** Also called when `problem` changes. */
  onStateChange(listener: (state: ConnectionState) => void): Unsubscribe {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  }

  /** As each hello's answer is read, before the messages after it. */
  onHello(listener: (hello: HelloResult) => void): Unsubscribe {
    this.helloListeners.add(listener);
    return () => this.helloListeners.delete(listener);
  }

  /** After each batch of messages read together. */
  onBatch(listener: () => void): Unsubscribe {
    this.batchListeners.add(listener);
    return () => this.batchListeners.delete(listener);
  }

  /** As each `subscribe` answer is read, with the `since` it was sent with. */
  onSubscribed(listener: (answer: SubscribeAnswer) => void): Unsubscribe {
    this.subscribedListeners.add(listener);
    return () => this.subscribedListeners.delete(listener);
  }

  /** When a connection ends, whatever ended it. */
  onDisconnect(listener: () => void): Unsubscribe {
    this.disconnectListeners.add(listener);
    return () => this.disconnectListeners.delete(listener);
  }

  // Private

  private unavailableMessage(): string {
    return this.opts.unavailableMessage?.(this._problem) ?? 'The engine is not connected.';
  }

  private start() {
    this.attempt = 0;
    this.minDelay = 0;
    void this.run();
  }

  /** Stops the loop and ends the connection. */
  private halt(message?: string, code?: number, reason?: string) {
    this.generation += 1;
    this.looping = false;
    const conn = this.conn;
    if (conn) this.drop(conn, message, code, reason);
    this.wake?.();
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

      if (this.opts.preflight) {
        let refused: Extract<Preflight<P>, { ok: false }> | null = null;
        try {
          const pre = await this.opts.preflight();
          if (pre.ok === false) refused = pre;
        } catch {
          // No answer is no objection; the connection attempt says what is wrong.
        }
        if (!live()) return;
        if (refused) {
          if (refused.stop) return this.stop(refused.problem);
          this.setProblem(refused.problem);
          continue;
        }
      }

      const endpoint = this.endpoint;
      if (!endpoint) {
        this.looping = false;
        this.setState('idle');
        return;
      }
      const end = await this.connectOnce(endpoint);
      if (!live()) return;
      if (end.opened && end.openMs >= (this.opts.stableMs ?? 30_000)) this.attempt = 0;
      const step: NextStep<P> = this.opts.onEnd?.(end) ?? { next: 'retry' };
      if (step.next === 'stop') return this.stop(step.problem);
      if (step.next === 'wait') {
        this.looping = false;
        this.setState('idle');
        return;
      }
      if (step.minDelayMs) this.minDelay = step.minDelayMs;
      this.setProblem(step.problem ?? null);
    }
  }

  /** One connection from open to end. Resolves when it ends, whoever ended it. */
  private connectOnce(endpoint: Endpoint): Promise<SessionEnd> {
    return new Promise((resolve) => {
      let ended = false;
      let openEarly = false;
      const conn: Connection = {
        transport: null,
        peer: new RpcPeer(() => conn.transport, {
          timeoutMs: this.opts.timeoutMs,
          timeoutFor: this.opts.timeoutFor,
          takeRaw: this.opts.takeRaw,
          handlers: this.handlers,
          onNotification: (n) => this.onNotificationIn(conn, n),
          onBatchEnd: () => this.onBatchEnd(conn),
        }),
        opened: false,
        openedAt: 0,
        hello: null,
        subscribed: false,
        handshakeFailed: false,
        dropAfterBatch: false,
        endMessage: null,
        early: [],
        end: (close) => {
          if (ended) return;
          ended = true;
          if (this.conn === conn) {
            this.conn = null;
            this.subscribedKey = null;
          }
          conn.peer.fail(
            new EngineUnavailableError(
              conn.endMessage ??
                this.opts.droppedMessage ??
                'The connection to the engine dropped.',
            ),
          );
          // Calls held for the handshake: the failed peer rejects them.
          const early = conn.early ?? [];
          conn.early = null;
          for (const send of early) send();
          emit(this.disconnectListeners);
          resolve({
            ...close,
            opened: conn.opened,
            handshakeFailed: conn.handshakeFailed,
            openMs: conn.opened ? Date.now() - conn.openedAt : 0,
          });
        },
      };
      this.conn = conn;
      const onOpen = () => {
        if (ended || this.conn !== conn) return;
        conn.peer.opened();
        this.handshake(conn);
      };
      try {
        conn.transport = endpoint.transport({
          open: () => {
            if (conn.transport) onOpen();
            else openEarly = true;
          },
          messages: (texts) => {
            if (!ended && this.conn === conn) conn.peer.receive(texts);
          },
          close: (close) => conn.end(close),
        });
      } catch (err) {
        console.warn('The engine transport failed to open', err);
        conn.end({ code: null });
        return;
      }
      if (openEarly) onOpen();
    });
  }

  private handshake(conn: Connection) {
    const failed = (err: unknown) => {
      if (this.conn !== conn || conn.opened) return;
      console.warn('The engine handshake failed', err);
      conn.handshakeFailed = true;
      this.drop(conn);
    };
    conn.peer
      .request(
        Method.Hello,
        { protocol: PROTOCOL_VERSION, client: this.opts.client },
        {
          onAnswer: (result, error) => {
            if (error || this.conn !== conn) return;
            const hello = result as HelloResult;
            conn.hello = hello;
            this._features = new Set(hello.features ?? []);
            this._capabilities = hello.capabilities ?? null;
            emit(this.helloListeners, hello);
            this.maybeOpen(conn);
          },
        },
      )
      .catch(failed);
    const params = this.opts.subscribe?.() ?? null;
    if (params) this.sendSubscribe(conn, params, failed, this.opts.since?.() ?? null);
    else conn.subscribed = true;
    const early = conn.early ?? [];
    conn.early = null;
    this.opts.afterHandshake?.();
    for (const send of early) send();
  }

  private sendSubscribe(
    conn: Connection,
    params: SubscribeParams,
    failed?: (err: unknown) => void,
    since: DeltaCursor | null = null,
  ) {
    // The key leaves `since` out: resubscribe() compares the filter only.
    const key = JSON.stringify(params);
    this.subscribedKey = key;
    const required = !!failed && !!this.opts.subscribeRequired;
    const sent = since ? { ...params, since } : params;
    conn.peer
      .request(Method.Subscribe, sent, {
        onAnswer: (result, error) => {
          if (this.conn !== conn || (error && required)) return;
          if (!error) {
            emit(this.subscribedListeners, {
              result: result as SubscribeResult,
              since,
              handshake: !!failed,
            });
          }
          conn.subscribed = true;
          this.maybeOpen(conn);
        },
      })
      .catch((err: unknown) => {
        if (required) return failed!(err);
        // The engine keeps the last subscription; the next resubscribe tries again.
        if (this.conn === conn && this.subscribedKey === key) this.subscribedKey = null;
        if (err instanceof RpcError) console.warn(`The engine refused subscribe: ${err.message}`);
      });
  }

  private maybeOpen(conn: Connection) {
    if (conn.opened || !conn.hello || !conn.subscribed) return;
    conn.opened = true;
    conn.openedAt = Date.now();
    this.setProblem(null);
    this.setState('open');
    const hello = conn.hello;
    for (const w of this.waiters) {
      clearTimeout(w.timer);
      w.resolve(hello);
    }
    this.waiters.clear();
  }

  private onNotificationIn(conn: Connection, n: IncomingNotification) {
    if (n.method === Method.Disconnect) {
      const reason = (n.params as DisconnectParams | undefined)?.reason;
      console.warn(`The engine dropped the connection: ${reason}`);
      conn.dropAfterBatch = true;
      return;
    }
    if (n.method === Method.CapabilitiesChanged && n.params && typeof n.params === 'object') {
      this._capabilities = n.params as Capabilities;
    }
    emit(this.notificationListeners, n);
  }

  private onBatchEnd(conn: Connection) {
    emit(this.batchListeners);
    if (conn.dropAfterBatch && this.conn === conn) this.drop(conn);
  }

  /** Ends `conn` now; the transport's own close event comes too late to matter. */
  private drop(conn: Connection, message?: string, code?: number, reason?: string) {
    if (message) conn.endMessage = message;
    const transport = conn.transport;
    conn.end({ code: code ?? null, reason });
    try {
      transport?.close(code, reason);
    } catch {
      // Already closed.
    }
  }

  /** Stops for good: only `connect()` starts again. */
  private stop(problem: P) {
    const message = this.opts.unavailableMessage?.(problem) ?? 'The engine is not connected.';
    this.halt(message, 1000);
    this.rejectWaiters(new EngineUnavailableError(message));
    this.setProblem(problem);
    this.setState('closed');
    this.opts.onStop?.(problem);
  }

  private waitReady(): Promise<HelloResult> {
    const hello = this.hello;
    if (this.conn?.opened && hello) return Promise.resolve(hello);
    return new Promise((resolve, reject) => {
      const w: Waiter = {
        resolve,
        reject,
        timer: setTimeout(() => {
          this.waiters.delete(w);
          reject(new EngineUnavailableError(this.unavailableMessage()));
        }, this.opts.waitMs ?? 10_000),
      };
      this.waiters.add(w);
    });
  }

  private rejectWaiters(err: Error) {
    for (const w of this.waiters) {
      clearTimeout(w.timer);
      w.reject(err);
    }
    this.waiters.clear();
  }

  private nextDelay(): number {
    const backoff = this.opts.backoff ?? ((n: number) => backoffDelay(n));
    const ms = Math.max(backoff(this.attempt), this.minDelay);
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

  private setState(s: ConnectionState) {
    if (this._state === s) return;
    this._state = s;
    emit(this.stateListeners, s);
  }

  private setProblem(p: P | null) {
    if (this._problem === p) return;
    this._problem = p;
    emit(this.stateListeners, this._state);
  }
}
