// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { EventEmitter } from 'events';
import fs from 'fs';
import net from 'net';
import path from 'path';
import { EngineServeProcess, ENGINE_SHUTDOWN_GRACE_MS } from '../src/mailsync-process';
import {
  AccountJSON,
  EngineClient,
  EngineClientHost,
  ENGINE_IN_USE_RETRIES,
  ENGINE_IN_USE_RETRY_MS,
  RESET_STRAGGLER_WAIT_MS,
  serveEngineSelected,
} from '../src/engine-client';
import { EXIT_SOCKET_IN_USE, serveSocketPath } from '../src/engine-protocol';

// Lets real I/O (the socket) and promise callbacks run; the spec helper fakes setTimeout.
const tick = () => new Promise((resolve) => setImmediate(resolve));

async function until(what: string, condition: () => boolean) {
  for (let i = 0; i < 5000; i++) {
    if (condition()) return;
    await tick();
  }
  throw new Error(`timed out waiting for ${what}`);
}

/*
Stands in for `mailsync-rs --mode serve`: each spawn gets a stub child whose stdin answers the
parent-channel requests on stdout, and one Unix socket server at the serve socket path takes
the client's connections.
*/
class FakeEngine {
  dir: string;
  socketPath: string;
  server: net.Server;
  conn: net.Socket = null;
  conns: net.Socket[] = [];
  spawns: { mode: string; args: string[]; child: any }[] = [];
  stdin: any[] = [];
  socket: any[] = [];
  socketText = '';
  stillRunning: string[] = [];
  // account.add responses held back by account id, when set.
  holdAdds = new Set<string>();
  heldAdds: (() => void)[] = [];
  // The next account.add result by account id, instead of `running`.
  addResults: { [id: string]: any } = {};
  exitOnShutdown = true;
  // False stands in for an engine that could not bind and never reads stdin.
  answerList = true;

  constructor(dir: string) {
    this.dir = dir;
    this.socketPath = serveSocketPath(dir).socketPath;
    fs.mkdirSync(path.dirname(this.socketPath), { recursive: true });
  }

  start() {
    return new Promise<void>((resolve) => {
      this.server = net.createServer((socket) => {
        this.conn = socket;
        this.conns.push(socket);
        let buffer = '';
        socket.setEncoding('utf8');
        socket.on('data', (chunk: string) => {
          this.socketText += chunk;
          buffer += chunk;
          const lines = buffer.split('\n');
          buffer = lines.pop();
          for (const line of lines) {
            const req = JSON.parse(line);
            this.socket.push(req);
            const result = req.method === 'calendar.sync' ? { started: true } : {};
            socket.write(`${JSON.stringify({ jsonrpc: '2.0', id: req.id, result })}\n`);
          }
        });
      });
      this.server.listen(this.socketPath, () => resolve());
    });
  }

  stop() {
    for (const c of this.conns) c.destroy();
    this.server.close();
  }

  get child() {
    return this.spawns[this.spawns.length - 1].child;
  }

  spawn(mode: string, args: string[]) {
    const child: any = new EventEmitter();
    child.pid = 4000 + this.spawns.length;
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.stdin = new EventEmitter();
    child.kill = jasmine.createSpy('kill');
    child.stdin.write = (line: string) => {
      for (const text of line.split('\n').filter(Boolean)) {
        const req = JSON.parse(text);
        this.stdin.push({ ...req, child });
        this._answer(child, req);
      }
    };
    this.spawns.push({ mode, args, child });
    return child;
  }

  _answer(child: any, req: any) {
    const respond = (result: any, error?: any) =>
      Promise.resolve().then(() => {
        const msg = error
          ? { jsonrpc: '2.0', id: req.id, error }
          : { jsonrpc: '2.0', id: req.id, result };
        child.stdout.emit('data', Buffer.from(`${JSON.stringify(msg)}\n`));
      });
    switch (req.method) {
      case 'account.list':
        return this.answerList ? respond({ accounts: [] }) : null;
      case 'account.add': {
        const id = req.params.account.id;
        const result = this.addResults[id] || { accountId: id, state: 'running' };
        delete this.addResults[id];
        const send = () => respond(result);
        if (this.holdAdds.has(id)) {
          this.heldAdds.push(send);
          return null;
        }
        return send();
      }
      case 'account.remove':
        return respond({ stillRunning: this.stillRunning });
      case 'engine.shutdown':
        if (!this.exitOnShutdown) return null;
        return respond({}).then(() => child.emit('exit', 0, null));
      default:
        return respond(null, { code: -32601, message: `unknown method ${req.method}` });
    }
  }

  // The requests of one method on stdin, in order.
  stdinOf(method: string) {
    return this.stdin.filter((r) => r.method === method);
  }

  socketOf(method: string) {
    return this.socket.filter((r) => r.method === method);
  }

  notify(method: string, params: unknown) {
    this.conn.write(`${JSON.stringify({ jsonrpc: '2.0', method, params })}\n`);
  }
}

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
      createEngine: () =>
        new EngineServeProcess({ configDirPath: dir, resourcePath: dir, syncEngine: 'rust' }),
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

  it('resolves the shutdown once the engine exits on its own', async function () {
    await started();
    await client.shutdown();
    expect(fake.child.kill).not.toHaveBeenCalled();
  });
});

describe('choosing the serve engine', function () {
  let saved: { [key: string]: string } = {};

  beforeEach(function () {
    saved = {
      TERN_ENGINE_PROCESS: process.env.TERN_ENGINE_PROCESS,
      TERN_ENGINE_TRANSPORT: process.env.TERN_ENGINE_TRANSPORT,
    };
    delete process.env.TERN_ENGINE_PROCESS;
    delete process.env.TERN_ENGINE_TRANSPORT;
  });

  afterEach(function () {
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  });

  it('runs the Rust engine as one serve process by default', function () {
    expect(serveEngineSelected('rust', '/tmp/tern')).toBe(process.platform !== 'win32');
    expect(serveEngineSelected('cpp', '/tmp/tern')).toBe(false);
  });

  it('keeps per-account processes when asked to', function () {
    process.env.TERN_ENGINE_PROCESS = 'per-account';
    expect(serveEngineSelected('rust', '/tmp/tern')).toBe(false);
    delete process.env.TERN_ENGINE_PROCESS;
    process.env.TERN_ENGINE_TRANSPORT = 'stdio';
    expect(serveEngineSelected('rust', '/tmp/tern')).toBe(false);
  });

  it('places the socket at run/engine.sock', function () {
    expect(serveSocketPath('/Users/me/.tern').socketPath).toEqual(
      path.join('/Users/me/.tern', 'run', 'engine.sock')
    );
  });
});
