// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The `ai.*` calls on the window's engine connection through the Engine
export, and the providers' keys. An engine that keeps the keys (keysInEngine) has them in its
secret store and its own copy of core.ai, so a call names the provider by id; with an older engine
each call carries the provider config with the key from the keyset. Either way a call carries the
AI switches from config (core.ai), read when the call is made. Streamed calls pick a streamId,
collect the `ai.delta` notifications for it and resolve with the engine's reply, which has the
full text.
*/
import {
  AI,
  Engine,
  KeyManager,
  localized,
  AiFeature,
  AiProvider,
  AiCompletion,
  AiModelInfo,
  AiSuggestion,
  AiTestReport,
} from 'mailspring-exports';

// ms-ipc `code::AI_FAILED` and `code::ENGINE_BUSY`.
const AI_FAILED = -32011;
const ENGINE_BUSY = -32012;
const ENGINE_FEATURE = 'ai';
// ms-ipc `feature::AI_KEYS` (stored providers read `integration:ai-<id>`) and
// `feature::ACCOUNT_SETUP` (`secrets.*` on the socket).
const KEYS_FEATURE = 'ai-keys';
const SECRETS_FEATURE = 'account-setup';

/**
 * crates/ms-ai error keys, plus the desktop's own: `engine-unavailable` (the engine doesn't serve
 * ai.*), `engine-disconnected` (the connection dropped), `engine-busy` (the engine's AI lane is
 * full, -32012), `keyset` (the saved key could not be read) and `stopped` (a triage job ended
 * early: the account stopped or triage was turned off).
 */
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
  | 'engine-unavailable'
  | 'engine-disconnected'
  | 'engine-busy'
  | 'keyset'
  | 'stopped'
  | 'unknown';

const KNOWN_KEYS: AiErrorKey[] = [
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
  'engine-unavailable',
  'engine-disconnected',
  'engine-busy',
  'keyset',
];

export class AiError extends Error {
  key: AiErrorKey;
  retryable: boolean;
  status?: number;
  retryAfter?: number;

  constructor(fields: {
    key: AiErrorKey;
    message?: string;
    retryable?: boolean;
    status?: number;
    retryAfter?: number;
  }) {
    super(fields.message || fields.key);
    this.name = 'AiError';
    this.key = fields.key;
    this.retryable = !!fields.retryable;
    if (typeof fields.status === 'number') this.status = fields.status;
    if (typeof fields.retryAfter === 'number') this.retryAfter = fields.retryAfter;
  }
}

/** A socket error or the serialized `{key, message, ...}`, as an AiError. */
export function toAiError(err: unknown): AiError {
  if (err instanceof AiError) return err;
  const e = err && typeof err === 'object' ? (err as any) : null;
  if (e && e.code === AI_FAILED && e.data && typeof e.data.key === 'string') {
    const key = KNOWN_KEYS.includes(e.data.key) ? e.data.key : 'unknown';
    return new AiError({ ...e.data, key });
  }
  if (e && typeof e.key === 'string') {
    return new AiError({ ...e, key: KNOWN_KEYS.includes(e.key) ? e.key : 'unknown' });
  }
  if (e && e.code === ENGINE_BUSY) {
    return new AiError({ key: 'engine-busy', message: e.message, retryable: true });
  }
  // EngineConnectionError: the request was sent but the connection closed before the reply.
  if (e && e.code === null) {
    return new AiError({ key: 'engine-disconnected', message: e.message, retryable: true });
  }
  return new AiError({ key: 'unknown', message: e && e.message ? e.message : String(err) });
}

export function isCancelled(err: unknown) {
  return err instanceof AiError && err.key === 'cancelled';
}

/** A short message for the user. */
export function errorMessage(err: unknown, providerName?: string): string {
  const e = toAiError(err);
  const name = providerName || localized('The model provider');
  switch (e.key) {
    case 'not-configured':
      return localized('%@ is not set up. Check Preferences > AI.', name);
    case 'not-enabled':
      return localized('AI is turned off for this account or feature. Check Preferences > AI.');
    case 'auth':
      return localized('%@ rejected the API key.', name);
    case 'rate-limited':
      return e.retryAfter
        ? localized(
            '%1$@ is limiting requests. Try again in %2$@ seconds.',
            name,
            `${e.retryAfter}`
          )
        : localized('%@ is limiting requests. Try again in a moment.', name);
    case 'quota-exceeded':
      return localized("%@ says the account's quota or credit is used up.", name);
    case 'offline':
      return localized('Could not reach %@. Check the URL and your connection.', name);
    case 'bad-request':
      return localized('%1$@ refused the request: %2$@', name, e.message);
    case 'model-not-found':
      return localized("%@ doesn't have that model. Pick another in Preferences > AI.", name);
    case 'context-too-long':
      return localized('This thread is too long for the model.');
    case 'provider-error':
      return localized('%@ had a problem. Try again later.', name);
    case 'cancelled':
      return localized('Stopped.');
    case 'unsupported':
      return localized("This thread can't be sent to a model.");
    case 'engine-unavailable':
      return localized('Waiting for engine support.');
    case 'engine-disconnected':
      return localized('The sync engine disconnected. Try again.');
    case 'engine-busy':
      return localized('Tern is busy with other AI requests. Try again in a moment.');
    case 'keyset':
      return localized('Could not read the saved API key: %@', e.message);
    case 'stopped':
      return localized('Stopped before the end.');
    default:
      return localized('Something went wrong with %@.', name);
  }
}

let streamCount = 0;

export function newStreamId() {
  streamCount += 1;
  return `ai-${process.pid}-${Date.now().toString(36)}-${streamCount}`;
}

/** A call in flight: its promise, and `cancel`, which makes the engine stop the provider request. */
export interface AiCall<R> {
  streamId: string;
  promise: Promise<R>;
  cancel(): void;
}

function requireEngine() {
  if (!Engine.has(ENGINE_FEATURE)) {
    throw new AiError({
      key: 'engine-unavailable',
      message: 'The sync engine does not serve ai.* yet.',
    });
  }
}

// -- Keys ------------------------------------------------------------------------------------

/**
 * Whether the engine keeps the AI keys (`integration:ai-<id>`) and
 * the providers, so calls name a provider and send no key. An engine before AI6 lists
 * `account-setup` and `ai-triage` too, but reads no key from its secret store.
 */
export function keysInEngine() {
  return Engine.has(KEYS_FEATURE);
}

// Until the engine connects, which one holds the key is unknown: one that keeps keys may have
// it, so saying "none saved" or forgetting only the keyset's copy could let it reach a new server.
function requireConnected() {
  if (!Engine.connected) {
    throw new AiError({
      key: 'engine-disconnected',
      message: localized('The sync engine is not connected.'),
      retryable: true,
    });
  }
}

/** The engine's secret for the provider's key (ms-secrets `names::ai_provider`). */
export function secretName(providerId: string) {
  return `integration:ai-${providerId}`;
}

async function dropKeysetCopy(providerId: string) {
  if (await KeyManager.getPassword(AI.keyName(providerId))) {
    await KeyManager.deletePassword(AI.keyName(providerId));
  }
}

/** Saves the provider's key in the engine, dropping a keyset copy, or in the keyset when the
 * engine can't keep it. */
export async function saveKey(providerId: string, value: string) {
  if (!keysInEngine()) {
    await KeyManager.replacePassword(AI.keyName(providerId), value);
    return;
  }
  await Engine.request('secrets.set', { name: secretName(providerId), value });
  await dropKeysetCopy(providerId);
}

/** Deletes the provider's key from the engine and the keyset; throws while disconnected. */
export async function forgetKey(providerId: string) {
  requireConnected();
  if (keysInEngine()) {
    await Engine.request('secrets.delete', { name: secretName(providerId) });
  } else if (Engine.has(SECRETS_FEATURE)) {
    // This engine reads no AI key, but one that does may have run before and kept it; it
    // also drops the key itself when the provider changes server.
    await Engine.request('secrets.delete', { name: secretName(providerId) }).catch(() => {});
  }
  await dropKeysetCopy(providerId);
}

/** Whether a key is saved for the provider where calls take it from; throws while disconnected. */
export async function hasSavedKey(providerId: string) {
  requireConnected();
  if (keysInEngine()) {
    const name = secretName(providerId);
    const has = await Engine.request<{ [name: string]: boolean }>('secrets.has', {
      names: [name],
    });
    return !!(has && has[name]);
  }
  return !!((await KeyManager.getPassword(AI.keyName(providerId))) || '').trim();
}

/**
 * Moves the keys still in the keyset to an engine that keeps them: credentials.import took the
 * keyset's keys only once, and a key saved after that by a build without keysInEngine went to
 * the keyset. The keyset's copy is the newer one.
 */
export async function moveKeysToEngine() {
  if (!keysInEngine()) return;
  for (const provider of AI.readSettings().providers) {
    const key = ((await KeyManager.getPassword(AI.keyName(provider.id))) || '').trim();
    if (!key) continue;
    await Engine.request('secrets.set', { name: secretName(provider.id), value: key });
    await KeyManager.deletePassword(AI.keyName(provider.id));
  }
}

/**
 * The config for `provider`, as the Preferences form shows it: with the typed key when given.
 * Without one, an engine that keeps keys adds the saved key itself for the provider's own
 * server; an older engine gets the keyset's.
 */
export async function configFor(provider: AiProvider, typedKey?: string) {
  let key = (typedKey || '').trim();
  if (!key && !keysInEngine()) {
    try {
      key = await KeyManager.getPassword(AI.keyName(provider.id));
    } catch (err) {
      throw new AiError({ key: 'keyset', message: (err && err.message) || String(err) });
    }
  }
  return AI.providerConfig(provider, key);
}

/** How a call names its provider: by id to an engine that keeps the keys, else by config. */
export async function providerParams(provider: AiProvider) {
  if (keysInEngine()) return { provider: provider.id };
  return { config: await configFor(provider) };
}

function start<R>(
  method: string,
  build: () => Promise<object>,
  onDelta?: (text: string) => void
): AiCall<R> {
  const streamId = newStreamId();
  let cancelled = false;
  let sent = false;
  const subscription = onDelta
    ? Engine.subscribe({ methods: ['ai.delta'] }, (n) => {
        if (!cancelled && n.params && n.params.streamId === streamId) onDelta(n.params.text || '');
      })
    : null;
  const promise = (async () => {
    try {
      requireEngine();
      const params = await build();
      if (cancelled) throw new AiError({ key: 'cancelled' });
      sent = true;
      return await Engine.request<R>(method, { ...params, streamId });
    } catch (err) {
      throw cancelled ? new AiError({ key: 'cancelled' }) : toAiError(err);
    } finally {
      if (subscription) subscription.dispose();
    }
  })();
  return {
    streamId,
    promise,
    cancel() {
      if (cancelled) return;
      cancelled = true;
      if (sent) Engine.request('ai.cancel', { streamId }).catch(() => {});
    },
  };
}

/**
 * Builds a thread call's params: the feature's provider and the switches, after checking here
 * what the engine checks, so a call that would be refused reads no key.
 */
async function threadParams(feature: AiFeature, accountId: string, threadId: string) {
  const settings = AI.readSettings();
  const provider = AI.providerFor(settings, feature);
  if (!settings.features[feature].enabled || !provider) {
    throw new AiError({ key: 'not-configured', message: `no model is set up for ${feature}` });
  }
  if (!AI.accountEnabled(settings, accountId)) {
    throw new AiError({ key: 'not-enabled', message: 'AI is off for this account' });
  }
  return {
    accountId,
    threadId,
    ...(await providerParams(provider)),
    settings: AI.requestSettings(settings),
  };
}

export function summarize(
  accountId: string,
  threadId: string,
  { refresh = false, onDelta }: { refresh?: boolean; onDelta?: (text: string) => void } = {}
): AiCall<AiCompletion> {
  return start<AiCompletion>(
    'ai.summarize',
    async () => ({ ...(await threadParams('summary', accountId, threadId)), refresh }),
    onDelta
  );
}

export function suggestReplies(
  accountId: string,
  threadId: string,
  count = 3
): AiCall<AiSuggestion[]> {
  return start<AiSuggestion[]>('ai.suggestReplies', async () => ({
    ...(await threadParams('reply', accountId, threadId)),
    count,
  }));
}

export function draftReply(
  accountId: string,
  threadId: string,
  {
    instruction,
    suggestion,
    onDelta,
  }: { instruction?: string; suggestion?: string; onDelta?: (text: string) => void }
): AiCall<AiCompletion> {
  return start<AiCompletion>(
    'ai.draftReply',
    async () => {
      const params: any = await threadParams('reply', accountId, threadId);
      if (instruction && instruction.trim()) params.instruction = instruction.trim();
      if (suggestion) params.suggestion = suggestion;
      return params;
    },
    onDelta
  );
}

/** `ai.models` for a provider as the form shows it (the typed key wins). */
export async function listModels(provider: AiProvider, typedKey?: string) {
  requireEngine();
  try {
    const config = await configFor(provider, typedKey);
    return await Engine.request<AiModelInfo[]>('ai.models', { config });
  } catch (err) {
    throw toAiError(err);
  }
}

export async function testProvider(provider: AiProvider, typedKey?: string) {
  requireEngine();
  try {
    const config = await configFor(provider, typedKey);
    return await Engine.request<AiTestReport>('ai.test', { config });
  } catch (err) {
    throw toAiError(err);
  }
}

/**
 * "Sent to api.anthropic.com", "Sent to 192.168.1.20 (your network)" for a private address, or
 * "Runs on this computer (Ollama)" for loopback only.
 */
export function destinationNote(provider: AiProvider | null) {
  if (!provider) return '';
  if (AI.isLoopback(provider)) {
    return localized('Runs on this computer (%@)', AI.providerName(provider));
  }
  const host = AI.hostOf(provider) || AI.providerName(provider);
  if (AI.isLocal(provider)) return localized('Sent to %@ (your network)', host);
  return localized('Sent to %@', host);
}
