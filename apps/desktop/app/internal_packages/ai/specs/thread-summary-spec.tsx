// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { CapabilityStore, Message, Thread } from 'mailspring-exports';
import { ThreadSummary, forgetSummaries, worthSummarizing } from '../lib/thread-summary';
import {
  ACCOUNT_ID,
  SYNTHETIC_KEY,
  aiError,
  fakeEngine,
  fakeKeyset,
  setUpAi,
  settle,
} from './fake-engine';

const thread = new Thread({ id: 't1', accountId: ACCOUNT_ID, subject: 'Lunch' });
const messages = [
  new Message({ id: 'm1', accountId: ACCOUNT_ID, threadId: 't1', body: 'Lunch?' }),
  new Message({ id: 'm2', accountId: ACCOUNT_ID, threadId: 't1', body: 'Sure' }),
];

async function render(props = { thread, messages }) {
  let renderer: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(<ThreadSummary {...props} />);
  });
  return renderer;
}

function button(renderer: TestRenderer.ReactTestRenderer, label: string) {
  return renderer.root.find((n) => n.type === 'button' && n.props.children === label);
}

function textOf(renderer: TestRenderer.ReactTestRenderer) {
  return JSON.stringify(renderer.toJSON());
}

describe('The thread summary', () => {
  beforeEach(() => {
    forgetSummaries();
    fakeKeyset({ 'integration-ai-anthropic-key': SYNTHETIC_KEY });
    spyOn(CapabilityStore, 'get').andReturn({ available: true, source: 'ai' });
  });

  it('shows nothing while summaries are off', async () => {
    fakeEngine({});
    const renderer = await render();
    expect(renderer.toJSON()).toBe(null);
    renderer.unmount();
  });

  it('is offered for two messages or one long one', () => {
    expect(worthSummarizing(messages)).toBe(true);
    expect(worthSummarizing([messages[0]])).toBe(false);
    const long = new Message({ id: 'm3', body: 'x'.repeat(5000) });
    expect(worthSummarizing([long])).toBe(true);
    const draft = new Message({ id: 'm4', draft: true, body: 'y' });
    expect(worthSummarizing([messages[0], draft])).toBe(false);
  });

  it('streams the summary into a card with the model and host, as plain text', async () => {
    setUpAi();
    let finish: () => void;
    let emit: (text: string) => void;
    const sent = fakeEngine({
      'ai.summarize': (params, emitDelta) =>
        new Promise((resolve) => {
          emit = emitDelta;
          finish = () =>
            resolve({ text: 'Ann asks <a href="https://x.test">here</a>.', model: 'claude-test' });
        }),
    });
    const renderer = await render();
    await act(async () => {
      button(renderer, 'Summarize').props.onClick();
      await settle(() => sent.length > 0);
    });
    await act(async () => emit('Ann asks '));
    expect(textOf(renderer)).toContain('Ann asks ');
    await act(async () => {
      finish();
      await settle();
    });
    const text = renderer.root.find((n) => n.props.className === 'ai-summary-text');
    expect(text.props.children).toBe('Ann asks <a href="https://x.test">here</a>.');
    expect(renderer.root.findAll((n) => n.type === 'a')).toEqual([]);
    expect(textOf(renderer)).toContain('claude-test · Sent to api.anthropic.com');
    renderer.unmount();
  });

  it('regenerates with refresh, and shows the summary again when the thread reopens', async () => {
    setUpAi();
    const sent = fakeEngine({
      'ai.summarize': (params) => ({ text: params.refresh ? 'Fresh.' : 'First.', model: 'm' }),
    });
    const renderer = await render();
    await act(async () => {
      await button(renderer, 'Summarize').props.onClick();
    });
    await act(async () => {
      await button(renderer, 'Regenerate').props.onClick();
    });
    expect(sent.map((s) => s.params.refresh)).toEqual([false, true]);
    renderer.unmount();

    const again = await render();
    expect(textOf(again)).toContain('Fresh.');
    expect(sent.length).toBe(2);
    again.unmount();
  });

  it('shows the error in the card', async () => {
    setUpAi();
    fakeEngine({
      'ai.summarize': () => {
        throw aiError('auth', 'bad key');
      },
    });
    const renderer = await render();
    await act(async () => {
      await button(renderer, 'Summarize').props.onClick();
    });
    expect(textOf(renderer)).toContain('Anthropic rejected the API key.');
    renderer.unmount();
  });

  it('is disabled for an account that is not opted in', async () => {
    setUpAi({ accounts: [] });
    fakeEngine({});
    const renderer = await render();
    const summarize = button(renderer, 'Summarize');
    expect(summarize.props.disabled).toBe(true);
    expect(summarize.props.title).toBe('Turn on AI for this account in Preferences > AI');
    renderer.unmount();
  });

  it('cancels the call when the thread is closed mid-stream', async () => {
    setUpAi();
    const sent = fakeEngine({ 'ai.summarize': () => new Promise(() => {}) });
    const renderer = await render();
    await act(async () => {
      button(renderer, 'Summarize').props.onClick();
      await settle(() => sent.length > 0);
    });
    renderer.unmount();
    expect(sent.map((s) => s.method)).toEqual(['ai.summarize', 'ai.cancel']);
  });
});
