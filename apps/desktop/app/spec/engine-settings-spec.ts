// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';
import ConfigPersistenceManager, {
  registerConfigIPC,
} from '../src/browser/config-persistence-manager';
import { EngineSupervisor, SupervisorConfig } from '../src/browser/engine-supervisor';
import { EngineSettingsMirror, PUSHED_KEY, touchesKey } from '../src/browser/engine-settings';
import { KeyManager } from '../src/browser/key-manager';
import { ConfigChannel } from '../src/config-channels';
import { EngineServeProcess } from '../src/mailsync-process';
import { FakeEngine, until } from './fake-serve-engine';

// The supervisor reads only `accounts` and the log level from the main process's Config.
const noAccounts: SupervisorConfig = {
  get: () => undefined,
  onDidChange: () => ({ dispose() {} }),
};

const SIGNATURE = {
  id: 's1',
  title: 'Work',
  body: '<b>Ann</b>',
  data: { title: '', templateName: '' },
};

describe('the engine-owned settings in the main process', function () {
  let dir: string = null;
  let fake: FakeEngine = null;
  let manager: ConfigPersistenceManager = null;
  let mirror: EngineSettingsMirror = null;
  let supervisor: EngineSupervisor = null;
  let changes: Array<{ settings: any; source?: number }> = [];
  let ready = false;

  const writeConfig = (settings: object) =>
    fs.writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ '*': settings }));
  const configOnDisk = () =>
    JSON.parse(fs.readFileSync(path.join(dir, 'config.json')).toString())['*'];
  const settingsCalls = () => fake.socket.filter((r) => r.method.startsWith('settings.'));

  // Launch as application.ts does: config.json loaded, then the supervisor and its mirror.
  const launch = async () => {
    changes = [];
    ready = false;
    manager = new ConfigPersistenceManager({
      configDirPath: dir,
      resourcePath: dir,
      onChange: (settings, source) =>
        changes.push({ settings: JSON.parse(JSON.stringify(settings)), source }),
    });
    mirror = new EngineSettingsMirror(manager);
    manager.engineSettings = mirror;
    supervisor = new EngineSupervisor({
      configDirPath: dir,
      resourcePath: dir,
      config: noAccounts,
      keyManager: new KeyManager({ config: { get: () => undefined, set: () => {} } as any }),
      broadcast: () => {},
      settings: mirror,
    });
    supervisor.start();
    mirror.whenReady(60000).then(() => (ready = true));
    await until('the first sync', () => ready);
  };

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
  });

  afterEach(function () {
    if (supervisor && supervisor.client.engine) supervisor.client.engine._closeSocket();
    fake.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('pushes config.json once, and from then on takes the engine values', async function () {
    fake.settings = {};
    writeConfig({
      core: { sending: { undoSend: 15000 }, themes: ['ui-dark'] },
      signatures: { s1: SIGNATURE },
    });
    await launch();
    expect(fake.settings).toEqual({
      'core.sending.undoSend': 15000,
      signatures: { s1: SIGNATURE },
    });
    // One call per key, and nothing that isn't engine-owned.
    expect(settingsCalls().filter((r) => r.method === 'settings.set').length).toBe(2);
    expect(configOnDisk()[PUSHED_KEY]).toBe(true);

    // Another client changed one key and cleared the other while the app was closed.
    supervisor.client.engine._closeSocket();
    fake.settings = { 'core.sending.undoSend': 30000, 'core.composing.html': false };
    fake.socket = [];
    await launch();
    expect(settingsCalls().map((r) => r.method)).toEqual(['settings.get']);
    expect(manager.get('core.sending.undoSend')).toBe(30000);
    expect(manager.get('core.composing.html')).toBe(false);
    expect(manager.get('signatures')).toBeUndefined();
    expect(manager.get('core.themes')).toEqual(['ui-dark']);
    // config.json keeps the keys too, for an engine without settings.
    expect(configOnDisk().core.sending.undoSend).toBe(30000);
  });

  it('gives a window booting after launch the engine signature synchronously', async function () {
    fake.settings = { signatures: { s1: SIGNATURE }, defaultSignatures: { 'ann@x.test': 's1' } };
    writeConfig({ [PUSHED_KEY]: true, signatures: {} });
    await launch();

    const handlers = {};
    registerConfigIPC({ on: (channel, fn) => (handlers[channel] = fn) } as any, manager);
    const event: any = { sender: { id: 9 } };
    handlers[ConfigChannel.GetRaw](event);
    const boot = JSON.parse(event.returnValue.value);
    expect(boot.signatures.s1.body).toBe('<b>Ann</b>');
    expect(boot.defaultSignatures['ann@x.test']).toBe('s1');

    // A window's write over the channel reaches the engine, and the other windows.
    handlers[ConfigChannel.SetRaw](event, 'defaultSignatures', { 'ann@x.test': null });
    expect(event.returnValue).toEqual({});
    await until('the set', () => fake.socketOf('settings.set').length === 1);
    expect(fake.socketOf('settings.set')[0].params).toEqual({
      values: { defaultSignatures: { 'ann@x.test': null } },
    });
    expect(changes[changes.length - 1].source).toBe(9);
  });

  it('forwards local writes of engine keys only and applies settings.changed', async function () {
    fake.settings = {};
    writeConfig({ [PUSHED_KEY]: true });
    await launch();

    manager.setRawValue('core.themes', ['ui-dark'], null);
    manager.setRawValue('core.sending', { undoSend: 60000, sounds: false }, null);
    await until('the set', () => fake.socketOf('settings.set').length === 1);
    expect(fake.socketOf('settings.set')[0].params).toEqual({
      values: { 'core.sending.undoSend': 60000 },
    });
    await until('the answer', () => mirror._inFlight.size === 0);

    const before = changes.length;
    fake.notify(
      'settings.changed',
      { values: { 'core.composing.html': false, 'core.sending.undoSend': null } },
      fake.conns[0]
    );
    await until('the change', () => changes.length > before);
    expect(manager.get('core.composing.html')).toBe(false);
    expect(manager.get('core.sending.undoSend')).toBeUndefined();
    expect(manager.get('core.sending.sounds')).toBe(false);
    expect(configOnDisk().core.composing.html).toBe(false);
    // Not sent back.
    expect(fake.socketOf('settings.set').length).toBe(1);
  });

  it('keeps a local write over a notification the engine applied before it', async function () {
    fake.settings = {};
    writeConfig({ [PUSHED_KEY]: true });
    await launch();
    const engine = supervisor.client.engine;
    manager.setRawValue('core.composing.html', false, null);
    // Arrives while the set is unanswered: the engine applied it first.
    mirror.engineChanged(engine, { values: { 'core.composing.html': true } });
    expect(manager.get('core.composing.html')).toBe(false);
    await until('the answer', () => mirror._inFlight.size === 0);
    mirror.engineChanged(engine, { values: { 'core.composing.html': true } });
    expect(manager.get('core.composing.html')).toBe(true);
  });

  it('leaves an engine without settings alone and keeps config.json', async function () {
    writeConfig({ core: { sending: { undoSend: 15000 } } });
    await launch();
    manager.setRawValue('core.sending.undoSend', 5000, null);
    expect(settingsCalls()).toEqual([]);
    expect(configOnDisk().core.sending.undoSend).toBe(5000);
    expect(configOnDisk()[PUSHED_KEY]).toBeUndefined();
  });

  it('matches writes to the keys they can change', function () {
    expect(touchesKey('core.sending', 'core.sending.undoSend')).toBe(true);
    expect(touchesKey('signatures.s1', 'signatures')).toBe(true);
    expect(touchesKey(null, 'signatures')).toBe(true);
    expect(touchesKey('core.sendingX', 'core.sending.undoSend')).toBe(false);
    expect(touchesKey('core.themes', 'core.sending.undoSend')).toBe(false);
  });
});
