// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { KeyManager } from 'mailspring-exports';
import PreferencesAI from '../lib/preferences-ai';
import { ACCOUNT_ID, SYNTHETIC_KEY, fakeEngine, fakeKeyset, settle } from './fake-engine';

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
      await button(renderer, 'Save Key').props.onClick();
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
    expect(textOf(renderer)).toContain('Sent to api.anthropic.com');
    renderer.unmount();
  });

  it('turns features and accounts on, and includes addresses only when asked', async () => {
    fakeEngine({});
    const renderer = await render();
    await addAnthropic(renderer);
    await act(async () => byId(renderer, 'ai-feature-summary').props.onChange());
    await act(async () => byId(renderer, `ai-account-${ACCOUNT_ID}`).props.onChange());
    const settings = AppEnv.config.get('core.ai');
    expect(settings.features.summary.enabled).toBe(true);
    expect(settings.features.reply.enabled).toBe(false);
    expect(settings.accounts).toEqual([ACCOUNT_ID]);
    expect(settings.includeAddresses).toBe(false);
    await act(async () => byId(renderer, 'ai-include-addresses').props.onChange());
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
      await button(renderer, 'Load Models').props.onClick();
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
    expect(textOf(renderer)).toContain('Sent to 192.168.1.20 (your network)');

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
    expect(textOf(renderer)).toContain('When you click Summarize');
    expect(textOf(renderer)).toContain('Attachments are never sent');
    renderer.unmount();
  });
});
