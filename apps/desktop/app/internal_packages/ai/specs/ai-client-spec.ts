// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { Engine, KeyManager } from 'mailspring-exports';
import {
  draftReply,
  errorMessage,
  listModels,
  summarize,
  suggestReplies,
  testProvider,
} from '../lib/ai-client';
import {
  ACCOUNT_ID,
  SYNTHETIC_KEY,
  aiError,
  fakeEngine,
  fakeKeyset,
  setUpAi,
  settle,
} from './fake-engine';

async function rejection(promise: Promise<unknown>) {
  try {
    await promise;
  } catch (err) {
    return err;
  }
  throw new Error('expected a rejection');
}

describe('ai.* calls', () => {
  beforeEach(() => {
    fakeKeyset({ 'integration-ai-anthropic-key': SYNTHETIC_KEY });
  });

  it('sends the provider config with its key and the switches', async () => {
    setUpAi();
    const sent = fakeEngine({
      'ai.summarize': () => ({ text: 'A summary.', model: 'claude-test', cached: false }),
    });
    const result = await summarize(ACCOUNT_ID, 't1').promise;
    expect(result.text).toBe('A summary.');
    expect(sent.length).toBe(1);
    expect(sent[0].method).toBe('ai.summarize');
    const { streamId, ...params } = sent[0].params;
    expect(typeof streamId).toBe('string');
    expect(params).toEqual({
      accountId: ACCOUNT_ID,
      threadId: 't1',
      refresh: false,
      config: { id: 'anthropic', kind: 'anthropic', model: 'claude-test', apiKey: SYNTHETIC_KEY },
      settings: {
        accounts: [ACCOUNT_ID],
        features: { summary: true, reply: true, triage: false },
        includeAddresses: false,
      },
    });
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

  it('waits for an engine that serves ai.*', async () => {
    setUpAi();
    const sent = fakeEngine({}, { hasAi: false });
    const err = await rejection(summarize(ACCOUNT_ID, 't1').promise);
    expect(err.key).toBe('engine-unavailable');
    expect(errorMessage(err)).toBe('Waiting for engine support.');
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
    let release: (v: unknown) => void;
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
    await testProvider(provider);
    expect(sent[1].params.config.apiKey).toBe(SYNTHETIC_KEY);
  });
});
