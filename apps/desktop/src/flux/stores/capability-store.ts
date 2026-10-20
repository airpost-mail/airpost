// Answers "is feature X available, and from where" (docs/integrations-plan.md, section 1).
// Local features always are; integration features when the user set up a provider in
// Preferences > Integrations and the engine can call it; AI features when a model is set up in
// Preferences > AI and the engine serves `ai.*` (docs/ai-plan.md section 3); server features need
// a Tern server, which there is no way to connect yet.
//
// The connected engine decides (its `capabilities`, from hello and capabilities.changed): the
// provider settings are mirrored into its settings store. Keys are in the engine's secret store
// or, from before, the keyset (KeyManager). For a capability the engine leaves to the client
// (`credentials: 'request'`: integrations, and AI with an engine that keeps no AI keys) the store
// checks here that the provider's key is saved in one. Without an engine that reports
// capabilities it decides from config alone.

import { useEffect, useState } from 'react';
import MailspringStore from 'mailspring-store';
import { localized } from '../../shared/intl';
import EngineEvents, { EngineEvents as EngineEventsType } from '../../engine/engine-events';
import { CapabilityReason, EngineCapability, Feature } from '../../shared/engine-protocol';
import AI, { AiFeature, AiProvider } from '../../engine/ai';
import {
  Integrations,
  IntegrationKind,
  IntegrationProvider,
  PROVIDERS,
  keyName,
  missingSetting,
  readSettings,
} from '../../engine/integrations';

export type CapabilitySource = 'local' | 'integration' | 'ai' | 'server';

export type CapabilityFeature =
  | 'snooze'
  | 'send-later'
  | 'reminders'
  | 'templates'
  | 'insights'
  | 'translation'
  | 'grammar-check'
  | 'contact-enrichment'
  | 'ai-summary'
  | 'ai-reply'
  | 'ai-triage'
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
  /** For AI features: the host the model runs on, and whether it is on this computer or network. */
  host?: string;
  local?: boolean;
}

const LOCAL: CapabilityFeature[] = ['snooze', 'send-later', 'reminders', 'templates', 'insights'];
const INTEGRATION: { [feature: string]: IntegrationKind } = {
  translation: 'translation',
  'grammar-check': 'grammar',
  'contact-enrichment': 'enrichment',
};
const AI_FEATURE: { [feature: string]: AiFeature } = {
  'ai-summary': 'summary',
  'ai-reply': 'reply',
  'ai-triage': 'triage',
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
  ...(Object.keys(AI_FEATURE) as CapabilityFeature[]),
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
    case 'ai-summary':
      return localized('Summaries');
    case 'ai-reply':
      return localized('Reply suggestions');
    case 'ai-triage':
      return localized('Inbox triage');
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
    const aiSettings = AppEnv.config.onDidChange(AI.CONFIG_KEY, () => {
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
      () => aiSettings.dispose(),
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
    if (AI_FEATURE[feature]) return this._ai(AI_FEATURE[feature]);
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

  private _ai(feature: AiFeature): Capability {
    const engine = this.engineEvents.capabilities;
    const reported = engine && engine.ai && engine.ai[feature];
    const settings = AI.readSettings();
    const provider = AI.providerFor(settings, feature);
    const where = provider ? { host: AI.hostOf(provider), local: AI.isLocal(provider) } : {};
    const unavailable = (reason: string): Capability => ({
      available: false,
      source: 'ai',
      reason,
      ...where,
    });
    if (reported && !reported.available) {
      return unavailable(aiEngineReason(reported, provider));
    }
    const hasKey = !!(provider && this._hasKey[aiKeyEntry(provider.id)]);
    const missing = AI.missingSetting(settings, feature, hasKey);
    const keysFromUs = !reported || reported.credentials === 'request';
    if (missing === 'disabled' || missing === 'provider' || missing === 'model') {
      return unavailable(localized('Set up a model in Preferences > AI'));
    }
    if (missing === 'key' && keysFromUs) {
      return unavailable(
        localized('Add your %@ API key in Preferences > AI', AI.providerName(provider))
      );
    }
    // Triage runs in the engine's own worker, which needs the triage methods and settings keys.
    if (!this.engineEvents.has(feature === 'triage' ? Feature.AiTriage : Feature.Ai)) {
      return unavailable(localized('Waiting for engine support'));
    }
    return { available: true, source: 'ai', ...where };
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
    return this.keySource || require('../../window/key-manager').default;
  }

  // Only reads the keyset when an integration is turned on, so a profile without integrations
  // never decrypts it here.
  private _refreshKeys() {
    const generation = ++this._keyReadGeneration;
    const providers = KINDS.map((kind) => readSettings(kind))
      .filter((s) => s.enabled && s.provider)
      .map((s) => s.provider as string);
    const ai = AI.readSettings();
    const aiProviders = AI.AI_FEATURES.filter((f) => ai.features[f].enabled)
      .map((f) => AI.providerFor(ai, f))
      .filter(Boolean)
      .map((p) => p.id);

    this._keyRead = (async () => {
      const next: { [provider: string]: boolean } = {};
      const names = [
        ...providers.map((p) => ({ entry: p, name: keyName(p as IntegrationProvider) })),
        ...[...new Set(aiProviders)].map((id) => ({ entry: aiKeyEntry(id), name: AI.keyName(id) })),
      ];
      for (const { entry, name } of names) {
        try {
          next[entry] = !!((await this._keys().getPassword(name)) || '').trim();
        } catch (err) {
          next[entry] = false;
        }
        if (!next[entry] && isProvider(entry)) {
          next[entry] = await Integrations.hasEngineKey(entry);
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

// AI providers' entries in _hasKey, apart from the integration providers'.
function aiKeyEntry(providerId: string) {
  return `ai:${providerId}`;
}

function aiEngineReason(capability: EngineCapability, provider: AiProvider | null) {
  switch (capability.reason) {
    case CapabilityReason.NotSetUp:
      return localized('Set up a model in Preferences > AI');
    case 'not-enabled':
      return localized('Turn on AI for this account');
    case CapabilityReason.NeedsEngineKey:
      return provider
        ? localized('Add your %@ API key in Preferences > AI', AI.providerName(provider))
        : localized('Add your API key in Preferences > AI');
    case CapabilityReason.RateLimited:
      return localized('The model provider is limiting requests; triage goes on by itself');
    case CapabilityReason.QuotaExceeded:
      return localized('The token quota or the daily limit is used up; triage goes on later');
    default:
      return localized('Not available from the sync engine');
  }
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
    case CapabilityReason.NotOnServer:
      return localized('Your Tern server doesn’t offer it');
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
