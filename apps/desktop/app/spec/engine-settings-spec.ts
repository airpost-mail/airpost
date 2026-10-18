// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';
import ConfigPersistenceManager, {
  registerConfigIPC,
} from '../src/browser/config-persistence-manager';
import { EngineSupervisor, SupervisorConfig } from '../src/browser/engine-supervisor';
import {
  COPIED_KEY,
  ENGINE_SETTINGS_KEYS,
  EngineSettingsMirror,
  KEPT_KEY,
  STORE_KEY,
  touchesKey,
} from '../src/browser/engine-settings';
import { KeyManager } from '../src/browser/key-manager';
import { ConfigChannel } from '../src/config-channels';
import { resurfacingBodies } from '../src/resurfacing';
import { EngineServeProcess } from '../src/mailsync-process';
import { FakeEngine, until } from './fake-serve-engine';

// The supervisor reads only `accounts`, the deleted ones and the log level from the main
// process's Config.
const noAccounts: SupervisorConfig = {
  get: () => undefined,
  set: () => {},
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
    expect(configOnDisk()[STORE_KEY]).toBe('store-1');

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

  it('copies config.json again into a store the engine started anew', async function () {
    fake.settings = {};
    writeConfig({ core: { sending: { undoSend: 15000 } }, signatures: { s1: SIGNATURE } });
    await launch();
    supervisor.client.engine._closeSocket();

    // settings.json was lost or set aside: an empty store with a new id.
    fake.settings = {};
    fake.settingsStoreId = 'store-2';
    fake.socket = [];
    await launch();
    expect(manager.get('core.sending.undoSend')).toBe(15000);
    expect(manager.get('signatures')).toEqual({ s1: SIGNATURE });
    expect(fake.settings).toEqual({
      'core.sending.undoSend': 15000,
      signatures: { s1: SIGNATURE },
    });
    await until('the store id', () => configOnDisk()[STORE_KEY] === 'store-2');
  });

  it('copies a key mirrored since the store was copied, and takes the engine for the rest', async function () {
    const TRANSLATION = { provider: 'deepl', url: 'https://api.example.test' };
    fake.settings = { 'core.sending.undoSend': 30000 };
    fake.settingsFeatures = ['settings', 'integrations'];
    // A store copied by a build that didn't mirror the integrations yet: no COPIED_KEY.
    writeConfig({
      [STORE_KEY]: 'store-1',
      core: { sending: { undoSend: 15000 }, integrations: { translation: TRANSLATION } },
    });
    await launch();
    await until('the copied keys', () => Array.isArray(configOnDisk()[COPIED_KEY]));
    expect(fake.socketOf('settings.set').map((r) => r.params)).toEqual([
      { values: { 'core.integrations.translation': TRANSLATION } },
    ]);
    expect(manager.get('core.integrations.translation')).toEqual(TRANSLATION);
    expect(manager.get('core.sending.undoSend')).toBe(30000);
    expect([...configOnDisk()[COPIED_KEY]].sort()).toEqual([...ENGINE_SETTINGS_KEYS].sort());
    supervisor.client.engine._closeSocket();

    // Copied now: the engine wins for it too.
    delete fake.settings['core.integrations.translation'];
    fake.socket = [];
    await launch();
    expect(settingsCalls().map((r) => r.method)).toEqual(['settings.get']);
    expect(manager.get('core.integrations.translation')).toBeUndefined();
  });

  it('leaves the integration keys alone with an engine that does not mirror them', async function () {
    const TRANSLATION = { provider: 'deepl', url: 'https://api.example.test' };
    fake.settings = {};
    writeConfig({
      [STORE_KEY]: 'store-1',
      core: { integrations: { translation: TRANSLATION } },
    });
    await launch();
    expect(settingsCalls().map((r) => r.method)).toEqual(['settings.get']);
    expect(fake.socketOf('settings.get')[0].params.keys).not.toContain(
      'core.integrations.translation'
    );
    expect(manager.get('core.integrations.translation')).toEqual(TRANSLATION);

    // A local write stays in config.json and isn't sent.
    const GRAMMAR = { provider: 'languagetool' };
    manager.setRawValue('core.integrations.grammar', GRAMMAR, null);
    manager.setRawValue('core.sending.undoSend', 5000, null);
    await until('the set', () => fake.socketOf('settings.set').length > 0);
    expect(fake.socketOf('settings.set').map((r) => Object.keys(r.params.values))).toEqual([
      ['core.sending.undoSend'],
    ]);
    expect(configOnDisk().core.integrations).toEqual({
      translation: TRANSLATION,
      grammar: GRAMMAR,
    });
    supervisor.client.engine._closeSocket();

    // The next engine that mirrors them gets both.
    fake.settingsFeatures = ['settings', 'integrations'];
    fake.socket = [];
    await launch();
    await until('the copy', () => fake.settings['core.integrations.grammar'] !== undefined);
    expect(fake.settings['core.integrations.translation']).toEqual(TRANSLATION);
    expect(fake.settings['core.integrations.grammar']).toEqual(GRAMMAR);
    expect(manager.get('core.integrations.translation')).toEqual(TRANSLATION);
  });

  it('keeps config.json when the engine cannot read its settings', async function () {
    fake.settings = {};
    fake.socketErrors['settings.get'] = { code: -32603, message: 'could not read settings.json' };
    writeConfig({ [STORE_KEY]: 'store-1', core: { sending: { undoSend: 15000 } } });
    await launch();
    expect(settingsCalls().map((r) => r.method)).toEqual(['settings.get']);
    expect(manager.get('core.sending.undoSend')).toBe(15000);
    expect(configOnDisk().core.sending.undoSend).toBe(15000);
  });

  it('keeps a value whose copy the engine refused', async function () {
    fake.settings = {};
    fake.socketErrors['settings.set'] = { code: -32602, message: 'refused' };
    writeConfig({ core: { sending: { undoSend: 15000 } }, signatures: { s1: SIGNATURE } });
    await launch();
    await until('the store id', () => configOnDisk()[STORE_KEY] === 'store-1');
    expect(configOnDisk()[KEPT_KEY]).toEqual(['core.sending.undoSend', 'signatures']);
    expect(manager.get('core.sending.undoSend')).toBe(15000);
    supervisor.client.engine._closeSocket();

    // The same store, still without the values: config.json keeps them and copies nothing.
    // (The fake stores a set before it answers with the error.)
    delete fake.socketErrors['settings.set'];
    fake.settings = {};
    fake.socket = [];
    await launch();
    expect(settingsCalls().map((r) => r.method)).toEqual(['settings.get']);
    expect(manager.get('core.sending.undoSend')).toBe(15000);
    expect(manager.get('signatures')).toEqual({ s1: SIGNATURE });

    // Once the engine has a value, it wins and the key is no longer kept.
    const before = changes.length;
    fake.notify('settings.changed', { values: { 'core.sending.undoSend': 5000 } }, fake.conn);
    await until('the change', () => changes.length > before);
    expect(manager.get('core.sending.undoSend')).toBe(5000);
    expect(configOnDisk()[KEPT_KEY]).toEqual(['signatures']);
    // A local write the engine takes ends the other.
    manager.setRawValue('signatures', { s1: SIGNATURE, s2: SIGNATURE }, null);
    await until('the set', () => configOnDisk()[KEPT_KEY] === undefined);
    expect(fake.settings.signatures.s2).toEqual(SIGNATURE);
  });

  it('gives a window booting after launch the engine signature synchronously', async function () {
    fake.settings = { signatures: { s1: SIGNATURE }, defaultSignatures: { 'ann@x.test': 's1' } };
    writeConfig({ [STORE_KEY]: 'store-1', signatures: {} });
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
    writeConfig({ [STORE_KEY]: 'store-1' });
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
    writeConfig({ [STORE_KEY]: 'store-1' });
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

  it('writes the resurfacing texts in the configured language to an engine with the executor', async function () {
    fake.settings = {};
    fake.settingsFeatures = ['settings', 'expiration-executor'];
    writeConfig({ [STORE_KEY]: 'store-1', core: { intl: { language: 'de' } } });
    await launch();
    await until('the texts', () => fake.socketOf('settings.set').length === 1);
    expect(fake.settings['resurfacing.snoozeBody']).toContain('Tern-Erinnerung');
    expect(fake.settings['resurfacing.reminderBody']).toContain('Tern-Erinnerung');
    // Not config.json's: the windows build their own.
    expect(manager.get('resurfacing.snoozeBody')).toBeUndefined();

    // A store that has them already isn't written again.
    supervisor.client.engine._closeSocket();
    fake.socket = [];
    await launch();
    expect(fake.socketOf('settings.set').length).toBe(0);

    manager.setRawValue('core.intl.language', 'en', null);
    await until('the rewrite', () => fake.socketOf('settings.set').length === 1);
    expect(fake.settings['resurfacing.snoozeBody']).toBe(resurfacingBodies((en) => en).snoozeBody);
  });

  it('writes no resurfacing texts to an engine without the executor', async function () {
    fake.settings = {};
    writeConfig({ [STORE_KEY]: 'store-1' });
    await launch();
    manager.setRawValue('core.intl.language', 'de', null);
    // A later write that does reach the engine: the language change sent nothing before it.
    manager.setRawValue('core.sending.undoSend', 5000, null);
    await until('the set', () => fake.socketOf('settings.set').length > 0);
    expect(fake.socketOf('settings.set').map((r) => Object.keys(r.params.values))).toEqual([
      ['core.sending.undoSend'],
    ]);
  });

  it('leaves an engine without settings alone and keeps config.json', async function () {
    writeConfig({ core: { sending: { undoSend: 15000 } } });
    await launch();
    manager.setRawValue('core.sending.undoSend', 5000, null);
    expect(settingsCalls()).toEqual([]);
    expect(configOnDisk().core.sending.undoSend).toBe(5000);
    expect(configOnDisk()[STORE_KEY]).toBeUndefined();
  });

  it('matches writes to the keys they can change', function () {
    expect(touchesKey('core.sending', 'core.sending.undoSend')).toBe(true);
    expect(touchesKey('signatures.s1', 'signatures')).toBe(true);
    expect(touchesKey(null, 'signatures')).toBe(true);
    expect(touchesKey('core.sendingX', 'core.sending.undoSend')).toBe(false);
    expect(touchesKey('core.themes', 'core.sending.undoSend')).toBe(false);
  });
});
