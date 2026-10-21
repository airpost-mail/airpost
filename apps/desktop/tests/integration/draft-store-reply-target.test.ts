import { Thread } from '../../src/data/models/thread';
import { Contact } from '../../src/data/models/contact';
import { Message } from '../../src/data/models/message';
import { Account } from '../../src/data/models/account';
import DraftStore from '../../src/data/drafts/store';
import { AccountStore } from '../../src/data/stores/account';
import DatabaseStore from '../../src/data/database/store';
import { MessageStore } from '../../src/data/stores/message';
import { SignatureStore } from '../../src/data/stores/signature';

import DraftFactory from '../../src/data/drafts/factory';
import SignatureComposerExtension from '../../src/features/composer-signature/composer-extension';

// Answers queries from an in-memory set of models, honouring where, order and limit.
function runAgainst(models: any[]) {
  return (query) => {
    let results = models.filter(
      (m) => m instanceof query._klass && query.matchers().every((matcher) => matcher.evaluate(m))
    );
    for (const order of [...query.orderSortDescriptors()].reverse()) {
      const key = order.attr.modelKey;
      const dir = order.direction === 'DESC' ? -1 : 1;
      results = [...results].sort((a, b) => (a[key] > b[key] ? dir : a[key] < b[key] ? -dir : 0));
    }
    const { offset, limit } = query.range();
    if (offset) results = results.slice(offset);
    if (limit) results = results.slice(0, limit);
    return Promise.resolve(query.formatResult(results));
  };
}

describe('DraftStore reply target', function replyTarget() {
  beforeEach(() => {
    (AccountStore as any)._accounts = [
      new Account({ id: 'acct', name: 'Me', emailAddress: 'me@example.com', provider: 'gmail' }),
    ];
    (AccountStore as any)._caches = {};

    const me = new Contact({ email: 'me@example.com', name: 'Me' });
    const alice = new Contact({ email: 'alice@example.com', name: 'Alice' });
    const bob = new Contact({ email: 'bob@example.com', name: 'Bob' });

    this.thread = new Thread({ id: 'thread-1', accountId: 'acct', subject: 'Hello' });
    this.received = new Message({
      id: 'received',
      accountId: 'acct',
      threadId: 'thread-1',
      headerMessageId: 'received@example.com',
      subject: 'Hello',
      draft: false,
      date: new Date(1000),
      from: [alice],
      to: [me],
      cc: [],
      body: 'original',
    });
    // An unsent reply to `received`, newer than it.
    this.replyDraft = new Message({
      id: 'reply-draft',
      accountId: 'acct',
      threadId: 'thread-1',
      headerMessageId: 'reply-draft@example.com',
      replyToHeaderMessageId: 'received@example.com',
      subject: 'Re: Hello',
      draft: true,
      date: new Date(2000),
      from: [me],
      to: [alice, bob],
      cc: [],
      body: 'unsent',
    });

    spyOn(DatabaseStore, 'run').andCallFake(
      runAgainst([this.thread, this.received, this.replyDraft])
    );
    spyOn(DraftStore, '_finalizeAndPersistNewMessage').andCallFake((draft) =>
      Promise.resolve({ headerMessageId: draft.headerMessageId, draft })
    );
    spyOn(DraftFactory, 'createDraftForReply').andReturn(
      Promise.resolve(new Message({ id: 'new', headerMessageId: 'new@example.com', draft: true }))
    );
    spyOn(DraftFactory, 'createDraftForForward').andReturn(
      Promise.resolve(new Message({ id: 'fwd', headerMessageId: 'fwd@example.com', draft: true }))
    );
  });

  afterEach(() => {
    DraftStore._cleanupAllSessions();
  });

  it('replies to the last received message, not a newer draft, given only a threadId', async () => {
    this.replyDraft.replyToHeaderMessageId = undefined;
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      behavior: 'prefer-existing',
    });
    const call = (DraftFactory.createDraftForReply as any).mostRecentCall.args[0];
    expect(call.message.id).toEqual('received');
    expect(call.message.body).toEqual('original');
    expect(call.thread.id).toEqual('thread-1');
  });

  it('reopens the reply draft the user already edited instead of making a new one', async () => {
    this.replyDraft.pristine = false;
    const result = await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      behavior: 'prefer-existing',
    });
    expect(DraftFactory.createDraftForReply).not.toHaveBeenCalled();
    expect(DraftStore._finalizeAndPersistNewMessage).not.toHaveBeenCalled();
    expect(result.headerMessageId).toEqual('reply-draft@example.com');
    expect(DraftStore._draftSessions['reply-draft@example.com'].draft().body).toEqual('unsent');
  });

  it('keeps the session the composers already hold, and runs no new-draft hooks on it', async () => {
    spyOn(SignatureStore, 'signatureForEmail').andReturn({
      id: 'sig-1',
      body: '<div>Signed</div>',
    });
    const session = await DraftStore.sessionForClientId('reply-draft@example.com');
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      behavior: 'prefer-existing',
    });
    expect(DraftStore._draftSessions['reply-draft@example.com']).toBe(session);
    expect(session.draft().body).not.toContain('Signed');
  });

  it('reopens a reply draft that answers an older message of the thread, keeping its recipients', async () => {
    const newer = new Message({
      ...this.received,
      id: 'received-2',
      headerMessageId: 'received-2@example.com',
      date: new Date(3000),
      from: [new Contact({ email: 'carol@example.com', name: 'Carol' })],
    });
    (DatabaseStore.run as any).andCallFake(
      runAgainst([this.thread, this.received, newer, this.replyDraft])
    );
    const session = await DraftStore.sessionForClientId('reply-draft@example.com');
    spyOn(session.changes, 'add');
    const result = await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply-all',
      behavior: 'prefer-existing',
    });
    expect(DraftFactory.createDraftForReply).not.toHaveBeenCalled();
    expect(result.headerMessageId).toEqual('reply-draft@example.com');
    expect(session.changes.add).not.toHaveBeenCalled();
  });

  it('switches the draft to Reply All through its session', async () => {
    this.received.cc = [new Contact({ email: 'dave@example.com', name: 'Dave' })];
    this.replyDraft.to = [new Contact({ email: 'alice@example.com', name: 'Alice' })];
    const session = await DraftStore.sessionForClientId('reply-draft@example.com');
    spyOn(session.changes, 'add').andCallThrough();
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply-all',
      behavior: 'prefer-existing',
    });
    const changes = (session.changes.add as any).mostRecentCall.args[0];
    expect(changes.cc.map((c) => c.email)).toEqual(['dave@example.com']);
  });

  it('focuses the draft in the open thread, or pops it out when asked', async () => {
    spyOn(MessageStore, 'threadId').andReturn('thread-1');
    spyOn(MessageStore, 'items').andReturn([this.received, this.replyDraft]);
    spyOn(MessageStore, 'focusDraft');
    spyOn(DraftStore, '_onPopoutDraft').andReturn(Promise.resolve());
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      behavior: 'prefer-existing',
    });
    expect(MessageStore.focusDraft).toHaveBeenCalledWith('reply-draft@example.com');
    expect(DraftStore._onPopoutDraft).not.toHaveBeenCalled();

    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      popout: true,
      behavior: 'prefer-existing',
    });
    expect(DraftStore._onPopoutDraft).toHaveBeenCalledWith('reply-draft@example.com');
    expect(DraftFactory.createDraftForReply).not.toHaveBeenCalled();
  });

  it('still makes a new reply when only a pristine draft may be reused and this one is edited', async () => {
    this.replyDraft.pristine = false;
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      behavior: 'prefer-existing-if-pristine',
    });
    expect(DraftFactory.createDraftForReply).toHaveBeenCalled();
  });

  it('does not reopen a forward as the reply', async () => {
    this.replyDraft.replyToHeaderMessageId = undefined;
    this.replyDraft.forwardedHeaderMessageId = 'received@example.com';
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      behavior: 'prefer-existing',
    });
    expect(DraftFactory.createDraftForReply).toHaveBeenCalled();
  });

  it('forwards the last received message, not a newer draft, given only a threadId', async () => {
    await DraftStore._onComposeForward({ threadId: 'thread-1' });
    const call = (DraftFactory.createDraftForForward as any).mostRecentCall.args[0];
    expect(call.message.id).toEqual('received');
  });

  it('resurfaces against the last received message, not a newer draft', async () => {
    spyOn(DraftFactory, 'createDraft').andCallFake((fields) => Promise.resolve(fields));
    const draft: any = await DraftFactory.createDraftForResurfacing(this.thread, null, 'body');
    expect(draft.replyToHeaderMessageId).toEqual('received@example.com');
  });
});
