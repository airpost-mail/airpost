// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { AccountStore, KeyManager } from 'mailspring-exports';
import { ProviderCard } from 'mailspring-component-kit';
import PreferencesAI, { ProviderSection } from '../lib/preferences-ai';
import {
  ACCOUNT_ID,
  ENGINE_KEYS,
  SYNTHETIC_KEY,
  fakeEngine,
  fakeKeyset,
  secretAnswers,
  settle,
} from './fake-engine';

async function render() {
  let renderer: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(<PreferencesAI />);
    await settle();
  });
  return renderer;
}

function byId(renderer: TestRenderer.ReactTestRenderer, id: string) {
  return renderer.root.find((node) => node.props.id === id);
}

function button(renderer: TestRenderer.ReactTestRenderer, label: string) {
  return renderer.root.find((n) => n.type === 'button' && n.props.children === label);
}

function toggle(renderer: TestRenderer.ReactTestRenderer, label: string) {
  return renderer.root
    .find((n) => n.type === 'div' && n.props.role === 'switch' && n.props['aria-label'] === label)
    .props.onClick();
}

function accountName(accountId: string) {
  const account = AccountStore.accountForId(accountId);
  return account.label && account.label !== account.emailAddress
    ? account.label
    : account.emailAddress;
}

/** Opens a saved provider's card, which starts closed once it has a model. */
async function openProvider(renderer: TestRenderer.ReactTestRenderer, id: string) {
  const section = renderer.root.find(
    (n) => n.type === ProviderSection && n.props.provider.id === id
  );
  await act(async () => {
    section.findByType(ProviderCard).props.onToggle(true);
  });
}

async function type(renderer: TestRenderer.ReactTestRenderer, id: string, value: string) {
  await act(async () => {
    byId(renderer, id).props.onChange({ target: { value } });
  });
  await act(async () => {
    await byId(renderer, id).props.onBlur?.();
  });
}

function textOf(renderer: TestRenderer.ReactTestRenderer) {
  return JSON.stringify(renderer.toJSON());
}

describe('Preferences > AI', () => {
  let keys: { [name: string]: string };

  beforeEach(() => {
    keys = fakeKeyset();
  });

  async function addAnthropic(renderer: TestRenderer.ReactTestRenderer) {
    await act(async () => {
      byId(renderer, 'ai-add-provider').props.onChange({ target: { value: 'anthropic' } });
      await settle();
    });
  }

  it('adds a provider for every feature and keeps its key in the keyset only', async () => {
    fakeEngine({});
    const renderer = await render();
    await addAnthropic(renderer);
    await type(renderer, 'ai-provider-anthropic-model', 'claude-test');
    await act(async () => {
      byId(renderer, 'ai-provider-anthropic-key').props.onChange({
        target: { value: SYNTHETIC_KEY },
      });
    });
    await act(async () => {
      await button(renderer, 'Save key').props.onClick();
    });

    expect(KeyManager.replacePassword).toHaveBeenCalledWith(
      'integration-ai-anthropic-key',
      SYNTHETIC_KEY
    );
    expect(keys['integration-ai-anthropic-key']).toBe(SYNTHETIC_KEY);
    expect(AppEnv.config.get('core.ai')).toEqual({
      providers: [{ id: 'anthropic', kind: 'anthropic', model: 'claude-test' }],
      features: {
        summary: { enabled: false, provider: 'anthropic' },
        reply: { enabled: false, provider: 'anthropic' },
        triage: { enabled: false, provider: 'anthropic' },
      },
      accounts: [],
      includeAddresses: false,
    });
    expect(JSON.stringify(AppEnv.config.get('core'))).not.toContain(SYNTHETIC_KEY);
    expect(byId(renderer, 'ai-provider-anthropic-key').props.value).toBe('');
    expect(textOf(renderer)).not.toContain(SYNTHETIC_KEY);
    expect(textOf(renderer)).toContain('go to api.anthropic.com when you use a feature');
    renderer.unmount();
  });

  it('turns features and accounts on, and includes addresses only when asked', async () => {
    fakeEngine({});
    const renderer = await render();
    await addAnthropic(renderer);
    await act(async () => toggle(renderer, 'Thread summaries'));
    await act(async () => toggle(renderer, accountName(ACCOUNT_ID)));
    const settings = AppEnv.config.get('core.ai');
    expect(settings.features.summary.enabled).toBe(true);
    expect(settings.features.reply.enabled).toBe(false);
    expect(settings.accounts).toEqual([ACCOUNT_ID]);
    expect(settings.includeAddresses).toBe(false);
    await act(async () => toggle(renderer, 'Include email addresses'));
    expect(AppEnv.config.get('core.ai').includeAddresses).toBe(true);
    renderer.unmount();
  });

  it('picks a model from ai.models and tests with the typed key', async () => {
    const sent = fakeEngine({
      'ai.models': () => [
        { id: 'claude-a', name: 'Claude A' },
        { id: 'claude-b', name: 'claude-b' },
      ],
      'ai.test': () => ({
        host: 'api.anthropic.com',
        model: 'claude-b',
        local: false,
        latencyMs: 42,
      }),
    });
    const renderer = await render();
    await addAnthropic(renderer);
    await act(async () => {
      byId(renderer, 'ai-provider-anthropic-key').props.onChange({
        target: { value: 'typed-synthetic' },
      });
    });
    await act(async () => {
      await button(renderer, 'Load models').props.onClick();
    });
    expect(sent[0]).toEqual({
      method: 'ai.models',
      params: {
        config: { id: 'anthropic', kind: 'anthropic', model: '', apiKey: 'typed-synthetic' },
      },
    });
    await act(async () => {
      byId(renderer, 'ai-provider-anthropic-model-picker').props.onChange({
        target: { value: 'claude-b' },
      });
      await settle();
    });
    expect(AppEnv.config.get('core.ai').providers[0].model).toBe('claude-b');
    await act(async () => {
      await button(renderer, 'Test').props.onClick();
    });
    expect(sent[1].params.config.model).toBe('claude-b');
    expect(textOf(renderer)).toContain('claude-b answered from api.anthropic.com in 42 ms.');
    // Testing doesn't save the typed key.
    expect(keys['integration-ai-anthropic-key']).toBeUndefined();
    renderer.unmount();
  });

  it("removes the saved key when the provider's server changes", async () => {
    keys['integration-ai-openai-key'] = SYNTHETIC_KEY;
    const sent = fakeEngine({
      'ai.test': () => ({ host: '192.168.1.20', model: 'm', local: true, latencyMs: 7 }),
    });
    const renderer = await render();
    await act(async () => {
      byId(renderer, 'ai-add-provider').props.onChange({ target: { value: 'openai' } });
      await settle();
    });
    // Same server, written out: the key stays.
    await type(renderer, 'ai-provider-openai-url', 'https://api.openai.com/v1/');
    expect(keys['integration-ai-openai-key']).toBe(SYNTHETIC_KEY);

    await type(renderer, 'ai-provider-openai-url', 'http://192.168.1.20:1234/v1');
    expect(keys['integration-ai-openai-key']).toBeUndefined();
    expect(AppEnv.config.get('core.ai').providers[0].url).toBe('http://192.168.1.20:1234/v1');
    expect(textOf(renderer)).toContain('The server changed, so the saved API key was removed.');
    expect(textOf(renderer)).toContain('go to 192.168.1.20 on your network');

    await act(async () => {
      await button(renderer, 'Test').props.onClick();
    });
    expect(sent[0].params.config.apiKey).toBeUndefined();
    expect(textOf(renderer)).toContain('m answered from 192.168.1.20 in 7 ms.');
    renderer.unmount();
  });

  it('removes a provider with its key and its place in the features', async () => {
    keys['integration-ai-anthropic-key'] = SYNTHETIC_KEY;
    fakeEngine({});
    const renderer = await render();
    await addAnthropic(renderer);
    await act(async () => {
      await button(renderer, 'Remove').props.onClick();
    });
    expect(keys['integration-ai-anthropic-key']).toBeUndefined();
    const settings = AppEnv.config.get('core.ai');
    expect(settings.providers).toEqual([]);
    expect(settings.features.summary).toEqual({ enabled: false });
    renderer.unmount();
  });

  it('shows what is sent', async () => {
    fakeEngine({});
    const renderer = await render();
    expect(textOf(renderer)).not.toContain('When you click Summarize');
    await act(async () => {
      renderer.root
        .find((n) => n.type === 'button' && n.props.className === 'ai-disclosure')
        .props.onClick();
    });
    expect(textOf(renderer)).toContain('When you click Summarize');
    expect(textOf(renderer)).toContain('Attachments are never sent');
    renderer.unmount();
  });
});

describe('Preferences > AI with an engine that keeps the keys', () => {
  let keys: { [name: string]: string };
  let secrets: { [name: string]: string };

  beforeEach(() => {
    keys = fakeKeyset();
    secrets = {};
  });

  const engine = (answers = {}) =>
    fakeEngine({ ...secretAnswers(secrets), ...answers }, { features: ENGINE_KEYS });

  function withProvider(kind: 'anthropic' | 'openai') {
    AppEnv.config.set('core.ai', {
      providers: [{ id: kind, kind, model: 'm' }],
      features: {},
      accounts: [],
      includeAddresses: false,
    });
  }

  it('saves and forgets the key with secrets.set and secrets.delete', async () => {
    withProvider('anthropic');
    const sent = engine();
    const renderer = await render();
    await openProvider(renderer, 'anthropic');
    await act(async () => {
      byId(renderer, 'ai-provider-anthropic-key').props.onChange({
        target: { value: SYNTHETIC_KEY },
      });
    });
    await act(async () => {
      await button(renderer, 'Save key').props.onClick();
    });
    expect(secrets).toEqual({ 'integration:ai-anthropic': SYNTHETIC_KEY });
    expect(KeyManager.replacePassword).not.toHaveBeenCalled();
    expect(keys).toEqual({});
    expect(textOf(renderer)).not.toContain(SYNTHETIC_KEY);

    await act(async () => {
      await button(renderer, 'Forget key').props.onClick();
    });
    expect(secrets).toEqual({});
    expect(sent.map((s) => s.method)).toContain('secrets.delete');
    // ai.usage reads the counters and carries no key.
    expect(sent.some((s) => s.method.startsWith('ai.') && s.method !== 'ai.usage')).toBe(false);
    renderer.unmount();
  });

  it('shows a key the engine holds as saved', async () => {
    withProvider('anthropic');
    secrets['integration:ai-anthropic'] = SYNTHETIC_KEY;
    engine();
    const renderer = await render();
    await openProvider(renderer, 'anthropic');
    expect(byId(renderer, 'ai-provider-anthropic-key').props.placeholder).toBe(
      'Saved. Type a new key to replace it.'
    );
    renderer.unmount();
  });

  it("deletes the engine's key when the server changes or the provider goes", async () => {
    withProvider('openai');
    secrets['integration:ai-openai'] = SYNTHETIC_KEY;
    engine();
    const renderer = await render();
    await openProvider(renderer, 'openai');
    await type(renderer, 'ai-provider-openai-url', 'http://192.168.1.20:1234/v1');
    expect(secrets).toEqual({});
    expect(textOf(renderer)).toContain('The server changed, so the saved API key was removed.');

    secrets['integration:ai-openai'] = SYNTHETIC_KEY;
    await act(async () => {
      await button(renderer, 'Remove').props.onClick();
    });
    expect(secrets).toEqual({});
    expect(AppEnv.config.get('core.ai').providers).toEqual([]);
    renderer.unmount();
  });

  it('keeps the server and the provider while the engine is down', async () => {
    withProvider('openai');
    secrets['integration:ai-openai'] = SYNTHETIC_KEY;
    fakeEngine(secretAnswers(secrets), { hasAi: false });
    spyOn(AppEnv, 'showErrorDialog');
    const renderer = await render();
    await openProvider(renderer, 'openai');
    await type(renderer, 'ai-provider-openai-url', 'http://192.168.1.20:1234/v1');
    expect(AppEnv.config.get('core.ai').providers[0].url).toBeUndefined();
    expect(textOf(renderer)).toContain('The sync engine is not connected.');
    await act(async () => {
      await button(renderer, 'Remove').props.onClick();
    });
    expect(AppEnv.showErrorDialog).toHaveBeenCalled();
    expect(AppEnv.config.get('core.ai').providers.length).toBe(1);
    expect(secrets).toEqual({ 'integration:ai-openai': SYNTHETIC_KEY });
    renderer.unmount();
  });

  it("shows today's usage and the provider's daily cap", async () => {
    withProvider('anthropic');
    AppEnv.config.set('core.ai.limits', { dailyTokens: { anthropic: 50000 } });
    const pad = (n: number) => `${n}`.padStart(2, '0');
    const now = new Date();
    const today = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
    engine({
      'ai.usage': () => [
        {
          day: today,
          provider: 'anthropic',
          feature: 'summary',
          inputTokens: 1000,
          outputTokens: 200,
          requests: 3,
        },
        {
          day: '2000-01-01',
          provider: 'anthropic',
          feature: 'summary',
          inputTokens: 9,
          outputTokens: 9,
          requests: 9,
        },
      ],
    });
    const renderer = await render();
    expect(textOf(renderer)).toContain('3 requests · 1,200 tokens');
    expect(textOf(renderer)).toContain('Anthropic: 1,200 of 50,000 tokens a day.');
    renderer.unmount();
  });

  it('keeps the provider when the engine cannot forget its key', async () => {
    withProvider('anthropic');
    secrets['integration:ai-anthropic'] = SYNTHETIC_KEY;
    engine({
      'secrets.delete': () => {
        throw new Error('The sync engine connection closed');
      },
    });
    spyOn(AppEnv, 'showErrorDialog');
    const renderer = await render();
    await openProvider(renderer, 'anthropic');
    await act(async () => {
      await button(renderer, 'Remove').props.onClick();
    });
    expect(AppEnv.showErrorDialog).toHaveBeenCalled();
    expect(AppEnv.config.get('core.ai').providers.length).toBe(1);
    renderer.unmount();
  });
});
