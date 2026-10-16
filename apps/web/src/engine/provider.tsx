import { useQueryClient } from '@tanstack/react-query';
import { useEffect, type ReactNode } from 'react';
import { applyDelta } from './cache';
import type { EngineClient } from './client';
import { ensureConnected } from './connect';
import { EngineContext } from './context';
import { seedProcessState } from './hooks';

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
    const off = client.onNotification((n) => {
      if (n.method === 'delta') applyDelta(queryClient, n.params);
      // TODO(engine): `account.state` (phase C) should drive per-account error banners.
    });
    const seed = () =>
      ensureConnected(client)
        .then(() => seedProcessState(queryClient, client))
        .catch((err: unknown) => console.error('engine connect failed', err));
    // Again on every reconnect: accounts may have gone on- or offline meanwhile.
    const offState = client.onStateChange((s) => {
      if (s === 'open') void seed();
    });
    void seed();
    return () => {
      off();
      offState();
    };
  }, [client, queryClient]);

  return <EngineContext.Provider value={client}>{children}</EngineContext.Provider>;
}
