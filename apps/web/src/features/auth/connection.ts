import { useEffect, useState, useSyncExternalStore } from 'react';
import {
  useConnectionProblem,
  useConnectionState,
  type ConnectionProblem,
  type ConnectionState,
} from '@/engine';

export type ConnectionStatusKind = 'connected' | 'connecting' | 'offline';

export interface ConnectionStatusInfo {
  status: ConnectionStatusKind;
  /** The engine client's own state, for finer labels. */
  state: ConnectionState;
  /** Why it isn't connected, when the client knows. */
  problem: ConnectionProblem | null;
  /** False when the browser itself reports no network. */
  online: boolean;
}

export function statusFor(state: ConnectionState, online: boolean): ConnectionStatusKind {
  if (!online) return 'offline';
  if (state === 'open') return 'connected';
  if (state === 'connecting' || state === 'reconnecting') return 'connecting';
  return 'offline';
}

function subscribeOnline(cb: () => void) {
  window.addEventListener('online', cb);
  window.addEventListener('offline', cb);
  return () => {
    window.removeEventListener('online', cb);
    window.removeEventListener('offline', cb);
  };
}

export function useBrowserOnline(): boolean {
  return useSyncExternalStore(
    subscribeOnline,
    () => navigator.onLine,
    () => true,
  );
}

export function useConnectionStatus(): ConnectionStatusInfo {
  const state = useConnectionState();
  const problem = useConnectionProblem();
  const online = useBrowserOnline();
  return { status: statusFor(state, online), state, problem, online };
}

/** True once `active` has held for `ms`; keeps quick connects from flashing a banner. */
export function useAfterDelay(active: boolean, ms: number): boolean {
  const [elapsed, setElapsed] = useState(false);
  useEffect(() => {
    if (!active) return;
    const t = setTimeout(() => setElapsed(true), ms);
    return () => {
      clearTimeout(t);
      setElapsed(false);
    };
  }, [active, ms]);
  return active && elapsed;
}
