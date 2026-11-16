// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import { Thread } from '../../src/data/models/thread';
import { Message } from '../../src/data/models/message';
import { Contact } from '../../src/data/models/contact';
import DatabaseStore from '../../src/data/database/store';
import EngineReads, {
  EngineUnavailableError,
  READ_WAIT_MS,
} from '../../src/data/database/engine-reads';
import { MockEngine, settle } from '../support/mock-engine';
import { RpcError } from '../../src/engine/protocol';

const thread = (id: string, ts: number, extra = {}) =>
  new Thread({
    id,
    accountId: TEST_ACCOUNT_ID,
    subject: `Subject ${id}`,
    lastMessageReceivedTimestamp: new Date(ts * 1000),
    unread: false,
    starred: false,
    ...extra,
  });

describe('DatabaseStore on the engine', function () {
  let engine: MockEngine = null;

  beforeEach(async function () {
    engine = new MockEngine().install();
    engine.add(
      thread('t1', 100, { starred: true }),
      thread('t2', 300),
      thread('t3', 200, { starred: true, unread: true })
    );
    engine.add(
      new Message({
        id: 'm1',
        accountId: TEST_ACCOUNT_ID,
        threadId: 't1',
        date: new Date(1000),
        body: '<p>one</p>',
      }),
      new Message({ id: 'm2', accountId: TEST_ACCOUNT_ID, threadId: 't1', date: new Date(2000) })
    );
    // The app's own subscriptions run again on the hello (the 'resync' after an earlier spec's).
    await settle();
    engine.requests = [];
    (DatabaseStore._query as jasmine.Spy).reset();
  });

  afterEach(function () {
    engine.uninstall();
  });

  it('runs find, findAll, count and idsOnly as the query method', async function () {
    const one = await DatabaseStore.find<Thread>(Thread, 't2');
    expect(engine.queries().length).toBe(1);
    expect(one instanceof Thread).toBe(true);
    expect(one.subject).toBe('Subject t2');

    const starred = await DatabaseStore.findAll<Thread>(Thread).where(
      Thread.attributes.starred.equal(true)
    );
    // The natural order, newest first.
    expect(starred.map((t) => t.id)).toEqual(['t3', 't1']);

    expect(await DatabaseStore.count(Thread, [Thread.attributes.unread.equal(true)])).toBe(1);
    const ids = await DatabaseStore.findAll<Thread>(Thread).limit(2).idsOnly();
    expect(ids as unknown).toEqual(['t2', 't3']);

    expect(DatabaseStore._query).not.toHaveBeenCalled();
    const sent = engine.queries('Thread');
    expect(sent[0]).toEqual(DatabaseStore.find<Thread>(Thread, 't2').toJSON());
    expect(sent[2].count).toBe(true);
    expect(sent[3].idsOnly).toBe(true);
  });

  it('loads included bodies, and null for one the engine has not downloaded', async function () {
    const messages = await DatabaseStore.findAll<Message>(Message, { threadId: 't1' }).include(
      Message.attributes.body
    );
    expect(messages.map((m) => [m.id, m.body])).toEqual([
      ['m1', '<p>one</p>'],
      ['m2', null],
    ]);
    const bare = await DatabaseStore.find<Message>(Message, 'm1');
    expect(bare.body).toBeUndefined();
  });

  it('sends a plain search() as search without the grammar', async function () {
    engine.add(new Contact({ id: 'c1', accountId: TEST_ACCOUNT_ID, email: 'ann@example.com' }));
    const found = await DatabaseStore.findAll<Contact>(Contact).search('ann');
    expect(found.map((c) => c.id)).toEqual(['c1']);
    expect(engine.queries('Contact')[0].search).toEqual({ text: 'ann', grammar: false });
  });

  it('reads SQLite while the flag is off, but for structuredSearch', async function () {
    const sqlite = DatabaseStore._query as jasmine.Spy;
    const threadReads = () =>
      sqlite.calls.filter((c) => (c.args[0] as string).includes('FROM `Thread`')).length;

    (EngineReads._flag as jasmine.Spy).andReturn('sqlite');
    await DatabaseStore.find<Thread>(Thread, 't1');
    expect(engine.queries('Thread')).toEqual([]);
    expect(threadReads()).toBe(1);

    // The engine reads the search grammar whatever the flag says.
    const { SearchQueryParser } = require('../../src/mail/search/parser');
    const found = await DatabaseStore.findAll<Thread>(Thread).structuredSearch(
      SearchQueryParser.parse('t2')
    );
    expect(found.map((t) => t.id)).toEqual(['t2']);
    await DatabaseStore.findAll<Thread>(Thread).structuredSearch('in:inbox');
    expect(engine.queries('Thread').map((q) => [q.search?.text, q.search?.grammar])).toEqual([
      ['t2', true],
      ['in:inbox', true],
    ]);
    expect(threadReads()).toBe(1);
  });

  it('rejects with the engine error for a query the engine refuses, without retrying', async function () {
    const query = DatabaseStore.findAll<Thread>(Thread).order(
      Thread.attributes.subject.ascending()
    );
    spyOn(engine, 'answer').andCallFake(() => {
      throw new RpcError({ code: -32602, message: 'invalid params: subject is not queryable' });
    });
    let error: any = null;
    await query.run().catch((err) => (error = err));
    expect(error.message).toContain('not queryable');
    expect(error.code).toBe(-32602);
    expect(engine.queries('Thread').length).toBe(1);
  });

  describe('while the engine is away', function () {
    it('waits for the next hello', async function () {
      engine.kill();
      let result: Thread = null;
      DatabaseStore.find<Thread>(Thread, 't3').then((t) => (result = t));
      await settle();
      expect(result).toBe(null);

      // Short of the wait by more than the real clock can move while the read sets its timer.
      advanceClock(READ_WAIT_MS - 100);
      engine.start();
      await settle();
      expect(result.id).toBe('t3');
      expect(engine.queries('Thread').length).toBe(1);
    });

    it('rejects with EngineUnavailable after the wait', async function () {
      engine.kill();
      let error: Error = null;
      DatabaseStore.find<Thread>(Thread, 't3')
        .run()
        .catch((err) => (error = err));
      await settle();
      advanceClock(READ_WAIT_MS);
      await settle();
      expect(error instanceof EngineUnavailableError).toBe(true);
      expect(error.name).toBe('EngineUnavailable');
    });

    it('sends a read again when the connection drops under it', async function () {
      let result: Thread = null;
      DatabaseStore.find<Thread>(Thread, 't1').then((t) => (result = t));
      // The request is out; the socket closes before the answer.
      engine.kill();
      await settle();
      expect(result).toBe(null);
      engine.start();
      await settle();
      expect(result.id).toBe('t1');
      expect(engine.queries('Thread').length).toBe(2);
    });
  });
});
