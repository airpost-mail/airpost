// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { Contact } from '../../src/data/models/contact';
import { Message } from '../../src/data/models/message';
import DraftStore from '../../src/data/drafts/store';
import { DatabaseChangeRecord } from '../../src/data/database/change-record';
import { DraftEditingSession } from '../../src/data/drafts/editing-session';

const HMID = 'draft-1@tern';

function localDraft() {
  return new Message({
    id: 'old-id',
    accountId: 'a1',
    headerMessageId: HMID,
    draft: true,
    pristine: false,
    subject: 'Local subject',
    to: [new Contact({ email: 'local@example.com' })],
    body: '<p>Local body</p>',
  });
}

// What the engine persists when it adopts another device's revision of the draft.
function remoteRevision(overrides: Record<string, any> = {}) {
  return new Message({
    id: 'old-id',
    accountId: 'a1',
    headerMessageId: HMID,
    draft: true,
    pristine: false,
    subject: 'Remote subject',
    to: [new Contact({ email: 'remote@example.com' })],
    cc: [new Contact({ email: 'cc@example.com' })],
    body: '<p>Remote body</p>',
    ...overrides,
  });
}

function deliver(type: 'persist' | 'unpersist', message: Message) {
  DraftStore._onDataChanged(
    new DatabaseChangeRecord<Message>({
      type,
      objectClass: Message.name,
      objects: [message],
      objectsRawJSON: [message.toJSON()],
    })
  );
}

describe('DraftEditingSession with drafts coming in from the engine', () => {
  let session: DraftEditingSession;

  beforeEach(() => {
    // Edits and saves within one test can share a millisecond; each needs its own time.
    let now = 0;
    spyOn(performance, 'now').andCallFake(() => ++now);
    DraftStore._draftSessions = {};
    session = DraftStore._createSession(HMID, localDraft());
    spyOn(session, 'changeSetCommit').andReturn(Promise.resolve());
  });

  afterEach(() => {
    session.teardown();
    DraftStore._draftSessions = {};
  });

  it('takes the remote values in fields the user has not edited', () => {
    const triggered = jasmine.createSpy('triggered');
    const unlisten = session.listen(triggered);
    deliver('persist', remoteRevision());
    unlisten();

    const draft = session._loadedDraft();
    expect(draft.subject).toBe('Remote subject');
    expect(draft.to.map((c) => c.email)).toEqual(['remote@example.com']);
    expect(draft.cc.map((c) => c.email)).toEqual(['cc@example.com']);
    expect(draft.body).toBe('<p>Remote body</p>');
    expect(triggered).toHaveBeenCalled();
  });

  it('keeps the local value of every field edited since the session opened', () => {
    session.changes.add({ subject: 'Typed subject', body: '<p>Typed body</p>' });
    deliver('persist', remoteRevision());

    const draft = session._loadedDraft();
    expect(draft.subject).toBe('Typed subject');
    expect(draft.body).toBe('<p>Typed body</p>');
    expect(draft.to.map((c) => c.email)).toEqual(['remote@example.com']);
  });

  it('keeps the fields a save in flight carries, and takes them once it has landed', async () => {
    let finishSave: () => void = () => {};
    (session.changeSetCommit as any).andCallFake(
      () => new Promise<void>((resolve) => (finishSave = resolve))
    );
    session.changes.add({ subject: 'Typed subject' });
    const committed = session.changes.commit();

    deliver('persist', remoteRevision());
    expect(session._loadedDraft().subject).toBe('Typed subject');

    finishSave();
    await committed;
    deliver('persist', remoteRevision({ subject: 'Later remote subject' }));
    expect(session._loadedDraft().subject).toBe('Later remote subject');
  });

  it('keeps a body the user edits after the last save', async () => {
    session.changes.add({ body: '<p>Saved body</p>' });
    await session.changes.commit();
    session.changes.add({ body: '<p>Saved body, and more</p>' });

    deliver('persist', remoteRevision());
    expect(session._loadedDraft().body).toBe('<p>Saved body, and more</p>');
    expect(session._loadedDraft().subject).toBe('Remote subject');
  });

  it('ignores a sent copy with the same Message-ID', () => {
    deliver('persist', remoteRevision({ draft: false }));
    expect(session._loadedDraft().subject).toBe('Local subject');
  });

  it('keeps the session across a re-key and follows the new id', () => {
    session.changes.add({ body: '<p>Typed body</p>' });
    const rekeyed = remoteRevision({ id: 'new-id', subject: 'Local subject' });
    // The re-keyed row's delta has no body: the engine sends one only with the save that set it.
    (rekeyed as any).body = undefined;

    deliver('persist', rekeyed);
    deliver('unpersist', localDraft());

    expect(DraftStore._draftSessions[HMID]).toBe(session);
    expect(session._destroyed).toBe(false);
    const draft = session._loadedDraft();
    expect(draft.id).toBe('new-id');
    expect(draft.body).toBe('<p>Typed body</p>');
    expect(draft.subject).toBe('Local subject');
  });
});
