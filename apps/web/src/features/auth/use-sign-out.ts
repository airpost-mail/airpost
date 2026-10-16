import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useCallback } from 'react';
import { useEngine } from '@/engine';
import { signOut } from './session';

/**
 * Drops what this browser could see: cached mail and the engine connection. Queries
 * reconnect through `ensureConnected` after the next pairing.
 */
export function useEndSessionCleanup() {
  const queryClient = useQueryClient();
  const engine = useEngine();
  return useCallback(() => {
    queryClient.clear();
    engine.close();
  }, [engine, queryClient]);
}

/** Unpairs this browser (best effort on the gateway) and goes to the pair screen. */
export function useSignOut() {
  const navigate = useNavigate();
  const cleanup = useEndSessionCleanup();
  return useCallback(async () => {
    await signOut();
    await navigate({ to: '/pair' });
    cleanup();
  }, [cleanup, navigate]);
}
