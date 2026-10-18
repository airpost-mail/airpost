import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import { useEffect, type ReactNode } from 'react';
import { applyDelta } from './cache';
import type { EngineClient } from './client';
import { ensureConnected } from './connect';
import { EngineContext } from './context';
import { seedProcessState } from './hooks';
import { engineKeys, EVERY_ACCOUNT } from './keys';
import type { AccountRunState, AccountStateParams } from './rpc';
import { applyCapabilitiesChanged, applySettingsChanged, loadEngineState } from './settings';

/** What an account shows; refetched when the engine restarts it (a reset, or a re-add). */
function invalidateAccount(qc: QueryClient, accountId: string) {
  for (const key of [
    engineKeys.folders(accountId),
    engineKeys.labels(accountId),
    engineKeys.threadLists(accountId),
    engineKeys.threadLists(EVERY_ACCOUNT),
    engineKeys.messagesAll(accountId),
  ]) {
    void qc.invalidateQueries({ queryKey: key });
  }
}

/** Connects the client and keeps the query cache in step with its delta stream. */
export function EngineProvider({
  client,
  children,
}: {
  client: EngineClient;
  children: ReactNode;
}) {
  const queryClient = useQueryClient();

  useEffect(() => {
    const lastState = new Map<string, AccountRunState>();
    const onAccountState = (p: AccountStateParams) => {
      const before = lastState.get(p.accountId);
      lastState.set(p.accountId, p.state);
      // The accounts query maps `stopped` to a sync error, which drives the account banners.
      void queryClient.invalidateQueries({ queryKey: engineKeys.accounts() });
      if (before === 'starting' && p.state === 'running')
        invalidateAccount(queryClient, p.accountId);
    };
    const off = client.onNotification((n) => {
      if (n.method === 'delta') applyDelta(queryClient, n.params);
      else if (n.method === 'account.state') onAccountState(n.params);
      else if (n.method === 'settings.changed') applySettingsChanged(n.params.values);
      else if (n.method === 'capabilities.changed') applyCapabilitiesChanged(n.params);
    });
    const seed = () =>
      ensureConnected(client)
        .then((hello) =>
          Promise.all([
            seedProcessState(queryClient, client),
            loadEngineState(client, hello).catch((err: unknown) =>
              console.warn('engine settings unavailable', err),
            ),
          ]),
        )
        .catch((err: unknown) => {
          // An unpaired browser is sent to /pair; that's not a failure worth logging.
          if (client.problem !== 'unpaired') console.error('engine connect failed', err);
        });
    // Deltas sent while the connection was down are lost, so after a reconnect every query
    // re-reads (engine-protocol-plan 1.4). The first open has nothing stale to refetch.
    let opened = false;
    const offState = client.onStateChange((s) => {
      if (s !== 'open') return;
      if (opened) void queryClient.invalidateQueries();
      opened = true;
      void seed();
    });
    void seed();
    return () => {
      off();
      offState();
    };
  }, [client, queryClient]);

  return <EngineContext.Provider value={client}>{children}</EngineContext.Provider>;
}
