// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { EventEmitter } from 'events';
import fs from 'fs';
import { Actions, Message, SendDraftTask } from 'mailspring-exports';
import SoundRegistry from '../../src/packages/sound-registry';
import OnlineStatusStore from '../../src/data/stores/online-status';
import { EngineConnection } from '../../src/engine/connection';
import MailsyncBridge, { applyDeltas } from '../../src/engine/bridge';
import { EngineStatus } from '../../src/shared/engine/supervisor-channels';
import { FakeEngine, tick, until } from '../fake-serve-engine';

const status = (
  socketPath: string,
  accounts: EngineStatus['accounts'],
  generation = 1
): EngineStatus => ({ generation, socketPath, accounts });

describe('a window engine connection', function () {
  let dir: string = null;
  let fake: FakeEngine = null;
  let connections: EngineConnection[] = [];

  beforeEach(async function () {
    dir = fs.mkdtempSync('/tmp/tern-spec-');
    fake = new FakeEngine(dir);
    await fake.start();
    connections = [];
    spyOn(console, 'log');
    spyOn(console, 'warn');
  });

  afterEach(function () {
    for (const c of connections) c.close();
    fake.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const added = { state: 'added', held: false } as const;

  const open = (accounts: EngineStatus['accounts'] = {}, { mainWindow = true } = {}) => {
    const c = new EngineConnection({ clientName: 'tern-desktop/spec', mainWindow });
    connections.push(c);
    c.setStatus(status(fake.socketPath, accounts));
    return c;
  };

  const subscribed = (n: number) =>
    until(`${n} subscriptions`, () => fake.socketOf('subscribe').length === n);

  it('says hello and subscribes to every delta', async function () {
    const c = open();
    let hello = null;
    c.on('connected', (h) => (hello = h));
    await subscribed(1);
    expect(fake.connRequests[0].map((r) => [r.method, r.params])).toEqual([
      ['hello', { protocol: 1, client: 'tern-desktop/spec' }],
      ['subscribe', { accounts: null, classes: null }],
      ['engine.status', {}],
    ]);
    await until('hello', () => hello !== null);
  });

  it('gives a composer window the deltas on its own connection', async function () {
    const main = open({ a1: added });
    const composer = open({ a1: added }, { mainWindow: false });
    const got = { main: [] as string[], composer: [] as string[] };
    main.on('deltas', (msgs) => got.main.push(...msgs));
    composer.on('deltas', (msgs) => got.composer.push(...msgs));
    await subscribed(2);

    const delta = { accountId: 'a1', modelClass: 'Message', type: 'persist', modelJSONs: [] };
    fake.publishDelta(delta);
    await until('both', () => got.main.length === 1 && got.composer.length === 1);
    expect(JSON.parse(got.composer[0])).toEqual(delta);

    // Its own: the main window's connection going away doesn't touch it.
    main.close();
    fake.publishDelta(delta);
    await until('the composer', () => got.composer.length === 2);
    expect(got.main.length).toBe(1);
  });

  it('leaves the startup backlog to the main window when another window subscribes first', async function () {
    const backlogged = { accountId: 'a1', modelClass: 'ProcessState', type: 'persist' };
    fake.backlog = [{ ...backlogged, modelJSONs: [{ accountId: 'a1', connectionError: true }] }];
    const got = { main: [] as string[], hot: [] as string[] };
    const hot = open({ a1: added }, { mainWindow: false });
    hot.on('deltas', (msgs) => got.hot.push(...msgs));
    await subscribed(1);
    expect(fake.connRequests[0][1].params).toEqual({ accounts: ['a1'], classes: null });

    const main = open({ a1: added });
    main.on('deltas', (msgs) => got.main.push(...msgs));
    await until('the backlog', () => got.main.some((m) => m.includes('"connectionError":true')));
    expect(got.hot).toEqual([]);
  });

  it('subscribes a non-main window to the accounts the engine runs as they change', async function () {
    const composer = open(
      { a1: added, a2: { state: 'adding', held: false } },
      { mainWindow: false }
    );
    const got: string[] = [];
    composer.on('deltas', (msgs) => got.push(...msgs));
    await subscribed(1);
    expect(fake.socketOf('subscribe')[0].params).toEqual({ accounts: ['a1'], classes: null });

    composer.setStatus(status(fake.socketPath, { a1: added, a2: added }));
    await subscribed(2);
    expect(fake.socketOf('subscribe')[1].params).toEqual({ accounts: ['a1', 'a2'], classes: null });
    // The same accounts again: no new subscribe.
    composer.setStatus(status(fake.socketPath, { a1: added, a2: added }));
    fake.publishDelta({ accountId: 'a2', modelClass: 'Message', type: 'persist', modelJSONs: [] });
    await until('the a2 delta', () => got.length === 1);
    expect(fake.socketOf('subscribe').length).toBe(2);
    // Only the main window asks for the connection states.
    expect(fake.socketOf('engine.status')).toEqual([]);
  });

  it('gives the main window each account connection state from engine.status, in stream order', async function () {
    fake.socketResults['engine.status'] = {
      accounts: [
        { accountId: 'a1', email: 'a1@x', state: 'running', connectionError: true },
        { accountId: 'a2', email: 'a2@x', state: 'running', connectionError: false },
      ],
    };
    const c = open({ a1: added, a2: added });
    const got: any[] = [];
    c.on('deltas', (msgs) => got.push(...msgs.map((m) => JSON.parse(m))));
    await until('the states', () => got.length === 2);
    expect(
      got.map((d) => [d.modelClass, d.modelJSONs[0].accountId, d.modelJSONs[0].connectionError])
    ).toEqual([
      ['ProcessState', 'a1', true],
      ['ProcessState', 'a2', false],
    ]);

    // A reloaded main window's OnlineStatusStore starts empty and learns a1 is offline.
    OnlineStatusStore._offlineProcesses = {};
    spyOn(OnlineStatusStore, 'trigger');
    applyDeltas(
      got.map((d) => JSON.stringify(d)),
      { mainWindow: true }
    );
    expect(OnlineStatusStore._offlineProcesses).toEqual({ a1: true });
    OnlineStatusStore._offlineProcesses = {};
  });

  it('keeps commands sent while the socket is down and sends them after the reconnect', async function () {
    const c = open({ a1: added });
    await subscribed(1);
    fake.notify('disconnect', { reason: 'slow consumer' }, fake.conns[0]);
    await until('the drop', () => c._socket === null);

    const task = { __cls: 'ChangeStarredTask', id: 't1' };
    expect(c.send('a1', { type: 'queue-task', task })).toBe(true);
    await tick();
    expect(fake.socketOf('task.queue')).toEqual([]);

    advanceClock(500);
    await until('the task', () => fake.socketOf('task.queue').length === 1);
    expect(fake.connRequests[1].map((r) => r.method)).toEqual([
      'hello',
      'subscribe',
      'engine.status',
      'task.queue',
    ]);
    expect(fake.socketOf('task.queue')[0].params).toEqual({ accountId: 'a1', task });
  });

  it('routes commands by the account states in the status', async function () {
    const c = open({ a1: { state: 'adding', held: false }, a2: { state: 'added', held: false } });
    await subscribed(1);
    const task = { __cls: 'ChangeStarredTask', id: 't1' };
    expect(c.send('a1', { type: 'queue-task', task })).toBe(true);
    expect(c.send('a2', { type: 'need-bodies', ids: ['m1'] })).toBe(true);
    expect(c.send('nope', { type: 'need-bodies', ids: ['m1'] })).toBe(false);
    await until('the bodies', () => fake.socketOf('bodies.fetch').length === 1);
    expect(fake.socketOf('task.queue')).toEqual([]);

    c.setStatus(
      status(fake.socketPath, {
        a1: { state: 'added', held: false },
        a2: { state: null, held: true },
      })
    );
    await until('the task', () => fake.socketOf('task.queue').length === 1);
    expect(fake.socketOf('task.queue')[0].params).toEqual({ accountId: 'a1', task });

    // Held for a reset: dropped, not refused.
    expect(c.send('a2', { type: 'wake-workers' })).toBe(true);
    c.setStatus(status(fake.socketPath, { a1: { state: 'removing', held: false } }));
    expect(c.send('a1', { type: 'wake-workers' })).toBe(false);
    await tick();
    expect(fake.socketOf('sync.wake')).toEqual([]);
  });

  it('drops the commands held for an account the engine stopped running', async function () {
    const c = open({ a1: { state: 'adding', held: false } });
    await subscribed(1);
    c.send('a1', { type: 'wake-workers' });
    c.setStatus(status(fake.socketPath, {}));
    c.setStatus(status(fake.socketPath, { a1: { state: 'added', held: false } }));
    await tick();
    expect(fake.socketOf('sync.wake')).toEqual([]);
  });

  it('connects again when the engine drops it, and to the next engine', async function () {
    const c = open();
    await subscribed(1);
    fake.notify('disconnect', { reason: 'slow consumer' }, fake.conns[0]);
    await until('the drop', () => c._socket === null);
    advanceClock(500);
    await subscribed(2);

    c.setStatus(status(null, {}, null));
    await tick();
    expect(c._socket).toBe(null);
    c.setStatus(status(fake.socketPath, {}, 2));
    await subscribed(3);
    expect(c._generation).toBe(2);
  });
});

describe('task handlers with two windows open', function () {
  const delta = (task: any) =>
    JSON.stringify({
      accountId: 'a1',
      modelClass: 'SendDraftTask',
      type: 'persist',
      modelJSONs: [{ ...task.toJSON(), __cls: 'SendDraftTask' }],
    });

  const completed = (error?: any) => {
    const task = new SendDraftTask({
      accountId: 'a1',
      draft: new Message({ accountId: 'a1', headerMessageId: 'h1', threadId: 't1', draft: true }),
    });
    task.status = 'complete';
    if (error) task.error = error;
    return task;
  };

  beforeEach(function () {
    spyOn(SoundRegistry, 'playSound');
    spyOn(Actions, 'draftDeliverySucceeded');
    spyOn(Actions, 'draftDeliveryFailed');
    spyOn(AppEnv.config, 'get').andCallFake((key: string) => key === 'core.sending.sounds');
  });

  // Each window receives the delta on its own connection; only the main window acts on it.
  const inBothWindows = (msg: string) => {
    applyDeltas([msg], { mainWindow: true });
    applyDeltas([msg], { mainWindow: false });
  };

  it('plays one send sound', function () {
    inBothWindows(delta(completed()));
    expect((SoundRegistry.playSound as jasmine.Spy).calls.length).toBe(1);
    expect((Actions.draftDeliverySucceeded as unknown as jasmine.Spy).calls.length).toBe(1);
  });

  it('leaves the task handlers to the main window when a composer bridge reads the delta', function () {
    spyOn(AppEnv, 'inSpecMode').andReturn(false);
    spyOn(AppEnv, 'isMainWindow').andReturn(false);
    spyOn(AppEnv, 'onReadyToUnload');
    const connection = new EventEmitter();
    spyOn(MailsyncBridge.prototype, '_createConnection').andCallFake(function () {
      connection.on('deltas', this._onIncomingMessages);
      return connection;
    });
    const composer = new MailsyncBridge();
    expect(composer._mainWindow).toBe(false);
    expect(composer._connection).toBe(connection);

    const msg = delta(completed());
    connection.emit('deltas', [msg]);
    expect(SoundRegistry.playSound).not.toHaveBeenCalled();
    expect(Actions.draftDeliverySucceeded).not.toHaveBeenCalled();
    applyDeltas([msg], { mainWindow: true });
    expect((SoundRegistry.playSound as jasmine.Spy).calls.length).toBe(1);
  });

  it('reports one delivery failure', function () {
    inBothWindows(delta(completed({ key: 'send-failed', debuginfo: 'ErrorConnection' })));
    expect((Actions.draftDeliveryFailed as unknown as jasmine.Spy).calls.length).toBe(1);
    expect(SoundRegistry.playSound).not.toHaveBeenCalled();
  });
});
