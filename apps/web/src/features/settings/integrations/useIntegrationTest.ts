import { useMutation } from '@tanstack/react-query';
import { RpcError, RpcErrorCode, useEngine, type IntegrationConfig } from '@/engine';
import { callEngine, engineErrorMessage } from '../engine-call';
import { providerDef, type FieldKey, type ProviderId } from './providers';
import { providerParams } from './store';

export type TestOutcome = { ok: true; host: string } | { ok: false; error: string };

/** The engine's provider names (airpost-integrations `ProviderConfig`). */
const ENGINE_PROVIDER: Record<ProviderId, IntegrationConfig['provider']> = {
  libretranslate: 'libretranslate',
  deepl: 'deepl',
  languagetool: 'languagetool',
  pdl: 'peopledatalabs',
};

/** The saved settings overlaid by unsaved edits, as `integrations.*` take them. */
export function integrationConfig(
  id: ProviderId,
  draft: Partial<Record<FieldKey, string>> = {},
): IntegrationConfig {
  const { provider: _, ...fields } = providerParams(id, draft);
  return { ...fields, provider: ENGINE_PROVIDER[id] };
}

function failure(err: unknown): string {
  // -32010 "Integration failed: <why>": the why is the part for the person.
  if (err instanceof RpcError && err.code === RpcErrorCode.IntegrationFailed) {
    const why = err.message.replace(/^Integration failed:\s*/, '');
    return why ? why[0]!.toUpperCase() + why.slice(1) : 'The provider rejected the settings.';
  }
  // `integrations.*` are local-only (airpost-ipc policy): only an admin device gets through.
  if (err instanceof RpcError && err.code === RpcErrorCode.NotAllowedRelayed) {
    return 'Testing providers from the web needs an admin device.';
  }
  return engineErrorMessage(err);
}

/** Runs `integrations.test` with the saved settings overlaid by unsaved edits. */
export function useIntegrationTest(id: ProviderId) {
  const client = useEngine();
  return useMutation<TestOutcome, never, Partial<Record<FieldKey, string>>>({
    mutationFn: async (draft) => {
      const config = integrationConfig(id, draft);
      // The engine refuses a config without them as invalid params.
      const missing = providerDef(id).fields.find((f) => f.required && !config[f.key]);
      if (missing) return { ok: false, error: `Enter the ${missing.label} first.` };
      try {
        const report = await callEngine(client, 'integrations.test', { config });
        return { ok: true, host: report.host };
      } catch (err) {
        return { ok: false, error: failure(err) };
      }
    },
  });
}
