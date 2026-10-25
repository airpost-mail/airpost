// The engine through apps/gateway on this page's origin: `@tern/client`'s Session over its
// WebSocket transport, with the gateway's parts here: the session probe before each connect,
// what its close codes mean, and the file routes. The gateway authorises the upgrade with the
// pairing cookie, so nothing here carries a token.
import {
  BACKOFF_MS,
  backoffDelay,
  Session,
  webSocketTransport,
  type NextStep,
  type SessionEnd,
} from '@tern/client';
import type { ConnectionProblem, ConnectionState, EngineClient, Unsubscribe } from './client';
import type {
  EngineNotification,
  HelloResult,
  RpcMethodName,
  RpcParamsArg,
  RpcResult,
} from './rpc';
import { GATEWAY_BASE } from './mode';
import type { FileJSON } from './types';

export { BACKOFF_MS, backoffDelay };

/** What `GET /gateway/v1/session` said before a connect. */
export type HealthOutcome =
  { ok: true; engineUp: boolean } | { ok: false; reason: 'unpaired' | 'unreachable' };

/** Close codes the gateway sends, plus the standard ones the client acts on. */
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

/** A connection that lasted this long resets the backoff. */
export const STABLE_MS = 30_000;
/** First delay after the gateway says too many connections or frames. */
export const RATE_LIMITED_MS = 5000;

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
  /** The gateway's file routes. */
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
 * Calls that wait on a model: the engine gives a provider up to 120 s,
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

/** What an ended connection means, by the gateway's close code. */
function nextStep(end: SessionEnd): NextStep<ConnectionProblem> {
  // hello refused or unanswered: the engine side failed.
  const code = end.handshakeFailed ? CloseCode.EngineUnreachable : end.code;
  switch (code) {
    case CloseCode.Unpaired:
      return { next: 'stop', problem: 'unpaired' };
    case CloseCode.Protocol:
    case CloseCode.Forbidden:
      console.error(`the gateway refused the connection (close ${code})`);
      return { next: 'stop', problem: 'rejected' };
    case CloseCode.RateLimited:
      return { next: 'retry', problem: null, minDelayMs: RATE_LIMITED_MS };
    case CloseCode.EngineUnreachable:
      return { next: 'retry', problem: 'engine-unreachable' };
    default:
      // A drop after a good connection is usually the network or a gateway restart;
      // one that never got through the handshake means the engine side failed.
      return { next: 'retry', problem: end.opened ? null : 'engine-unreachable' };
  }
}

export class WsEngineClient implements EngineClient {
  readonly kind = 'ws' as const;
  private readonly session: Session<ConnectionProblem>;
  private readonly opts: Required<Pick<WsEngineClientOptions, 'filesBase' | 'fetch'>> &
    WsEngineClientOptions;

  constructor(opts: WsEngineClientOptions) {
    this.opts = {
      filesBase: `${GATEWAY_BASE}/files`,
      fetch: (input, init) => fetch(input, init),
      ...opts,
    };
    const random = opts.random ?? Math.random;
    const health = opts.health;
    this.session = new Session<ConnectionProblem>({
      client: CLIENT_NAME,
      endpoint: {
        transport: webSocketTransport(opts.url, {
          createSocket: opts.createSocket ?? ((url) => new WebSocket(url)),
        }),
      },
      subscribe: () => ({ accounts: null, classes: opts.classes ?? null }),
      subscribeRequired: true,
      timeoutMs: opts.timeoutMs ?? 30_000,
      timeoutFor: (method) =>
        LONG_CALLS.has(method) ? Math.max(opts.timeoutMs ?? 30_000, LONG_CALL_MS) : undefined,
      waitMs: opts.waitMs ?? 10_000,
      backoff: (attempt) => backoffDelay(attempt, random),
      stableMs: STABLE_MS,
      preflight: health && (() => preflight(health)),
      onEnd: nextStep,
      onStop: (problem) => {
        if (problem === 'unpaired') opts.onUnpaired?.();
      },
      unavailableMessage: problemMessage,
    });
  }

  get state(): ConnectionState {
    return this.session.state;
  }

  get problem(): ConnectionProblem | null {
    return this.session.problem;
  }

  get features() {
    return this.session.features;
  }

  connect(): Promise<HelloResult> {
    return this.session.connect();
  }

  close() {
    this.session.close();
  }

  retryNow() {
    this.session.retryNow();
  }

  call<M extends RpcMethodName>(method: M, ...args: RpcParamsArg<M>): Promise<RpcResult<M>> {
    return this.session.call(method, ...args);
  }

  onNotification(listener: (n: EngineNotification) => void): Unsubscribe {
    return this.session.onNotification((n) => listener(n as EngineNotification));
  }

  onStateChange(listener: (s: ConnectionState) => void): Unsubscribe {
    return this.session.onStateChange(listener);
  }

  /** Null until the first handshake. */
  fileUrl(file: FileJSON, opts: { inline?: boolean } = {}): string | null {
    if (!this.session.features) return null;
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
}

/** The session probe as the session's preflight. */
async function preflight(health: () => Promise<HealthOutcome>) {
  let h: HealthOutcome;
  try {
    h = await health();
  } catch {
    h = { ok: false, reason: 'unreachable' };
  }
  if (!h.ok && h.reason === 'unpaired') {
    return { ok: false, problem: 'unpaired', stop: true } as const;
  }
  if (!h.ok) return { ok: false, problem: 'server-unreachable' } as const;
  if (!h.engineUp) return { ok: false, problem: 'engine-down' } as const;
  return { ok: true } as const;
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
