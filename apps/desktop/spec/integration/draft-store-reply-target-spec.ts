import {
  Thread,
  Contact,
  Message,
  Account,
  DraftStore,
  AccountStore,
  DatabaseStore,
  MessageStore,
  SignatureStore,
} from 'mailspring-exports';

import DraftFactory from '../../src/flux/stores/draft-factory';
import SignatureComposerExtension from '../../features/composer-signature/lib/signature-composer-extension';

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
    spyOn(DraftFactory, 'createOrUpdateDraftForReply').andCallThrough();
    spyOn(DraftFactory, 'createDraftForReply').andReturn(
      Promise.resolve(new Message({ id: 'new', headerMessageId: 'new@example.com', draft: true }))
    );
    spyOn(DraftFactory, 'createDraftForForward').andReturn(
      Promise.resolve(new Message({ id: 'fwd', headerMessageId: 'fwd@example.com', draft: true }))
    );
  });

  it('replies to the last received message, not a newer draft, given only a threadId', async () => {
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      behavior: 'prefer-existing-if-pristine',
    });
    const call = (DraftFactory.createOrUpdateDraftForReply as any).mostRecentCall.args[0];
    expect(call.message.id).toEqual('received');
    expect(call.message.body).toEqual('original');
    expect(call.thread.id).toEqual('thread-1');
  });

  it('reuses the existing reply draft to that message with prefer-existing', async () => {
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply',
      behavior: 'prefer-existing',
    });
    expect(DraftFactory.createDraftForReply).not.toHaveBeenCalled();
    const finalized = (DraftStore._finalizeAndPersistNewMessage as any).mostRecentCall.args[0];
    expect(finalized.headerMessageId).toEqual('reply-draft@example.com');
  });

  it('loads the body of an existing reply draft the open thread holds without one', async () => {
    // MessageStore keeps bodies of expanded messages only (TD40).
    const collapsed = new Message(this.replyDraft);
    collapsed.body = undefined;
    spyOn(MessageStore, 'threadId').andReturn('thread-1');
    spyOn(MessageStore, 'items').andReturn([this.received, collapsed]);
    spyOn(SignatureStore, 'signatureForEmail').andReturn({
      id: 'sig-1',
      body: '<div>Signed</div>',
    });
    (DraftStore._finalizeAndPersistNewMessage as any).andCallFake((draft) => {
      SignatureComposerExtension.prepareNewDraft({ draft });
      return Promise.resolve({ headerMessageId: draft.headerMessageId, draft });
    });
    await DraftStore._onComposeReply({
      threadId: 'thread-1',
      type: 'reply-all',
      behavior: 'prefer-existing',
    });
    expect(DraftFactory.createDraftForReply).not.toHaveBeenCalled();
    const finalized = (DraftStore._finalizeAndPersistNewMessage as any).mostRecentCall.args[0];
    expect(finalized.headerMessageId).toEqual('reply-draft@example.com');
    expect(finalized.body).toContain('unsent');
    expect(finalized.body).toContain('Signed');
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
