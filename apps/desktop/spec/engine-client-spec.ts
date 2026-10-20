// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import os from 'os';
import path from 'path';
import { EngineServeProcess, ENGINE_SHUTDOWN_GRACE_MS } from '../src/shared/mailsync-process';
import {
  AccountJSON,
  EngineClient,
  EngineClientHost,
  ENGINE_IN_USE_RETRIES,
  ENGINE_IN_USE_RETRY_MS,
  RESET_STRAGGLER_WAIT_MS,
} from '../src/main/engine-host';
import { EXIT_SOCKET_IN_USE, serveSocketPath } from '../src/shared/engine-protocol';
import { FakeEngine, tick, until } from './fake-serve-engine';

const accountJSON = (id: string): AccountJSON => ({
  id,
  emailAddress: `${id}@example.com`,
  syncState: 'ok',
  settings: { imap_password: `pw-${id}` },
});

describe('the serve engine client', function () {
  let dir: string = null;
  let fake: FakeEngine = null;
  let accounts: { id: string }[] = [];
  let host: EngineClientHost = null;
  let client: EngineClient = null;
  let deltas: string[] = [];
  let exited: string[] = [];
  let failures: { accountId: string; exit: any; authFailure: boolean }[] = [];

  beforeEach(async function () {
    dir = fs.mkdtempSync('/tmp/tern-spec-');
    fake = new FakeEngine(dir);
    await fake.start();
    accounts = [{ id: 'a1' }, { id: 'a2' }];
    deltas = [];
    exited = [];
    failures = [];
    spyOn(EngineServeProcess.prototype as any, '_spawnProcess').andCallFake(function (
      mode: string,
      args: string[]
    ) {
      this._proc = fake.spawn(mode, args);
    });
    spyOn(console, 'log');
    spyOn(console, 'warn');
    host = {
      createEngine: () => new EngineServeProcess({ configDirPath: dir, resourcePath: dir }),
      accounts: () => accounts,
      accountJSONs: async (list) => list.map((a) => accountJSON(a.id)),
      onDeltas: (msgs) => deltas.push(...msgs),
      onAccountExited: (id) => exited.push(id),
      onAccountLaunched: () => {},
      onTooManyFailures: (accountId, exit, authFailure) =>
        failures.push({ accountId, exit, authFailure }),
    };
    client = new EngineClient(host);
  });

  afterEach(function () {
    if (client.engine) client.engine._closeSocket();
    fake.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const started = async () => {
    await client.ensure();
    await until('the adds', () => fake.stdinOf('account.add').length >= accounts.length);
    await until('the subscription', () => fake.socketOf('subscribe').length >= fake.spawns.length);
  };

  it('spawns one serve engine and adds every account over stdin only', async function () {
    await started();

    expect(fake.spawns.map((s) => [s.mode, ...s.args])).toEqual([
      ['serve', '--socket', fake.socketPath],
    ]);
    expect(fake.stdin.map((r) => r.method)).toEqual(['account.list', 'account.add', 'account.add']);
    expect(fake.stdinOf('account.add').map((r) => r.params)).toEqual([
      { account: accountJSON('a1'), identity: null },
      { account: accountJSON('a2'), identity: null },
    ]);
    expect(fake.socket.map((r) => r.method)).toEqual(['hello', 'subscribe']);
    expect(fake.socketOf('subscribe')[0].params).toEqual({ accounts: null, classes: null });
    expect(fake.socketText).not.toContain('pw-a1');
  });

  it('diffs the accounts into account.add and account.remove', async function () {
    await started();
    accounts = [{ id: 'a2' }, { id: 'a3' }];
    await client.ensure();
    await until('the remove', () => exited.includes('a1'));

    expect(fake.stdinOf('account.remove').map((r) => r.params)).toEqual([{ accountId: 'a1' }]);
    expect(fake.stdinOf('account.add').map((r) => r.params.account.id)).toEqual(['a1', 'a2', 'a3']);
    await client.ensure();
    await tick();
    expect(fake.stdin.length).toEqual(5);
    expect(fake.spawns.length).toEqual(1);
  });

  it('routes commands by account, holding them until the add answers', async function () {
    fake.holdAdds.add('a1');
    await client.ensure();
    await until('the subscription', () => fake.socketOf('subscribe').length === 1);
    const task = { __cls: 'ChangeStarredTask', id: 't1' };
    expect(client.send('a1', { type: 'queue-task', task })).toBe(true);
    expect(client.send('a2', { type: 'need-bodies', ids: ['m1'] })).toBe(true);
    expect(client.send('nope', { type: 'need-bodies', ids: ['m1'] })).toBe(false);
    await until('the bodies', () => fake.socketOf('bodies.fetch').length === 1);
    expect(fake.socketOf('task.queue')).toEqual([]);

    fake.heldAdds.forEach((send) => send());
    await until('the task', () => fake.socketOf('task.queue').length === 1);
    client.wakeAll();
    client.syncCalendar();
    await until('the calendar syncs', () => fake.socketOf('calendar.sync').length === 2);

    expect(fake.socketOf('task.queue')[0].params).toEqual({ accountId: 'a1', task });
    expect(fake.socketOf('bodies.fetch')[0].params).toEqual({ accountId: 'a2', ids: ['m1'] });
    expect(fake.socketOf('sync.wake')[0].params).toEqual({});
    expect(
      fake
        .socketOf('calendar.sync')
        .map((r) => r.params.accountId)
        .sort()
    ).toEqual(['a1', 'a2']);
  });

  it('passes on socket deltas and the secrets deltas from stdout', async function () {
    await started();
    const thread = { accountId: 'a1', modelClass: 'Thread', type: 'persist', modelJSONs: [] };
    const secrets = {
      accountId: 'a2',
      modelClass: 'ProcessAccountSecretsUpdated',
      type: 'persist',
      modelJSONs: [{ id: 'a2' }],
    };
    fake.notify('delta', thread);
    fake.child.stdout.emit(
      'data',
      Buffer.from(`${JSON.stringify({ jsonrpc: '2.0', method: 'delta', params: secrets })}\n`)
    );
    await until('both deltas', () => deltas.length === 2);
    expect(
      deltas.map((d) => JSON.parse(d)).sort((a, b) => (a.accountId < b.accountId ? -1 : 1))
    ).toEqual([thread, secrets]);
  });

  it('re-adds a stopped account until five stops in five minutes', async function () {
    await started();
    const error = { key: 'ErrorAuthentication', debuginfo: 'bad password', retryable: false };
    for (let i = 1; i <= 5; i++) {
      fake.notify('account.state', { accountId: 'a1', state: 'stopped', error });
      if (i < 5) {
        await until(`re-add ${i}`, () => fake.stdinOf('account.add').length === 2 + i);
      }
    }
    await until('the cutoff', () => failures.length === 1);
    await tick();

    expect(fake.stdinOf('account.add').filter((r) => r.params.account.id === 'a1').length).toBe(5);
    expect(fake.stdinOf('account.add').filter((r) => r.params.account.id === 'a2').length).toBe(1);
    expect(failures[0].accountId).toEqual('a1');
    expect(failures[0].authFailure).toBe(true);
    expect(failures[0].exit.error.message).toContain('ErrorAuthentication');
    expect(failures[0].exit.error.syncError).toEqual(error);
    expect(exited.filter((id) => id === 'a1').length).toBe(5);
    expect(client.send('a1', { type: 'wake-workers' })).toBe(false);

    // Until "Try Again".
    await client.ensure();
    expect(fake.stdinOf('account.add').length).toBe(6);
    await client.forceRelaunch({ id: 'a1' });
    await until('the forced add', () => fake.stdinOf('account.add').length === 7);
    expect(fake.stdinOf('account.remove')).toEqual([]);
  });

  it('leaves a stopped account to an engine that restarts it, until it gives up', async function () {
    fake.keepsAccounts([]);
    const launched: string[] = [];
    host.onAccountLaunched = (json) => launched.push(json.id);
    await started();
    expect(launched).toEqual(['a1', 'a2']);
    const error = { key: 'ErrorConnection', debuginfo: 'refused', retryable: true };

    fake.notify('account.state', { accountId: 'a1', state: 'stopped', error, retryAt: 1 });
    await until('the exit', () => exited.includes('a1'));
    fake.notify('account.state', { accountId: 'a1', state: 'running' });
    await until('the relaunch', () => launched.length === 3);
    expect(fake.stdinOf('account.add').length).toBe(2);

    const auth = { key: 'ErrorAuthentication', debuginfo: 'bad password', retryable: false };
    fake.notify('account.state', {
      accountId: 'a1',
      state: 'stopped',
      error: auth,
      authFailed: true,
    });
    await until('the cutoff', () => failures.length === 1);
    expect(failures[0].accountId).toBe('a1');
    expect(failures[0].authFailure).toBe(true);
    expect(failures[0].exit.error.syncError).toEqual(auth);
    expect(client.send('a1', { type: 'wake-workers' })).toBe(false);
    await client.ensure();
    expect(fake.stdinOf('account.add').length).toBe(2);

    // "Try Again" adds it without account.remove, which would delete its stored secrets.
    await client.forceRelaunch({ id: 'a1' });
    await until('the forced add', () => fake.stdinOf('account.add').length === 3);
    expect(fake.stdinOf('account.remove')).toEqual([]);
  });

  it("starts the engine again on the engine's Try Again, after a give-up or during a backoff", function () {
    const ensure = spyOn(client, 'ensure').andReturn(Promise.resolve());
    client.engineGaveUp = true;
    client.inUseExits = 3;
    client.retryEngine();
    expect(client.engineGaveUp).toBe(false);
    expect(client.inUseExits).toBe(0);
    expect(ensure.callCount).toBe(1);

    const respawn = jasmine.createSpy('respawn');
    client.respawnTimer = setTimeout(respawn, 8000);
    client.retryEngine();
    expect(client.respawnTimer).toBe(null);
    expect(ensure.callCount).toBe(2);
    advanceClock(8000);
    expect(respawn).not.toHaveBeenCalled();
  });

  it('removes only the accounts the app deleted from an engine that has others', async function () {
    fake.keepsAccounts(['a1', 'a2', 'other', 'gone']);
    let deleted = ['gone'];
    const removed: string[] = [];
    host.removedAccounts = () => deleted;
    host.onAccountRemoved = (id) => {
      removed.push(id);
      deleted = deleted.filter((d) => d !== id);
    };
    await started();
    await until('the remove', () => removed.length === 1);
    expect(fake.stdinOf('account.remove').map((r) => r.params)).toEqual([{ accountId: 'gone' }]);
    expect(removed).toEqual(['gone']);

    // A later delete while the engine runs goes the usual way and is acknowledged too.
    accounts = [{ id: 'a1' }];
    deleted = ['a2'];
    await client.ensure();
    await until('the second remove', () => removed.length === 2);
    await client.ensure();
    await tick();
    expect(fake.stdinOf('account.remove').map((r) => r.params.accountId)).toEqual(['gone', 'a2']);
  });

  it('keeps an account reset with account.reset registered', async function () {
    fake.keepsAccounts([]);
    await started();
    await client.resetCache('a1', () => client.engine.resetAccount('a1', {}));
    expect(fake.stdinOf('account.reset').length).toBe(1);
    expect(fake.stdinOf('account.remove')).toEqual([]);
    await client.ensure();
    await until('the add back', () => fake.stdinOf('account.add').length === 3);
  });

  it('removes a running account before a forced relaunch', async function () {
    await started();
    await client.forceRelaunch({ id: 'a2' });
    await until('the add', () => fake.stdinOf('account.add').length === 3);
    expect(fake.stdin.map((r) => r.method).slice(3)).toEqual(['account.remove', 'account.add']);
  });

  it('respawns an engine that exited and adds every account again', async function () {
    await started();
    fake.child.stderr.emit('data', Buffer.from('panicked at engine-wide thread'));
    fake.child.emit('exit', 101, null);
    expect(exited.sort()).toEqual(['a1', 'a2']);
    expect(client.send('a1', { type: 'wake-workers' })).toBe(false);

    advanceClock(1);
    await until('the second engine', () => fake.spawns.length === 2);
    await until('the adds', () => fake.stdinOf('account.add').length === 4);
    expect(fake.stdinOf('account.add').map((r) => [r.child.pid, r.params.account.id])).toEqual([
      [4000, 'a1'],
      [4000, 'a2'],
      [4001, 'a1'],
      [4001, 'a2'],
    ]);

    // The next one in a row waits a second.
    fake.child.emit('exit', 101, null);
    advanceClock(999);
    await tick();
    expect(fake.spawns.length).toBe(2);
    advanceClock(1);
    await until('the third engine', () => fake.spawns.length === 3);
  });

  it('stops respawning after five engine exits in five minutes', async function () {
    await started();
    for (let i = 1; i <= 5; i++) {
      fake.child.emit('exit', 101, null);
      if (i < 5) {
        advanceClock(10000);
        await until(`engine ${i + 1}`, () => fake.spawns.length === i + 1);
      }
    }
    expect(failures.map((f) => f.accountId)).toEqual(['a1', 'a2']);
    advanceClock(60000);
    await client.ensure();
    expect(fake.spawns.length).toBe(5);

    await client.forceRelaunch({ id: 'a1' });
    await until(
      'a sixth engine with both accounts',
      () =>
        fake.stdin.filter((r) => r.method === 'account.add' && r.child.pid === 4005).length === 2
    );
  });

  it('does not respawn an engine killed with SIGTERM until the next ensure', async function () {
    await started();
    fake.child.emit('exit', null, 'SIGTERM');
    advanceClock(10000);
    await tick();
    expect(fake.spawns.length).toBe(1);
    await client.ensure();
    expect(fake.spawns.length).toBe(2);
  });

  it('resets an account out of the engine and adds it back after', async function () {
    await started();
    fake.stillRunning = ['background'];
    const reset = jasmine.createSpy('reset').andReturn(Promise.resolve('done'));
    const result = client.resetCache('a1', reset);
    await until('the remove', () => exited.includes('a1'));

    // Commands for it are dropped, ensure() leaves it out, and the reset waits for the
    // thread the engine left running.
    expect(client.send('a1', { type: 'wake-workers' })).toBe(true);
    await client.ensure();
    await tick();
    expect(fake.stdinOf('account.add').length).toBe(2);
    expect(reset).not.toHaveBeenCalled();
    advanceClock(RESET_STRAGGLER_WAIT_MS);

    expect(await result).toEqual('done');
    expect(reset).toHaveBeenCalled();
    await until('the add', () => fake.stdinOf('account.add').length === 3);
    expect(fake.stdinOf('account.add')[2].params.account.id).toEqual('a1');
  });

  it('does not add back an account removed from AccountStore after its reset', async function () {
    await started();
    accounts = [{ id: 'a2' }];
    await client.ensure();
    await client.resetCache('a1', () => Promise.resolve());
    await tick();
    expect(fake.stdinOf('account.remove').length).toBe(1);
    expect(fake.stdinOf('account.add').length).toBe(2);
  });

  it('re-adds an account whose add answered stopped', async function () {
    const error = { key: 'ErrorDatabase', debuginfo: 'disk I/O error', retryable: false };
    fake.addResults.a1 = { accountId: 'a1', state: 'stopped', error };
    spyOn(client.crashes, 'recordClientCrash').andCallThrough();
    await started();
    await until('the re-add', () => fake.stdinOf('account.add').length === 3);
    await until('a1 added', () => client.entries.get('a1')?.state === 'added');

    expect((client.crashes.recordClientCrash as jasmine.Spy).callCount).toBe(1);
    const exit = (client.crashes.recordClientCrash as jasmine.Spy).calls[0].args[1];
    expect(exit.error.syncError).toEqual(error);
    expect(exited).toEqual(['a1']);
    expect(client.entries.get('a2').state).toEqual('added');
  });

  it("redacts the added accounts' secrets from what the engine reports", async function () {
    spyOn(client.crashes, 'recordClientCrash').andCallThrough();
    await started();
    expect(client.engine._stripSecrets('login pw-a1 and pw-a2 refused')).toEqual(
      'login ********* and ********* refused'
    );
    const error = { key: 'ErrorAuthentication', debuginfo: 'LOGIN me pw-a1', retryable: false };
    fake.notify('account.state', { accountId: 'a1', state: 'stopped', error });
    await until('the crash', () => (client.crashes.recordClientCrash as jasmine.Spy).callCount > 0);
    const exit = (client.crashes.recordClientCrash as jasmine.Spy).calls[0].args[1];
    expect(exit.error.message).not.toContain('pw-a1');
    expect(exit.error.syncError.debuginfo).toEqual('LOGIN me *********');
  });

  it('respawns an engine that found the socket in use without counting crashes', async function () {
    fake.answerList = false;
    await client.ensure();
    for (let i = 1; i <= ENGINE_IN_USE_RETRIES; i++) {
      fake.child.emit('exit', EXIT_SOCKET_IN_USE, null);
      advanceClock(ENGINE_IN_USE_RETRY_MS - 1);
      await tick();
      expect(fake.spawns.length).toBe(i);
      advanceClock(1);
      await until(`engine ${i + 1}`, () => fake.spawns.length === i + 1);
    }
    expect(client.engineCrashes.recentCrashes({ id: 'engine', settings: {} })).toBe(0);
    expect(failures).toEqual([]);

    // Past the retries it is a crash, until an engine binds.
    fake.child.emit('exit', EXIT_SOCKET_IN_USE, null);
    expect(client.engineCrashes.recentCrashes({ id: 'engine', settings: {} })).toBe(1);
    fake.answerList = true;
    advanceClock(1);
    await until('the adds', () => fake.stdinOf('account.add').length >= 2 * fake.spawns.length);
    await until('the ready engine', () => client.inUseExits === 0);
  });

  it('waits for the threads an earlier removal left before a reset', async function () {
    await started();
    fake.stillRunning = ['background'];
    accounts = [{ id: 'a2' }];
    await client.ensure();
    await until('the remove', () => exited.includes('a1'));
    await tick();
    expect(client.entries.has('a1')).toBe(false);

    const reset = jasmine.createSpy('reset').andReturn(Promise.resolve());
    const result = client.resetCache('a1', reset);
    await tick();
    advanceClock(RESET_STRAGGLER_WAIT_MS - 1000);
    await tick();
    expect(reset).not.toHaveBeenCalled();
    advanceClock(1000);
    await result;
    expect(reset).toHaveBeenCalled();
    expect(fake.stdinOf('account.remove').length).toBe(1);
    expect(fake.stdinOf('account.add').length).toBe(2);
  });

  it('shuts the engine down, killing it after the grace period', async function () {
    await started();
    fake.exitOnShutdown = false;
    let done = false;
    client.shutdown().then(() => (done = true));
    await until('the shutdown request', () => fake.stdinOf('engine.shutdown').length === 1);
    advanceClock(ENGINE_SHUTDOWN_GRACE_MS - 1);
    expect(fake.child.kill).not.toHaveBeenCalled();
    advanceClock(1);
    expect(fake.child.kill).toHaveBeenCalled();
    fake.child.emit('exit', null, 'SIGTERM');
    await until('the shutdown', () => done);
    advanceClock(10000);
    await client.ensure();
    expect(fake.spawns.length).toBe(1);
    expect(failures).toEqual([]);
  });

  it('runs the engine with no accounts and spawns one for a stdin call', async function () {
    accounts = [];
    await client.ensure();
    expect(fake.spawns.length).toBe(1);
    expect(fake.stdinOf('account.add').length).toBe(0);

    fake.child.emit('exit', null, 'SIGTERM');
    expect(client.engine).toBe(null);
    const result = await client.withEngine((engine) => engine.resetAccount('a1'));
    expect(result).toEqual({});
    expect(fake.spawns.length).toBe(2);
    expect(fake.stdinOf('account.reset')[0].child).toBe(fake.child);
  });

  it('forgets every account on a database reset and adds none back', async function () {
    await started();
    await client.resetDatabase();
    expect(fake.stdinOf('engine.resetDatabase').length).toBe(1);
    expect(client.entries.size).toBe(0);
    expect(exited.sort()).toEqual(['a1', 'a2']);
    await client.ensure();
    await tick();
    expect(fake.stdinOf('account.add').length).toBe(2);
  });

  it('resolves the shutdown once the engine exits on its own', async function () {
    await started();
    await client.shutdown();
    expect(fake.child.kill).not.toHaveBeenCalled();
  });
});

describe('the serve socket path', function () {
  it('is <configDir>/run/engine.sock when it fits', function () {
    expect(serveSocketPath('/Users/me/.tern')).toEqual({
      socketPath: path.join('/Users/me/.tern', 'run', 'engine.sock'),
      ownDir: null,
    });
  });

  it('falls back to a short name in a per-user temp dir', function () {
    const configDir = `/Users/me/${'x'.repeat(100)}`;
    const { socketPath, ownDir } = serveSocketPath(configDir);
    expect(ownDir).toEqual(path.join(os.tmpdir(), `tern-${process.getuid()}`));
    expect(path.dirname(socketPath)).toEqual(ownDir);
    expect(path.basename(socketPath)).toMatch(/^[0-9a-f]{12}\.sock$/);
    expect(Buffer.byteLength(socketPath)).not.toBeGreaterThan(103);
  });
});
