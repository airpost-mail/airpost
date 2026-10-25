// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { Account } from '../../src/data/models/account';
import { AccountStore } from '../../src/data/stores/account';
import * as Actions from '../../src/data/actions';
import { Contact } from '../../src/data/models/contact';
import DatabaseStore from '../../src/data/database/store';
import DraftStore from '../../src/data/drafts/store';
import * as ExtensionRegistry from '../../src/packages/extension-registry';
import { Message } from '../../src/data/models/message';
import { SyncbackMetadataTask } from '../../src/data/tasks/syncback-metadata';
import TaskQueue from '../../src/data/stores/task-queue';
import { Thread } from '../../src/data/models/thread';
import UndoRedoStore from '../../src/data/stores/undo-redo';
import EngineEvents from '../../src/engine/client/events';
import { Feature, Method } from '../../src/engine/protocol';
import DraftFactory from '../../src/data/drafts/factory';
import { sendMessageNow } from '../../src/features/undo-redo/toast';

const ME = { name: 'Me', email: 'me@example.com' };
const ALICE = { name: 'Alice', email: 'alice@example.com' };

// The spec runner fakes timers; the engine's answers settle in microtasks.
async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

describe('the draft pipeline in the windows', function draftPipeline() {
  let calls: Array<{ method: string; params: any }>;
  let answers: { [method: string]: (params: any) => unknown };
  let extensions: any[];
  let thread: Thread;
  let message: Message;

  beforeEach(() => {
    (AccountStore as any)._accounts = [
      new Account({ id: 'acct', name: 'Me', emailAddress: ME.email, provider: 'gmail' }),
    ];
    (AccountStore as any)._caches = {};
    calls = [];
    answers = {};
    extensions = [];
    EngineEvents.connected([Feature.DraftPipeline], (method, params) => {
      calls.push({ method, params });
      const answer = answers[method];
      return answer ? Promise.resolve(answer(params)) : Promise.reject(new Error('refused'));
    });
    spyOn(ExtensionRegistry.Composer, 'extensions').andCallFake(() => extensions);
    spyOn(AppEnv.keymaps, 'getIsAltKeyDown').andReturn(false);
    spyOn(AppEnv, 'showErrorDialog');
    const realGet = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key === 'core.composing.html' ? true : realGet(key)
    );

    thread = new Thread({ id: 'thread-1', accountId: 'acct', subject: 'Hello' });
    message = new Message({
      id: 'received',
      accountId: 'acct',
      threadId: 'thread-1',
      headerMessageId: 'received@example.com',
      subject: 'Hello',
      draft: false,
      date: new Date(1000),
      from: [new Contact(ALICE)],
      to: [new Contact(ME)],
      cc: [],
      files: [],
      body: 'original',
    });
  });

  afterEach(() => {
    EngineEvents.gone();
  });

  const engineReply = (params: any) => ({
    __cls: 'Message',
    id: 'engine-id',
    aid: params.accountId,
    hMsgId: 'reply@example.com',
    rthMsgId: 'received@example.com',
    threadId: 'thread-1',
    subject: 'Re: Hello',
    body: '<br/>quoted by the engine',
    from: [{ ...ME, __cls: 'Contact' }],
    to: [{ ...ALICE, __cls: 'Contact' }],
    cc: [],
    bcc: [],
    files: [],
    date: 1000,
    draft: true,
    pristine: true,
    plaintext: false,
  });

  it('builds a reply with draft.create and keeps it unsaved', async function () {
    answers[Method.DraftCreate] = engineReply;
    const draft = await DraftFactory.createDraftForReply({
      message,
      thread,
      type: 'reply-all',
    });
    expect(calls.length).toBe(1);
    const { params } = calls[0];
    expect(params.kind).toBe('reply-all');
    expect(params.accountId).toBe('acct');
    expect(params.messageId).toBe('received');
    expect(params.fields).toEqual({ threadId: 'thread-1' });
    expect(params.save).toBe(false);
    expect(params.html).toBe(true);
    expect(draft instanceof Message).toBe(true);
    expect(draft.body).toBe('<br/>quoted by the engine');
    expect(draft.replyToHeaderMessageId).toBe('received@example.com');
    expect(draft.to.map((c) => c.email)).toEqual([ALICE.email]);
    expect(draft.id).toBeUndefined();
  });

  it('passes a new draft its fields as message JSON', async function () {
    answers[Method.DraftCreate] = engineReply;
    await DraftFactory.createNewDraft({ to: [new Contact(ALICE)] });
    expect(calls[0].params.kind).toBe('new');
    expect(calls[0].params.accountId).toBe('acct');
    expect(calls[0].params.fields.to.map((c) => c.email)).toEqual([ALICE.email]);
  });

  it('rejects when the engine refuses the draft', async function () {
    let error = null;
    await DraftFactory.createDraftForReply({ message, thread, type: 'reply' }).catch(
      (err) => (error = err)
    );
    expect(calls.length).toBe(1);
    expect(error.message).toBe('refused');
  });

  it('runs every prepareNewDraft hook on a draft the engine built', async function () {
    const first = jasmine.createSpy('first');
    const plugin = jasmine.createSpy('plugin');
    extensions = [
      { name: 'FirstPluginExtension', prepareNewDraft: first },
      { name: 'SomePluginExtension', prepareNewDraft: plugin },
    ];
    spyOn(Actions, 'queueTask');
    spyOn(TaskQueue, 'waitForPerformLocal').andReturn(Promise.resolve());
    answers[Method.DraftCreate] = engineReply;
    const draft = await DraftFactory.createDraftForReply({
      message,
      thread,
      type: 'reply',
    });
    await DraftStore._finalizeAndPersistNewMessage(draft);
    expect(first).toHaveBeenCalled();
    expect(plugin).toHaveBeenCalled();
  });

  // Harnesses set PLAYWRIGHT so page.evaluate works; the engine still runs and saves drafts.
  describe('under a Playwright harness', () => {
    let was: string | undefined;
    beforeEach(() => {
      was = process.env.PLAYWRIGHT;
      process.env.PLAYWRIGHT = '1';
    });
    afterEach(() => {
      if (was === undefined) delete process.env.PLAYWRIGHT;
      else process.env.PLAYWRIGHT = was;
    });

    it('waits for the engine to save a task locally', async function () {
      const task = new SyncbackMetadataTask({ accountId: 'acct', modelId: 'm', value: {} } as any);
      let resolved = false;
      TaskQueue.waitForPerformLocal(task).then(() => (resolved = true));
      await flush();
      expect(resolved).toBe(false);
      (TaskQueue as any)._waitingForLocal = (TaskQueue as any)._waitingForLocal.filter(
        (w) => w.task !== task
      );
    });

    it('leaves a new draft to the engine instead of announcing it itself', async function () {
      spyOn(Actions, 'queueTask');
      spyOn(TaskQueue, 'waitForPerformLocal').andReturn(Promise.resolve());
      spyOn(DatabaseStore, 'trigger');
      const draft = new Message({ ...message, id: undefined, draft: true } as any);
      await DraftStore._finalizeAndPersistNewMessage(draft);
      expect(draft.id).toBeUndefined();
      expect(DatabaseStore.trigger).not.toHaveBeenCalled();
    });
  });

  describe('sending', () => {
    beforeEach(function () {
      this.draft = new Message({
        accountId: 'acct',
        headerMessageId: 'reply@example.com',
        threadId: 'thread-1',
        subject: 'Re: Hello',
        draft: true,
        from: [new Contact(ME)],
        to: [new Contact(ALICE)],
        cc: [],
        bcc: [],
        files: [],
        body: '<p>hi</p>',
      });
      answers[Method.DraftSend] = (params) =>
        params.delayMs ? { scheduledAt: 2000000000 } : { taskId: 't1' };
      answers[Method.DraftCancelSend] = () => ({ cancelled: true });
    });

    it("sends a test extension's bodies with draft.send", async function () {
      extensions = [
        {
          name: 'StampExtension',
          applyTransformsForSending: ({ draftBodyRootNode }) => {
            draftBodyRootNode.innerHTML += '<p>stamped</p>';
          },
        },
      ];
      spyOn(Actions, 'queueUndoOnlyTask');
      await DraftStore._sendWithEngine(this.draft, { delay: 0, actionKey: 'send' });
      expect(calls.length).toBe(1);
      expect(calls[0].method).toBe(Method.DraftSend);
      expect(calls[0].params).toEqual({
        accountId: 'acct',
        headerMessageId: 'reply@example.com',
        delayMs: undefined,
        after: 'send',
        bodies: { self: '<p>hi</p><p>stamped</p>' },
      });
      expect(Actions.queueUndoOnlyTask).not.toHaveBeenCalled();
    });

    it("sends each recipient's body when an extension asks for them", async function () {
      extensions = [
        {
          name: 'PerRecipientExtension',
          needsPerRecipientBodies: () => true,
          applyTransformsForSending: ({ draftBodyRootNode, recipient }) => {
            if (recipient) draftBodyRootNode.innerHTML += `<p>for ${recipient.email}</p>`;
          },
        },
      ];
      await DraftStore._sendWithEngine(this.draft, { delay: 0, actionKey: 'send' });
      expect(calls[0].params.bodies).toEqual({
        self: '<p>hi</p>',
        perRecipient: { [ALICE.email]: `<p>hi</p><p>for ${ALICE.email}</p>` },
      });
    });

    it('sends no bodies when nothing transforms the draft', async function () {
      await DraftStore._sendWithEngine(this.draft, { delay: 0, actionKey: 'send-and-archive' });
      expect(calls[0].params.bodies).toBeUndefined();
      expect(calls[0].params.after).toBe('send-and-archive');
    });

    it('undoes a delayed send with draft.cancelSend', async function () {
      let undoTask = null;
      spyOn(Actions, 'queueUndoOnlyTask').andCallFake((task) => {
        undoTask = task;
        // As the action bridge hands it to the main window: as JSON.
        UndoRedoStore._onQueueUndoOnly(
          new SyncbackMetadataTask(JSON.parse(JSON.stringify(task.toJSON())))
        );
      });
      await DraftStore._sendWithEngine(this.draft, { delay: 5000, actionKey: 'send' });
      expect(calls[0].params.delayMs).toBe(5000);
      expect(undoTask.value.isUndoSend).toBe(true);
      expect(undoTask.value.actionKey).toBe('send');

      spyOn(Actions, 'queueTasks');
      UndoRedoStore.undo();
      await flush();
      expect(calls[1]).toEqual({
        method: Method.DraftCancelSend,
        params: { accountId: 'acct', headerMessageId: 'reply@example.com' },
      });
      // The engine writes the metadata; no task of the window's undoes it.
      expect(Actions.queueTasks).not.toHaveBeenCalled();
      expect(AppEnv.showErrorDialog).not.toHaveBeenCalled();
    });

    it('keeps the undo send and its after-send action on Send now', async function () {
      spyOn(Actions, 'queueUndoOnlyTask').andCallFake((task) =>
        UndoRedoStore._onQueueUndoOnly(
          new SyncbackMetadataTask(JSON.parse(JSON.stringify(task.toJSON())))
        )
      );
      await DraftStore._sendWithEngine(this.draft, { delay: 5000, actionKey: 'send-and-archive' });
      const stored = new Message({ ...this.draft, id: 'draft-id' });
      spyOn(DatabaseStore, 'find').andReturn(Promise.resolve(stored));
      let moved = null;
      spyOn(Actions, 'queueTask').andCallFake((task) => {
        moved = task;
      });
      const before = Math.floor(Date.now() / 1000);
      expect(await sendMessageNow(UndoRedoStore.getMostRecent())).toBe(true);
      expect(moved.pluginId).toBe('send-later');
      expect(moved.value.isUndoSend).toBe(true);
      expect(moved.value.actionKey).toBe('send-and-archive');
      expect(moved.value.expiration).not.toBeLessThan(before);
      expect(moved.value.expiration).toBeLessThan(2000000000);
    });

    it('says so when the send had gone out before the undo', async function () {
      answers[Method.DraftCancelSend] = () => ({ cancelled: false });
      spyOn(Actions, 'queueUndoOnlyTask').andCallFake((task) =>
        UndoRedoStore._onQueueUndoOnly(task)
      );
      await DraftStore._sendWithEngine(this.draft, { delay: 5000, actionKey: 'send' });
      UndoRedoStore.undo();
      await flush();
      expect(AppEnv.showErrorDialog).toHaveBeenCalled();
    });

    it('clears the sending state when draft.send refuses', async function () {
      answers[Method.DraftSend] = () => {
        throw new Error('no');
      };
      DraftStore._draftsSending['reply@example.com'] = true;
      await DraftStore._sendWithEngine(this.draft, { delay: 0, actionKey: 'send' });
      expect(DraftStore._draftsSending['reply@example.com']).toBe(false);
      expect(AppEnv.showErrorDialog).toHaveBeenCalled();
    });
  });
});
