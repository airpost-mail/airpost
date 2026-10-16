import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ConnectionState, EngineClient } from './client';
import { engineKeys } from './keys';
import { EngineProvider } from './provider';
import type { EngineNotification } from './rpc';

function fakeClient() {
  const stateListeners = new Set<(s: ConnectionState) => void>();
  const notificationListeners = new Set<(n: EngineNotification) => void>();
  const client = {
    kind: 'ws' as const,
    state: 'idle' as ConnectionState,
    problem: null,
    connect: vi.fn(async () => ({ protocol: 1, engine: 'x', pid: 1, accounts: [] })),
    close: vi.fn(),
    retryNow: vi.fn(),
    call: vi.fn(async () => ({ protocol: 1, engine: 'x', pid: 1, uptimeSecs: 0, accounts: [] })),
    onNotification(l: (n: EngineNotification) => void) {
      notificationListeners.add(l);
      return () => notificationListeners.delete(l);
    },
    onStateChange(l: (s: ConnectionState) => void) {
      stateListeners.add(l);
      return () => stateListeners.delete(l);
    },
    fileUrl: () => null,
    set(s: ConnectionState) {
      client.state = s;
      for (const l of stateListeners) l(s);
    },
    notify(n: EngineNotification) {
      for (const l of notificationListeners) l(n);
    },
  };
  return client;
}

function setup() {
  const client = fakeClient();
  const qc = new QueryClient();
  const invalidate = vi.spyOn(qc, 'invalidateQueries');
  render(
    <QueryClientProvider client={qc}>
      <EngineProvider client={client as unknown as EngineClient}>{null}</EngineProvider>
    </QueryClientProvider>,
  );
  return { client, qc, invalidate };
}

describe('EngineProvider', () => {
  it('invalidates every query after a reconnect, not after the first open', async () => {
    const { client, invalidate } = setup();
    await act(async () => client.set('open'));
    expect(invalidate).not.toHaveBeenCalled();

    await act(async () => client.set('reconnecting'));
    await act(async () => client.set('open'));
    expect(invalidate).toHaveBeenCalledWith();
  });

  it('refetches an account the engine restarted', async () => {
    const { client, invalidate } = setup();
    const state = (s: 'starting' | 'running') =>
      act(async () =>
        client.notify({ method: 'account.state', params: { accountId: 'a1', state: s } }),
      );
    await state('starting');
    expect(invalidate).toHaveBeenCalledWith({ queryKey: engineKeys.accounts() });
    expect(invalidate).not.toHaveBeenCalledWith({ queryKey: engineKeys.threadLists('a1') });
    await state('running');
    expect(invalidate).toHaveBeenCalledWith({ queryKey: engineKeys.threadLists('a1') });
  });
});
