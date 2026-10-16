import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useCallback } from 'react';
import { useEngine } from '@/engine';
import { signOut } from './session';

/**
 * Drops what the previous user could see: cached mail and the engine connection. Queries
 * reconnect through `ensureConnected` after the next sign-in.
 */
export function useEndSessionCleanup() {
  const queryClient = useQueryClient();
  const engine = useEngine();
  return useCallback(() => {
    queryClient.clear();
    engine.close();
  }, [engine, queryClient]);
}

/** Signs out, revokes the device token (best effort) and goes to the login screen. */
export function useSignOut() {
  const navigate = useNavigate();
  const cleanup = useEndSessionCleanup();
  return useCallback(async () => {
    await signOut('user');
    await navigate({ to: '/login' });
    cleanup();
  }, [cleanup, navigate]);
}
