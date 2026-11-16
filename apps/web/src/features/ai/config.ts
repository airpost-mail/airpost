// The engine's AI settings (`core.ai.*`), read-only here:
// providers, feature switches and account opt-in are set on the desktop, and a relayed
// connection may read them but never write them. The provider rules mirror airpost-ai's
// ProviderConfig (and the desktop's src/engine/client/ai.ts).

export type AiProviderKind = 'openai' | 'anthropic' | 'ollama';
export type AiFeature = 'summary' | 'reply' | 'triage';

export const AI_FEATURES: AiFeature[] = ['summary', 'reply', 'triage'];

export const AI_SETTING_KEYS = [
  'core.ai.providers',
  'core.ai.features',
  'core.ai.accounts',
  'core.ai.includeAddresses',
] as const;

export interface AiProvider {
  id: string;
  kind: AiProviderKind;
  name?: string;
  url?: string;
  model: string;
}

export interface AiSettings {
  providers: AiProvider[];
  features: Record<AiFeature, { enabled: boolean; provider?: string }>;
  accounts: string[];
  includeAddresses: boolean;
}

export const KINDS: Record<AiProviderKind, { name: string; defaultUrl: string }> = {
  openai: { name: 'OpenAI-compatible', defaultUrl: 'https://api.openai.com/v1' },
  anthropic: { name: 'Anthropic', defaultUrl: 'https://api.anthropic.com' },
  ollama: { name: 'Ollama', defaultUrl: 'http://127.0.0.1:11434' },
};

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

function readProvider(raw: unknown): AiProvider | null {
  if (!isObject(raw) || typeof raw.id !== 'string' || !raw.id) return null;
  if (typeof raw.kind !== 'string' || !(raw.kind in KINDS)) return null;
  const p: AiProvider = {
    id: raw.id,
    kind: raw.kind as AiProviderKind,
    model: typeof raw.model === 'string' ? raw.model : '',
  };
  if (typeof raw.name === 'string' && raw.name.trim()) p.name = raw.name.trim();
  if (typeof raw.url === 'string' && raw.url.trim()) p.url = raw.url.trim();
  return p;
}

/** The settings from `settings.get` values; anything of the wrong shape reads as unset. */
export function readAiSettings(values: Record<string, unknown>): AiSettings {
  const providers: AiProvider[] = [];
  const rawProviders = values['core.ai.providers'];
  for (const raw of Array.isArray(rawProviders) ? rawProviders : []) {
    const p = readProvider(raw);
    if (p && !providers.some((o) => o.id === p.id)) providers.push(p);
  }
  const rawFeatures = isObject(values['core.ai.features']) ? values['core.ai.features'] : {};
  const features = {} as AiSettings['features'];
  for (const f of AI_FEATURES) {
    const raw = rawFeatures[f];
    // As airpost-ipc AiSettings::feature_enabled: `true`, or an object with `enabled: true`.
    const enabled = raw === true || (isObject(raw) && raw.enabled === true);
    const provider = isObject(raw) && typeof raw.provider === 'string' ? raw.provider : undefined;
    features[f] = provider ? { enabled, provider } : { enabled };
  }
  const accounts = values['core.ai.accounts'];
  return {
    providers,
    features,
    accounts: Array.isArray(accounts)
      ? accounts.filter((a): a is string => typeof a === 'string')
      : [],
    includeAddresses: values['core.ai.includeAddresses'] === true,
  };
}

export const NO_AI_SETTINGS: AiSettings = readAiSettings({});

export function providerName(p: AiProvider): string {
  return p.name || KINDS[p.kind].name;
}

export function providerFor(s: AiSettings, feature: AiFeature): AiProvider | null {
  const id = s.features[feature].provider;
  return s.providers.find((p) => p.id === id) ?? null;
}

/** The base URL as airpost-ai's ProviderConfig::base_url builds it. */
export function baseUrl(p: AiProvider): string {
  let url = (p.url ?? '').trim() || KINDS[p.kind].defaultUrl;
  url = url.replace(/\/+$/, '');
  const suffixes = {
    openai: ['/chat/completions'],
    anthropic: ['/v1/messages', '/v1'],
    ollama: ['/api/chat', '/api', '/v1'],
  }[p.kind];
  for (const suffix of suffixes) {
    if (url.endsWith(suffix)) {
      url = url.slice(0, -suffix.length).replace(/\/+$/, '');
      break;
    }
  }
  return url;
}

/** Where requests go ("api.anthropic.com"); empty for a URL that doesn't parse. */
export function hostOf(p: AiProvider): string {
  try {
    return new URL(baseUrl(p)).hostname.replace(/^\[|\]$/g, '');
  } catch {
    return '';
  }
}

function isPrivateV4([a, b]: number[]) {
  return (
    a === 127 ||
    a === 10 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 169 && b === 254)
  );
}

/** Loopback or a private address, as airpost-ai's ProviderConfig::is_local. */
export function isLocal(p: AiProvider): boolean {
  const host = hostOf(p).toLowerCase().replace(/\.$/, '');
  if (!host) return false;
  if (host === 'localhost' || host.endsWith('.localhost')) return true;
  const v4 = /^(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(host);
  if (v4) return isPrivateV4(v4.slice(1).map(Number));
  if (host.includes(':')) {
    // URL serialises an IPv4-mapped address in hex: [::ffff:192.168.1.5] is ::ffff:c0a8:105.
    const mapped = /^::ffff:([0-9a-f]{1,4}):([0-9a-f]{1,4})$/.exec(host);
    if (mapped) {
      const hi = parseInt(mapped[1], 16);
      return isPrivateV4([hi >> 8, hi & 255]);
    }
    return host === '::1' || /^f[cd]/.test(host) || /^fe[89ab]/.test(host);
  }
  return false;
}

/** On the engine's own computer. A private address is another machine on its network. */
export function isLoopback(p: AiProvider): boolean {
  const host = hostOf(p).toLowerCase().replace(/\.$/, '');
  if (!host) return false;
  if (host === 'localhost' || host.endsWith('.localhost') || host === '::1') return true;
  if (/^::ffff:7f[0-9a-f]{2}:/.test(host)) return true;
  const v4 = /^(\d+)\.\d+\.\d+\.\d+$/.exec(host);
  return !!v4 && Number(v4[1]) === 127;
}

/** The engine refuses these without a key: Anthropic, and OpenAI's own API. */
export function keyRequired(p: AiProvider): boolean {
  if (p.kind === 'anthropic') return true;
  return p.kind === 'openai' && baseUrl(p) === KINDS.openai.defaultUrl;
}

/**
 * What the UI says before it sends mail to a model. The engine makes the call, so "this
 * computer" is the engine's, not the browser's.
 */
export function destinationNote(p: AiProvider | null): string {
  if (!p) return '';
  if (isLoopback(p)) return `Runs on the engine's computer (${providerName(p)})`;
  const host = hostOf(p) || providerName(p);
  return isLocal(p) ? `Sent to ${host} (the engine's network)` : `Sent to ${host}`;
}
