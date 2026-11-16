// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import { Engine, KeyManager } from 'mailspring-exports';
import {
  AiError,
  destinationNote,
  draftReply,
  errorMessage,
  forgetKey,
  hasSavedKey,
  listModels,
  moveKeysToEngine,
  saveKey,
  summarize,
  suggestReplies,
  testProvider,
} from '../lib/client';
import {
  ACCOUNT_ID,
  ENGINE_KEYS,
  SYNTHETIC_KEY,
  aiError,
  fakeEngine,
  fakeKeyset,
  secretAnswers,
  setUpAi,
  settle,
} from './fake-engine';

async function rejection(promise: Promise<unknown>): Promise<AiError> {
  try {
    await promise;
  } catch (err) {
    if (err instanceof AiError) return err;
    throw err;
  }
  throw new Error('expected a rejection');
}

describe('ai.* calls', () => {
  beforeEach(() => {
    fakeKeyset({ 'integration-ai-anthropic-key': SYNTHETIC_KEY });
  });

  it("streams only its own call's ai.delta", async () => {
    setUpAi();
    fakeEngine({
      'ai.summarize': (params, emitDelta) => {
        emitDelta('One, ');
        Engine.events.notification('ai.delta', {
          streamId: 'someone-else',
          text: 'not mine',
        });
        emitDelta('two.');
        return { text: 'One, two.', model: 'claude-test' };
      },
    });
    const deltas: string[] = [];
    const result = await summarize(ACCOUNT_ID, 't1', { onDelta: (t) => deltas.push(t) }).promise;
    expect(deltas).toEqual(['One, ', 'two.']);
    expect(result.text).toBe('One, two.');
  });

  it('reads no key and calls nothing for an account that is not opted in', async () => {
    setUpAi({ accounts: [] });
    const sent = fakeEngine({});
    const err = await rejection(suggestReplies(ACCOUNT_ID, 't1').promise);
    expect(err.key).toBe('not-enabled');
    expect(sent).toEqual([]);
    expect(KeyManager.getPassword).not.toHaveBeenCalled();
  });

  it('reads no key and calls nothing while the feature is off', async () => {
    setUpAi({ features: { summary: { enabled: false, provider: 'anthropic' } } });
    const sent = fakeEngine({});
    const err = await rejection(summarize(ACCOUNT_ID, 't1').promise);
    expect(err.key).toBe('not-configured');
    expect(sent).toEqual([]);
    expect(KeyManager.getPassword).not.toHaveBeenCalled();
  });

  it('waits for the engine', async () => {
    setUpAi();
    const sent = fakeEngine({}, { hasAi: false });
    const err = await rejection(summarize(ACCOUNT_ID, 't1').promise);
    expect(err.key).toBe('engine-disconnected');
    expect(sent).toEqual([]);
    expect(KeyManager.getPassword).not.toHaveBeenCalled();
  });

  it('turns the engine error into a short message', async () => {
    setUpAi();
    fakeEngine({
      'ai.draftReply': () => {
        throw aiError('rate-limited', 'slow down', { retryable: true, retryAfter: 20 });
      },
    });
    const err = await rejection(draftReply(ACCOUNT_ID, 't1', { instruction: 'no' }).promise);
    expect(err.key).toBe('rate-limited');
    expect(errorMessage(err, 'Anthropic')).toBe(
      'Anthropic is limiting requests. Try again in 20 seconds.'
    );
  });

  it("doesn't blame the provider for local failures", async () => {
    setUpAi();
    fakeEngine({
      'ai.summarize': () => {
        throw Object.assign(new Error('The sync engine connection closed'), { code: null });
      },
      'ai.draftReply': () => {
        throw Object.assign(new Error('busy'), { code: -32012 });
      },
    });
    const dropped = await rejection(summarize(ACCOUNT_ID, 't1').promise);
    expect(dropped.key).toBe('engine-disconnected');
    expect(errorMessage(dropped, 'Anthropic')).toBe('The sync engine disconnected. Try again.');

    const busy = await rejection(draftReply(ACCOUNT_ID, 't1', { instruction: 'x' }).promise);
    expect(busy.key).toBe('engine-busy');
    expect(errorMessage(busy, 'Anthropic')).toBe(
      'Airpost is busy with other AI requests. Try again in a moment.'
    );
  });

  it('passes the instruction or the picked suggestion to ai.draftReply', async () => {
    setUpAi();
    const sent = fakeEngine({ 'ai.draftReply': () => ({ text: 'Hi', model: 'm' }) });
    await draftReply(ACCOUNT_ID, 't1', { instruction: "  say I'll call Tuesday " }).promise;
    await draftReply(ACCOUNT_ID, 't1', { suggestion: 'Sounds good' }).promise;
    expect(sent[0].params.instruction).toBe("say I'll call Tuesday");
    expect(sent[0].params.suggestion).toBeUndefined();
    expect(sent[1].params.suggestion).toBe('Sounds good');
  });

  it('cancels through ai.cancel with the streamId', async () => {
    setUpAi();
    let release!: (v: unknown) => void;
    const sent = fakeEngine({
      'ai.summarize': () =>
        new Promise((resolve, reject) => {
          release = () => reject(aiError('cancelled'));
        }),
    });
    const call = summarize(ACCOUNT_ID, 't1');
    await settle(() => sent.length > 0);
    call.cancel();
    release(null);
    const err = await rejection(call.promise);
    expect(err.key).toBe('cancelled');
    expect(sent.map((s) => s.method)).toEqual(['ai.summarize', 'ai.cancel']);
    expect(sent[1].params).toEqual({ streamId: call.streamId });
  });

  it('says "this computer" for loopback only and names a host on the network', () => {
    const at = (url: string) => destinationNote({ id: 'o', kind: 'openai', url, model: 'm' });
    expect(at('http://127.0.0.1:1234/v1')).toBe('Runs on this computer (OpenAI-compatible)');
    expect(at('http://localhost:1234/v1')).toBe('Runs on this computer (OpenAI-compatible)');
    expect(at('http://[::1]:1234/v1')).toBe('Runs on this computer (OpenAI-compatible)');
    expect(at('http://192.168.1.20:1234/v1')).toBe('Sent to 192.168.1.20 (your network)');
    expect(at('http://10.0.0.5/v1')).toBe('Sent to 10.0.0.5 (your network)');
    expect(at('https://openrouter.ai/api/v1')).toBe('Sent to openrouter.ai');
    expect(destinationNote({ id: 'ollama', kind: 'ollama', model: 'llama3' })).toBe(
      'Runs on this computer (Ollama)'
    );
  });

  it('lists models and tests with the typed key over the saved one', async () => {
    const sent = fakeEngine({
      'ai.models': () => [{ id: 'llama3', name: 'llama3' }],
      'ai.test': () => ({ host: '127.0.0.1', model: 'llama3', local: true, latencyMs: 5 }),
    });
    const provider = { id: 'anthropic', kind: 'anthropic' as const, model: '' };
    expect(await listModels(provider, 'typed-synthetic')).toEqual([
      { id: 'llama3', name: 'llama3' },
    ]);
    expect(sent[0].params).toEqual({
      config: { id: 'anthropic', kind: 'anthropic', model: '', apiKey: 'typed-synthetic' },
    });
    // Without a typed key the engine adds the saved one.
    await testProvider(provider);
    expect(sent[1].params.config.apiKey).toBeUndefined();
  });
});

describe('ai.* calls to an engine that keeps the keys', () => {
  let keys: { [name: string]: string };
  let secrets: { [name: string]: string };

  beforeEach(() => {
    keys = fakeKeyset();
    secrets = {};
  });

  const engine = (answers = {}) =>
    fakeEngine({ ...secretAnswers(secrets), ...answers }, { features: ENGINE_KEYS });

  it('names the provider and sends no key', async () => {
    setUpAi();
    const sent = engine({ 'ai.summarize': () => ({ text: 'A summary.', model: 'claude-test' }) });
    await summarize(ACCOUNT_ID, 't1').promise;
    const { streamId, ...params } = sent[0].params;
    expect(params).toEqual({
      accountId: ACCOUNT_ID,
      threadId: 't1',
      refresh: false,
      provider: 'anthropic',
      settings: {
        accounts: [ACCOUNT_ID],
        features: { summary: true, reply: true, triage: false },
        includeAddresses: false,
      },
    });
    expect(KeyManager.getPassword).not.toHaveBeenCalled();
  });

  it("tests the form's config with the typed key, or none for the engine to add", async () => {
    const sent = engine({
      'ai.test': () => ({ host: 'h', model: 'm', local: false, latencyMs: 1 }),
    });
    const provider = { id: 'anthropic', kind: 'anthropic' as const, model: 'draft' };
    await testProvider(provider);
    await testProvider(provider, 'typed-synthetic');
    expect(sent[0].params).toEqual({
      config: { id: 'anthropic', kind: 'anthropic', model: 'draft' },
    });
    expect(sent[1].params.config.apiKey).toBe('typed-synthetic');
    expect(KeyManager.getPassword).not.toHaveBeenCalled();
  });

  it('saves, finds and forgets keys in the secret store, dropping keyset copies', async () => {
    engine();
    keys['integration-ai-anthropic-key'] = 'old-synthetic';
    await saveKey('anthropic', SYNTHETIC_KEY);
    expect(secrets).toEqual({ 'integration:ai-anthropic': SYNTHETIC_KEY });
    expect(keys).toEqual({});
    expect(KeyManager.replacePassword).not.toHaveBeenCalled();
    expect(await hasSavedKey('anthropic')).toBe(true);
    expect(await hasSavedKey('openai')).toBe(false);
    await forgetKey('anthropic');
    expect(secrets).toEqual({});
    expect(await hasSavedKey('anthropic')).toBe(false);
  });

  it('moves keys left in the keyset to the engine once it is connected', async () => {
    setUpAi({
      providers: [
        { id: 'anthropic', kind: 'anthropic', model: 'claude-test' },
        { id: 'ollama', kind: 'ollama', model: 'llama3' },
      ],
    });
    keys['integration-ai-anthropic-key'] = SYNTHETIC_KEY;
    fakeEngine(secretAnswers(secrets), { hasAi: false });
    await moveKeysToEngine();
    expect(secrets).toEqual({});
    expect(keys['integration-ai-anthropic-key']).toBe(SYNTHETIC_KEY);

    Engine.events.features = new Set(ENGINE_KEYS);
    await moveKeysToEngine();
    expect(secrets).toEqual({ 'integration:ai-anthropic': SYNTHETIC_KEY });
    expect(keys).toEqual({});
  });
});

describe('keys while the engine is down', () => {
  let keys: { [name: string]: string };
  let secrets: { [name: string]: string };

  beforeEach(() => {
    keys = fakeKeyset({ 'integration-ai-anthropic-key': SYNTHETIC_KEY });
    secrets = {};
  });

  it('forgets nowhere and knows no saved key while the engine is down', async () => {
    const sent = fakeEngine(secretAnswers(secrets), { hasAi: false });
    const forgot = await rejection(forgetKey('anthropic'));
    expect(forgot.key).toBe('engine-disconnected');
    expect((await rejection(hasSavedKey('anthropic'))).key).toBe('engine-disconnected');
    expect(keys['integration-ai-anthropic-key']).toBe(SYNTHETIC_KEY);
    expect(sent).toEqual([]);
  });
});
