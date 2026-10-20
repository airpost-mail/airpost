import { Thread, Contact, Message, Account, AccountStore, DatabaseStore } from 'mailspring-exports';
import ThreadListContextMenu from '../lib/thread-list-context-menu';

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

describe('ThreadListContextMenu', function threadListContextMenu() {
  beforeEach(() => {
    (AccountStore as any)._accounts = [
      new Account({ id: 'acct', name: 'Me', emailAddress: 'me@example.com', provider: 'gmail' }),
    ];
    (AccountStore as any)._caches = {};

    const me = new Contact({ email: 'me@example.com', name: 'Me' });
    const alice = new Contact({ email: 'alice@example.com', name: 'Alice' });
    const bob = new Contact({ email: 'bob@example.com', name: 'Bob' });

    this.thread = new Thread({ id: 'thread-1', accountId: 'acct', subject: 'Hello' });
    // One-to-one received message, so Reply All doesn't apply to it.
    const received = new Message({
      id: 'received',
      accountId: 'acct',
      threadId: 'thread-1',
      draft: false,
      date: new Date(1000),
      from: [alice],
      to: [me],
      cc: [],
    });
    // A newer unsent draft that has several recipients.
    const draft = new Message({
      id: 'draft',
      accountId: 'acct',
      threadId: 'thread-1',
      draft: true,
      date: new Date(2000),
      from: [me],
      to: [alice, bob],
      cc: [],
    });
    spyOn(DatabaseStore, 'run').andCallFake(runAgainst([this.thread, received, draft]));
  });

  it('decides Reply All from the last received message, not a newer draft', async () => {
    const menu = new ThreadListContextMenu({ threadIds: ['thread-1'], accountIds: ['acct'] });
    menu.threads = [this.thread];
    expect(await menu.replyAllItem()).toBe(null);
  });
});
