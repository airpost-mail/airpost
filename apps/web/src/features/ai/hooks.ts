import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { useEffect, useMemo } from 'react';
import {
  EngineFeature,
  ensureConnected,
  useEngine,
  useEngineCapabilities,
  useEngineFeatures,
  type EngineCapabilities,
  type EngineClient,
} from '@/engine';
import {
  AI_SETTING_KEYS,
  NO_AI_SETTINGS,
  keyRequired,
  providerFor,
  readAiSettings,
  type AiFeature,
  type AiProvider,
  type AiSettings,
} from './config';

export const aiKeys = {
  all: ['engine', 'ai'] as const,
  settings: () => ['engine', 'ai', 'settings'] as const,
  needsReplyAll: () => ['engine', 'ai', 'needs-reply'] as const,
  needsReply: (accounts: readonly string[]) => ['engine', 'ai', 'needs-reply', accounts] as const,
  usage: () => ['engine', 'ai', 'usage'] as const,
};

type Values = Record<string, unknown>;

const watched = new WeakMap<EngineClient, QueryClient>();

/** One `settings.changed` listener per client, folding `core.ai.*` changes into the query. */
function watchSettings(client: EngineClient, qc: QueryClient) {
  if (watched.get(client) === qc) return;
  watched.set(client, qc);
  client.onNotification((n) => {
    if (n.method !== 'settings.changed') return;
    const changed = Object.entries(n.params.values).filter(([k]) =>
      (AI_SETTING_KEYS as readonly string[]).includes(k),
    );
    if (changed.length === 0) return;
    qc.setQueryData<Values>(aiKeys.settings(), (prev) => {
      const next = { ...prev };
      for (const [k, v] of changed) {
        if (v === null) delete next[k];
        else next[k] = v;
      }
      return next;
    });
  });
}

/** The engine's `core.ai.*`; empty on an engine without `settings` or `ai`. */
export function useAiSettings(): AiSettings {
  const client = useEngine();
  const qc = useQueryClient();
  const features = useEngineFeatures();
  const enabled = !!features?.has(EngineFeature.Settings) && !!features.has(EngineFeature.Ai);
  useEffect(() => {
    if (enabled) watchSettings(client, qc);
  }, [client, qc, enabled]);
  const { data } = useQuery({
    queryKey: aiKeys.settings(),
    queryFn: async () => {
      await ensureConnected(client);
      const r = await client.call('settings.get', { keys: [...AI_SETTING_KEYS] });
      return r.values;
    },
    enabled,
    staleTime: Infinity,
  });
  return useMemo(() => (enabled && data ? readAiSettings(data) : NO_AI_SETTINGS), [enabled, data]);
}

/**
 * Why a feature can't run: the engine lacks it, it is off, the account isn't opted in, no
 * model is set up, the engine holds no key for the provider, or the engine's own reason
 * (`rate-limited`, `quota-exceeded`, …).
 */
export type AiUnavailable =
  'engine' | 'off' | 'account' | 'not-set-up' | 'needs-engine-key' | (string & {});

export interface AiAvailability {
  available: boolean;
  provider: AiProvider | null;
  reason?: AiUnavailable;
}

export function aiAvailability(
  feature: AiFeature,
  accountId: string | undefined,
  ctx: {
    settings: AiSettings;
    features: ReadonlySet<string> | null;
    capabilities: EngineCapabilities | null;
  },
): AiAvailability {
  const { settings, features } = ctx;
  const provider = providerFor(settings, feature);
  const no = (reason: AiUnavailable): AiAvailability => ({ available: false, provider, reason });
  if (!features?.has(EngineFeature.Ai)) return no('engine');
  if (feature === 'triage' && !features.has(EngineFeature.AiTriage)) return no('engine');
  if (!settings.features[feature].enabled) return no('off');
  if (accountId !== undefined && !settings.accounts.includes(accountId)) return no('account');
  if (!provider || !provider.model.trim()) return no('not-set-up');
  const cap = ctx.capabilities?.ai?.[feature];
  // `request`: the engine leaves the judgement to a client that sends keys; this one sends
  // none, so a provider that needs a key can't run until the engine holds it.
  if (!cap || cap.credentials === 'request') {
    return keyRequired(provider) ? no('needs-engine-key') : { available: true, provider };
  }
  return cap.available ? { available: true, provider } : no(cap.reason ?? 'not-set-up');
}

export function useAiAvailability(feature: AiFeature, accountId?: string): AiAvailability {
  const settings = useAiSettings();
  const features = useEngineFeatures();
  const capabilities = useEngineCapabilities();
  return useMemo(
    () => aiAvailability(feature, accountId, { settings, features, capabilities }),
    [feature, accountId, settings, features, capabilities],
  );
}

/** Whether triage results show for the account: the engine triages it and triage is on. */
export function useTriageShown(accountId: string): boolean {
  const settings = useAiSettings();
  const features = useEngineFeatures();
  return (
    !!features?.has(EngineFeature.AiTriage) &&
    settings.features.triage.enabled &&
    settings.accounts.includes(accountId)
  );
}

export function unavailableMessage(reason: AiUnavailable | undefined): string {
  switch (reason) {
    case 'engine':
      return 'Waiting for engine support.';
    case 'off':
      return 'Turned off. Turn it on in the desktop app, Preferences > AI.';
    case 'account':
      return 'AI is off for this account. Opt it in on the desktop, Preferences > AI.';
    case 'needs-engine-key':
      return "The engine doesn't hold this provider's API key yet. Use a local model, or the desktop app.";
    case 'rate-limited':
      return 'The provider is limiting requests. Triage resumes by itself.';
    case 'quota-exceeded':
      return "The provider's quota or the daily token cap is used up.";
    default:
      return 'Set up a model in the desktop app, Preferences > AI.';
  }
}
