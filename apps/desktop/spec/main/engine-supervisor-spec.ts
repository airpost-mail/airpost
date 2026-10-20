// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { spawn } from 'child_process';
import fs from 'fs';
import path from 'path';
import {
  detachedEngineToReuse,
  EngineSupervisor,
  SupervisorConfig,
  SupervisorTarget,
  waitForEarlierEngine,
} from '../../src/main/engine-supervisor';
import { KeyManager, secureStorage } from '../../src/main/key-manager';
import { EngineConnection } from '../../src/engine/engine-connection';
import { engineBuildId, EngineServeProcess } from '../../src/shared/engine-process';
import { EngineStatus, SupervisorChannel } from '../../src/shared/supervisor-channels';
import { FakeEngine, tick, until } from '../fake-serve-engine';

// config.json as the main process's Config holds it, with onDidChange.
class FakeConfig implements SupervisorConfig {
  values: { [key: string]: any } = {};
  listeners: { [key: string]: Array<(event: unknown) => void> } = {};

  get(key: string) {
    return this.values[key];
  }

  set(key: string, value: any) {
    this.values[key] = value;
    for (const cb of this.listeners[key] || []) cb({ newValue: value });
  }

  onDidChange(key: string, cb: (event: unknown) => void) {
    (this.listeners[key] = this.listeners[key] || []).push(cb);
    return {
      dispose: () => {
        this.listeners[key] = this.listeners[key].filter((l) => l !== cb);
      },
    };
  }
}

// A window's renderer as the supervisor sees it: what it was sent.
class FakeTarget implements SupervisorTarget {
  sent: Array<{ channel: string; payload: any }> = [];

  send(channel: string, payload: any) {
    this.sent.push({ channel, payload });
  }

  events() {
    return this.sent
      .filter((m) => m.channel === SupervisorChannel.AccountEvent)
      .map((m) => m.payload);
  }
}

const configAccount = (id: string) => ({
  id,
  emailAddress: `${id}@example.com`,
  provider: 'imap',
  syncState: 'ok',
  settings: { imap_host: 'imap.example.com' },
});

describe('the engine supervisor', function () {
  let dir: string = null;
  let fake: FakeEngine = null;
  let config: FakeConfig = null;
  let keyManager: KeyManager = null;
  let supervisor: EngineSupervisor = null;
  let statuses: EngineStatus[] = [];
  let windows: EngineConnection[] = [];

  beforeEach(async function () {
    dir = fs.mkdtempSync('/tmp/tern-spec-');
    fake = new FakeEngine(dir);
    await fake.start();
    spyOn(EngineServeProcess.prototype as any, '_spawnProcess').andCallFake(function (
      mode: string,
      args: string[]
    ) {
      this._proc = fake.spawn(mode, args);
    });
    spyOn(console, 'log');
    spyOn(console, 'warn');

    // The keyset in plain text: nothing here reaches safeStorage or a keychain.
    spyOn(secureStorage, 'isAvailable').andCallFake(() => Promise.resolve(true));
    spyOn(secureStorage, 'encrypt').andCallFake((text: string) =>
      Promise.resolve(Buffer.from(text))
    );
    spyOn(secureStorage, 'decrypt').andCallFake((blob: Buffer) =>
      Promise.resolve({ result: blob.toString(), shouldReEncrypt: false })
    );
    config = new FakeConfig();
    config.values.credentials = Buffer.from(
      JSON.stringify({
        'a1@example.com-imap': 'pw-a1',
        'a2@example.com-imap': 'pw-a2',
        'a3@example.com-imap': 'pw-a3',
      })
    );
    config.values.credentialsKeyOwner = 'tern';
    config.values.accounts = [configAccount('a1'), configAccount('a2')];
    keyManager = new KeyManager({ config });

    statuses = [];
    windows = [];
    supervisor = new EngineSupervisor({
      configDirPath: dir,
      resourcePath: dir,
      config,
      keyManager,
      broadcast: (channel, status: EngineStatus) => {
        expect(channel).toBe(SupervisorChannel.Status);
        statuses.push(status);
        for (const w of windows) w.setStatus(status);
      },
    });
  });

  afterEach(function () {
    for (const w of windows) w.close();
    if (supervisor.client.engine) supervisor.client.engine._closeSocket();
    fake.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const lastStatus = () => statuses[statuses.length - 1];

  const started = async () => {
    supervisor.start();
    await until('the adds', () => fake.stdinOf('account.add').length >= 2);
    await until('its subscription', () => (fake.connRequests[0] || []).length >= 2);
    await until('both added', () => {
      const s = lastStatus();
      return !!s && !!s.socketPath && Object.values(s.accounts).every((a) => a.state === 'added');
    });
  };

  const openWindow = ({ mainWindow = false } = {}) => {
    const w = new EngineConnection({ clientName: 'tern-desktop/spec', mainWindow });
    windows.push(w);
    w.setStatus(supervisor.status());
    return w;
  };

  it('adds the accounts in config.json with their secrets on stdin only', async function () {
    await started();

    expect(fake.spawns.map((s) => s.mode)).toEqual(['serve']);
    const added = fake.stdinOf('account.add').map((r) => r.params.account);
    expect(added.map((a) => [a.id, a.settings.imap_password, a.settings.imap_host])).toEqual([
      ['a1', 'pw-a1', 'imap.example.com'],
      ['a2', 'pw-a2', 'imap.example.com'],
    ]);
    // Its own connection takes account.state and no deltas, leaving the backlog to a window.
    expect(fake.connRequests[0].map((r) => r.method)).toEqual(['hello', 'subscribe']);
    expect(fake.connRequests[0][1].params).toEqual({ accounts: null, classes: [] });
    expect(fake.socketText).not.toContain('pw-a1');
    expect(lastStatus()).toEqual({
      generation: 1,
      socketPath: fake.socketPath,
      accounts: { a1: { state: 'added', held: false }, a2: { state: 'added', held: false } },
    });
  });

  it('follows the accounts AccountStore saves to config.json', async function () {
    await started();
    config.set('accounts', [configAccount('a2'), configAccount('a3')]);
    await until('the remove and the add', () => fake.stdinOf('account.add').length === 3);
    await until('a1 gone', () => !lastStatus().accounts.a1);

    expect(fake.stdinOf('account.remove').map((r) => r.params)).toEqual([{ accountId: 'a1' }]);
    expect(fake.stdinOf('account.add')[2].params.account.settings.imap_password).toBe('pw-a3');
    expect(fake.spawns.length).toBe(1);
  });

  it('stores a token rotation from stdout in KeyManager in main', async function () {
    await started();
    const main = new FakeTarget();
    supervisor.attachMainWindow(main);
    const rotated = {
      ...configAccount('a1'),
      settings: { imap_password: 'pw-a1', refresh_token: 'rotated-token' },
    };
    const secrets = {
      accountId: 'a1',
      modelClass: 'ProcessAccountSecretsUpdated',
      type: 'persist',
      modelJSONs: [rotated],
    };
    fake.child.stdout.emit(
      'data',
      Buffer.from(`${JSON.stringify({ jsonrpc: '2.0', method: 'delta', params: secrets })}\n`)
    );
    await until('the stored token', () => {
      const keys = JSON.parse(config.values.credentials.toString());
      return keys['a1@example.com-refresh-token'] === 'rotated-token';
    });
    expect(await keyManager.getPassword('a2@example.com-imap')).toBe('pw-a2');
    expect(main.sent).toEqual([]);
  });

  it('gives each window its own connection and moves them to a respawned engine', async function () {
    await started();
    const mainWindow = openWindow({ mainWindow: true });
    const composer = openWindow();
    const mainDeltas: string[] = [];
    const composerDeltas: string[] = [];
    mainWindow.on('deltas', (msgs) => mainDeltas.push(...msgs));
    composer.on('deltas', (msgs) => composerDeltas.push(...msgs));
    await until(
      'three connections',
      () => fake.connRequests.filter((r) => r.length >= 2).length === 3
    );

    const thread = { accountId: 'a1', modelClass: 'Thread', type: 'persist', modelJSONs: [] };
    fake.publishDelta(thread);
    await until('the deltas', () => mainDeltas.length === 1 && composerDeltas.length === 1);
    expect(JSON.parse(composerDeltas[0])).toEqual(thread);
    expect(fake.connRequests[1][1].params).toEqual({ accounts: null, classes: null });
    expect(fake.connRequests[2][1].params).toEqual({ accounts: ['a1', 'a2'], classes: null });

    fake.child.emit('exit', 101, null);
    await until('no engine', () => lastStatus().generation === null);
    expect(mainWindow.send('a1', { type: 'wake-workers' })).toBe(false);

    advanceClock(1);
    await until('the second engine', () => fake.spawns.length === 2);
    await until(
      'both re-added',
      () => lastStatus().generation === 2 && lastStatus().accounts.a2?.state === 'added'
    );
    await until(
      'the windows on it',
      () => mainWindow._generation === 2 && composer._generation === 2
    );
    await until(
      'their subscriptions',
      () => fake.connRequests.filter((r) => r.length >= 2).length === 6
    );
    // The composer's connection is the one subscribed to a list of accounts.
    const composerAccounts = () =>
      fake.connRequests
        .slice(3)
        .map((reqs) => reqs.filter((r) => r.method === 'subscribe').pop())
        .filter((sub) => sub && Array.isArray(sub.params.accounts))
        .map((sub) => sub.params.accounts.join(','));
    await until('the composer on both accounts', () => composerAccounts()[0] === 'a1,a2');

    fake.publishDelta(thread);
    await until('the deltas', () => mainDeltas.length === 2 && composerDeltas.length === 2);
  });

  it('keeps the engine through a main-window reload and replays the events it missed', async function () {
    await started();
    const before = new FakeTarget();
    expect(
      supervisor.attachMainWindow(before).map((e: any) => [e.type, e.accountId, e.syncState])
    ).toEqual([
      ['launched', 'a1', 'ok'],
      ['launched', 'a2', 'ok'],
    ]);
    supervisor.ackAccountEvents(2);

    // An account stops while the renderer reloads; the supervisor re-adds it.
    supervisor.detachMainWindow(before);
    const error = { key: 'ErrorConnection', debuginfo: 'timed out', retryable: true };
    fake.notify('account.state', { accountId: 'a1', state: 'stopped', error }, fake.conns[0]);
    await until('the re-add', () => fake.stdinOf('account.add').length === 3);

    const after = new FakeTarget();
    const missed = supervisor.attachMainWindow(after);
    expect(missed.map((e) => [e.type, e.accountId])).toEqual([
      ['exited', 'a1'],
      ['launched', 'a1'],
    ]);
    expect(before.events()).toEqual([]);
    expect(fake.spawns.length).toBe(1);
    expect(fake.child.kill).not.toHaveBeenCalled();
    expect(fake.stdinOf('engine.shutdown')).toEqual([]);

    supervisor.ackAccountEvents((missed[1] as any).seq);
    expect(supervisor.attachMainWindow(after)).toEqual([]);
  });

  it('tells the main window when an account gave up, with its error', async function () {
    await started();
    const main = new FakeTarget();
    supervisor.attachMainWindow(main);
    const error = { key: 'ErrorAuthentication', debuginfo: 'bad password', retryable: false };
    for (let i = 1; i <= 5; i++) {
      fake.notify('account.state', { accountId: 'a1', state: 'stopped', error }, fake.conns[0]);
      if (i < 5) await until(`re-add ${i}`, () => fake.stdinOf('account.add').length === 2 + i);
    }
    await until('the cutoff', () => main.events().some((e) => e.type === 'too-many-failures'));
    const gaveUp = main.events().find((e) => e.type === 'too-many-failures');
    expect(gaveUp.accountId).toBe('a1');
    expect(gaveUp.authFailure).toBe(true);
    expect(gaveUp.exit.error.message).toContain('ErrorAuthentication');
    expect(gaveUp.exit.error.syncError).toEqual(error);
    expect(lastStatus().accounts.a1).toBeUndefined();

    await supervisor.forceRelaunch('a1');
    await until('the forced add', () => fake.stdinOf('account.add').length === 7);
  });

  it('resets an account with account.reset while it is out of the engine', async function () {
    await started();
    let heldDuringReset = false;
    spyOn(EngineServeProcess.prototype, 'resetAccount').andCallFake(function (id: string) {
      heldDuringReset = supervisor.status().accounts.a1.held;
      return this._parentCall('account.reset', { accountId: id });
    });
    await supervisor.resetCache('a1');
    expect(heldDuringReset).toBe(true);
    expect(fake.stdinOf('account.reset').map((r) => r.params)).toEqual([{ accountId: 'a1' }]);
    expect(fake.stdinOf('account.remove').map((r) => r.params)).toEqual([{ accountId: 'a1' }]);
    await until('the add', () => fake.stdinOf('account.add').length === 3);
  });

  it('drops the plugin metadata of an account being removed', async function () {
    await started();
    await supervisor.resetCache('a1', { keepMetadata: false });
    expect(fake.stdinOf('account.reset').map((r) => r.params)).toEqual([
      { accountId: 'a1', keepMetadata: false },
    ]);
    await supervisor.resetDatabase({ keepMetadata: false });
    expect(fake.stdinOf('engine.resetDatabase').map((r) => r.params)).toEqual([
      { keepMetadata: false },
    ]);
  });

  it('passes a verbose logging change to the running engine', async function () {
    await started();
    const until30 = Date.now() + 30 * 60 * 1000;
    config.set('core.sync.verboseUntil', until30);
    await until('the call', () => fake.stdinOf('engine.setLogLevel').length === 1);
    expect(fake.stdinOf('engine.setLogLevel')[0].params).toEqual({ verboseUntil: until30 });
    expect(fake.spawns.length).toBe(1);
  });

  it('empties the database through the engine and leaves its accounts out', async function () {
    await started();
    await supervisor.resetDatabase();
    expect(fake.stdinOf('engine.resetDatabase').map((r) => r.params)).toEqual([{}]);
    expect(lastStatus().accounts).toEqual({});
    config.set('accounts', [configAccount('a1'), configAccount('a2'), configAccount('a3')]);
    await tick();
    expect(fake.stdinOf('account.add').length).toBe(2);
  });

  it('stops the engine on shutdown and starts none after', async function () {
    await started();
    await supervisor.shutdown();
    expect(fake.stdinOf('engine.shutdown').length).toBe(1);
    config.set('accounts', [configAccount('a3')]);
    await tick();
    advanceClock(10000);
    await tick();
    expect(fake.spawns.length).toBe(1);
    expect(lastStatus().socketPath).toBe(null);
  });

  describe('with an engine that keeps the accounts', function () {
    const keys = () => JSON.parse(config.values.credentials.toString());
    const addedPasswords = () =>
      fake
        .stdinOf('account.add')
        .map((r) => [r.params.account.id, r.params.account.settings.imap_password]);

    it('imports the accounts with their secrets before adding them, then drops them from KeyManager', async function () {
      config.values.credentials = Buffer.from(
        JSON.stringify({
          'a1@example.com-imap': 'pw-a1',
          'a2@example.com-imap': 'pw-a2',
          'integration-deepl-key': 'k',
        })
      );
      fake.keepsAccounts([]);
      await started();

      const methods = fake.stdin.map((r) => r.method);
      expect(methods.filter((m) => m === 'credentials.import').length).toBe(1);
      expect(methods.indexOf('credentials.import')).toBeLessThan(methods.indexOf('account.add'));
      const [imported] = fake.stdinOf('credentials.import');
      expect(imported.params.accounts.map((a) => [a.id, a.settings.imap_password])).toEqual([
        ['a1', 'pw-a1'],
        ['a2', 'pw-a2'],
      ]);
      expect(imported.params.secrets).toEqual({ 'integration-deepl-key': 'k' });
      expect(addedPasswords()).toEqual([
        ['a1', undefined],
        ['a2', undefined],
      ]);
      expect(fake.socketText).not.toContain('pw-a1');
      expect(config.values.credentialsMovedToEngine).toBe(true);
      expect(keys()).toEqual({ 'integration-deepl-key': 'k' });
      const before = JSON.parse(config.values.credentialsBeforeEngine.toString());
      expect(before['a1@example.com-imap']).toBe('pw-a1');
    });

    it('imports what a downgraded build added, from the keyset kept before the move', async function () {
      config.values.credentialsMovedToEngine = true;
      config.values.credentialsBeforeEngine = Buffer.from(
        JSON.stringify({
          'a1@example.com-imap': 'old-a1',
          'a2@example.com-imap': 'old-a2',
          'a3@example.com-imap': 'pw-a3',
        })
      );
      // The older build signed a2 in again and added a3, which the engine never saw.
      config.values.credentials = Buffer.from(JSON.stringify({ 'a2@example.com-imap': 'new-a2' }));
      config.values.accounts = [configAccount('a1'), configAccount('a2'), configAccount('a3')];
      fake.keepsAccounts(['a1', 'a2']);
      supervisor.start();
      await until('the adds', () => fake.stdinOf('account.add').length === 3);

      const [imported] = fake.stdinOf('credentials.import');
      expect(imported.params.accounts.map((a) => [a.id, a.settings.imap_password])).toEqual([
        ['a2', 'new-a2'],
        ['a3', 'pw-a3'],
      ]);
      expect(imported.params.secrets).toEqual({});
      expect(addedPasswords().every(([, pw]) => pw === undefined)).toBe(true);
      expect(keys()).toEqual({});
    });

    it('drops the secrets of an account the engine stored on account.add', async function () {
      fake.keepsAccounts(['a1', 'a2']);
      config.values.credentialsMovedToEngine = true;
      config.values.credentials = Buffer.from('{}');
      await started();
      expect(fake.stdinOf('credentials.import')).toEqual([]);

      // Onboarding stores a new account's secrets, then AccountStore saves it.
      await keyManager.extractAndStoreAccountSecrets({
        ...configAccount('a3'),
        settings: { imap_password: 'pw-a3' },
      });
      fake.addResults.a3 = { accountId: 'a3', state: 'running', stored: true };
      config.set('accounts', [configAccount('a1'), configAccount('a2'), configAccount('a3')]);
      await until('the add', () => fake.stdinOf('account.add').length === 3);
      expect(fake.stdinOf('account.add')[2].params.account.settings.imap_password).toBe('pw-a3');
      await until('the dropped copy', () => !keys()['a3@example.com-imap']);
    });

    it('leaves the secrets in KeyManager while the engine refuses an account', async function () {
      fake.keepsAccounts([]);
      fake.importResult = {
        imported: ['a1'],
        failed: [{ accountId: 'a2', email: 'a2@example.com', error: 'no imap_password' }],
        secrets: [],
      };
      await started();
      // account.add answered `stored` for both; the copies stay until the move completes.
      for (let i = 0; i < 20; i++) await tick();

      expect(config.values.credentialsMovedToEngine).toBe(undefined);
      expect(addedPasswords()).toEqual([
        ['a1', 'pw-a1'],
        ['a2', 'pw-a2'],
      ]);
      expect(keys()['a1@example.com-imap']).toBe('pw-a1');
      expect(keys()['a2@example.com-imap']).toBe('pw-a2');
    });

    it('moves the secrets even with an account that has none to move', async function () {
      config.values.accounts = [configAccount('a1'), configAccount('a2'), configAccount('a4')];
      fake.keepsAccounts([]);
      supervisor.start();
      await until('the adds', () => fake.stdinOf('account.add').length === 3);

      const [imported] = fake.stdinOf('credentials.import');
      expect(imported.params.accounts.map((a) => a.id)).toEqual(['a1', 'a2']);
      expect(config.values.credentialsMovedToEngine).toBe(true);
    });

    it('remembers an account deleted while no engine runs and removes it from the next', async function () {
      config.values.credentialsMovedToEngine = true;
      config.values.credentials = Buffer.from('{}');
      config.values.accountsToRemoveFromEngine = ['gone'];
      fake.keepsAccounts(['a1', 'a2', 'other', 'gone']);
      await started();
      await until('the remove', () => fake.stdinOf('account.remove').length === 1);
      expect(fake.stdinOf('account.remove')[0].params).toEqual({ accountId: 'gone' });
      await until('the list emptied', () => config.values.accountsToRemoveFromEngine.length === 0);

      config.set('accounts', [configAccount('a1')]);
      expect(config.values.accountsToRemoveFromEngine).toEqual(['a2']);
      await until('the second remove', () => fake.stdinOf('account.remove').length === 2);
      await until('the list emptied', () => config.values.accountsToRemoveFromEngine.length === 0);
      expect(fake.stdinOf('account.remove').map((r) => r.params.accountId)).toEqual(['gone', 'a2']);
    });

    it('joins a detached engine on its socket and leaves it running', async function () {
      config.values.credentialsMovedToEngine = true;
      config.values.credentials = Buffer.from('{}');
      // `other` was added by another client (`tern account add`, the web).
      fake.keepsAccounts(['a1', 'a2', 'other'], { detached: true });
      fake.socketResults['account.add'] = { state: 'running' };
      const joined = new EngineSupervisor({
        configDirPath: dir,
        resourcePath: dir,
        config,
        keyManager,
        attachTo: fake.socketPath,
        broadcast: (_channel, status: EngineStatus) => statuses.push(status),
      });
      joined.start();
      await until('the adds', () => fake.socketOf('account.add').length === 2);
      await until(
        'the ready engine',
        () => !!lastStatus() && lastStatus().socketPath === fake.socketPath
      );

      expect(fake.spawns).toEqual([]);
      expect(fake.stdin).toEqual([]);
      expect(fake.socketOf('account.add').map((r) => r.params.account.id)).toEqual(['a1', 'a2']);
      expect(fake.socketOf('credentials.import')).toEqual([]);
      for (let i = 0; i < 20; i++) await tick();
      expect(fake.socketOf('account.remove')).toEqual([]);
      await joined.shutdown();
      expect(fake.socketOf('engine.shutdown')).toEqual([]);
    });

    it('reuses a detached engine running the bundled build and stops any other', async function () {
      const binary = path.join(dir, 'mailsync-rs');
      fs.writeFileSync(binary, 'the bundled engine');
      const build = await engineBuildId(binary);
      const child = spawn('/bin/sleep', ['30']);
      const discovery = (extra: any) =>
        fs.writeFileSync(
          path.join(dir, 'run', 'engine.json'),
          JSON.stringify({ pid: child.pid, socket: fake.socketPath, protocol: 1, ...extra })
        );
      try {
        discovery({ detached: true });
        fake.keepsAccounts([], { build, detached: true });
        expect(await detachedEngineToReuse(dir, dir, { canReuse: true })).toBe(fake.socketPath);
        expect(fake.socketOf('engine.shutdown')).toEqual([]);

        // Credentials still in the app: the bundled engine has to import them on its stdin.
        expect(await detachedEngineToReuse(dir, dir, { canReuse: false })).toBe(null);
        expect(fake.socketOf('engine.shutdown').length).toBe(1);

        // An older engine left running across an update.
        fake.keepsAccounts([], { build: 'sha256:older', detached: true });
        expect(await detachedEngineToReuse(dir, dir, { canReuse: true })).toBe(null);
        expect(fake.socketOf('engine.shutdown').length).toBe(2);
        fake.keepsAccounts([], { build, detached: true, protocol: 2 });
        expect(await detachedEngineToReuse(dir, dir, { canReuse: true })).toBe(null);
        expect(fake.socketOf('engine.shutdown').length).toBe(3);

        // An engine an earlier app run spawned exits on its own once its stdin closed.
        discovery({});
        expect(await detachedEngineToReuse(dir, dir, { canReuse: true })).toBe(null);
        expect(fake.socketOf('hello').length).toBe(7);
      } finally {
        child.kill();
      }
    });

    it('restarts an engine unit of another build and joins the restarted one', async function () {
      const binary = path.join(dir, 'mailsync-rs');
      fs.writeFileSync(binary, 'the bundled engine');
      const build = await engineBuildId(binary);
      const children = [spawn('/bin/sleep', ['30']), spawn('/bin/sleep', ['30'])];
      const discovery = (pid: number) =>
        fs.writeFileSync(
          path.join(dir, 'run', 'engine.json'),
          JSON.stringify({ pid, socket: fake.socketPath, protocol: 1, detached: true })
        );
      try {
        // The unit still runs the build from before the update.
        discovery(children[0].pid);
        fake.keepsAccounts([], { build: 'sha256:older', detached: true });
        let restarts = 0;
        const restartUnit = async () => {
          restarts++;
          fake.keepsAccounts([], { build, detached: true });
          discovery(children[1].pid);
          return true;
        };
        expect(await detachedEngineToReuse(dir, dir, { canReuse: true, restartUnit })).toBe(
          fake.socketPath
        );
        expect(restarts).toBe(1);
        expect(fake.socketOf('engine.shutdown')).toEqual([]);

        // Still another build after the restart: stopped, and the app runs its own.
        discovery(children[0].pid);
        fake.keepsAccounts([], { build: 'sha256:older', detached: true });
        const sameBuild = async () => {
          restarts++;
          discovery(children[1].pid);
          return true;
        };
        expect(
          await detachedEngineToReuse(dir, dir, { canReuse: true, restartUnit: sameBuild })
        ).toBe(null);
        expect(restarts).toBe(2);
        expect(fake.socketOf('engine.shutdown').length).toBe(1);

        // A unit that doesn't come back leaves nothing to stop.
        discovery(children[0].pid);
        const gone = async () => {
          restarts++;
          return true;
        };
        expect(
          await detachedEngineToReuse(dir, dir, {
            canReuse: true,
            restartUnit: gone,
            restartWaitMs: 0,
          })
        ).toBe(null);
        expect(fake.socketOf('engine.shutdown').length).toBe(1);

        // Credentials still in the app: a restart wouldn't make it joinable.
        expect(await detachedEngineToReuse(dir, dir, { canReuse: false, restartUnit })).toBe(null);
        expect(restarts).toBe(3);
        expect(fake.socketOf('engine.shutdown').length).toBe(2);
      } finally {
        for (const child of children) child.kill();
      }
    });
  });

  it('starts the engine without accounts, so onboarding has one', async function () {
    config.values.accounts = [];
    supervisor.start();
    await until('the ready engine', () => !!lastStatus() && !!lastStatus().socketPath);
    expect(fake.spawns.length).toBe(1);
    expect(fake.stdinOf('account.add').length).toBe(0);
  });
});

describe('waiting for an earlier engine before migrating', function () {
  let dir: string = null;

  beforeEach(function () {
    dir = fs.mkdtempSync('/tmp/tern-spec-');
    fs.mkdirSync(path.join(dir, 'run'));
  });

  afterEach(function () {
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const writeDiscovery = (pid: number) =>
    fs.writeFileSync(
      path.join(dir, 'run', 'engine.json'),
      JSON.stringify({
        pid,
        socket: path.join(dir, 'run', 'engine.sock'),
        protocol: 1,
        accounts: [],
      })
    );

  it('goes ahead without a discovery file or with a dead pid', async function () {
    expect(await waitForEarlierEngine(dir)).toBe(true);
    const child = spawn('/bin/sh', ['-c', 'exit 0']);
    await new Promise((resolve) => child.on('exit', resolve));
    writeDiscovery(child.pid);
    expect(await waitForEarlierEngine(dir)).toBe(true);
  });

  it('waits for a live engine pid to exit', async function () {
    spyOn(console, 'log');
    const child = spawn('/bin/sleep', ['30']);
    writeDiscovery(child.pid);
    let done: boolean = null;
    waitForEarlierEngine(dir, { pollMs: 100 }).then((result) => (done = result));
    for (let i = 0; i < 5; i++) {
      advanceClock(100);
      await tick();
    }
    expect(done).toBe(null);

    child.kill();
    await new Promise((resolve) => child.on('exit', resolve));
    advanceClock(100);
    await until('the wait', () => done !== null);
    expect(done).toBe(true);
  });

  it('gives up on an engine that outlives the timeout', async function () {
    spyOn(console, 'log');
    spyOn(console, 'warn');
    const child = spawn('/bin/sleep', ['30']);
    writeDiscovery(child.pid);
    try {
      expect(await waitForEarlierEngine(dir, { timeoutMs: 0 })).toBe(false);
    } finally {
      child.kill();
    }
  });
});
