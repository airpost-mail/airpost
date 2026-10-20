// The desktop's client for third-party integrations (translation, grammar, contact
// enrichment). Provider logic lives in the engine (crates/ms-integrations); this module only
// reads the user's provider settings and key, builds the `integrations.*` request params from
// docs/integrations-plan.md (section 4) and maps the engine's errors to short messages.
//
// Settings live in config under `core.integrations.<kind>`. API keys go to the engine's secret
// store (`secrets.set`, `integration:<provider>`), which adds them to the requests that come
// without one; an engine without account setup gets them from the keyset (KeyManager,
// `integration-<provider>-key`), read per request and never logged.

import { EventEmitter } from 'events';
import { localized } from '../../shared/intl';

export type IntegrationKind = 'translation' | 'grammar' | 'enrichment';
export type IntegrationProvider = 'libretranslate' | 'deepl' | 'languagetool' | 'peopledatalabs';

export type IntegrationMethod =
  | 'integrations.translate'
  | 'integrations.detect'
  | 'integrations.languages'
  | 'integrations.grammar'
  | 'integrations.enrich'
  | 'integrations.test';

export const LANGUAGETOOL_DEFAULT_URL = 'https://api.languagetool.org';

export interface ProviderInfo {
  id: IntegrationProvider;
  kind: IntegrationKind;
  name: string;
  /** The API key is required (DeepL, People Data Labs) rather than optional. */
  keyRequired: boolean;
  /** A URL has to be entered (LibreTranslate); others have a fixed or default endpoint. */
  urlRequired: boolean;
}

export const PROVIDERS: { [id in IntegrationProvider]: ProviderInfo } = {
  libretranslate: {
    id: 'libretranslate',
    kind: 'translation',
    name: 'LibreTranslate',
    keyRequired: false,
    urlRequired: true,
  },
  deepl: { id: 'deepl', kind: 'translation', name: 'DeepL', keyRequired: true, urlRequired: false },
  languagetool: {
    id: 'languagetool',
    kind: 'grammar',
    name: 'LanguageTool',
    keyRequired: false,
    urlRequired: false,
  },
  peopledatalabs: {
    id: 'peopledatalabs',
    kind: 'enrichment',
    name: 'People Data Labs',
    keyRequired: true,
    urlRequired: false,
  },
};

export const PROVIDERS_FOR_KIND: { [kind in IntegrationKind]: IntegrationProvider[] } = {
  translation: ['libretranslate', 'deepl'],
  grammar: ['languagetool'],
  enrichment: ['peopledatalabs'],
};

/** What config holds for one kind. Never a key. */
export interface IntegrationSettings {
  provider: IntegrationProvider | null;
  enabled: boolean;
  /** LibreTranslate (required) and LanguageTool (optional, default LANGUAGETOOL_DEFAULT_URL). */
  url?: string;
  /** LanguageTool premium account. Its API key is in the keyset. */
  username?: string;
}

export function configKeyPath(kind: IntegrationKind) {
  return `core.integrations.${kind}`;
}

export function keyName(provider: IntegrationProvider) {
  return `integration-${provider}-key`;
}

/** The engine's name for a provider's key (crates/ms-secrets `names::integration`). */
export function secretName(provider: IntegrationProvider) {
  return `integration:${provider}`;
}

/** Bumped when a key is saved in or deleted from the engine, so every window's
 * CapabilityStore reads the keys again. */
export const KEYS_CHANGED_KEY = 'core.integrations.keysChangedAt';

export function readSettings(kind: IntegrationKind): IntegrationSettings {
  const raw = (AppEnv.config.get(configKeyPath(kind)) || {}) as Partial<IntegrationSettings>;
  const choices = PROVIDERS_FOR_KIND[kind];
  let provider = raw.provider && choices.includes(raw.provider) ? raw.provider : null;
  if (!provider && choices.length === 1) provider = choices[0];
  return {
    provider,
    enabled: !!raw.enabled && !!provider,
    url: typeof raw.url === 'string' ? raw.url : undefined,
    username: typeof raw.username === 'string' ? raw.username : undefined,
  };
}

export function writeSettings(kind: IntegrationKind, settings: IntegrationSettings) {
  const value: IntegrationSettings = { provider: settings.provider, enabled: !!settings.enabled };
  if (settings.url && settings.url.trim()) value.url = settings.url.trim();
  if (settings.username && settings.username.trim()) value.username = settings.username.trim();
  AppEnv.config.set(configKeyPath(kind), value);
}

/**
 * Why these settings can't make a request yet, or null when they can. `hasKey` is whether the
 * keyset holds the provider's key.
 */
export function missingSetting(
  settings: IntegrationSettings,
  hasKey: boolean
): 'provider' | 'url' | 'key' | 'username' | null {
  if (!settings.provider) return 'provider';
  const info = PROVIDERS[settings.provider];
  if (info.urlRequired && !(settings.url || '').trim()) return 'url';
  if (info.keyRequired && !hasKey) return 'key';
  if (settings.provider === 'languagetool') {
    // Premium needs both; one without the other is not-configured in the engine too.
    const hasUser = !!(settings.username || '').trim();
    if (hasUser && !hasKey) return 'key';
    if (!hasUser && hasKey) return 'username';
  }
  return null;
}

/** The provider config of plan section 3, as the engine deserializes it. Includes the key. */
export type ProviderConfig =
  | { provider: 'libretranslate'; url: string; apiKey?: string }
  | { provider: 'deepl'; apiKey: string }
  | { provider: 'languagetool'; url?: string; username?: string; apiKey?: string }
  | { provider: 'peopledatalabs'; apiKey: string };

export function buildProviderConfig(
  settings: IntegrationSettings,
  key: string | null | undefined
): ProviderConfig | null {
  const apiKey = (key || '').trim() || undefined;
  const url = (settings.url || '').trim() || undefined;
  switch (settings.provider) {
    case 'libretranslate':
      return apiKey
        ? { provider: 'libretranslate', url: url || '', apiKey }
        : { provider: 'libretranslate', url: url || '' };
    case 'deepl':
      return { provider: 'deepl', apiKey: apiKey || '' };
    case 'languagetool': {
      const config: ProviderConfig = { provider: 'languagetool' };
      if (url) config.url = url;
      const username = (settings.username || '').trim();
      if (username) config.username = username;
      if (apiKey) config.apiKey = apiKey;
      return config;
    }
    case 'peopledatalabs':
      return { provider: 'peopledatalabs', apiKey: apiKey || '' };
    default:
      return null;
  }
}

// Results, as crates/ms-integrations serializes them.

export interface DetectedLanguage {
  language: string;
  confidence?: number;
}
export interface Translation {
  translatedText: string;
  detectedLanguage?: DetectedLanguage;
}
export interface Language {
  code: string;
  name: string;
  targets?: string[];
}
export interface GrammarMatch {
  offset: number;
  length: number;
  message: string;
  shortMessage: string;
  replacements: string[];
  ruleId: string;
  ruleDescription: string;
  category: string;
  categoryName: string;
  issueType: string;
  sentence: string;
  contextText: string;
  contextOffset: number;
  contextLength: number;
}
export interface GrammarCheck {
  matches: GrammarMatch[];
  language?: Language;
  detectedLanguage?: Language;
}
export interface Handle {
  handle: string;
}
export interface Person {
  name?: string;
  facebook?: Handle;
  twitter?: Handle;
  linkedin?: Handle;
  employment?: { title?: string; name?: string };
  location?: string;
  bio?: string;
}
export interface Company {
  name: string;
  domain?: string;
  category?: { industry?: string; sector?: string };
  description?: string;
  location?: string;
  timeZone?: string;
  logo?: string;
  facebook?: Handle;
  twitter?: Handle;
  linkedin?: Handle;
  crunchbase?: Handle;
  type?: string;
  ticker?: string;
  phone?: string;
  metrics?: { raised?: string; marketCap?: string; employees?: string; employeesRange?: string };
}
export interface ContactProfile {
  person?: Person;
  company?: Company;
}
export interface TestReport {
  host: string;
  usage?: { used: number; limit?: number };
}

export interface IntegrationResults {
  'integrations.translate': Translation;
  'integrations.detect': DetectedLanguage[];
  'integrations.languages': Language[];
  'integrations.grammar': GrammarCheck;
  'integrations.enrich': ContactProfile;
  'integrations.test': TestReport;
}

// Errors

/** crates/ms-integrations error keys, plus the desktop's own `engine-unavailable`. */
export type IntegrationErrorKey =
  | 'not-configured'
  | 'auth'
  | 'rate-limited'
  | 'quota-exceeded'
  | 'offline'
  | 'bad-request'
  | 'provider-error'
  | 'unsupported'
  | 'engine-unavailable'
  | 'unknown';

const KNOWN_KEYS: IntegrationErrorKey[] = [
  'not-configured',
  'auth',
  'rate-limited',
  'quota-exceeded',
  'offline',
  'bad-request',
  'provider-error',
  'unsupported',
  'engine-unavailable',
];

export class IntegrationError extends Error {
  key: IntegrationErrorKey;
  retryable: boolean;
  status?: number;
  retryAfter?: number;

  constructor(fields: {
    key: IntegrationErrorKey;
    message?: string;
    retryable?: boolean;
    status?: number;
    retryAfter?: number;
  }) {
    super(fields.message || fields.key);
    this.name = 'IntegrationError';
    this.key = fields.key;
    this.retryable = !!fields.retryable;
    if (typeof fields.status === 'number') this.status = fields.status;
    if (typeof fields.retryAfter === 'number') this.retryAfter = fields.retryAfter;
  }
}

/**
 * Anything a transport throws, as an IntegrationError. Accepts the serialized
 * `{key, message, retryable, status?, retryAfter?}` itself or an error carrying it as `data`
 * (the socket's -32010 "Integration failed").
 */
export function toIntegrationError(err: unknown): IntegrationError {
  if (err instanceof IntegrationError) return err;
  const candidate = err && typeof err === 'object' ? (err as any) : null;
  const fields = candidate && typeof candidate.key === 'string' ? candidate : candidate?.data;
  if (fields && typeof fields.key === 'string') {
    const key = KNOWN_KEYS.includes(fields.key) ? fields.key : 'unknown';
    return new IntegrationError({ ...fields, key });
  }
  return new IntegrationError({
    key: 'unknown',
    message: candidate && typeof candidate.message === 'string' ? candidate.message : String(err),
  });
}

/** A short message for the user. `providerName` is used where it reads better. */
export function errorMessage(err: unknown, providerName?: string): string {
  const e = toIntegrationError(err);
  const name = providerName || localized('the provider');
  const Name = name.charAt(0).toUpperCase() + name.slice(1);
  switch (e.key) {
    case 'not-configured':
      return localized('%@ is not set up. Check Preferences > Integrations.', Name);
    case 'auth':
      return localized('%@ rejected the API key or account.', Name);
    case 'rate-limited':
      return e.retryAfter
        ? localized(
            '%1$@ is limiting requests. Try again in %2$@ seconds.',
            Name,
            `${e.retryAfter}`
          )
        : localized('%@ is limiting requests. Try again in a moment.', Name);
    case 'quota-exceeded':
      return providerName
        ? localized("Your %@ account's quota is used up.", name)
        : localized("The provider account's quota is used up.");
    case 'offline':
      return localized('Could not reach %@. Check the URL and your connection.', name);
    case 'bad-request':
      return localized('%@ refused the request.', Name);
    case 'provider-error':
      return localized('%@ had a problem. Try again later.', Name);
    case 'unsupported':
      return localized("%@ doesn't support this.", Name);
    case 'engine-unavailable':
      return localized('Waiting for engine support.');
    default:
      return localized('Something went wrong with %@.', name);
  }
}

// Transport

export interface IntegrationsTransport {
  /** False for the stub: the capability store then reports integration features unavailable. */
  readonly available: boolean;
  /** Resolves with the result, rejects with the serialized IntegrationError (or carrying it as `data`). */
  send(method: IntegrationMethod, params: object): Promise<unknown>;
}

// The transport until a window wires its engine connection (api.ts
// wireIntegrationsTransport), and in specs.
export const UnavailableTransport: IntegrationsTransport = {
  available: false,
  send: async () => {
    throw new IntegrationError({
      key: 'engine-unavailable',
      message: 'The engine does not support integrations yet.',
    });
  },
};

const METHOD_KIND: { [method in IntegrationMethod]?: IntegrationKind } = {
  'integrations.translate': 'translation',
  'integrations.detect': 'translation',
  'integrations.grammar': 'grammar',
  'integrations.enrich': 'enrichment',
};

interface KeySource {
  getPassword(name: string): Promise<string | undefined>;
  replacePassword?(name: string, value: string): Promise<void>;
  deletePassword?(name: string): Promise<void>;
}

interface SecretSource {
  available(): boolean;
  has(names: string[]): Promise<{ [name: string]: boolean }>;
  set(name: string, value: string): Promise<unknown>;
  delete(name: string): Promise<unknown>;
}

class IntegrationsClient {
  // The module's helpers, for plugins that only see the mailspring-exports instance.
  PROVIDERS = PROVIDERS;
  PROVIDERS_FOR_KIND = PROVIDERS_FOR_KIND;
  IntegrationError = IntegrationError;
  errorMessage = errorMessage;
  toIntegrationError = toIntegrationError;
  readSettings = readSettings;
  writeSettings = writeSettings;
  missingSetting = missingSetting;
  keyName = keyName;
  secretName = secretName;
  buildProviderConfig = buildProviderConfig;
  LANGUAGETOOL_DEFAULT_URL = LANGUAGETOOL_DEFAULT_URL;

  private _transport: IntegrationsTransport = UnavailableTransport;
  private _emitter = new EventEmitter();
  /** Swappable for specs. */
  keySource: KeySource = null;
  secretSource: SecretSource = null;

  transport() {
    return this._transport;
  }

  isAvailable() {
    return this._transport.available;
  }

  setTransport(transport: IntegrationsTransport | null) {
    this._transport = transport || UnavailableTransport;
    this._emitter.emit('transport-changed');
  }

  onTransportChange(callback: () => void) {
    this._emitter.on('transport-changed', callback);
    return () => this._emitter.removeListener('transport-changed', callback);
  }

  private _keys(): KeySource {
    return this.keySource || require('../../window/key-manager').default;
  }

  private _secrets(): SecretSource {
    if (this.secretSource) return this.secretSource;
    const engine = require('./accounts');
    return {
      available: engine.accountSetupAvailable,
      has: engine.hasSecrets,
      set: engine.setSecret,
      delete: engine.deleteSecret,
    };
  }

  private _keysChanged() {
    AppEnv.config.set(KEYS_CHANGED_KEY, Date.now());
  }

  /** Saves `provider`'s key in the engine, dropping a keyset copy, or in the keyset when the
   * engine can't keep it. */
  async saveKey(provider: IntegrationProvider, value: string) {
    const secrets = this._secrets();
    const keys = this._keys();
    if (!secrets.available()) {
      await keys.replacePassword(keyName(provider), value);
      return;
    }
    await secrets.set(secretName(provider), value);
    if (await keys.getPassword(keyName(provider))) await keys.deletePassword(keyName(provider));
    this._keysChanged();
  }

  /** Deletes `provider`'s key from the engine and the keyset. */
  async deleteKey(provider: IntegrationProvider) {
    const secrets = this._secrets();
    const keys = this._keys();
    if (secrets.available()) {
      await secrets.delete(secretName(provider));
      this._keysChanged();
    }
    if (await keys.getPassword(keyName(provider))) await keys.deletePassword(keyName(provider));
  }

  /** Whether the engine keeps a key for `provider`. */
  async hasEngineKey(provider: IntegrationProvider) {
    const secrets = this._secrets();
    if (!secrets.available()) return false;
    try {
      const has = await secrets.has([secretName(provider)]);
      return !!has[secretName(provider)];
    } catch (err) {
      return false;
    }
  }

  /** Whether a key is saved for `provider`, in the keyset or the engine. */
  async hasKey(provider: IntegrationProvider) {
    const key = await this._keys().getPassword(keyName(provider));
    return !!(key || '').trim() || this.hasEngineKey(provider);
  }

  /** The saved provider config for `kind`, or null when no provider is chosen. The key is the
   * keyset's when there is one there; otherwise the engine adds its own. */
  async savedConfig(kind: IntegrationKind): Promise<ProviderConfig | null> {
    const settings = readSettings(kind);
    if (!settings.provider) return null;
    const key = await this._keys().getPassword(keyName(settings.provider));
    return buildProviderConfig(settings, key);
  }

  /**
   * Calls `method` with `params` plus `config`: `options.config` when given (the Integrations
   * tab's Test), else the saved config of `options.kind` or the method's kind. A disabled
   * integration is `not-configured` unless the config was passed explicitly.
   */
  async request<M extends IntegrationMethod>(
    method: M,
    params: object = {},
    options: { kind?: IntegrationKind; config?: ProviderConfig } = {}
  ): Promise<IntegrationResults[M]> {
    const transport = this._transport;
    if (!transport.available) {
      // Don't read the keyset for a request that can't be sent.
      throw new IntegrationError({
        key: 'engine-unavailable',
        message: 'The engine does not support integrations yet.',
      });
    }
    let config = options.config || null;
    if (!config) {
      const kind = options.kind || METHOD_KIND[method];
      if (!kind) {
        throw new IntegrationError({ key: 'not-configured', message: `${method} needs a kind` });
      }
      if (!readSettings(kind).enabled) {
        throw new IntegrationError({ key: 'not-configured', message: `${kind} is turned off` });
      }
      config = await this.savedConfig(kind);
    }
    if (!config) {
      throw new IntegrationError({ key: 'not-configured', message: 'no provider is chosen' });
    }
    try {
      return (await transport.send(method, { ...params, config })) as IntegrationResults[M];
    } catch (err) {
      throw toIntegrationError(err);
    }
  }
}

export const Integrations = new IntegrationsClient();
export default Integrations;
