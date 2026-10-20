import { CapabilityStoreClass } from '../../src/data/stores/capability';
import { Integrations, IntegrationsTransport } from '../../src/engine/client/integrations';
import { EngineEvents } from '../../src/engine/client/events';

const engineTransport: IntegrationsTransport = { available: true, send: async () => ({}) };

// A keyset that is a plain object: nothing here reaches the real KeyManager or a keychain.
function makeStore(keys: { [name: string]: string }) {
  const store = new CapabilityStoreClass({ autoStart: false });
  store.keySource = {
    getPassword: jasmine.createSpy('getPassword').andCallFake(async (name: string) => keys[name]),
  };
  store.start();
  return store;
}

describe('CapabilityStore', () => {
  let store: CapabilityStoreClass;
  let keys: { [name: string]: string };

  beforeEach(() => {
    keys = {};
  });

  afterEach(() => {
    if (store) store.stop();
    Integrations.setTransport(null);
  });

  it('reports local features as available from the local source', () => {
    store = makeStore(keys);
    for (const feature of ['snooze', 'send-later', 'reminders', 'templates', 'insights'] as const) {
      expect(store.get(feature)).toEqual({ available: true, source: 'local' });
      expect(store.unavailableMessage(feature)).toBe(null);
    }
  });

  it('reports server features as needing a Tern server', () => {
    store = makeStore(keys);
    for (const feature of [
      'read-receipts',
      'link-tracking',
      'thread-sharing',
      'activity-sharing',
      'metadata-sync',
    ] as const) {
      expect(store.get(feature)).toEqual({
        available: false,
        source: 'server',
        reason: 'Needs a Tern server',
      });
    }
    expect(store.unavailableMessage('read-receipts')).toBe('Open tracking: Needs a Tern server');
  });

  it('turns tracking on when the engine reports a server that offers it', () => {
    store = new CapabilityStoreClass({ autoStart: false });
    const events = new EngineEvents();
    store.engineEvents = events;
    events.connected([], async () => ({}), {
      server: {
        readReceipts: { available: true },
        linkTracking: { available: false, reason: 'not-on-server' },
      },
    });
    expect(store.get('read-receipts')).toEqual({ available: true, source: 'server' });
    expect(store.unavailableMessage('read-receipts')).toBe(null);
    expect(store.get('link-tracking').reason).toBe('Your Tern server doesn’t offer it');
  });

  it('points unconfigured integrations at Preferences > Integrations without reading the keyset', async () => {
    store = makeStore(keys);
    await store.whenReady();
    const translation = store.get('translation');
    expect(translation.available).toBe(false);
    expect(translation.source).toBe('integration');
    expect(translation.reason).toBe('Set up a provider in Preferences > Integrations');
    expect(store.unavailableMessage('grammar-check')).toBe(
      'Grammar check: Set up a provider in Preferences > Integrations'
    );
    expect(store.keySource.getPassword).not.toHaveBeenCalled();
  });

  it('asks for a missing key or URL', async () => {
    store = makeStore(keys);
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    await store.whenReady();
    expect(store.get('translation').reason).toBe(
      'Add your DeepL API key in Preferences > Integrations'
    );
    expect(store.get('translation').provider).toBe('deepl');

    AppEnv.config.set('core.integrations.translation', {
      provider: 'libretranslate',
      enabled: true,
    });
    await store.whenReady();
    expect(store.get('translation').reason).toBe(
      'Add the LibreTranslate URL in Preferences > Integrations'
    );
  });

  it('follows the keyset and says it waits for the engine while the transport is the stub', async () => {
    store = makeStore(keys);
    AppEnv.config.set('core.integrations.enrichment', { enabled: true });
    await store.whenReady();
    expect(store.get('contact-enrichment').reason).toBe(
      'Add your People Data Labs API key in Preferences > Integrations'
    );

    const listener = jasmine.createSpy('listener');
    const unlisten = store.listen(listener);
    keys['integration-peopledatalabs-key'] = 'synthetic-key';
    // KeyManager rewrites this config key on every keyset change.
    AppEnv.config.set('credentials', 'synthetic-blob-1');
    await store.whenReady();
    expect(listener).toHaveBeenCalled();
    expect(store.get('contact-enrichment')).toEqual({
      available: false,
      source: 'integration',
      reason: 'Waiting for engine support',
      provider: 'peopledatalabs',
    });

    Integrations.setTransport(engineTransport);
    expect(store.get('contact-enrichment')).toEqual({
      available: true,
      source: 'integration',
      provider: 'peopledatalabs',
    });
    expect(store.unavailableMessage('contact-enrichment')).toBe(null);

    delete keys['integration-peopledatalabs-key'];
    AppEnv.config.set('credentials', 'synthetic-blob-2');
    await store.whenReady();
    expect(store.isAvailable('contact-enrichment')).toBe(false);
    unlisten();
  });

  it('triggers when settings or the transport change', async () => {
    store = makeStore(keys);
    const listener = jasmine.createSpy('listener');
    const unlisten = store.listen(listener);

    AppEnv.config.set('core.integrations.grammar', { enabled: true });
    expect(listener.callCount).toBe(1);
    await store.whenReady();
    const before = listener.callCount;
    // LanguageTool needs no key on the free tier.
    Integrations.setTransport(engineTransport);
    expect(listener.callCount).toBe(before + 1);
    expect(store.isAvailable('grammar-check')).toBe(true);

    AppEnv.config.set('core.integrations.grammar', { enabled: false });
    expect(store.isAvailable('grammar-check')).toBe(false);
    unlisten();
  });

  it('does not report a disabled integration even with a key saved', async () => {
    keys['integration-deepl-key'] = 'synthetic';
    store = makeStore(keys);
    Integrations.setTransport(engineTransport);
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: false });
    await store.whenReady();
    expect(store.isAvailable('translation')).toBe(false);
    AppEnv.config.set('core.integrations.translation', { provider: 'deepl', enabled: true });
    await store.whenReady();
    expect(store.isAvailable('translation')).toBe(true);
  });
});
