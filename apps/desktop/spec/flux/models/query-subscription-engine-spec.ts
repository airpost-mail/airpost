// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { Thread } from '../../../src/flux/models/thread';
import { Folder } from '../../../src/flux/models/folder';
import DatabaseStore from '../../../src/flux/stores/database-store';
import { DatabaseChangeRecord } from '../../../src/flux/stores/database-change-record';
import QuerySubscriptionPool from '../../../src/flux/models/query-subscription-pool';
import { QuerySubscription } from '../../../src/flux/models/query-subscription';
import UnreadQuerySubscription from '../../../src/flux/models/unread-query-subscription';
import { READ_WAIT_MS } from '../../../src/flux/stores/engine-reads';
import { MockEngine, settle } from '../../mock-engine';

const inbox = new Folder({ id: 'inbox', accountId: TEST_ACCOUNT_ID, role: 'inbox', path: 'INBOX' });

const thread = (id: string, ts: number, extra = {}) =>
  new Thread({
    id,
    accountId: TEST_ACCOUNT_ID,
    subject: `Subject ${id}`,
    folders: [inbox],
    inAllMail: true,
    unread: false,
    starred: false,
    lastMessageReceivedTimestamp: new Date(ts * 1000),
    ...extra,
  });

const persist = (...objects: Thread[]) =>
  DatabaseStore.trigger(
    new DatabaseChangeRecord({
      type: 'persist',
      objectClass: 'Thread',
      objects,
      objectsRawJSON: objects.map((o) => o.toJSON()),
    })
  );

describe('query subscriptions on the engine', function () {
  let engine: MockEngine = null;
  let unsubscribe: () => void = null;

  beforeEach(async function () {
    engine = new MockEngine().install();
    engine.add(thread('t1', 100), thread('t2', 300, { unread: true }), thread('t3', 200));
    await settle();
    engine.requests = [];
  });

  afterEach(function () {
    if (unsubscribe) unsubscribe();
    unsubscribe = null;
    engine.uninstall();
    advanceClock(10);
  });

  const listen = (query) => {
    const results = [];
    unsubscribe = QuerySubscriptionPool.add(query, (r) => results.push(r));
    return results;
  };

  it('keys the pool on the JSON form of the query', function () {
    const query = DatabaseStore.findAll<Thread>(Thread).where({ starred: true });
    expect(QuerySubscriptionPool._keyForQuery(query)).toBe(JSON.stringify(query.toJSON()));
  });

  it('reads through query and applies deltas in the renderer', async function () {
    const results = listen(DatabaseStore.findAll<Thread>(Thread).where({ inAllMail: true }));
    await settle();
    expect(results.pop().map((t) => t.id)).toEqual(['t2', 't3', 't1']);
    expect(engine.queries('Thread').length).toBe(1);

    // A change that keeps the order is applied without a read.
    persist(thread('t3', 200, { subject: 'Renamed' }));
    await settle();
    expect(results.pop().find((t) => t.id === 't3').subject).toBe('Renamed');
    expect(engine.queries('Thread').length).toBe(1);

    // A new thread changes the set, so the ids are read again.
    const t4 = thread('t4', 400);
    engine.add(t4);
    persist(t4);
    await settle();
    expect(results.pop().map((t) => t.id)).toEqual(['t4', 't2', 't3', 't1']);
    expect(engine.queries('Thread').length).toBe(2);
  });

  it('re-runs after a reconnect, since the deltas sent meanwhile are lost', async function () {
    const results = listen(DatabaseStore.findAll<Thread>(Thread).where({ inAllMail: true }));
    await settle();
    expect(results.pop().map((t) => t.id)).toEqual(['t2', 't3', 't1']);

    engine.kill();
    // While away the engine archived t2 and renamed t1; no delta reaches the window.
    engine.remove(Thread, 't2');
    engine.add(thread('t1', 100, { subject: 'Changed while away' }));
    engine.start();
    await settle();
    const after = results.pop();
    expect(after.map((t) => t.id)).toEqual(['t3', 't1']);
    expect(after[1].subject).toBe('Changed while away');
  });

  it('keeps the last result when a read gives up, and runs again on the next hello', async function () {
    const query = DatabaseStore.findAll<Thread>(Thread).where({ inAllMail: true });
    const results = listen(query);
    await settle();
    const first = results.pop();
    expect(first.length).toBe(3);

    engine.kill();
    const t4 = thread('t4', 400);
    engine.add(t4);
    // A delta that changes the set needs a read, which waits and then fails.
    spyOn(console, 'warn');
    persist(t4);
    await settle();
    advanceClock(READ_WAIT_MS);
    await settle();
    expect(console.warn).toHaveBeenCalled();
    expect(results).toEqual([]);
    const subscription = QuerySubscriptionPool._subscriptions[
      QuerySubscriptionPool._keyForQuery(query)
    ] as QuerySubscription<Thread>;
    expect(subscription._updateInFlight).toBe(false);

    engine.start();
    await settle();
    expect(results.pop().map((t) => t.id)).toEqual(['t4', 't2', 't3', 't1']);
  });

  it('runs the Unread view on the engine', async function () {
    const subscription = new UnreadQuerySubscription(['inbox']);
    const results = [];
    unsubscribe = QuerySubscriptionPool.addPrivateSubscription('unread', subscription, (r) =>
      results.push(r)
    );
    subscription.replaceRange({ start: 0, end: 50 });
    await settle();
    expect(
      results
        .pop()
        .models()
        .map((t) => t.id)
    ).toEqual(['t2']);
    const sent = engine.queries('Thread').pop();
    expect((sent.where as any).and[0]).toEqual({
      attr: 'categories',
      op: 'containsAny',
      value: ['inbox'],
    });
  });
});
