// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The desktop's AI settings (docs/ai-plan.md A3, A4): the model providers, which one each feature
uses, the feature switches, the accounts opted in and "Include email addresses". They live in
config under `core.ai.*`, which the main process mirrors into the engine's settings store
(engine-settings.ts) when the engine lists `ai-triage`. An engine that lists `ai-keys` keeps the
keys in its secret store as `integration:ai-<providerId>`, and the `ai.*` requests name the
provider; with an older engine the key is read from the keyset (KeyManager) as
`integration-ai-<providerId>-key` (keyName) and goes with each request. Keys are never in config
and never logged.

The requests themselves are made by internal_packages/ai through the Engine export; this module
is shared with the CapabilityStore, which needs the settings to say whether a feature is set up.
*/

export type AiProviderKind = 'openai' | 'anthropic' | 'ollama';
export type AiFeature = 'summary' | 'reply' | 'triage';

export const AI_FEATURES: AiFeature[] = ['summary', 'reply', 'triage'];

/** One configured provider, as config holds it. Never a key. */
export interface AiProvider {
  id: string;
  kind: AiProviderKind;
  /** The user's name for it ("Work Claude"); the kind's name when absent. */
  name?: string;
  url?: string;
  model: string;
  contextTokens?: number;
  maxOutputTokens?: number;
}

export interface AiFeatureSettings {
  enabled: boolean;
  /** An AiProvider id. */
  provider?: string;
}

export interface AiSettings {
  providers: AiProvider[];
  features: { [feature in AiFeature]: AiFeatureSettings };
  /** Account ids opted in. */
  accounts: string[];
  includeAddresses: boolean;
}

export const CONFIG_KEY = 'core.ai';

export const KINDS: { [kind in AiProviderKind]: { name: string; defaultUrl: string } } = {
  openai: { name: 'OpenAI-compatible', defaultUrl: 'https://api.openai.com/v1' },
  anthropic: { name: 'Anthropic', defaultUrl: 'https://api.anthropic.com' },
  ollama: { name: 'Ollama', defaultUrl: 'http://127.0.0.1:11434' },
};

export function keyName(providerId: string) {
  return `integration-ai-${providerId}-key`;
}

function isKind(kind: unknown): kind is AiProviderKind {
  return typeof kind === 'string' && kind in KINDS;
}

function positive(n: unknown) {
  return typeof n === 'number' && Number.isFinite(n) && n > 0 ? Math.floor(n) : undefined;
}

function readProvider(raw: any): AiProvider | null {
  if (!raw || typeof raw !== 'object') return null;
  if (typeof raw.id !== 'string' || !raw.id || !isKind(raw.kind)) return null;
  const provider: AiProvider = {
    id: raw.id,
    kind: raw.kind,
    model: typeof raw.model === 'string' ? raw.model : '',
  };
  if (typeof raw.name === 'string' && raw.name.trim()) provider.name = raw.name.trim();
  if (typeof raw.url === 'string' && raw.url.trim()) provider.url = raw.url.trim();
  const contextTokens = positive(raw.contextTokens);
  if (contextTokens) provider.contextTokens = contextTokens;
  const maxOutputTokens = positive(raw.maxOutputTokens);
  if (maxOutputTokens) provider.maxOutputTokens = maxOutputTokens;
  return provider;
}

export function readSettings(): AiSettings {
  const raw = (AppEnv.config.get(CONFIG_KEY) || {}) as any;
  const providers: AiProvider[] = [];
  for (const p of Array.isArray(raw.providers) ? raw.providers : []) {
    const provider = readProvider(p);
    if (provider && !providers.some((other) => other.id === provider.id)) {
      providers.push(provider);
    }
  }
  const rawFeatures = raw.features && typeof raw.features === 'object' ? raw.features : {};
  const features = {} as AiSettings['features'];
  for (const feature of AI_FEATURES) {
    const f = rawFeatures[feature] || {};
    const provider =
      typeof f.provider === 'string' && providers.some((p) => p.id === f.provider)
        ? f.provider
        : undefined;
    features[feature] = provider ? { enabled: !!f.enabled, provider } : { enabled: !!f.enabled };
  }
  const accounts = Array.isArray(raw.accounts)
    ? raw.accounts.filter((a: unknown) => typeof a === 'string')
    : [];
  return { providers, features, accounts, includeAddresses: !!raw.includeAddresses };
}

export function writeSettings(settings: AiSettings) {
  const raw = (AppEnv.config.get(CONFIG_KEY) || {}) as any;
  const next: any = {
    providers: settings.providers.map((p) => readProvider(p)).filter(Boolean),
    features: settings.features,
    accounts: [...new Set(settings.accounts)],
    includeAddresses: !!settings.includeAddresses,
  };
  // The engine's triage limits (`core.ai.limits`, set with `tern settings set`) have no form here.
  if (raw.limits !== undefined) next.limits = raw.limits;
  AppEnv.config.set(CONFIG_KEY, next);
}

/** A new provider id, unique among `providers`. Ids name keys, so they stay plain. */
export function newProviderId(kind: AiProviderKind, providers: AiProvider[]) {
  for (let n = 1; ; n++) {
    const id = n === 1 ? kind : `${kind}-${n}`;
    if (!providers.some((p) => p.id === id)) return id;
  }
}

export function providerName(provider: AiProvider) {
  return provider.name || KINDS[provider.kind].name;
}

/** The provider the feature uses, when it has one that still exists. */
export function providerFor(settings: AiSettings, feature: AiFeature): AiProvider | null {
  const id = settings.features[feature].provider;
  return settings.providers.find((p) => p.id === id) || null;
}

/** The base URL as ms-ai's ProviderConfig::base_url builds it. */
export function baseUrl(provider: AiProvider) {
  let url = (provider.url || '').trim() || KINDS[provider.kind].defaultUrl;
  url = url.replace(/\/+$/, '');
  const suffixes = {
    openai: ['/chat/completions'],
    anthropic: ['/v1/messages', '/v1'],
    ollama: ['/api/chat', '/api', '/v1'],
  }[provider.kind];
  for (const suffix of suffixes) {
    if (url.endsWith(suffix)) {
      url = url.slice(0, -suffix.length).replace(/\/+$/, '');
      break;
    }
  }
  return url;
}

/** The host requests go to ("api.anthropic.com"); empty when the URL is not valid. */
export function hostOf(provider: AiProvider) {
  try {
    return new URL(baseUrl(provider)).hostname.replace(/^\[|\]$/g, '');
  } catch (err) {
    return '';
  }
}

function isLocalV4(parts: number[]) {
  const [a, b] = parts;
  return (
    a === 127 ||
    a === 10 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 169 && b === 254)
  );
}

/** Loopback or a private address, as ms-ai's ProviderConfig::is_local decides it. */
export function isLocal(provider: AiProvider) {
  const host = hostOf(provider).toLowerCase().replace(/\.$/, '');
  if (!host) return false;
  if (host === 'localhost' || host.endsWith('.localhost')) return true;
  const v4 = /^(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(host);
  if (v4) {
    return isLocalV4(v4.slice(1).map(Number));
  }
  if (host.includes(':')) {
    const mapped = /^::ffff:(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(host);
    if (mapped) return isLocalV4(mapped.slice(1).map(Number));
    return host === '::1' || /^f[cd]/.test(host) || /^fe[89ab]/.test(host);
  }
  return false;
}

/** Loopback only: a model on this computer. A private address is another machine on the network. */
export function isLoopback(provider: AiProvider) {
  const host = hostOf(provider).toLowerCase().replace(/\.$/, '');
  if (!host) return false;
  if (host === 'localhost' || host.endsWith('.localhost') || host === '::1') return true;
  // URL writes ::ffff:127.0.0.1 as ::ffff:7f00:1.
  if (/^::ffff:7f[0-9a-f]{2}:/.test(host)) return true;
  const v4 = /^(\d+)\.\d+\.\d+\.\d+$/.exec(host);
  return !!v4 && Number(v4[1]) === 127;
}

/** Whether the engine refuses a call without a key: Anthropic, and OpenAI's own API. */
export function keyRequired(provider: AiProvider) {
  if (provider.kind === 'anthropic') return true;
  return provider.kind === 'openai' && baseUrl(provider) === KINDS.openai.defaultUrl;
}

/** Why the feature can't make a request yet, or null when it can. */
export function missingSetting(
  settings: AiSettings,
  feature: AiFeature,
  hasKey: boolean
): 'disabled' | 'provider' | 'model' | 'key' | null {
  if (!settings.features[feature].enabled) return 'disabled';
  const provider = providerFor(settings, feature);
  if (!provider) return 'provider';
  if (!provider.model.trim()) return 'model';
  if (keyRequired(provider) && !hasKey) return 'key';
  return null;
}

/** ms-ai's ProviderConfig JSON, key included when there is one. */
export interface AiProviderConfig {
  id: string;
  kind: AiProviderKind;
  url?: string;
  apiKey?: string;
  model: string;
  contextTokens?: number;
  maxOutputTokens?: number;
}

export function providerConfig(provider: AiProvider, key: string | null | undefined) {
  const config: AiProviderConfig = { id: provider.id, kind: provider.kind, model: provider.model };
  if (provider.url) config.url = provider.url;
  const apiKey = (key || '').trim();
  if (apiKey) config.apiKey = apiKey;
  if (provider.contextTokens) config.contextTokens = provider.contextTokens;
  if (provider.maxOutputTokens) config.maxOutputTokens = provider.maxOutputTokens;
  return config;
}

/** The `settings` the thread methods take until the engine keeps them (ms_ipc::AiSettings). */
export function requestSettings(settings: AiSettings) {
  const features: { [feature: string]: boolean } = {};
  for (const feature of AI_FEATURES) features[feature] = settings.features[feature].enabled;
  return {
    accounts: settings.accounts,
    features,
    includeAddresses: settings.includeAddresses,
  };
}

export function accountEnabled(settings: AiSettings, accountId: string) {
  return settings.accounts.includes(accountId);
}

const AI = {
  AI_FEATURES,
  CONFIG_KEY,
  KINDS,
  keyName,
  readSettings,
  writeSettings,
  newProviderId,
  providerName,
  providerFor,
  baseUrl,
  hostOf,
  isLocal,
  isLoopback,
  keyRequired,
  missingSetting,
  providerConfig,
  requestSettings,
  accountEnabled,
};

export default AI;
