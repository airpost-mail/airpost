// Answers "is feature X available, and from where" (docs/integrations-plan.md, section 1).
// Local features always are; integration features when the user set up a provider in
// Preferences > Integrations and the engine can call it; server features need a Tern server,
// which there is no way to connect yet.
//
// The connected engine decides (its `capabilities`, from hello and capabilities.changed): the
// provider settings are mirrored into its settings store. Keys still go with each request, read
// from the keyset (KeyManager), so the store checks here that the provider's key is saved. Without
// an engine that reports capabilities it decides from config alone, as before.

import { useEffect, useState } from 'react';
import MailspringStore from 'mailspring-store';
import { localized } from '../../intl';
import EngineEvents, { EngineEvents as EngineEventsType } from '../../engine-events';
import { CapabilityReason, EngineCapability } from '../../engine-protocol';
import {
  Integrations,
  IntegrationKind,
  IntegrationProvider,
  PROVIDERS,
  keyName,
  missingSetting,
  readSettings,
} from '../../integrations';

export type CapabilitySource = 'local' | 'integration' | 'server';

export type CapabilityFeature =
  | 'snooze'
  | 'send-later'
  | 'reminders'
  | 'templates'
  | 'insights'
  | 'translation'
  | 'grammar-check'
  | 'contact-enrichment'
  | 'read-receipts'
  | 'link-tracking'
  | 'thread-sharing'
  | 'activity-sharing'
  | 'metadata-sync';

export interface Capability {
  available: boolean;
  source: CapabilitySource;
  /** Why it is unavailable; absent when available. */
  reason?: string;
  /** For integration features, the chosen provider. */
  provider?: IntegrationProvider;
}

const LOCAL: CapabilityFeature[] = ['snooze', 'send-later', 'reminders', 'templates', 'insights'];
const INTEGRATION: { [feature: string]: IntegrationKind } = {
  translation: 'translation',
  'grammar-check': 'grammar',
  'contact-enrichment': 'enrichment',
};
const SERVER: CapabilityFeature[] = [
  'read-receipts',
  'link-tracking',
  'thread-sharing',
  'activity-sharing',
  'metadata-sync',
];
// The engine's names for them (ms-ipc `server_feature`).
const ENGINE_SERVER_FEATURE: { [feature: string]: string } = {
  'read-receipts': 'readReceipts',
  'link-tracking': 'linkTracking',
  'thread-sharing': 'threadSharing',
  'activity-sharing': 'activitySharing',
  'metadata-sync': 'metadataSync',
};

export const ALL_FEATURES: CapabilityFeature[] = [
  ...LOCAL,
  ...(Object.keys(INTEGRATION) as CapabilityFeature[]),
  ...SERVER,
];

const KINDS: IntegrationKind[] = ['translation', 'grammar', 'enrichment'];

function featureLabel(feature: CapabilityFeature) {
  switch (feature) {
    case 'translation':
      return localized('Translation');
    case 'grammar-check':
      return localized('Grammar check');
    case 'contact-enrichment':
      return localized('Contact info');
    case 'read-receipts':
      return localized('Open tracking');
    case 'link-tracking':
      return localized('Link tracking');
    case 'thread-sharing':
      return localized('Sharing threads');
    case 'activity-sharing':
      return localized('Sharing reports');
    case 'metadata-sync':
      return localized('Syncing snoozes and reminders');
    default:
      return feature;
  }
}

interface KeySource {
  getPassword(name: string): Promise<string | undefined>;
}

export class CapabilityStoreClass extends MailspringStore {
  /** Whether the keyset holds each provider's key. Never the key itself. */
  private _hasKey: { [provider: string]: boolean } = {};
  private _keyRead: Promise<void> = Promise.resolve();
  private _keyReadGeneration = 0;
  private _disposables: Array<() => void> = [];
  keySource: KeySource = null;
  /** The window's engine connection; swappable for specs. */
  engineEvents: EngineEventsType = EngineEvents;

  constructor({ autoStart = true }: { autoStart?: boolean } = {}) {
    super();
    if (autoStart && typeof AppEnv !== 'undefined' && AppEnv.config) this.start();
  }

  /** Subscribes to config (settings and the keyset blob) and the integrations transport. */
  start() {
    this.stop();
    const settings = AppEnv.config.onDidChange('core.integrations', () => {
      this.trigger();
      this._refreshKeys();
    });
    // KeyManager keeps the whole keyset encrypted under this config key, so this fires for a
    // key saved in any window.
    const keyset = AppEnv.config.onDidChange('credentials', () => this._refreshKeys());
    const events = this.engineEvents;
    const onCapabilities = () => this.trigger();
    events.on('capabilities', onCapabilities);
    this._disposables = [
      () => settings.dispose(),
      () => keyset.dispose(),
      Integrations.onTransportChange(() => this.trigger()),
      () => events.removeListener('capabilities', onCapabilities),
    ];
    this._refreshKeys();
  }

  stop() {
    for (const dispose of this._disposables) dispose();
    this._disposables = [];
  }

  /** Resolves once the latest keyset read finished. For specs and the Integrations tab. */
  whenReady() {
    return this._keyRead;
  }

  get(feature: CapabilityFeature): Capability {
    if (LOCAL.includes(feature)) {
      return { available: true, source: 'local' };
    }
    const engine = this.engineEvents.capabilities;
    if (SERVER.includes(feature)) {
      const reported = engine && engine.server && engine.server[ENGINE_SERVER_FEATURE[feature]];
      if (reported && reported.available) return { available: true, source: 'server' };
      const reason = reported ? engineReason(reported, undefined) : serverReason();
      return { available: false, source: 'server', reason };
    }
    const kind = INTEGRATION[feature];
    if (!kind) {
      return { available: false, source: 'local', reason: localized('Unknown feature') };
    }
    const settings = readSettings(kind);
    const reported = engine && engine.integrations && engine.integrations[kind];
    const provider =
      (reported && isProvider(reported.provider) && reported.provider) ||
      settings.provider ||
      undefined;
    const unavailable = (reason: string): Capability => ({
      available: false,
      source: 'integration',
      reason,
      provider,
    });
    if (reported && !reported.available) {
      return unavailable(engineReason(reported, provider));
    }
    if (!settings.enabled) {
      return unavailable(localized('Set up a provider in Preferences > Integrations'));
    }
    const missing = missingSetting(settings, !!this._hasKey[settings.provider]);
    const name = PROVIDERS[settings.provider].name;
    // The key goes with each request unless the engine says it holds the keys itself.
    const keysFromUs = !reported || reported.credentials === 'request';
    if (missing === 'url') {
      return unavailable(localized('Add the %@ URL in Preferences > Integrations', name));
    }
    if (missing === 'key' && keysFromUs) {
      return unavailable(localized('Add your %@ API key in Preferences > Integrations', name));
    }
    if (missing === 'username' && keysFromUs) {
      return unavailable(localized('Add your %@ username in Preferences > Integrations', name));
    }
    if (!Integrations.isAvailable()) {
      return unavailable(localized('Waiting for engine support'));
    }
    return { available: true, source: 'integration', provider };
  }

  isAvailable(feature: CapabilityFeature) {
    return this.get(feature).available;
  }

  /** A tooltip for a disabled control, e.g. "Open tracking: Needs a Tern server"; null if available. */
  unavailableMessage(feature: CapabilityFeature): string | null {
    const capability = this.get(feature);
    if (capability.available) return null;
    return localized('%1$@: %2$@', featureLabel(feature), capability.reason);
  }

  all(): { [feature: string]: Capability } {
    const result = {};
    for (const feature of ALL_FEATURES) result[feature] = this.get(feature);
    return result;
  }

  private _keys(): KeySource {
    return this.keySource || require('../../key-manager').default;
  }

  // Only reads the keyset when an integration is turned on, so a profile without integrations
  // never decrypts it here.
  private _refreshKeys() {
    const generation = ++this._keyReadGeneration;
    const providers = KINDS.map((kind) => readSettings(kind))
      .filter((s) => s.enabled && s.provider)
      .map((s) => s.provider);

    this._keyRead = (async () => {
      const next: { [provider: string]: boolean } = {};
      for (const provider of providers) {
        try {
          next[provider] = !!((await this._keys().getPassword(keyName(provider))) || '').trim();
        } catch (err) {
          next[provider] = false;
        }
      }
      if (generation !== this._keyReadGeneration) return;
      const changed = JSON.stringify(next) !== JSON.stringify(this._hasKey);
      this._hasKey = next;
      if (changed) this.trigger();
    })();
  }
}

function isProvider(p: unknown): p is IntegrationProvider {
  return typeof p === 'string' && p in PROVIDERS;
}

function serverReason() {
  return localized('Needs a Tern server');
}

// The words for an engine's reason; one this client doesn't know reads as unavailable.
function engineReason(capability: EngineCapability, provider: IntegrationProvider | undefined) {
  switch (capability.reason) {
    case CapabilityReason.NotSetUp:
      return localized('Set up a provider in Preferences > Integrations');
    case CapabilityReason.MissingUrl:
      return provider
        ? localized('Add the %@ URL in Preferences > Integrations', PROVIDERS[provider].name)
        : localized('Set up a provider in Preferences > Integrations');
    case CapabilityReason.NeedsTernServer:
      return serverReason();
    default:
      return localized('Not available from the sync engine');
  }
}

export const CapabilityStore = new CapabilityStoreClass();
export default CapabilityStore;

/** The capability, re-rendering when it changes. */
export function useCapability(feature: CapabilityFeature): Capability {
  const [capability, setCapability] = useState(() => CapabilityStore.get(feature));
  useEffect(() => {
    const update = () => setCapability(CapabilityStore.get(feature));
    update();
    const unlisten = CapabilityStore.listen(update);
    return () => {
      unlisten();
    };
  }, [feature]);
  return capability;
}
