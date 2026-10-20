// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import net from 'net';
import { ipcRenderer } from 'electron';
import DatabaseStore from '../src/flux/stores/database-store';
import { ThreadCountsStore } from '../src/flux/stores/thread-counts-store';
import DefaultEngineEvents, { EngineEvents } from '../src/engine/engine-events';
import { EngineConnection } from '../src/engine/engine-connection';
import { Feature } from '../src/shared/engine-protocol';
import { applyDeltas, feedEngineEvents } from '../src/flux/engine-bridge';
import { Notifier } from '../features/unread-notifications/lib/main';
import { EngineStatus } from '../src/shared/supervisor-channels';
import { FakeEngine, until } from './fake-serve-engine';
import { EngineReads } from '../src/flux/stores/engine-reads';

const hello = (features: string[]) => ({
  protocol: 1,
  engine: 'fake',
  pid: 1,
  accounts: [],
  features,
});

const countsDelta = (categoryId: string, unread: number, total: number) => ({
  accountId: 'a1',
  modelClass: 'ThreadCounts',
  type: 'persist',
  modelJSONs: [{ id: categoryId, categoryId, unread, total }],
});

describe('stage 1 engine events on a window connection', function () {
  let dir: string = null;
  let fake: FakeEngine = null;
  let connection: EngineConnection = null;
  let events: EngineEvents = null;

  beforeEach(async function () {
    dir = fs.mkdtempSync('/tmp/tern-spec-');
    fake = new FakeEngine(dir);
    await fake.start();
    events = new EngineEvents();
    spyOn(console, 'log');
    spyOn(console, 'warn');
    spyOn(AppEnv, 'isMainWindow').andReturn(true);
    // ThreadCounts deltas go to the window's EngineEvents; this spec's stands in for it.
    spyOn(DefaultEngineEvents, 'threadCounts').andCallFake((delta) => events.threadCounts(delta));
  });

  afterEach(function () {
    if (connection) connection.close();
    connection = null;
    fake.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const open = (socketPath = fake.socketPath, { mainWindow = true } = {}) => {
    connection = new EngineConnection({ clientName: 'tern-desktop/spec', mainWindow });
    feedEngineEvents(connection, events);
    connection.on('deltas', (msgs) => applyDeltas(msgs, { mainWindow }));
    const status: EngineStatus = {
      generation: 1,
      socketPath,
      accounts: { a1: { state: 'added', held: false } },
    };
    connection.setStatus(status);
    return connection;
  };

  const counts = (store: ThreadCountsStore, id: string) => [
    store.unreadCountForCategoryId(id),
    store.totalCountForCategoryId(id),
  ];

  it('gives ThreadCountsStore the hello features, counts.get on the connection and the deltas after it', async function () {
    (DatabaseStore._query as any).andCallFake(() =>
      Promise.resolve([{ categoryId: 'c1', unread: 1, total: 2 }])
    );
    spyOn(DatabaseStore, 'trigger');
    spyOn(AppEnv, 'reportError');
    fake.socketResults.hello = hello([Feature.Counts, Feature.MailNew]);
    fake.socketResults['counts.get'] = { counts: { c1: { unread: 3, total: 5 } } };
    const store = new ThreadCountsStore(events);

    open();
    await until('counts.get', () => store.unreadCountForCategoryId('c1') === 3);
    expect(events.has(Feature.Counts)).toBe(true);
    expect(fake.connRequests[0].map((r) => r.method)).toContain('counts.get');

    fake.publishDelta(countsDelta('c1', 1, 5));
    await until('the delta', () => store.unreadCountForCategoryId('c1') === 1);
    expect(counts(store, 'c1')).toEqual([1, 5]);
    // Taken out before the models: never inflated, never a DatabaseStore change.
    expect(DatabaseStore.trigger).not.toHaveBeenCalled();
    expect(AppEnv.reportError).not.toHaveBeenCalled();

    // The connection drops: no features or requests until the next hello.
    fake.notify('disconnect', { reason: 'slow consumer' }, fake.conns[0]);
    await until('the drop', () => connection._socket === null);
    expect(events.features).toBe(null);
    expect(store._fromEngine).toBe(false);
    const refused = await events.request('counts.get', {}).then(
      () => false,
      () => true
    );
    expect(refused).toBe(true);

    fake.socketResults['counts.get'] = { counts: { c1: { unread: 0, total: 5 } } };
    advanceClock(500);
    await until('counts.get again', () => store.unreadCountForCategoryId('c1') === 0);
    expect(fake.socketOf('counts.get').length).toBe(2);
  });

  it('knows the features before the deltas read with the hello answer', async function () {
    // An engine that sends a delta in the same write as its hello answer.
    const socketPath = `${dir}/one-write.sock`;
    const server = net.createServer((socket) => {
      socket.setEncoding('utf8');
      let buffer = '';
      socket.on('data', (chunk: string) => {
        buffer += chunk;
        const lines = buffer.split('\n');
        buffer = lines.pop();
        for (const line of lines) {
          const req = JSON.parse(line);
          if (req.method !== 'hello') continue;
          const answer = { jsonrpc: '2.0', id: req.id, result: hello([Feature.Counts]) };
          const delta = { jsonrpc: '2.0', method: 'delta', params: countsDelta('c1', 4, 4) };
          socket.write(`${JSON.stringify(answer)}\n${JSON.stringify(delta)}\n`);
        }
      });
    });
    await new Promise<void>((resolve) => server.listen(socketPath, () => resolve()));
    try {
      const seen: boolean[] = [];
      events.on('thread-counts', () => seen.push(events.has(Feature.Counts)));
      open(socketPath);
      await until('the delta', () => seen.length === 1);
      expect(seen).toEqual([true]);
    } finally {
      server.close();
    }
  });

  it('hands mail.new to the notifier', async function () {
    fake.socketResults.hello = hello([Feature.MailNew]);
    const notifier = new Notifier(events);
    spyOn(notifier, '_onMailNewBatch');
    try {
      open();
      await until('hello', () => events.has(Feature.MailNew));
      fake.notify('mail.new', { accountId: 'a1', threadId: 't1', messageId: 'm1' });
      await until('mail.new', () => notifier.mailNewBatch.length === 1);
      expect(notifier.mailNewBatch).toEqual([{ accountId: 'a1', threadId: 't1', messageId: 'm1' }]);
      advanceClock(100);
      expect(notifier._onMailNewBatch).toHaveBeenCalled();
    } finally {
      notifier.unlisten();
    }
  });

  it('feeds a non-main window too', async function () {
    fake.socketResults.hello = hello([Feature.Counts]);
    const got = [];
    events.on('thread-counts', (delta) => got.push(delta));
    open(fake.socketPath, { mainWindow: false });
    await until('hello', () => events.has(Feature.Counts));
    fake.publishDelta(countsDelta('c2', 0, 1));
    await until('the delta', () => got.length === 1);
    expect(got[0]).toEqual({ type: 'persist', rows: countsDelta('c2', 0, 1).modelJSONs });
  });

  it('sends reads as query, holds them while the socket is down and resyncs after the reconnect', async function () {
    fake.socketResults.hello = hello([Feature.Query]);
    fake.socketResults.query = { rows: [{ __cls: 'Thread', id: 't1', aid: 'a1' }] };
    const reads = new EngineReads(events, () => 'engine');
    const resyncs = jasmine.createSpy('resync');
    reads.on('resync', resyncs);
    const json: any = { class: 'Thread', where: null, include: [], limit: 1 };

    open(fake.socketPath, { mainWindow: false });
    expect((await reads.query(json)).rows[0].id).toBe('t1');
    expect(fake.socketOf('query')[0].params).toEqual(json);
    expect(resyncs).not.toHaveBeenCalled();

    fake.notify('disconnect', { reason: 'slow consumer' }, fake.conns[0]);
    await until('the drop', () => connection._socket === null);
    let answer = null;
    reads.query(json).then((r) => (answer = r));
    await until('a turn', () => true);
    expect(answer).toBe(null);
    expect(reads.unavailable()).toBe('waiting');

    advanceClock(500);
    await until('the read after the reconnect', () => answer !== null);
    expect(fake.socketOf('query').length).toBe(2);
    expect(resyncs.callCount).toBe(1);
    expect(reads.unavailable()).toBe(null);
  });
});

describe('ThreadCounts deltas', function () {
  it('go to EngineEvents and not to DatabaseStore', function () {
    spyOn(DefaultEngineEvents, 'threadCounts');
    spyOn(ipcRenderer, 'send');
    spyOn(DatabaseStore, 'trigger');
    const delta = countsDelta('c1', 2, 3);
    applyDeltas([JSON.stringify(delta)], { mainWindow: true });
    expect(DefaultEngineEvents.threadCounts).toHaveBeenCalledWith({
      type: 'persist',
      rows: delta.modelJSONs,
    });
    expect(ipcRenderer.send).not.toHaveBeenCalled();
    expect(DatabaseStore.trigger).not.toHaveBeenCalled();
  });
});
