// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import ReactDOM from 'react-dom';
import { Folder } from '../../src/data/models/folder';
import { Message } from '../../src/data/models/message';
import { Thread } from '../../src/data/models/thread';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import ThreadListDataSource from '../../src/features/thread-list/data-source';
import * as ThreadListColumns from '../../src/features/thread-list/columns';
import { applyDeltas } from '../../src/engine/client/bridge';
import { MockEngine, settle } from '../support/mock-engine';

const inbox = new Folder({ id: 'inbox', accountId: TEST_ACCOUNT_ID, role: 'inbox', path: 'INBOX' });

async function until(what: string, condition: () => boolean) {
  for (let i = 0; i < 200; i++) {
    if (condition()) return;
    await settle(5);
    advanceClock(1);
  }
  throw new Error(`Timed out waiting for ${what}`);
}

// The row the engine saves for a reply draft.create builds (MCP reply_to_thread), as its delta.
const engineReplyDraft = {
  __cls: 'Message',
  _sa: 0,
  _suc: 0,
  aid: TEST_ACCOUNT_ID,
  id: 'engine-draft',
  v: 1,
  hMsgId: 'A4B2282A@example.com',
  threadId: 't1',
  rthMsgId: 'm1@example.com',
  subject: 'Re: Plans',
  draft: true,
  pristine: false,
  object: 'draft',
  date: 1700000100,
  from: [{ __cls: 'Contact', email: 'me@example.com', name: '' }],
  to: [{ __cls: 'Contact', email: 'bob@example.net', name: 'Bob' }],
  cc: [],
  bcc: [],
  replyTo: [],
  files: [],
  file_ids: [],
  events: [],
  metadata: [],
  labels: [],
  folders: { drafts: 4 },
  gMsgId: '',
  unread: false,
  starred: false,
};

// The thread list's draft pencil reads its thread's messages (threadId = the thread's id,
// draft = true), so it shows for a draft the engine wrote for another client too.
describe('the thread list draft marker for drafts the engine writes', function () {
  let engine: MockEngine;
  let dataSource: ThreadListDataSource;

  beforeEach(async () => {
    engine = new MockEngine().install();
    engine.add(
      new Thread({
        id: 't1',
        accountId: TEST_ACCOUNT_ID,
        subject: 'Plans',
        folders: [inbox],
        inAllMail: true,
        lastMessageReceivedTimestamp: new Date(1700000000 * 1000),
      }),
      new Message({
        id: 'm1',
        accountId: TEST_ACCOUNT_ID,
        threadId: 't1',
        headerMessageId: 'm1@example.com',
        date: new Date(1700000000 * 1000),
        subject: 'Plans',
      })
    );
    await settle();
    dataSource = new ThreadListDataSource(MailboxPerspective.forCategory(inbox).threads());
    dataSource.setRetainedRange({ start: 0, end: 10 });
    await until('the thread', () => !!dataSource.get(0));
  });

  afterEach(() => {
    dataSource.cleanup();
    engine.uninstall();
  });

  const pencilShown = () => {
    const node = document.createElement('div');
    const participants = ThreadListColumns.Wide.find((c) => c.name === 'Participants');
    ReactDOM.render(<div>{participants.resolver(dataSource.get(0))}</div>, node);
    const shown = !!node.querySelector('.draft-icon');
    ReactDOM.unmountComponentAtNode(node);
    return shown;
  };

  const deliver = (row: any) => {
    engine.add(new Message({}).fromJSON(row));
    applyDeltas([JSON.stringify({ type: 'persist', modelClass: 'Message', modelJSONs: [row] })], {
      mainWindow: false,
    });
  };

  it('shows the pencil once the delta of a reply draft from draft.create arrives', async () => {
    expect(pencilShown()).toBe(false);
    deliver(engineReplyDraft);
    await until('the draft in the thread', () =>
      ((dataSource.get(0) as any).__messages || []).some((m) => m.id === 'engine-draft')
    );
    expect(pencilShown()).toBe(true);
  });

  it('shows no pencil for a draft with an empty threadId', async () => {
    deliver({ ...engineReplyDraft, threadId: '' });
    await settle(20);
    advanceClock(10);
    await settle(20);
    expect(pencilShown()).toBe(false);
  });
});
