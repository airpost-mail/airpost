// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import { Thread } from '../../src/data/models/thread';
import { Message } from '../../src/data/models/message';
import FocusedContentStore from '../../src/data/stores/focused-content';
import { MessageStore, MAX_DEFAULT_EXPANDED_UNREAD } from '../../src/data/stores/message';
import DatabaseStore from '../../src/data/database/store';
import { DatabaseChangeRecord } from '../../src/data/database/change-record';
import * as Actions from '../../src/data/actions';
import { MockEngine, settle } from '../support/mock-engine';

const thread = new Thread({
  id: 'th',
  accountId: TEST_ACCOUNT_ID,
  subject: 'Long thread',
  lastMessageReceivedTimestamp: new Date(10000),
});

const message = (i: number, extra = {}) =>
  new Message({
    id: `m${i}`,
    accountId: TEST_ACCOUNT_ID,
    threadId: 'th',
    date: new Date(i * 1000),
    unread: false,
    draft: false,
    files: [],
    body: `<p>body ${i}</p>`,
    ...extra,
  });

// What a sync change does: the engine's tables change, then its live queries and deltas say so.
let engine: MockEngine = null;
const change = (type: string, objects: any[]) => {
  for (const o of objects) {
    if (type === 'unpersist') engine.remove(o.constructor, o.id);
    else engine.add(o);
  }
  DatabaseStore.trigger(
    new DatabaseChangeRecord({
      type,
      objectClass: objects[0].constructor.name,
      objects,
      objectsRawJSON: objects.map((o) => o.toJSON()),
    })
  );
};

// The thread's messages and the thread come from live queries; only bodies are read.
describe('MessageStore on the engine', function () {
  beforeEach(async function () {
    engine = MockEngine.live().install();
    engine.add(thread);
    // Ten messages, the last six unread.
    for (let i = 1; i <= 10; i++) engine.add(message(i, { unread: i > 4 }));
    spyOn(FocusedContentStore, 'focused').andCallFake((c) => (c === 'thread' ? thread : null));
    spyOn(Actions, 'queueTask');
    spyOn(Actions, 'fetchBodies');
    await settle();
    engine.requests = [];
    MessageStore._thread = null;
    MessageStore._onApplyFocusChange();
    await settle();
  });

  afterEach(function () {
    MessageStore._setStoreDefaults();
    engine.uninstall();
    advanceClock(1000);
  });

  const bodyReads = () => engine.queries('Message').filter((q) => q.include.includes('body'));
  const otherReads = () => engine.queries().filter((q) => !q.include.includes('body'));

  it('follows the messages without bodies, then reads the bodies of the expanded ones', function () {
    const list = engine.subscriptions().find((q) => q.class === 'Message');
    expect(list.include).toEqual([]);
    expect(list.where).toEqual({ and: [{ attr: 'threadId', op: '=', value: 'th' }] });
    const rest = engine.queries('Message');
    expect(rest.length).toBe(1);
    const expanded = Object.keys(MessageStore.itemsExpandedState()).sort();
    // The newest unread messages, capped; the last one is among them.
    expect(expanded).toEqual(['m10', 'm8', 'm9']);
    expect(MAX_DEFAULT_EXPANDED_UNREAD).toBe(3);
    expect(rest[0].where).toEqual({ and: [{ attr: 'id', op: 'in', value: ['m8', 'm9', 'm10'] }] });
    const byId = new Map(MessageStore.items().map((m) => [m.id, m]));
    expect(byId.get('m9').body).toBe('<p>body 9</p>');
    expect(byId.get('m2').body).toBeUndefined();
    expect(MessageStore.itemsLoading()).toBe(false);
  });

  it('patches a message delta in place, keeping the body it does not carry', async function () {
    engine.requests = [];
    const starred = message(9, { unread: false, starred: true, body: undefined });
    change('persist', [starred]);
    await settle();
    const m9 = MessageStore.items().find((m) => m.id === 'm9');
    expect(m9.starred).toBe(true);
    expect(m9.body).toBe('<p>body 9</p>');
    expect(engine.queries()).toEqual([]);

    // A thread delta updates the thread only.
    change('persist', [new Thread({ ...thread, subject: 'Renamed' })]);
    await settle();
    expect(MessageStore.thread().subject).toBe('Renamed');
    expect(engine.queries()).toEqual([]);
  });

  it('adds, orders and removes messages from deltas', async function () {
    engine.requests = [];
    const reply = message(11, { unread: true, body: '<p>new</p>' });
    change('persist', [reply]);
    await settle();
    expect(
      MessageStore.items()
        .map((m) => m.id)
        .slice(-2)
    ).toEqual(['m10', 'm11']);
    expect(MessageStore.itemsExpandedState().m11).toBe('default');

    change('unpersist', [message(3)]);
    await settle();
    expect(MessageStore.items().map((m) => m.id)).not.toContain('m3');
    // A message moved to another thread leaves this one.
    change('persist', [message(5, { threadId: 'other' })]);
    await settle();
    expect(MessageStore.items().map((m) => m.id)).not.toContain('m5');
    // The new message's body: rows never carry one.
    expect(otherReads()).toEqual([]);
    expect(bodyReads().map((q) => (q.where as any).and[0].value)).toEqual(['m11']);
  });

  it('keeps one item per draft when a draft arrives under another id', async function () {
    const draft = (id: string) =>
      message(11, { id, draft: true, headerMessageId: 'draft-hmid@airpost.test', body: '<p>d</p>' });
    change('persist', [draft('local-draft')]);
    await settle();
    change('persist', [draft('engine-draft')]);
    await settle();
    const drafts = MessageStore.items().filter((m) => m.draft);
    expect(drafts.map((m) => m.id)).toEqual(['engine-draft']);
    // A sent copy with the same Message-ID is not a draft and stays.
    change('persist', [message(12, { headerMessageId: 'draft-hmid@airpost.test' })]);
    await settle();
    expect(MessageStore.items().map((m) => m.id)).toContain('m12');
    expect(MessageStore.items().map((m) => m.id)).toContain('engine-draft');
  });

  it('loads a body when a collapsed message is expanded, showing it expanded once loaded', async function () {
    engine.requests = [];
    Actions.toggleMessageIdExpanded('m2');
    expect(MessageStore.itemsExpandedState().m2).toBeUndefined();
    await settle();
    expect(bodyReads().map((q) => q.where)).toEqual([
      { and: [{ attr: 'id', op: '=', value: 'm2' }] },
    ]);
    expect(MessageStore.itemsExpandedState().m2).toBe('explicit');
    expect(MessageStore.items().find((m) => m.id === 'm2').body).toBe('<p>body 2</p>');
  });

  it('loads the rest of the bodies in one read on expand all', async function () {
    engine.requests = [];
    (Actions.fetchBodies as unknown as jasmine.Spy).reset();
    const triggered = jasmine.createSpy('trigger');
    const unlisten = MessageStore.listen(triggered);
    Actions.toggleAllMessagesExpanded();
    // Expanding again before the read returns doesn't ask for the same bodies.
    Actions.toggleMessageIdExpanded('m1');
    Actions.toggleMessageIdExpanded('m1');
    await settle();
    unlisten();
    const reads = bodyReads();
    expect(reads.length).toBe(1);
    expect((reads[0].where as any).and[0].value.sort()).toEqual(
      ['m1', 'm2', 'm3', 'm4', 'm5', 'm6', 'm7'].sort()
    );
    expect(MessageStore.items().every((m) => typeof m.body === 'string')).toBe(true);
    expect(Actions.fetchBodies).not.toHaveBeenCalled();
    expect(triggered.callCount).toBeLessThan(6);
  });

  it('asks the engine only for the expanded bodies it has not downloaded', async function () {
    engine.add(message(12, { unread: true, body: null }));
    engine.joined.set('m12', { body: null });
    MessageStore._thread = null;
    MessageStore._onApplyFocusChange();
    await settle();
    const asked = (Actions.fetchBodies as unknown as jasmine.Spy).calls.map((c) =>
      c.args[0].map((m) => m.id)
    );
    expect(asked).toEqual([['m12']]);
  });

  it('reads the open thread again after a reconnect, keeping the bodies it showed', async function () {
    engine.kill();
    engine.add(message(13, { body: '<p>arrived while away</p>' }));
    engine.requests = [];
    engine.start();
    await settle();
    expect(MessageStore.items().map((m) => m.id)).toContain('m13');
    // m13 is the last message, so its body is read; the others' come from what was shown.
    expect(bodyReads().map((q) => (q.where as any).and[0].value)).toEqual(['m13']);
    expect(MessageStore.items().find((m) => m.id === 'm9').body).toBe('<p>body 9</p>');
  });
});
