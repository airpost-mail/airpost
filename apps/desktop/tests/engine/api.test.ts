// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import fs from 'fs';
import path from 'path';
import { EngineEvents } from '../../src/engine/events';
import { EngineConnection } from '../../src/engine/connection';
import { ErrorCode, Feature, Method } from '../../src/shared/engine/protocol';
import {
  CallerPackage,
  ENGINE_METHOD_APPROVALS_KEY,
  EngineAPI,
  EngineRequestRefused,
  callerPackageFromStack,
  stackPaths,
  wireIntegrationsTransport,
} from '../../src/engine/api';
import { Integrations, IntegrationError } from '../../src/engine/integrations';
import { CapabilityStoreClass } from '../../src/data/stores/capability';
import { feedEngineEvents } from '../../src/engine/bridge';
import { EngineStatus } from '../../src/shared/engine/supervisor-channels';
import { FakeEngine, until } from '../support/fake-serve-engine';

const capabilities = (translation: object) => ({
  integrations: {
    translation,
    grammar: { available: false, provider: 'languagetool', reason: 'not-set-up' },
    enrichment: { available: false, provider: 'peopledatalabs', reason: 'not-set-up' },
  },
  server: { readReceipts: { available: false, reason: 'needs-tern-server' } },
});

const hello = (features: string[], caps: object = null) => ({
  protocol: 1,
  engine: 'fake',
  pid: 1,
  accounts: [],
  features,
  ...(caps ? { capabilities: caps } : {}),
});

describe('Engine and the integrations transport on a window connection', function () {
  let dir: string = null;
  let fake: FakeEngine = null;
  let connection: EngineConnection = null;
  let events: EngineEvents = null;
  let unwire: () => void = null;
  let store: CapabilityStoreClass = null;

  beforeEach(async function () {
    dir = fs.mkdtempSync('/tmp/tern-spec-');
    fake = new FakeEngine(dir);
    await fake.start();
    events = new EngineEvents();
    spyOn(console, 'log');
    spyOn(console, 'warn');
    unwire = wireIntegrationsTransport(events);
    // A keyset that is a plain object: nothing here reaches the real KeyManager or a keychain.
    Integrations.keySource = {
      getPassword: async (name: string) =>
        name === 'integration-deepl-key' ? 'synthetic-deepl-key' : undefined,
    };
  });

  afterEach(function () {
    if (store) store.stop();
    store = null;
    if (connection) connection.close();
    connection = null;
    unwire();
    Integrations.setTransport(null);
    Integrations.keySource = null;
    fake.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const open = () => {
    connection = new EngineConnection({ clientName: 'tern-desktop/spec', mainWindow: false });
    feedEngineEvents(connection, events);
    const status: EngineStatus = {
      generation: 1,
      socketPath: fake.socketPath,
      accounts: {},
    };
    connection.setStatus(status);
  };

  it('sends integrations.* with the saved config and key once the engine lists integrations', async function () {
    const changes = jasmine.createSpy('transport-changed');
    const stop = Integrations.onTransportChange(changes);
    expect(Integrations.isAvailable()).toBe(false);
    fake.socketResults.hello = hello([Feature.Integrations]);
    fake.socketResults[Method.IntegrationsTranslate] = { translatedText: 'Hola' };
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });

    open();
    await until('hello', () => events.has(Feature.Integrations));
    expect(Integrations.isAvailable()).toBe(true);
    expect(changes).toHaveBeenCalled();

    const result = await Integrations.request(Method.IntegrationsTranslate, {
      text: '<b>Hi</b>',
      target: 'es',
      format: 'html',
    });
    expect(result).toEqual({ translatedText: 'Hola' });
    expect(fake.socketOf(Method.IntegrationsTranslate)[0].params).toEqual({
      text: '<b>Hi</b>',
      target: 'es',
      format: 'html',
      config: { provider: 'deepl', apiKey: 'synthetic-deepl-key' },
    });
    stop();
  });

  it('turns engine errors into IntegrationErrors', async function () {
    fake.socketResults.hello = hello([Feature.Integrations]);
    const config = { provider: 'libretranslate' as const, url: 'http://127.0.0.1:9' };
    open();
    await until('hello', () => events.has(Feature.Integrations));

    const failure = (method: string) =>
      Integrations.request(method as any, {}, { config }).then(
        () => null,
        (e) => e as IntegrationError
      );
    fake.socketErrors[Method.IntegrationsTest] = {
      code: ErrorCode.IntegrationFailed,
      message: 'Integration failed: the provider rejected the credentials (HTTP 403)',
      data: { key: 'auth', message: 'rejected', retryable: false, status: 403 },
    } as any;
    let err = await failure(Method.IntegrationsTest);
    expect(err instanceof IntegrationError).toBe(true);
    expect(err.key).toBe('auth');
    expect(err.status).toBe(403);

    fake.socketErrors[Method.IntegrationsTest] = {
      code: ErrorCode.EngineBusy,
      message: 'the engine is busy',
    };
    err = await failure(Method.IntegrationsTest);
    expect(err.key).toBe('rate-limited');
    expect(err.retryable).toBe(true);

    connection.close();
    await until('gone', () => !events.features);
    expect(Integrations.isAvailable()).toBe(false);
    err = await failure(Method.IntegrationsTest);
    expect(err.key).toBe('engine-unavailable');
  });

  it('keeps the capabilities of hello and capabilities.changed, and the store follows them', async function () {
    fake.socketResults.hello = hello(
      [Feature.Integrations, Feature.Capabilities],
      capabilities({ available: false, reason: 'not-set-up' })
    );
    store = new CapabilityStoreClass({ autoStart: false });
    store.engineEvents = events;
    store.keySource = Integrations.keySource;
    store.start();
    const engine = new EngineAPI(events);
    const seen = [];
    const sub = engine.subscribe({ methods: [Method.CapabilitiesChanged] }, (n) => seen.push(n));
    const changed = jasmine.createSpy('changed');
    const watch = engine.onDidChange(changed);
    // Config says it is on; the engine hasn't seen that yet.
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });

    open();
    await until('hello', () => engine.connected);
    expect(engine.features).toEqual([Feature.Integrations, Feature.Capabilities]);
    expect(engine.capabilities.integrations.translation.available).toBe(false);
    expect(store.get('translation').reason).toBe('Set up a provider in Preferences > Integrations');
    expect(store.get('read-receipts').reason).toBe('Needs a Tern server');

    fake.notify(
      Method.CapabilitiesChanged,
      capabilities({ available: true, provider: 'deepl', credentials: 'request' })
    );
    await until('capabilities.changed', () => seen.length === 1);
    expect(seen[0].params.integrations.translation.provider).toBe('deepl');
    expect(changed).toHaveBeenCalled();
    await store.whenReady();
    // The key still comes from the keyset, which has it.
    expect(store.get('translation')).toEqual({
      available: true,
      source: 'integration',
      provider: 'deepl',
    });

    fake.notify(
      Method.CapabilitiesChanged,
      capabilities({ available: false, provider: 'libretranslate', reason: 'missing-url' })
    );
    await until('the second change', () => !engine.capabilities.integrations.translation.available);
    expect(store.get('translation').reason).toBe(
      'Add the LibreTranslate URL in Preferences > Integrations'
    );

    // Other notifications pass a filter only when they match it.
    const mail = [];
    const mailSub = engine.subscribe({ methods: [Method.MailNew], accounts: ['a1'] }, (n) =>
      mail.push(n)
    );
    fake.notify(Method.MailNew, { accountId: 'a2', threadId: 't', messageId: 'm' });
    fake.notify(Method.MailNew, { accountId: 'a1', threadId: 't', messageId: 'm' });
    await until('mail.new', () => mail.length === 1);
    expect(mail[0].params.accountId).toBe('a1');
    sub.dispose();
    mailSub.dispose();
    watch.dispose();
  });
});

describe('Engine.request and local-only methods (TD29)', function () {
  let events: EngineEvents;
  let engine: EngineAPI;
  let caller: CallerPackage;
  let sent: string[];

  beforeEach(() => {
    events = new EngineEvents();
    sent = [];
    events.connected([], async (method) => {
      sent.push(method);
      return {};
    });
    engine = new EngineAPI(events);
    caller = { name: 'third-party', bundled: false, engineMethods: [] };
    engine.findCaller = () => caller;
    engine.askUser = jasmine.createSpy('askUser').andReturn(false);
    AppEnv.config.set(ENGINE_METHOD_APPROVALS_KEY, {});
  });

  const outcome = (method: string) =>
    engine.request(method, {}).then(
      () => 'sent',
      (e) => (e instanceof EngineRequestRefused ? 'refused' : `error ${e}`)
    );

  it('lets any package make the other requests', async () => {
    expect(await outcome('threads.list')).toBe('sent');
    expect(await outcome('integrations.translate')).toBe('sent');
    expect(engine.askUser).not.toHaveBeenCalled();
  });

  it('refuses an undeclared local-only method without asking', async () => {
    expect(await outcome('secrets.set')).toBe('refused');
    expect(await outcome('account.test')).toBe('refused');
    expect(engine.askUser).not.toHaveBeenCalled();
    expect(sent).toEqual([]);
  });

  it('asks once for a declared one, and remembers a yes in config', async () => {
    caller.engineMethods = ['engine.logs.tail'];
    expect(await outcome('engine.logs.tail')).toBe('refused');
    expect(await outcome('engine.logs.tail')).toBe('refused');
    expect((engine.askUser as jasmine.Spy).callCount).toBe(1);

    engine = new EngineAPI(events);
    engine.findCaller = () => caller;
    engine.askUser = jasmine.createSpy('askUser').andReturn(true);
    expect(await outcome('engine.logs.tail')).toBe('sent');
    expect(await outcome('engine.logs.tail')).toBe('sent');
    expect((engine.askUser as jasmine.Spy).callCount).toBe(1);
    expect(AppEnv.config.get(ENGINE_METHOD_APPROVALS_KEY)).toEqual({
      'third-party': ['engine.logs.tail'],
    });
  });

  it('refuses hello and subscribe, which belong to the window connection', async () => {
    caller = null;
    expect(await outcome('subscribe')).toBe('refused');
    expect(await outcome('hello')).toBe('refused');
    expect(sent).toEqual([]);
  });

  it('never asks for the app or a bundled package', async () => {
    caller = { name: 'preferences', bundled: true, engineMethods: [] };
    expect(await outcome('secrets.set')).toBe('sent');
    caller = null;
    expect(await outcome('secrets.set')).toBe('sent');
    expect(engine.askUser).not.toHaveBeenCalled();
  });

  it('finds the calling package from the stack', () => {
    const resourcePath = AppEnv.getLoadSettings().resourcePath;
    const third = '/tmp/tern-spec-packages/third';
    const bundled = path.join(resourcePath, 'plugins', 'send-later');
    const feature = path.join(resourcePath, 'src', 'features', 'composer');
    spyOn(AppEnv.packages, 'getAvailablePackages').andReturn([
      { name: 'third', directory: third, engineMethods: () => ['secrets.set'] },
      { name: 'send-later', directory: bundled, engineMethods: () => [] },
      { name: 'composer', directory: feature, engineMethods: () => [] },
    ] as any);
    const stack = (file: string) =>
      [
        'Error',
        `    at EngineAPI.request (${path.join(resourcePath, 'src', 'engine', 'api.ts')}:10:5)`,
        `    at Object.run (file://${file}:3:9)`,
        `    at ${path.join(resourcePath, 'src', 'window', 'app-env.ts')}:1:1`,
      ].join('\n');
    expect(stackPaths(stack(`${third}/lib/main.js`))[1]).toBe(`${third}/lib/main.js`);
    expect(callerPackageFromStack(stack(`${third}/lib/main.js`))).toEqual({
      name: 'third',
      bundled: false,
      engineMethods: ['secrets.set'],
    });
    expect(callerPackageFromStack(stack(`${bundled}/lib/main.js`)).bundled).toBe(true);
    // Built-in features are app code under src/: their frames are passed over like any other.
    expect(callerPackageFromStack(stack(`${feature}/index.ts`))).toBe(null);
    // Outside the app and in no package: an unknown caller, with no local-only methods.
    expect(callerPackageFromStack(stack('/somewhere/else.js'))).toEqual({
      name: '/somewhere/else.js',
      bundled: false,
      engineMethods: [],
    });
    // The app's other files are the app.
    expect(callerPackageFromStack(stack(path.join(resourcePath, 'static', 'x.js')))).toBe(null);
  });

  it('finds a symlinked package, through app helpers and odd paths', () => {
    const resourcePath = AppEnv.getLoadSettings().resourcePath;
    const root = fs.realpathSync(fs.mkdtempSync('/tmp/tern-spec-engine-api-'));
    try {
      const real = path.join(root, 'checkout (dev)', 'third');
      fs.mkdirSync(real, { recursive: true });
      const link = path.join(root, 'packages-third');
      fs.symlinkSync(real, link);
      spyOn(AppEnv.packages, 'getAvailablePackages').andReturn([
        { name: 'third', directory: link, engineMethods: () => [] },
      ] as any);
      // V8 reports the realpath, here with parentheses, called through an app dependency.
      const s = [
        'Error',
        `    at EngineAPI.request (${path.join(resourcePath, 'src', 'engine', 'api.ts')}:10:5)`,
        `    at later (${path.join(resourcePath, 'node_modules', 'underscore', 'underscore.js')}:1:2)`,
        `    at Object.run (${real}/lib/main.js:3:9)`,
      ].join('\n');
      expect(stackPaths(s)[2]).toBe(`${real}/lib/main.js`);
      expect(callerPackageFromStack(s)).toEqual({
        name: 'third',
        bundled: false,
        engineMethods: [],
      });
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
});
