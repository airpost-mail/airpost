// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { CapabilityStoreClass } from '../../../src/flux/stores/capability-store';
import { EngineEvents } from '../../../src/engine-events';
import { EngineCapabilities } from '../../../src/engine-protocol';
import { SYNTHETIC_KEY, setUpAi } from './fake-engine';

describe('AI features in the capability store', () => {
  let store: CapabilityStoreClass;
  let events: EngineEvents;
  let keys: { [name: string]: string };

  beforeEach(() => {
    keys = {};
    events = new EngineEvents();
    store = new CapabilityStoreClass({ autoStart: false });
    store.engineEvents = events;
    store.keySource = { getPassword: async (name: string) => keys[name] };
    store.start();
  });

  afterEach(() => store.stop());

  const connect = (caps: EngineCapabilities = null) =>
    events.connected(['ai', 'capabilities'], async () => ({}), caps);

  it('asks for a model until one is set up', () => {
    connect();
    expect(store.get('ai-summary')).toEqual({
      available: false,
      source: 'ai',
      reason: 'Set up a model in Preferences > AI',
    });
    expect(store.unavailableMessage('ai-reply')).toBe(
      'Reply suggestions: Set up a model in Preferences > AI'
    );
  });

  it('asks for the API key of a provider that needs one', async () => {
    connect();
    setUpAi();
    await store.whenReady();
    expect(store.get('ai-summary').reason).toBe('Add your Anthropic API key in Preferences > AI');
  });

  it('is available with a saved key on an engine that serves ai.*', async () => {
    keys['integration-ai-anthropic-key'] = SYNTHETIC_KEY;
    connect({ ai: { summary: { available: true, credentials: 'request' } } });
    setUpAi();
    await store.whenReady();
    expect(store.get('ai-summary')).toEqual({
      available: true,
      source: 'ai',
      host: 'api.anthropic.com',
      local: false,
    });
  });

  it('needs no key for Ollama, and says it runs locally', async () => {
    connect();
    setUpAi({
      providers: [{ id: 'ollama', kind: 'ollama', model: 'llama3' }],
      features: { reply: { enabled: true, provider: 'ollama' } },
    });
    await store.whenReady();
    expect(store.get('ai-reply')).toEqual({
      available: true,
      source: 'ai',
      host: '127.0.0.1',
      local: true,
    });
  });

  it('waits for an engine without ai', async () => {
    keys['integration-ai-anthropic-key'] = SYNTHETIC_KEY;
    events.connected(['capabilities'], async () => ({}), null);
    setUpAi();
    await store.whenReady();
    expect(store.get('ai-summary').reason).toBe('Waiting for engine support');
  });

  it('needs an engine that triages for inbox triage', async () => {
    connect();
    setUpAi({
      providers: [{ id: 'ollama', kind: 'ollama', model: 'llama3' }],
      features: { triage: { enabled: true, provider: 'ollama' } },
    });
    await store.whenReady();
    expect(store.get('ai-triage').reason).toBe('Waiting for engine support');
    events.connected(['ai', 'ai-triage', 'capabilities'], async () => ({}), null);
    expect(store.get('ai-triage').available).toBe(true);
  });

  it("says why the engine's triage waits", async () => {
    keys['integration-ai-anthropic-key'] = SYNTHETIC_KEY;
    setUpAi({ features: { triage: { enabled: true, provider: 'anthropic' } } });
    const reason = (r: string) => {
      events.connected(['ai', 'ai-triage', 'capabilities'], async () => ({}), {
        ai: { triage: { available: false, reason: r } },
      });
      return store.unavailableMessage('ai-triage');
    };
    await store.whenReady();
    expect(reason('needs-engine-key')).toBe(
      'Inbox triage: Background triage needs a model without an API key, such as Ollama, for now'
    );
    expect(reason('rate-limited')).toContain('limiting requests');
    expect(reason('quota-exceeded')).toContain('used up');
  });

  it("takes the engine's word when it reports a feature unavailable", async () => {
    keys['integration-ai-anthropic-key'] = SYNTHETIC_KEY;
    connect({ ai: { summary: { available: false, reason: 'not-set-up' } } });
    setUpAi();
    await store.whenReady();
    expect(store.get('ai-summary').available).toBe(false);
    expect(store.get('ai-summary').reason).toBe('Set up a model in Preferences > AI');
  });
});
