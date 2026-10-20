// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import {
  Account,
  AccountStore,
  Actions,
  Contact,
  DatabaseStore,
  DraftStore,
  ExtensionRegistry,
  Message,
  SyncbackMetadataTask,
  TaskQueue,
  Thread,
  UndoRedoStore,
} from 'mailspring-exports';
import EngineEvents from '../../src/engine/events';
import { Feature, Method } from '../../src/shared/engine/protocol';
import { isEngineBuilt } from '../../src/engine/drafts';
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
    expect(isEngineBuilt(draft)).toBe(true);
  });

  it('passes a new draft its fields as message JSON', async function () {
    answers[Method.DraftCreate] = engineReply;
    await DraftFactory.createNewDraft({ to: [new Contact(ALICE)] });
    expect(calls[0].params.kind).toBe('new');
    expect(calls[0].params.accountId).toBe('acct');
    expect(calls[0].params.fields.to.map((c) => c.email)).toEqual([ALICE.email]);
  });

  it('builds the draft itself when the engine refuses or lacks the pipeline', async function () {
    const refused = await DraftFactory.createDraftForReply({
      message,
      thread,
      type: 'reply',
    });
    expect(calls.length).toBe(1);
    expect(isEngineBuilt(refused)).toBe(false);
    expect(refused.body).toContain('gmail_quote');

    EngineEvents.connected([], () => Promise.reject(new Error('not asked')));
    calls = [];
    const local = await DraftFactory.createDraftForReply({
      message,
      thread,
      type: 'reply',
    });
    expect(calls.length).toBe(0);
    expect(local.subject).toBe('Re: Hello');
  });

  it("runs only the plugins' prepareNewDraft on a draft the engine built", async function () {
    const builtIn = jasmine.createSpy('signature');
    const plugin = jasmine.createSpy('plugin');
    extensions = [
      { name: 'SignatureComposerExtension', prepareNewDraft: builtIn },
      { name: 'TrackingPixelsComposerExtension', prepareNewDraft: builtIn },
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
    expect(builtIn).not.toHaveBeenCalled();
    expect(plugin).toHaveBeenCalled();

    await DraftStore._finalizeAndPersistNewMessage(new Message({ ...message, draft: true } as any));
    expect(builtIn.callCount).toBe(2);
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
