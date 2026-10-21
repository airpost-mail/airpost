// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import ReactDOM from 'react-dom';
import * as Actions from '../../src/data/actions';
import { Message } from '../../src/data/models/message';
import { Thread } from '../../src/data/models/thread';
import { Contact } from '../../src/data/models/contact';
import DraftStore from '../../src/data/drafts/store';
import { DraftEditingSession } from '../../src/data/drafts/editing-session';
import { MessageStore } from '../../src/data/stores/message';
import TaskQueue from '../../src/data/stores/task-queue';
import { SyncbackDraftTask } from '../../src/data/tasks/syncback-draft';
import { DestroyDraftTask } from '../../src/data/tasks/destroy-draft';
import { applyDeltas } from '../../src/engine/client/bridge';
import ComponentRegistry from '../../src/packages/component-registry';
import InflatesDraftClientId from '../../src/components/decorators/inflates-draft-client-id';
import DraftPreview from '../../src/features/draft-list/preview';
import DraftListStore from '../../src/features/draft-list/store';
import { MockEngine, settle } from '../support/mock-engine';

let previewSession: DraftEditingSession | null = null;

class SessionProbe extends React.Component<{ session: DraftEditingSession }> {
  static displayName = 'SessionProbe';
  render() {
    previewSession = this.props.session;
    return <div className="probe" />;
  }
}
const PreviewComposer = InflatesDraftClientId(SessionProbe);

const ME = new Contact({ email: TEST_ACCOUNT_EMAIL, name: TEST_ACCOUNT_NAME });
const ALICE = new Contact({ email: 'alice@example.com', name: 'Alice' });

// A reply draft opened from the Drafts list in the reading pane, edited and saved, keeps the
// link to its thread and the message it answers, and stays inline in that thread.
describe('a reply draft edited in the Drafts reading pane', function () {
  let engine: MockEngine;
  let container: HTMLDivElement;
  let thread: Thread;
  let received: Message;
  let draft: Message;
  let queued: any[];

  beforeEach(async () => {
    engine = new MockEngine().install();
    thread = new Thread({ id: 't1', accountId: TEST_ACCOUNT_ID, subject: 'Plans' });
    received = new Message({
      id: 'm1',
      accountId: TEST_ACCOUNT_ID,
      threadId: 't1',
      headerMessageId: 'plans@example.com',
      subject: 'Plans',
      draft: false,
      date: new Date(1000000),
      from: [ALICE],
      to: [ME],
      body: '<p>Lunch?</p>',
    });
    draft = new Message({
      id: 'draft-1',
      accountId: TEST_ACCOUNT_ID,
      version: 2,
      threadId: 't1',
      headerMessageId: 'reply@example.com',
      replyToHeaderMessageId: 'plans@example.com',
      subject: 'Re: Plans',
      draft: true,
      pristine: false,
      date: new Date(2000000),
      from: [ME],
      to: [ALICE],
      folders: { drafts: 4 },
      body: '<p>Sure</p>',
    });
    engine.add(thread, received, draft);

    queued = [];
    spyOn(Actions, 'queueTask').andCallFake((task) => queued.push(task));
    spyOn(TaskQueue, 'waitForPerformLocal').andReturn(Promise.resolve());

    previewSession = null;
    ComponentRegistry.register(PreviewComposer, { role: 'Composer' });
    container = document.createElement('div');
    DraftListStore._focusedHeaderMessageId = 'reply@example.com';
    ReactDOM.render(<DraftPreview />, container);
    for (let i = 0; i < 20 && !previewSession; i++) await settle(5);
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
    ComponentRegistry.unregister(PreviewComposer);
    DraftListStore._focusedHeaderMessageId = null;
    DraftStore._cleanupAllSessions();
    MessageStore._setStoreDefaults();
    MessageStore._thread = null;
    engine.uninstall();
  });

  const saveEdit = async () => {
    previewSession.changes.add({ body: '<p>Sure, at noon</p>' });
    await previewSession.changes.commit();
    return queued.filter((t) => t instanceof SyncbackDraftTask).pop() as SyncbackDraftTask;
  };

  it('saves the row in place, with its thread and the message it answers', async () => {
    expect(previewSession).not.toBe(null);
    const task = await saveEdit();
    const json = task.toJSON().draft;
    expect(json.id).toBe('draft-1');
    expect(json.threadId).toBe('t1');
    expect(json.rthMsgId).toBe('plans@example.com');
    expect(json.hMsgId).toBe('reply@example.com');
    expect(json.body).toBe('<p>Sure, at noon</p>');
  });

  it('stays inline in its thread, after the message it answers, once the engine saves it', async () => {
    const task = await saveEdit();
    MessageStore._thread = thread;
    MessageStore._items = [received, draft];
    MessageStore._itemsExpanded = { 'draft-1': 'default' };

    // perform_local_save_draft: the client's keys over the stored row, the version bumped.
    const saved = { ...draft.toJSON(), ...task.toJSON().draft, v: 3, date: 3000 };
    applyDeltas([JSON.stringify({ type: 'persist', modelClass: 'Message', modelJSONs: [saved] })], {
      mainWindow: false,
    });
    await settle();
    const items = MessageStore.items();
    expect(items.map((m) => m.id)).toEqual(['m1', 'draft-1']);
    expect(MessageStore.itemsExpandedState()['draft-1']).toBeTruthy();
    expect(previewSession.draft().threadId).toBe('t1');
  });

  it('is not destroyed when the pane moves to another draft, even untouched', async () => {
    previewSession.draft().pristine = true;
    DraftListStore._focusedHeaderMessageId = null;
    DraftListStore.trigger();
    await settle();
    expect(queued.some((t) => t instanceof DestroyDraftTask)).toBe(false);
  });
});
