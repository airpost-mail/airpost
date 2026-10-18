// The `ai.*` calls (docs/ai-plan.md section 3). Streamed calls pick a streamId, collect
// the `ai.delta` notifications for it, and resolve with the engine's reply, which has the
// full text. No call carries `config` or `settings`: a relayed connection may not, so the
// engine uses the providers and switches it keeps.
import {
  EngineUnavailableError,
  RpcError,
  RpcErrorCode,
  type AiCompletion,
  type AiErrorJSON,
  type EngineClient,
  type RpcMethodName,
  type RpcParams,
  type RpcResult,
} from '@/engine';

/** ms-ai error keys, plus `engine-busy`, `engine-disconnected` and `unknown`. */
export type AiErrorKey =
  | 'not-configured'
  | 'not-enabled'
  | 'auth'
  | 'rate-limited'
  | 'quota-exceeded'
  | 'offline'
  | 'bad-request'
  | 'model-not-found'
  | 'context-too-long'
  | 'provider-error'
  | 'cancelled'
  | 'unsupported'
  | 'engine-busy'
  | 'engine-disconnected'
  | 'unknown';

const ENGINE_KEYS: readonly string[] = [
  'not-configured',
  'not-enabled',
  'auth',
  'rate-limited',
  'quota-exceeded',
  'offline',
  'bad-request',
  'model-not-found',
  'context-too-long',
  'provider-error',
  'cancelled',
  'unsupported',
];

export class AiError extends Error {
  readonly key: AiErrorKey;
  readonly retryAfter?: number;

  constructor(key: AiErrorKey, message?: string, retryAfter?: number) {
    super(message || key);
    this.name = 'AiError';
    this.key = key;
    if (retryAfter !== undefined) this.retryAfter = retryAfter;
  }
}

export function toAiError(err: unknown): AiError {
  if (err instanceof AiError) return err;
  if (err instanceof RpcError) {
    if (err.code === RpcErrorCode.AiFailed) {
      const data = (err.data ?? {}) as Partial<AiErrorJSON>;
      const key =
        typeof data.key === 'string' && ENGINE_KEYS.includes(data.key) ? data.key : 'unknown';
      return new AiError(key as AiErrorKey, data.message ?? err.message, data.retryAfter);
    }
    if (err.code === RpcErrorCode.EngineBusy) return new AiError('engine-busy', err.message);
    if (err.code === RpcErrorCode.NotFound) return new AiError('unsupported', err.message);
  }
  if (err instanceof EngineUnavailableError) return new AiError('engine-disconnected', err.message);
  return new AiError('unknown', err instanceof Error ? err.message : String(err));
}

export function isCancelled(err: unknown): boolean {
  return err instanceof AiError && err.key === 'cancelled';
}

/** A short message for the user; `name` is the provider's. */
export function errorMessage(err: unknown, name = 'The model provider'): string {
  const e = toAiError(err);
  switch (e.key) {
    case 'not-configured':
      return `${name} isn't set up on the engine. Check Preferences > AI on the desktop.`;
    case 'not-enabled':
      return 'AI is off for this account or feature.';
    case 'auth':
      return `${name} rejected the API key.`;
    case 'rate-limited':
      return e.retryAfter
        ? `${name} is limiting requests. Try again in ${e.retryAfter} seconds.`
        : `${name} is limiting requests. Try again in a moment.`;
    case 'quota-exceeded':
      return `${name} says the quota or credit is used up.`;
    case 'offline':
      return `The engine couldn't reach ${name}.`;
    case 'bad-request':
      return `${name} refused the request: ${e.message}`;
    case 'model-not-found':
      return `${name} doesn't have that model.`;
    case 'context-too-long':
      return 'This thread is too long for the model.';
    case 'provider-error':
      return `${name} had a problem. Try again later.`;
    case 'cancelled':
      return 'Stopped.';
    case 'unsupported':
      return "This thread can't be sent to a model.";
    case 'engine-busy':
      return 'The engine is busy with other AI requests. Try again in a moment.';
    case 'engine-disconnected':
      return 'The connection to the engine dropped. Try again.';
    default:
      return `Something went wrong with ${name}.`;
  }
}

let streams = 0;

export function newStreamId(): string {
  streams += 1;
  const rand = Math.random().toString(36).slice(2, 8);
  return `web-${Date.now().toString(36)}-${streams}-${rand}`;
}

/** A call in flight; `cancel` makes the engine stop the provider request. */
export interface AiCall<R> {
  streamId: string;
  promise: Promise<R>;
  cancel(): void;
}

type StreamMethod = 'ai.summarize' | 'ai.suggestReplies' | 'ai.draftReply';

export function startAiCall<M extends StreamMethod & RpcMethodName>(
  client: EngineClient,
  method: M,
  params: Omit<RpcParams<M>, 'streamId'>,
  onDelta?: (text: string) => void,
): AiCall<RpcResult<M>> {
  const streamId = newStreamId();
  let cancelled = false;
  const off = onDelta
    ? client.onNotification((n) => {
        if (n.method === 'ai.delta' && n.params.streamId === streamId && !cancelled)
          onDelta(n.params.text);
      })
    : null;
  const call = client.call.bind(client) as (m: M, p: RpcParams<M>) => Promise<RpcResult<M>>;
  const promise = call(method, { ...params, streamId } as RpcParams<M>)
    .catch((err: unknown) => {
      throw cancelled ? new AiError('cancelled') : toAiError(err);
    })
    .finally(() => off?.());
  return {
    streamId,
    promise,
    cancel() {
      if (cancelled) return;
      cancelled = true;
      void client.call('ai.cancel', { streamId }).catch(() => {});
    },
  };
}

export function summarize(
  client: EngineClient,
  p: { accountId: string; threadId: string; refresh?: boolean },
  onDelta?: (text: string) => void,
): AiCall<AiCompletion> {
  return startAiCall(client, 'ai.summarize', p, onDelta);
}
