// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { ReplySuggestionsPanel, SuggestReplyButton, isReplyDraft } from '../lib/reply-suggestions';
import { CapabilityStore } from 'mailspring-exports';
import { ReplySession } from '../lib/reply-insert';
import { ACCOUNT_ID, SYNTHETIC_KEY, fakeEngine, fakeKeyset, setUpAi, settle } from './fake-engine';

const QUOTE = '<blockquote class="gmail_quote"><p>Lunch?</p></blockquote>';
const draft = {
  accountId: ACCOUNT_ID,
  threadId: 't1',
  replyToHeaderMessageId: '<m1@x.test>',
  body: `<br/>${QUOTE}`,
  plaintext: false,
};

function fakeSession() {
  const added: { body: string }[] = [];
  return {
    added,
    session: { draft: () => draft, changes: { add: (c: { body: string }) => added.push(c) } },
  };
}

const SUGGESTIONS = [
  { label: 'Accept', text: 'Yes, lunch works.' },
  { label: 'Decline', text: "Sorry, I can't." },
  { label: 'Later', text: 'How about next week?' },
  { label: 'Extra', text: 'Never shown.' },
];

async function render(session: ReplySession, onInserted = () => {}) {
  let renderer!: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(
      <ReplySuggestionsPanel draft={draft} session={session} onInserted={onInserted} />
    );
    await settle();
  });
  return renderer;
}

async function getSuggestions(renderer: TestRenderer.ReactTestRenderer) {
  await act(async () => {
    renderer.root
      .find((n) => n.type === 'button' && n.props.children === 'Get Suggestions')
      .props.onClick();
    await settle();
  });
}

describe('Reply suggestions', () => {
  beforeEach(() => {
    fakeKeyset({ 'integration-ai-anthropic-key': SYNTHETIC_KEY });
    setUpAi();
  });

  it('is only for replies', () => {
    expect(isReplyDraft(draft)).toBe(true);
    expect(isReplyDraft({ ...draft, replyToHeaderMessageId: null })).toBe(false);
  });

  it('names the host and sends nothing until asked', async () => {
    const sent = fakeEngine({ 'ai.suggestReplies': () => SUGGESTIONS });
    const { session } = fakeSession();
    const renderer = await render(session);
    expect(sent).toEqual([]);
    const source = renderer.root.find((n) => n.props.className === 'ai-reply-source');
    expect(source.props.children).toBe('Sent to api.anthropic.com');
    await getSuggestions(renderer);
    expect(sent.map((s) => s.method)).toEqual(['ai.suggestReplies']);
    renderer.unmount();
  });

  it("names the host in the Suggest button's title", async () => {
    fakeEngine({});
    spyOn(CapabilityStore, 'get').andReturn({ available: true, source: 'ai' });
    const { session } = fakeSession();
    let renderer!: TestRenderer.ReactTestRenderer;
    await act(async () => {
      renderer = TestRenderer.create(
        <SuggestReplyButton draft={draft as any} session={session as any} />
      );
    });
    const suggest = renderer.root.find((n) => n.type === 'button');
    expect(suggest.props.title).toBe('Suggest a reply (Sent to api.anthropic.com)');
    expect(suggest.props['aria-label']).toBe(suggest.props.title);
    renderer.unmount();
  });

  it('shows up to 3 suggestions and streams the picked one into the body once', async () => {
    let emit: (t: string) => void;
    let finish: () => void;
    const sent = fakeEngine({
      'ai.suggestReplies': () => SUGGESTIONS,
      'ai.draftReply': (params, emitDelta) =>
        new Promise((resolve) => {
          emit = emitDelta;
          finish = () => resolve({ text: 'Yes, noon works.\n\nBob', model: 'm' });
        }),
    });
    const { added, session } = fakeSession();
    const onInserted = jasmine.createSpy('onInserted');
    const renderer = await render(session, onInserted);
    await getSuggestions(renderer);
    const picks = renderer.root.findAll((n) => n.props.className === 'btn ai-reply-suggestion');
    expect(picks.map((p) => p.props.children)).toEqual(['Accept', 'Decline', 'Later']);
    expect(sent[0].params.count).toBe(3);

    await act(async () => {
      picks[0].props.onClick();
      await settle(() => sent.length > 1);
    });
    expect(sent[1].method).toBe('ai.draftReply');
    expect(sent[1].params.suggestion).toBe('Yes, lunch works.');
    await act(async () => emit('Yes, '));
    const preview = renderer.root.find((n) => n.props.className === 'ai-reply-preview');
    expect(preview.props.children).toBe('Yes, ');
    expect(added).toEqual([]);

    await act(async () => {
      finish();
      await settle();
    });
    expect(added).toEqual([
      { body: `<div>Yes, noon works.</div><div><br/></div><div>Bob</div><div><br/></div>${QUOTE}` },
    ]);
    expect(onInserted).toHaveBeenCalled();
    renderer.unmount();
  });

  it('writes from the typed instruction', async () => {
    const sent = fakeEngine({
      'ai.suggestReplies': () => [],
      'ai.draftReply': () => ({ text: "I'll call you Tuesday.", model: 'm' }),
    });
    const { added, session } = fakeSession();
    const renderer = await render(session);
    const input = renderer.root.find((n) => n.props.className === 'ai-reply-instruction-input');
    await act(async () => {
      input.props.onChange({ target: { value: "say I'll call Tuesday" } });
    });
    await act(async () => {
      await renderer.root
        .find((n) => n.type === 'button' && n.props.children === 'Write')
        .props.onClick();
    });
    expect(sent.map((s) => s.method)).toEqual(['ai.draftReply']);
    expect(sent[0].params.instruction).toBe("say I'll call Tuesday");
    expect(added.length).toBe(1);
    expect(added[0].body).toContain('I&#39;ll call you Tuesday.');
    renderer.unmount();
  });

  it('cancels what is running when it closes', async () => {
    const sent = fakeEngine({ 'ai.suggestReplies': () => new Promise(() => {}) });
    const { session } = fakeSession();
    const renderer = await render(session);
    await getSuggestions(renderer);
    renderer.unmount();
    expect(sent.map((s) => s.method)).toEqual(['ai.suggestReplies', 'ai.cancel']);
    expect(sent[1].params.streamId).toBe(sent[0].params.streamId);
  });
});
