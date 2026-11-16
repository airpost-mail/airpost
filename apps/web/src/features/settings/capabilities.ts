// One answer to "is feature X available, and from where". Features ask here instead of
// checking settings themselves; a connected Airpost server later only adds the server source.
import { useMemo, useSyncExternalStore } from 'react';
import {
  getEngineCapabilities,
  useEngineCapabilities,
  type EngineCapabilities,
  type EngineCapability,
} from '@/engine';
import { PROVIDERS, type IntegrationKind, type ProviderId } from './integrations/providers';
import {
  getIntegrations,
  isProviderConfigured,
  subscribeIntegrations,
  type IntegrationsState,
} from './integrations/store';

export type CapabilitySource = 'local' | 'integration' | 'server';

export type CapabilityName =
  | 'snooze'
  | 'send-later'
  | 'reminders'
  | 'templates'
  | 'insights'
  | 'translation'
  | 'grammar'
  | 'enrichment'
  | 'read-receipts'
  | 'link-tracking'
  | 'sharing'
  | 'metadata-sync';

interface CapabilityDef {
  title: string;
  description: string;
  source: CapabilitySource;
  /** For integration capabilities: which providers can serve it. */
  integration?: IntegrationKind;
  /** For server capabilities: the entry of the engine's `capabilities.server`. */
  serverFeature?: string;
}

export const CAPABILITIES: Record<CapabilityName, CapabilityDef> = {
  snooze: {
    title: 'Snooze',
    description: 'Hide a thread until a time you pick.',
    source: 'local',
  },
  'send-later': {
    title: 'Send later',
    description: 'Schedule a message to go out at a set time.',
    source: 'local',
  },
  reminders: {
    title: 'Reminders',
    description: 'Bring a thread back if nobody replies.',
    source: 'local',
  },
  templates: {
    title: 'Templates',
    description: 'Reusable messages with fill-in fields.',
    source: 'local',
  },
  insights: {
    title: 'Insights',
    description: 'Activity and response times across your mail.',
    source: 'local',
  },
  translation: {
    title: 'Translation',
    description: 'Translate messages you read and drafts you write.',
    source: 'integration',
    integration: 'translation',
  },
  grammar: {
    title: 'Grammar check',
    description: 'Spelling, grammar and style suggestions in the composer.',
    source: 'integration',
    integration: 'grammar',
  },
  enrichment: {
    title: 'Contact enrichment',
    description: 'Company and role details next to a sender.',
    source: 'integration',
    integration: 'enrichment',
  },
  'read-receipts': {
    title: 'Read receipts',
    description: 'See when a recipient opens your message.',
    source: 'server',
    serverFeature: 'readReceipts',
  },
  'link-tracking': {
    title: 'Link tracking',
    description: 'See when a recipient clicks a link you sent.',
    source: 'server',
    serverFeature: 'linkTracking',
  },
  sharing: {
    title: 'Sharing',
    description: 'Share a thread with a link.',
    source: 'server',
    serverFeature: 'threadSharing',
  },
  'metadata-sync': {
    title: 'Sync across devices',
    description: 'Snoozes, reminders and tracking state on every device.',
    source: 'server',
    serverFeature: 'metadataSync',
  },
};

export const CAPABILITY_NAMES = Object.keys(CAPABILITIES) as CapabilityName[];

export type CapabilityReason = 'ok' | 'needs-integration' | 'needs-server';

export interface Capability {
  name: CapabilityName;
  title: string;
  description: string;
  source: CapabilitySource;
  available: boolean;
  reason: CapabilityReason;
  /** The provider serving an integration capability, when one is on and configured. */
  provider?: ProviderId;
}

export interface ServerState {
  connected: boolean;
  /** The engine's `capabilities.server` (DCP stage 5); absent from an engine without it. */
  features?: Record<string, EngineCapability>;
}

export interface CapabilityContext {
  integrations: IntegrationsState;
  server: ServerState;
}

/** No Airpost server: what an engine without `capabilities` amounts to. */
export const NO_SERVER: ServerState = { connected: false };

/**
 * The Airpost server as the engine reports it: the engine holds
 * the server connection (G8), and its `capabilities.server` says which features it gives.
 */
export function serverStateFrom(caps: EngineCapabilities | null): ServerState {
  const features = caps?.server;
  if (!features) return NO_SERVER;
  return { connected: Object.values(features).some((f) => f.available), features };
}

export function resolveCapability(name: CapabilityName, ctx: CapabilityContext): Capability {
  const def = CAPABILITIES[name];
  const base = { name, title: def.title, description: def.description, source: def.source };
  switch (def.source) {
    case 'local':
      return { ...base, available: true, reason: 'ok' };
    case 'server': {
      const entry = def.serverFeature ? ctx.server.features?.[def.serverFeature] : undefined;
      const available = ctx.server.features ? !!entry?.available : ctx.server.connected;
      return available
        ? { ...base, available: true, reason: 'ok' }
        : { ...base, available: false, reason: 'needs-server' };
    }
    case 'integration': {
      const provider = PROVIDERS.find(
        (p) =>
          p.kind === def.integration &&
          ctx.integrations.providers[p.id].enabled &&
          isProviderConfigured(ctx.integrations, p.id),
      );
      return provider
        ? { ...base, available: true, reason: 'ok', provider: provider.id }
        : { ...base, available: false, reason: 'needs-integration' };
    }
  }
}

function useCapabilityContext(): CapabilityContext {
  const integrations = useSyncExternalStore(subscribeIntegrations, getIntegrations);
  const caps = useEngineCapabilities();
  return useMemo(() => ({ integrations, server: serverStateFrom(caps) }), [integrations, caps]);
}

/** Whether a feature is available, and from which source. Re-renders when settings change. */
export function useCapability(name: CapabilityName): Capability {
  const ctx = useCapabilityContext();
  return useMemo(() => resolveCapability(name, ctx), [name, ctx]);
}

export function useCapabilities(): Capability[] {
  const ctx = useCapabilityContext();
  return useMemo(() => CAPABILITY_NAMES.map((n) => resolveCapability(n, ctx)), [ctx]);
}

/** Non-hook form, for code outside React (e.g. task builders). */
export function getCapability(name: CapabilityName): Capability {
  return resolveCapability(name, {
    integrations: getIntegrations(),
    server: serverStateFrom(getEngineCapabilities()),
  });
}
