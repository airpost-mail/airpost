import { useMutation } from '@tanstack/react-query';
import { useEngine } from '@/engine';
import { callEngine, engineErrorMessage } from '../engine-call';
import type { FieldKey, ProviderId } from './providers';
import { providerParams } from './store';

export type TestOutcome = { ok: true } | { ok: false; error: string };

function readResult(result: unknown): TestOutcome {
  // TODO(engine): `integrations.test`'s result shape comes with DCP stage 5; accept
  // {ok, error} or any non-error reply as success.
  if (result && typeof result === 'object' && 'ok' in result && result.ok === false) {
    const error = 'error' in result && typeof result.error === 'string' ? result.error : '';
    return { ok: false, error: error || 'The provider rejected the settings.' };
  }
  return { ok: true };
}

/** Runs `integrations.test` with the saved settings overlaid by unsaved edits. */
export function useIntegrationTest(id: ProviderId) {
  const client = useEngine();
  return useMutation<TestOutcome, never, Partial<Record<FieldKey, string>>>({
    mutationFn: async (draft) => {
      try {
        return readResult(await callEngine(client, 'integrations.test', providerParams(id, draft)));
      } catch (err) {
        return { ok: false, error: engineErrorMessage(err) };
      }
    },
  });
}
