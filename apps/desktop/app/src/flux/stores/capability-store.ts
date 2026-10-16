// Answers "is feature X available, and from where" (docs/integrations-plan.md, section 1).
// Local features always are; integration features when the user set up a provider in
// Preferences > Integrations and the engine can call it; server features need a Tern server,
// which there is no way to connect yet.

import { useEffect, useState } from 'react';
import MailspringStore from 'mailspring-store';
import { localized } from '../../intl';
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
    this._disposables = [
      () => settings.dispose(),
      () => keyset.dispose(),
      Integrations.onTransportChange(() => this.trigger()),
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
    if (SERVER.includes(feature)) {
      return { available: false, source: 'server', reason: localized('Needs a Tern server') };
    }
    const kind = INTEGRATION[feature];
    if (!kind) {
      return { available: false, source: 'local', reason: localized('Unknown feature') };
    }
    const settings = readSettings(kind);
    const provider = settings.provider || undefined;
    const unavailable = (reason: string): Capability => ({
      available: false,
      source: 'integration',
      reason,
      provider,
    });
    if (!settings.enabled) {
      return unavailable(localized('Set up a provider in Preferences > Integrations'));
    }
    const missing = missingSetting(settings, !!this._hasKey[settings.provider]);
    const name = PROVIDERS[settings.provider].name;
    if (missing === 'url') {
      return unavailable(localized('Add the %@ URL in Preferences > Integrations', name));
    }
    if (missing === 'key') {
      return unavailable(localized('Add your %@ API key in Preferences > Integrations', name));
    }
    if (missing === 'username') {
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
