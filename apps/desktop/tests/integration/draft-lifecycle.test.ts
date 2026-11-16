// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// A draft's life in the windows, from compose to send, against a small fake engine: it saves
// SyncbackDraftTasks into a draft table keyed by account and headerMessageId, answers task.queue
// with the saved task as the engine does, and serves draft.* from the same table. The invariant
// throughout: the draft sent is the one the user wrote, in the account they send from.

import { Account } from '../../src/data/models/account';
import { AccountStore } from '../../src/data/stores/account';
import * as Actions from '../../src/data/actions';
import { Contact } from '../../src/data/models/contact';
import DatabaseStore from '../../src/data/database/store';
import DraftStore from '../../src/data/drafts/store';
import DraftFactory from '../../src/data/drafts/factory';
import * as ExtensionRegistry from '../../src/packages/extension-registry';
import { Message } from '../../src/data/models/message';
import MessageBodyProcessor from '../../src/mail/body/processor';
import { SyncbackDraftTask } from '../../src/data/tasks/syncback-draft';
import { SyncbackMetadataTask } from '../../src/data/tasks/syncback-metadata';
import { Thread } from '../../src/data/models/thread';
import UndoRedoStore from '../../src/data/stores/undo-redo';
import EngineEvents from '../../src/engine/client/events';
import { Feature, Method } from '../../src/engine/protocol';

const ME_A = { name: 'Me', email: 'me@a.example' };
const ME_B = { name: 'Me', email: 'me@b.example' };
const ALICE = { name: 'Alice', email: 'alice@example.com' };
const HMSG = 'new@airpost.example';

async function flush() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

describe('a draft from compose to send', function draftLifecycle() {
  let calls: Array<{ method: string; params: any }>;
  let answers: { [method: string]: (params: any) => unknown };
  let rows: Message[];
  let saves: SyncbackDraftTask[];
  let nextId: number;

  const row = (accountId: string, headerMessageId = HMSG) =>
    rows.find((r) => r.accountId === accountId && r.headerMessageId === headerMessageId);

  // What the engine does with a SyncbackDraftTask: saves the draft in its account, giving a new
  // one an id, and answers task.queue with the task after its local phase.
  const engineSaves = (task: SyncbackDraftTask) => {
    saves.push(task);
    const existing = row(task.accountId, task.headerMessageId);
    // The session's draft keeps its body behind a getter, which a spread would drop.
    const saved = new Message({
      ...task.draft,
      body: task.draft.body,
      id: existing ? existing.id : `${task.accountId}-draft-${nextId++}`,
    } as any);
    rows = rows.filter((r) => r !== existing).concat([saved]);
    const answer = { ...task.toJSON(), status: 'remote', draft: saved.toJSON() };
    Promise.resolve().then(() => EngineEvents.taskQueued(answer as any));
  };

  const newDraft = (fields: Partial<Message> = {}) =>
    new Message({
      accountId: 'a',
      headerMessageId: HMSG,
      subject: 'Plans',
      draft: true,
      from: [new Contact(ME_A)],
      to: [new Contact(ALICE)],
      cc: [],
      bcc: [],
      files: [],
      body: '<p>first words</p>',
      ...fields,
    } as any);

  beforeEach(() => {
    (AccountStore as any)._accounts = [
      new Account({ id: 'a', name: 'Me', emailAddress: ME_A.email, provider: 'imap' }),
      new Account({ id: 'b', name: 'Me', emailAddress: ME_B.email, provider: 'imap' }),
    ];
    (AccountStore as any)._caches = {};
    calls = [];
    rows = [];
    saves = [];
    nextId = 1;
    answers = {
      [Method.DraftSend]: (params) =>
        params.delayMs ? { scheduledAt: 2000000000 } : { taskId: 'send-1' },
      [Method.DraftCancelSend]: () => ({ cancelled: true }),
      [Method.DraftDelete]: (params) => {
        const existing = row(params.accountId, params.headerMessageId);
        if (!existing) throw new Error('no such draft');
        rows = rows.filter((r) => r !== existing);
        return { taskId: 'delete-1', cancelledSend: false };
      },
    };
    EngineEvents.connected([Feature.DraftPipeline], (method, params) => {
      calls.push({ method, params });
      const answer = answers[method];
      if (!answer) return Promise.reject(new Error('refused'));
      try {
        return Promise.resolve(answer(params));
      } catch (err) {
        return Promise.reject(err);
      }
    });
    spyOn(Actions, 'queueTask').andCallFake((task) => {
      if (task instanceof SyncbackDraftTask) engineSaves(task);
    });
    spyOn(DatabaseStore, 'findBy').andCallFake((klass, where: any) => ({
      include: () =>
        Promise.resolve(
          rows.find(
            (r) =>
              r.headerMessageId === where.headerMessageId &&
              (!where.accountId || r.accountId === where.accountId)
          ) || null
        ),
    }));
    spyOn(MessageBodyProcessor, 'updateCacheForMessage').andReturn(Promise.resolve());
    spyOn(ExtensionRegistry.Composer, 'extensions').andReturn([]);
    spyOn(AppEnv, 'showErrorDialog');
    spyOn(AppEnv, 'reportError');
    spyOn(AppEnv, 'newWindow');
    spyOn(console, 'warn');
    const realGet = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key === 'core.composing.html' ? true : realGet(key)
    );
  });

  afterEach(() => {
    EngineEvents.gone();
    Object.keys(DraftStore._draftSessions).forEach((key) =>
      DraftStore._doneWithSession(DraftStore._draftSessions[key])
    );
    DraftStore._draftsSending = {};
  });

  const sent = () => calls.filter((c) => c.method === Method.DraftSend);
  const deletes = () => calls.filter((c) => c.method === Method.DraftDelete);

  describe('composing', () => {
    it('gives a new draft the id in the task.queue answer, before its row arrives', async () => {
      const draft = newDraft();
      await DraftStore._finalizeAndPersistNewMessage(draft);
      expect(saves.length).toBe(1);
      expect(draft.id).toBe('a-draft-1');
      expect(DatabaseStore.findBy).not.toHaveBeenCalled();
    });

    it('opens a popout with the draft and its id', async () => {
      const draft = newDraft();
      await DraftStore._finalizeAndPersistNewMessage(draft, { popout: true });
      await flush();
      expect(AppEnv.newWindow).toHaveBeenCalled();
      const { windowProps } = (AppEnv.newWindow as jasmine.Spy).calls[0].args[0];
      expect(windowProps.headerMessageId).toBe(HMSG);
      expect(windowProps.draftJSON.id).toBe('a-draft-1');
      expect(windowProps.draftJSON.body).toBe('<p>first words</p>');
    });

    it('builds a new draft, a reply and a forward with draft.create', async () => {
      answers[Method.DraftCreate] = (params) => ({
        __cls: 'Message',
        aid: params.accountId,
        hMsgId: `${params.kind}@airpost.example`,
        subject: params.kind,
        body: '',
        from: [{ ...ME_A, __cls: 'Contact' }],
        to: [],
        cc: [],
        bcc: [],
        files: [],
        draft: true,
      });
      const thread = new Thread({ id: 'thread-1', accountId: 'a', subject: 'Hello' });
      const message = new Message({
        id: 'received',
        accountId: 'a',
        threadId: 'thread-1',
        headerMessageId: 'received@example.com',
        from: [new Contact(ALICE)],
        to: [new Contact(ME_A)],
        files: [],
      } as any);
      await DraftFactory.createNewDraft();
      await DraftFactory.createDraftForReply({ message, thread, type: 'reply' });
      const forward = await DraftFactory.createDraftForForward({ thread, message });
      expect(calls.map((c) => [c.params.kind, c.params.accountId, c.params.messageId])).toEqual([
        ['new', 'a', undefined],
        ['reply', 'a', 'received'],
        ['forward', 'a', 'received'],
      ]);
      expect(forward.headerMessageId).toBe('forward@airpost.example');
      expect(forward.id).toBeUndefined();
    });

    it('saves what is typed to the draft in its account', async () => {
      const draft = newDraft();
      await DraftStore._finalizeAndPersistNewMessage(draft);
      const session = await DraftStore.sessionForClientId(HMSG);
      session.changes.add({ body: '<p>more words</p>' });
      await session.changes.commit();
      expect(rows.length).toBe(1);
      expect(row('a').body).toBe('<p>more words</p>');
      expect(row('a').id).toBe('a-draft-1');
    });
  });

  describe('changing the From address to another account', () => {
    let session;

    beforeEach(async () => {
      await DraftStore._finalizeAndPersistNewMessage(newDraft({ id: undefined }));
      session = await DraftStore.sessionForClientId(HMSG);
      // The draft has no id in the session, as before its save comes back.
      session._draft.id = undefined;
    });

    it('saves the draft in the new account and deletes the old copy by headerMessageId', async () => {
      session.changes.add({ from: [new Contact(ME_B)], body: '<p>what I wrote</p>' });
      await session.ensureCorrectAccount();
      expect(saves[saves.length - 1].accountId).toBe('b');
      expect(saves[saves.length - 1].draft.body).toBe('<p>what I wrote</p>');
      expect(deletes().map((c) => c.params)).toEqual([{ accountId: 'a', headerMessageId: HMSG }]);
      expect(row('a')).toBeUndefined();
      expect(row('b').body).toBe('<p>what I wrote</p>');
    });

    it('keeps editing the new copy, so later saves never bring the old one back', async () => {
      session.changes.add({ from: [new Contact(ME_B)] });
      await session.ensureCorrectAccount();
      expect(session.draft().accountId).toBe('b');
      expect(session.draft().id).toBe(row('b').id);
      session.changes.add({ body: '<p>edited after the move</p>' });
      await session.changes.commit();
      expect(row('a')).toBeUndefined();
      expect(row('b').body).toBe('<p>edited after the move</p>');
    });

    it('takes no changes from the copy left in the old account', async () => {
      session.changes.add({ from: [new Contact(ME_B)], body: '<p>what I wrote</p>' });
      await session.ensureCorrectAccount();
      session._onDraftChanged({
        type: 'persist',
        objectClass: 'Message',
        objects: [newDraft({ id: 'a-draft-1', body: '<p>stale</p>' })],
      } as any);
      expect(session.draft().accountId).toBe('b');
      expect(session.draft().body).toBe('<p>what I wrote</p>');
    });

    it('only logs a failed delete of the old copy', async () => {
      answers[Method.DraftDelete] = () => {
        throw new Error('being sent');
      };
      session.changes.add({ from: [new Contact(ME_B)] });
      await session.ensureCorrectAccount();
      expect(console.warn).toHaveBeenCalled();
      expect(row('b')).toBeDefined();
    });

    it('sends the copy the user wrote from the new account', async () => {
      session.changes.add({ from: [new Contact(ME_B)], body: '<p>what I wrote</p>' });
      await DraftStore._onSendDraft(HMSG, { delay: 0, actionKey: 'send' });
      expect(sent().map((c) => c.params.accountId)).toEqual(['b']);
      expect(sent()[0].params.headerMessageId).toBe(HMSG);
      const read = (MessageBodyProcessor.updateCacheForMessage as jasmine.Spy).calls[0].args[0];
      expect(read.accountId).toBe('b');
      expect(read.body).toBe('<p>what I wrote</p>');
      expect(rows.map((r) => r.accountId)).toEqual(['b']);
      expect(AppEnv.showErrorDialog).not.toHaveBeenCalled();
    });

    it('sends the copy in the From account when the old copy is still there', async () => {
      answers[Method.DraftDelete] = () => {
        throw new Error('being sent');
      };
      session.changes.add({ from: [new Contact(ME_B)], body: '<p>what I wrote</p>' });
      await DraftStore._onSendDraft(HMSG, { delay: 0, actionKey: 'send' });
      expect(rows.length).toBe(2);
      expect(sent().map((c) => c.params.accountId)).toEqual(['b']);
      const read = (MessageBodyProcessor.updateCacheForMessage as jasmine.Spy).calls[0].args[0];
      expect(read.body).toBe('<p>what I wrote</p>');
    });

    it('undoes the delayed send in the account it was sent from', async () => {
      spyOn(Actions, 'queueUndoOnlyTask').andCallFake((task) =>
        UndoRedoStore._onQueueUndoOnly(
          new SyncbackMetadataTask(JSON.parse(JSON.stringify(task.toJSON())))
        )
      );
      session.changes.add({ from: [new Contact(ME_B)] });
      await DraftStore._onSendDraft(HMSG, { delay: 5000, actionKey: 'send' });
      UndoRedoStore.undo();
      await flush();
      expect(calls.find((c) => c.method === Method.DraftCancelSend).params).toEqual({
        accountId: 'b',
        headerMessageId: HMSG,
      });
    });
  });

  describe('sending', () => {
    beforeEach(async () => {
      await DraftStore._finalizeAndPersistNewMessage(newDraft());
    });

    it('saves the last edits, then sends now', async () => {
      const session = await DraftStore.sessionForClientId(HMSG);
      session.changes.add({ body: '<p>last words</p>' });
      await DraftStore._onSendDraft(HMSG, { delay: 0, actionKey: 'send' });
      expect(row('a').body).toBe('<p>last words</p>');
      expect(sent().map((c) => c.params)).toEqual([
        {
          accountId: 'a',
          headerMessageId: HMSG,
          delayMs: undefined,
          after: 'send',
          bodies: undefined,
        },
      ]);
      expect(DraftStore.isSendingDraft(HMSG)).toBe(true);
    });

    it('sends with an undo window and calls the send back on undo', async () => {
      spyOn(Actions, 'queueUndoOnlyTask').andCallFake((task) =>
        UndoRedoStore._onQueueUndoOnly(
          new SyncbackMetadataTask(JSON.parse(JSON.stringify(task.toJSON())))
        )
      );
      await DraftStore._onSendDraft(HMSG, { delay: 5000, actionKey: 'send' });
      expect(sent()[0].params.delayMs).toBe(5000);
      expect(Actions.queueUndoOnlyTask).toHaveBeenCalled();
      UndoRedoStore.undo();
      await flush();
      expect(calls[calls.length - 1]).toEqual({
        method: Method.DraftCancelSend,
        params: { accountId: 'a', headerMessageId: HMSG },
      });
      expect(row('a')).toBeDefined();
      expect(AppEnv.showErrorDialog).not.toHaveBeenCalled();
    });

    it('keeps the draft and says why when the engine refuses the send', async () => {
      answers[Method.DraftSend] = () => {
        throw Object.assign(new Error('rejected'), {
          code: -32002,
          data: { message: 'This draft has no recipients.' },
        });
      };
      await DraftStore._onSendDraft(HMSG, { delay: 0, actionKey: 'send' });
      expect(DraftStore.isSendingDraft(HMSG)).toBe(false);
      expect((AppEnv.showErrorDialog as jasmine.Spy).calls[0].args[0]).toContain(
        'This draft has no recipients.'
      );
      expect(row('a').body).toBe('<p>first words</p>');
    });

    it('reopens the draft when a send fails in the engine', () => {
      spyOn(AppEnv, 'isMainWindow').andReturn(true);
      spyOn(Actions, 'composePopoutDraft');
      DraftStore._draftsSending[HMSG] = true;
      DraftStore._onSendDraftFailed({ headerMessageId: HMSG, errorMessage: 'SMTP said no' });
      expect(DraftStore.isSendingDraft(HMSG)).toBe(false);
      advanceClock(300);
      expect(Actions.composePopoutDraft).toHaveBeenCalledWith(HMSG, {
        errorMessage: 'SMTP said no',
        errorDetail: undefined,
      });
    });

    it('ends the sending state when the send succeeds', () => {
      DraftStore._draftsSending[HMSG] = true;
      DraftStore._onSendDraftSuccess({ headerMessageId: HMSG });
      expect(DraftStore.isSendingDraft(HMSG)).toBe(false);
    });
  });

  describe('deleting', () => {
    it('deletes the draft with draft.delete and ends its session', async () => {
      const draft = newDraft();
      await DraftStore._finalizeAndPersistNewMessage(draft);
      await DraftStore._onDestroyDraft({
        accountId: 'a',
        headerMessageId: HMSG,
        id: draft.id,
      });
      expect(deletes().map((c) => c.params)).toEqual([{ accountId: 'a', headerMessageId: HMSG }]);
      expect(rows).toEqual([]);
      expect(DraftStore._draftSessions[HMSG]).toBeUndefined();
    });

    it('only logs a delete the engine refuses', async () => {
      await DraftStore._onDestroyDraft({ accountId: 'a', headerMessageId: 'gone@x', id: 'x' });
      expect(console.warn).toHaveBeenCalled();
    });
  });
});
