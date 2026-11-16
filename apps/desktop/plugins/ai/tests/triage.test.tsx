// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import React from 'react';
import TestRenderer, { act } from 'react-test-renderer';
import { Actions, Engine, SyncbackMetadataTask, Thread } from 'mailspring-exports';
import {
  PLUGIN_ID,
  TRIAGE_SINCE_DAYS,
  chipCategory,
  correctCategory,
  triageInbox,
  triageOf,
} from '../lib/triage';
import { TriageChip, CorrectionMenu } from '../lib/triage-chip';
import { NeedsReplySidebarExtension, NeedsReplySubscription } from '../lib/needs-reply';
import { ACCOUNT_ID, SYNTHETIC_KEY, fakeEngine, fakeKeyset, setUpAi, settle } from './fake-engine';

const TRIAGE_FEATURES = ['ai', 'ai-triage', 'ai-keys'];

function triaged(value: object | null, id = 't1') {
  const thread = new Thread({ id, accountId: ACCOUNT_ID, subject: 'Lunch' });
  if (value) thread.directlyAttachMetadata(PLUGIN_ID, value);
  return thread;
}

function setUpTriage(overrides: object = {}) {
  setUpAi({
    providers: [{ id: 'anthropic', kind: 'anthropic', model: 'claude-test' }],
    features: { triage: { enabled: true, provider: 'anthropic' } },
    ...overrides,
  });
}

describe('AI triage', () => {
  describe('the value', () => {
    it('is read only with a known category', () => {
      expect(triageOf(triaged({ category: 'important' }))).toEqual({ category: 'important' });
      expect(triageOf(triaged({ category: 'spam?' }))).toBe(null);
      expect(triageOf(triaged(null))).toBe(null);
    });

    it('shows a chip for needs-reply and important only', () => {
      expect(chipCategory({ category: 'needs-reply' })).toBe('needs-reply');
      expect(chipCategory({ category: 'newsletter', needsReply: true })).toBe('needs-reply');
      expect(chipCategory({ category: 'important' })).toBe('important');
      expect(chipCategory({ category: 'promotion' })).toBe(null);
      expect(chipCategory(null)).toBe(null);
    });

    it('drops the needs-reply chip once the user replied', () => {
      const at = (secs: number) => new Date(secs * 1000);
      const thread = triaged(null);
      thread.lastMessageReceivedTimestamp = at(100);
      thread.lastMessageSentTimestamp = at(200);
      expect(chipCategory({ category: 'needs-reply' }, thread)).toBe(null);
      expect(chipCategory({ category: 'important', needsReply: true }, thread)).toBe('important');
      // Filed by the user after replying: it stands.
      const filed = { category: 'needs-reply' as const, source: 'user' as const, at: 300 };
      expect(chipCategory(filed, thread)).toBe('needs-reply');
      expect(chipCategory({ ...filed, at: 150 }, thread)).toBe(null);
      // New mail after the reply.
      thread.lastMessageReceivedTimestamp = at(250);
      expect(chipCategory({ category: 'needs-reply' }, thread)).toBe('needs-reply');
    });

    it("files a correction as the user's, which triage never replaces", () => {
      const queueTask = spyOn(Actions, 'queueTask');
      const thread = triaged({
        category: 'needs-reply',
        needsReply: true,
        reason: 'Ann asks about lunch.',
        messageId: 'm1',
        v: 1,
      });
      correctCategory(thread, 'newsletter');
      const task = queueTask.calls[0].args[0] as SyncbackMetadataTask;
      expect(task instanceof SyncbackMetadataTask).toBe(true);
      expect(task.pluginId).toBe(PLUGIN_ID);
      expect(task.modelId).toBe('t1');
      expect(task.value.category).toBe('newsletter');
      expect(task.value.needsReply).toBe(false);
      expect(task.value.source).toBe('user');
      expect(task.value.messageId).toBe('m1');
      expect(task.value.reason).toBeUndefined();
    });
  });

  describe('the chip', () => {
    it('renders the category and opens the correction menu', async () => {
      const openPopover = spyOn(Actions, 'openPopover');
      let renderer!: TestRenderer.ReactTestRenderer;
      await act(async () => {
        renderer = TestRenderer.create(<TriageChip thread={triaged({ category: 'important' })} />);
      });
      expect(JSON.stringify(renderer.toJSON())).toContain('Important');
      const chip = renderer.root.find((n) => n.type === 'span' && n.props.onClick);
      chip.props.onClick({
        preventDefault() {},
        stopPropagation() {},
        currentTarget: { getBoundingClientRect: () => ({}) },
      });
      const [menu] = openPopover.calls[0].args;
      expect(menu.type).toBe(CorrectionMenu);
      renderer.unmount();
    });

    it('is not shown for a quiet category', async () => {
      let renderer!: TestRenderer.ReactTestRenderer;
      await act(async () => {
        renderer = TestRenderer.create(
          <TriageChip thread={triaged({ category: 'notification' })} />
        );
      });
      expect(renderer.toJSON()).toBe(null);
      renderer.unmount();
    });
  });

  describe('"Triage inbox"', () => {
    beforeEach(() => fakeKeyset({ 'integration-ai-anthropic-key': SYNTHETIC_KEY }));

    it('runs a job over the last week naming the provider and reports its progress', async () => {
      setUpTriage();
      const sent = fakeEngine(
        {
          'ai.triage.run': () => {
            // The first progress line can come before the reply's continuation runs.
            Engine.events.notification('ai.triage.progress', { jobId: 'j1', done: 1, total: 2 });
            return { jobId: 'j1', queued: 2 };
          },
        },
        { features: TRIAGE_FEATURES }
      );
      const progress: Array<[number, number]> = [];
      const run = triageInbox(ACCOUNT_ID, (done, total) => progress.push([done, total]));
      expect(await run.started).toEqual({ jobId: 'j1', queued: 2 });
      // Another job's progress is not this one's.
      Engine.events.notification('ai.triage.progress', { jobId: 'j0', done: 5, total: 5 });
      Engine.events.notification('ai.triage.progress', { jobId: 'j1', done: 2, total: 2 });
      await run.finished;
      expect(progress).toEqual([
        [1, 2],
        [2, 2],
      ]);
      const [call] = sent.filter((s) => s.method === 'ai.triage.run');
      expect(call.params.accountId).toBe(ACCOUNT_ID);
      expect(call.params).toEqual({
        accountId: ACCOUNT_ID,
        sinceDays: TRIAGE_SINCE_DAYS,
        provider: 'anthropic',
      });
    });

    it('cancels its job in the engine', async () => {
      setUpTriage();
      const sent = fakeEngine(
        { 'ai.triage.run': () => ({ jobId: 'j2', queued: 3 }) },
        { features: TRIAGE_FEATURES }
      );
      const run = triageInbox(ACCOUNT_ID);
      await run.started;
      run.cancel();
      let key = null;
      await run.finished.catch((e) => (key = e.key));
      expect(key).toBe('cancelled');
      expect(sent.find((s) => s.method === 'ai.cancel')!.params).toEqual({ jobId: 'j2' });
    });

    it('fails when the engine stops the job early', async () => {
      setUpTriage();
      fakeEngine(
        { 'ai.triage.run': () => ({ jobId: 'j3', queued: 3 }) },
        { features: TRIAGE_FEATURES }
      );
      const progress: Array<[number, number]> = [];
      const run = triageInbox(ACCOUNT_ID, (done, total) => progress.push([done, total]));
      await run.started;
      Engine.events.notification('ai.triage.progress', { jobId: 'j3', done: 1, total: 3 });
      Engine.events.notification('ai.triage.progress', {
        jobId: 'j3',
        done: 1,
        total: 3,
        stopped: true,
      });
      let key = null;
      await run.finished.catch((e) => (key = e.key));
      expect(key).toBe('stopped');
      expect(progress).toEqual([[1, 3]]);
    });

    it('fails when the engine goes away', async () => {
      setUpTriage();
      fakeEngine(
        { 'ai.triage.run': () => ({ jobId: 'j4', queued: 3 }) },
        { features: TRIAGE_FEATURES }
      );
      const run = triageInbox(ACCOUNT_ID);
      await run.started;
      Engine.events.emit('gone');
      let key = null;
      await run.finished.catch((e) => (key = e.key));
      expect(key).toBe('engine-disconnected');
    });

    it('cancels a job stopped before the engine answered', async () => {
      setUpTriage();
      const sent = fakeEngine(
        { 'ai.triage.run': () => ({ jobId: 'j5', queued: 3 }) },
        { features: TRIAGE_FEATURES }
      );
      const run = triageInbox(ACCOUNT_ID);
      run.cancel();
      await run.started;
      expect(sent.find((s) => s.method === 'ai.cancel')!.params).toEqual({ jobId: 'j5' });
    });

    it('stops listening once disposed', async () => {
      setUpTriage();
      fakeEngine(
        { 'ai.triage.run': () => ({ jobId: 'j6', queued: 2 }) },
        { features: TRIAGE_FEATURES }
      );
      const progress: Array<[number, number]> = [];
      // Stores that mailspring-exports loads in the background listen for 'gone' too.
      const goneListeners = Engine.events.listenerCount('gone');
      const run = triageInbox(ACCOUNT_ID, (done, total) => progress.push([done, total]));
      await run.started;
      run.dispose();
      Engine.events.notification('ai.triage.progress', { jobId: 'j6', done: 1, total: 2 });
      expect(progress).toEqual([]);
      expect(Engine.events.listenerCount('gone')).toBe(goneListeners);
    });

    it('asks for a model when triage has none', async () => {
      setUpAi({ features: { triage: { enabled: true } } });
      fakeEngine({}, { features: TRIAGE_FEATURES });
      let key = null;
      await triageInbox(ACCOUNT_ID).started.catch((e) => (key = e.key));
      expect(key).toBe('not-configured');
    });
  });

  describe('"Needs reply"', () => {
    it('is in the sidebar for the accounts with triage on', () => {
      setUpTriage();
      fakeEngine({}, { features: TRIAGE_FEATURES });
      const item = NeedsReplySidebarExtension.sidebarItem([ACCOUNT_ID, 'other-account']);
      expect(item!.perspective.accountIds).toEqual([ACCOUNT_ID]);
      expect(NeedsReplySidebarExtension.sidebarItem(['other-account'])).toBe(null);
    });

    it('is not there with triage off or the engine down', () => {
      setUpTriage({ features: { triage: { enabled: false, provider: 'anthropic' } } });
      fakeEngine({}, { features: TRIAGE_FEATURES });
      expect(NeedsReplySidebarExtension.sidebarItem([ACCOUNT_ID])).toBe(null);
      setUpTriage();
      Engine.events.features = null;
      expect(NeedsReplySidebarExtension.sidebarItem([ACCOUNT_ID])).toBe(null);
    });

    it("lists the engine's needs-reply threads", async () => {
      setUpTriage();
      const sent = fakeEngine(
        { 'ai.triage.list': () => ({ threadIds: ['t7', 't9'], total: 2 }) },
        { features: TRIAGE_FEATURES }
      );
      const subscription = new NeedsReplySubscription([ACCOUNT_ID]);
      await settle(() => subscription._query!.sql().includes('t9'));
      expect(subscription._query!.sql()).toContain("'t7'");
      expect(sent.find((s) => s.method === 'ai.triage.list')!.params).toEqual({
        accountIds: [ACCOUNT_ID],
        category: 'needs-reply',
        limit: 500,
      });
      subscription.onLastCallbackRemoved();
    });
  });
});
