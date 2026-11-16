// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import { Thread } from '../../src/data/models/thread';
import { Folder } from '../../src/data/models/folder';
import { Message } from '../../src/data/models/message';
import { Contact } from '../../src/data/models/contact';
import DatabaseStore from '../../src/data/database/store';
import { DatabaseChangeRecord } from '../../src/data/database/change-record';
import EngineReads from '../../src/data/database/engine-reads';
import EngineEvents from '../../src/engine/client/events';
import QuerySubscriptionPool from '../../src/data/query/subscription-pool';
import { QuerySubscription } from '../../src/data/query/subscription';
import { MutableQuerySubscription } from '../../src/data/query/mutable-subscription';
import UnreadQuerySubscription from '../../src/data/query/unread-subscription';
import { QueryResultSet } from '../../src/data/query/result-set';
import { ChangeUnreadTask } from '../../src/data/tasks/change-unread';
import * as Actions from '../../src/data/actions';
import ThreadListDataSource from '../../src/features/thread-list/data-source';
import { getTokens } from '../../src/features/thread-list/participants';
import { listMessageCount, listSnippet } from '../../src/features/thread-list/summary';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import { Method } from '../../src/engine/protocol';
import { MockEngine, settle } from '../support/mock-engine';

const inbox = new Folder({ id: 'inbox', accountId: TEST_ACCOUNT_ID, role: 'inbox', path: 'INBOX' });

const thread = (id: string, ts: number, extra = {}) =>
  new Thread({
    id,
    accountId: TEST_ACCOUNT_ID,
    subject: `Subject ${id}`,
    folders: [inbox],
    categories: [inbox],
    inAllMail: true,
    unread: false,
    starred: false,
    lastMessageReceivedTimestamp: new Date(ts * 1000),
    ...extra,
  });

const ids = (models: { id: string }[]) => models.map((m) => m.id);

describe('query subscriptions on live queries', function () {
  let engine: MockEngine = null;
  const offs: Array<() => void> = [];

  beforeEach(async function () {
    engine = MockEngine.live().install();
    engine.add(thread('t1', 100), thread('t2', 300, { unread: true }), thread('t3', 200));
    await settle();
    engine.requests = [];
  });

  afterEach(function () {
    while (offs.length) offs.pop()();
    engine.uninstall();
    advanceClock(10);
  });

  const listen = (query, results = []) => {
    offs.push(QuerySubscriptionPool.add(query, (r) => results.push(r)));
    return results;
  };
  const sent = (method: string) => engine.requests.filter((r) => r.method === method);

  it('subscribes with the first callback and follows the engine, reading nothing itself', async function () {
    const query = DatabaseStore.findAll<Thread>(Thread).where({ inAllMail: true });
    const subscription = new QuerySubscription<Thread>(query);
    await settle();
    expect(sent(Method.QuerySubscribe)).toEqual([]);

    const results: Thread[][] = [];
    const off = QuerySubscriptionPool.addPrivateSubscription('live-spec', subscription, (r) =>
      results.push(r)
    );
    await settle();
    expect(sent(Method.QuerySubscribe).length).toBe(1);
    expect(sent(Method.QuerySubscribe)[0].params.window).toBeNull();
    expect(ids(results.pop())).toEqual(['t2', 't3', 't1']);

    engine.add(thread('t4', 400));
    await settle();
    expect(ids(results.pop())).toEqual(['t4', 't2', 't3', 't1']);
    expect(subscription.liveTotal()).toBe(4);

    // A raw delta for a thread the engine doesn't have changes nothing here.
    DatabaseStore.trigger(
      new DatabaseChangeRecord({
        type: 'persist',
        objectClass: 'Thread',
        objects: [thread('t9', 999)],
        objectsRawJSON: [],
      })
    );
    await settle();
    expect(results).toEqual([]);
    expect(engine.queries()).toEqual([]);

    off();
    await settle();
    expect(sent(Method.QueryUnsubscribe).length).toBe(1);
    expect(engine.subscriptions().filter((q) => q.class === 'Thread')).toEqual([]);
  });

  it('keeps the models of rows that did not change', async function () {
    const results = listen(DatabaseStore.findAll<Thread>(Thread));
    await settle();
    const before = results.pop();
    engine.add(thread('t3', 200, { subject: 'Renamed' }));
    await settle();
    const after = results.pop();
    expect(after.find((t) => t.id === 't3').subject).toBe('Renamed');
    // MockEngine resets the window, so every row is new to it; one() picks a row by id.
    expect(after.length).toBe(before.length);
  });

  it('moves the window with the range', async function () {
    const subscription = new MutableQuerySubscription<Thread>(
      DatabaseStore.findAll<Thread>(Thread).limit(0),
      { emitResultSet: true }
    );
    const results: QueryResultSet<Thread>[] = [];
    offs.push(
      QuerySubscriptionPool.addPrivateSubscription('live-window', subscription, (r) =>
        results.push(r)
      )
    );
    await settle();
    subscription.replaceRange({ start: 1, end: 2 });
    await settle();
    expect(sent(Method.QuerySetWindow).map((r) => r.params.window)).toEqual([
      { offset: 0, limit: 200 },
    ]);
    const set = results.pop();
    expect(set._live).toBe(true);
    expect(set.ids()).toEqual(['t2', 't3', 't1']);
    expect(set.offsetOfId('t3')).toBe(1);
    expect(engine.queries()).toEqual([]);
  });

  it('subscribes again after a reconnect', async function () {
    const results = listen(DatabaseStore.findAll<Thread>(Thread));
    await settle();
    engine.kill();
    engine.remove(Thread, 't2');
    engine.start();
    await settle();
    expect(ids(results.pop())).toEqual(['t3', 't1']);
    const mine = sent(Method.QuerySubscribe).filter((r) => r.params.query.class === 'Thread');
    expect(mine.length).toBe(2);
    expect(mine[1].params.key).toBe(mine[0].params.key);
  });

  it('reads and patches from deltas when core.engine.reads is sqlite', async function () {
    spyOn(EngineReads, 'usesEngine').andReturn(false);
    (DatabaseStore._query as jasmine.Spy).reset();
    listen(DatabaseStore.findAll<Thread>(Thread).where({ starred: true }));
    await settle();
    expect(sent(Method.QuerySubscribe)).toEqual([]);
    expect(DatabaseStore._query).toHaveBeenCalled();
  });

  it('keeps a just-read thread in the Unread view by subscribing to the whitelist', async function () {
    const subscription = new UnreadQuerySubscription(['inbox']);
    const results: QueryResultSet<Thread>[] = [];
    offs.push(
      QuerySubscriptionPool.addPrivateSubscription('unread-live', subscription, (r) =>
        results.push(r)
      )
    );
    subscription.replaceRange({ start: 0, end: 50 });
    await settle();
    expect(results.pop().ids()).toEqual(['t2']);

    const t2 = engine.models.get('Thread').find((t) => t.id === 't2') as Thread;
    Actions.queueTask(new ChangeUnreadTask({ threads: [t2], unread: false, source: 'spec' }));
    engine.add(thread('t2', 300, { unread: false }));
    await settle();
    const subscribes = sent(Method.QuerySubscribe).filter((r) => r.params.query.class === 'Thread');
    expect(subscribes.length).toBe(2);
    expect(subscribes[1].params.key).toBe(subscribes[0].params.key);
    expect(JSON.stringify(subscribes[1].params.query.where)).toContain('"t2"');
    expect(results.pop().ids()).toEqual(['t2']);
  });
});

describe('the thread list on thread summaries', function () {
  let engine: MockEngine = null;
  let dataSource: ThreadListDataSource = null;

  beforeEach(async function () {
    engine = MockEngine.live().install();
    const ann = new Contact({ email: 'ann@example.com', name: 'Ann' });
    const bob = new Contact({ email: 'bob@example.com', name: 'Bob' });
    for (let i = 0; i < 3; i++) {
      engine.add(thread(`t${i}`, 1000 - i, { snippet: 'thread snippet' }));
      for (let j = 0; j < i + 1; j++) {
        engine.add(
          new Message({
            id: `m${i}-${j}`,
            accountId: TEST_ACCOUNT_ID,
            threadId: `t${i}`,
            date: new Date((100 + j) * 1000),
            from: [j % 2 ? bob : ann],
            to: [],
            unread: j === i,
            draft: false,
            files: [],
            snippet: `snippet ${i}-${j}`,
          })
        );
      }
    }
    await settle();
    engine.requests = [];
    dataSource = new ThreadListDataSource(MailboxPerspective.forCategory(inbox).threads());
    dataSource.setRetainedRange({ start: 0, end: 20 });
    await settle();
  });

  afterEach(function () {
    dataSource.cleanup();
    engine.uninstall();
    advanceClock(10);
  });

  it('reads one live query with summaries and no messages', function () {
    const subscribes = engine.requests.filter(
      (r) => r.method === Method.QuerySubscribe && r.params.query.class === 'Thread'
    );
    expect(subscribes.length).toBe(1);
    expect(subscribes[0].params.query.include).toEqual(['summary']);
    expect(engine.queries('Message')).toEqual([]);
    expect(engine.subscriptions().filter((q) => q.class === 'Message')).toEqual([]);

    const row = dataSource.getById('t2') as any;
    expect(listMessageCount(row)).toBe(3);
    expect(listSnippet(row)).toBe('snippet 2-2');
    const tokens = getTokens(row) as any[];
    expect(tokens.map((t) => t.contact.name)).toEqual(['Ann', 'Bob', 'Ann']);
    expect(tokens.map((t) => t.unread)).toEqual([false, false, true]);
  });

  it('loads __messages for v1 plugins when one reads it', async function () {
    const row = dataSource.getById('t1') as any;
    expect(row.__messages).toEqual([]);
    advanceClock(1);
    await settle();
    const messageSubs = engine.subscriptions().filter((q) => q.class === 'Message');
    expect(messageSubs.length).toBe(1);
    expect(JSON.stringify(messageSubs[0].where)).toContain('t1');
    const after = dataSource.getById('t1') as any;
    expect(after.__messages.map((m) => m.id)).toEqual(['m1-0', 'm1-1']);
  });

  it('takes a thread the engine removed out of the selection', async function () {
    dataSource.selection.toggle(dataSource.getById('t0'));
    expect(dataSource.selection.ids()).toEqual(['t0']);
    engine.remove(Thread, 't0');
    await settle();
    expect(dataSource.selection.ids()).toEqual([]);
  });
});

describe('waiting for tasks on the engine', function () {
  it('resolves local and remote waits from the task.queue answer and task.done', async function () {
    const TaskQueue = require('../../src/data/stores/task-queue').default;
    const task = new ChangeUnreadTask({ threadIds: ['t1'], unread: false, source: 'spec' });
    task.accountId = TEST_ACCOUNT_ID;
    const local = TaskQueue.waitForPerformLocal(task);
    const remote = TaskQueue.waitForPerformRemote(task);
    EngineEvents.taskQueued({ ...task.toJSON(), status: 'remote' } as any);
    expect((await local).status).toBe('remote');
    EngineEvents.notification('task.done', {
      accountId: TEST_ACCOUNT_ID,
      taskId: task.id,
      status: 'complete',
      task: { ...task.toJSON(), status: 'complete' },
    });
    const done = await remote;
    expect(done.status).toBe('complete');
    expect(done instanceof ChangeUnreadTask).toBe(true);
  });
});

describe('resync after a reconnect', function () {
  let engine: MockEngine = null;
  beforeEach(function () {
    engine = new MockEngine({ features: ['query', 'delta-seq'] }).install();
  });
  afterEach(function () {
    engine.uninstall();
  });

  it('runs the subscriptions again only when the engine could not replay the deltas', async function () {
    const resync = jasmine.createSpy('resync');
    EngineReads.on('resync', resync);
    engine.kill();
    engine.start();
    EngineEvents.subscribed({ resumed: true });
    expect(resync).not.toHaveBeenCalled();
    engine.kill();
    engine.start();
    expect(resync).not.toHaveBeenCalled();
    EngineEvents.subscribed({ resumed: false });
    expect(resync.callCount).toBe(1);
    EngineReads.removeListener('resync', resync);
    // The reads the resync sent finish here, not on the next spec's engine.
    await settle();
  });
});
