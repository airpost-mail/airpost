// Session: whether this browser is paired with the gateway that served it, and what the
// gateway says about itself and the engine.
//
//  1. Pair: the user runs `airpost-gateway pair` where the engine runs and opens the printed
//     `/pair#<code>` link (or types the code). `POST /gateway/v1/pair` sets an httpOnly
//     cookie; the browser never sees the device token, and nothing is stored here.
//  2. Session: `GET /gateway/v1/session` (the cookie goes along) answers the device, its
//     scopes and whether the engine is up; 401 means unpaired. The engine client calls it
//     before every connect, which keeps this store current.
//  3. Unpaired: a 401 here or close 4401 on the socket drops the session and sends the user
//     to /pair. Sign out is `POST /gateway/v1/unpair`.
import { redirect, type ParsedLocation } from '@tanstack/react-router';
import { useSyncExternalStore } from 'react';
import { engineMode, type HealthOutcome } from '@/engine';
import { normalizePairingCode } from './pairing-code';
import {
  AuthError,
  createHttpGatewayTransport,
  createMockGatewayTransport,
  type EngineHealth,
  type GatewaySession,
  type GatewayTransport,
  type PairedDevice,
} from './transport';

export interface Session {
  device: PairedDevice;
  engine: EngineHealth;
  gateway: { version: string };
  /** The in-browser mock engine, not a real gateway. */
  demo: boolean;
}

/**
 * `unknown` until the first answer; `unreachable` when the gateway didn't answer (the last
 * known session, if any, is kept).
 */
export type SessionStatus = 'unknown' | 'paired' | 'unpaired' | 'unreachable';

export type SignOutReason = 'user' | 'revoked';

export interface SessionSnapshot {
  status: SessionStatus;
  session: Session | null;
}

const MAX_DEVICE_NAME = 64;

export function isAdmin(session: Session | null): boolean {
  return !!session?.device.scopes.includes('admin');
}

export function createSessionStore(opts: { transport: GatewayTransport }) {
  const { transport } = opts;
  const listeners = new Set<() => void>();
  let snapshot: SessionSnapshot = { status: 'unknown', session: null };
  let notice: SignOutReason | null = null;
  let loaded: Promise<void> | null = null;

  function set(next: SessionSnapshot) {
    if (
      next.status === snapshot.status &&
      JSON.stringify(next.session) === JSON.stringify(snapshot.session)
    ) {
      return;
    }
    snapshot = next;
    for (const l of listeners) l();
  }

  function fromGateway(s: GatewaySession): Session {
    return {
      device: s.device,
      engine: s.engine,
      gateway: s.gateway,
      demo: transport.kind === 'mock',
    };
  }

  function subscribe(cb: () => void) {
    listeners.add(cb);
    return () => {
      listeners.delete(cb);
    };
  }

  /** Forgets the session. `revoked` is the gateway's doing; `user` a deliberate sign-out. */
  function markUnpaired(reason: SignOutReason | null) {
    if (snapshot.session && reason) notice = reason;
    loaded = Promise.resolve();
    set({ status: 'unpaired', session: null });
  }

  /** Asks the gateway; this is the engine client's health check too. */
  async function refresh(): Promise<HealthOutcome> {
    try {
      const s = await transport.session();
      set({ status: 'paired', session: fromGateway(s) });
      return { ok: true, engineUp: s.engine.state === 'up' };
    } catch (err) {
      if (err instanceof AuthError && err.code === 'unpaired') {
        markUnpaired('revoked');
        return { ok: false, reason: 'unpaired' };
      }
      set({ status: 'unreachable', session: snapshot.session });
      return { ok: false, reason: 'unreachable' };
    }
  }

  /** The first `refresh`, shared by every caller until it has answered. */
  function load(): Promise<void> {
    loaded ??= refresh().then(() => undefined);
    return loaded;
  }

  async function pair(code: string, deviceName: string): Promise<Session> {
    const normalized = normalizePairingCode(code);
    if (!normalized) {
      throw new AuthError('invalid-code', 'A pairing code has 12 letters and digits.');
    }
    const name = deviceName.trim().slice(0, MAX_DEVICE_NAME) || 'Browser';
    await transport.pair(normalized, name);
    const outcome = await refresh();
    if (!outcome.ok || !snapshot.session) {
      throw new AuthError('server', 'Paired, but the server did not confirm it. Reload the page.');
    }
    notice = null;
    loaded = Promise.resolve();
    return snapshot.session;
  }

  async function signOut() {
    notice = 'user';
    markUnpaired(null);
    await transport.unpair();
  }

  /** For `beforeLoad`: sends unpaired browsers to /pair and back afterwards. */
  async function requireSession({ location }: { location: Pick<ParsedLocation, 'href'> }) {
    await load();
    if (snapshot.status === 'unpaired') {
      throw redirect({ to: '/pair', search: { redirect: location.href } });
    }
  }

  /** For the pair route's `beforeLoad`: paired browsers go straight to where they were headed. */
  async function redirectIfPaired({ search }: { search: { redirect?: string } }) {
    await load();
    if (snapshot.status === 'paired')
      throw redirect({ href: search.redirect ?? '/', replace: true });
  }

  return {
    transport,
    get: () => snapshot.session,
    snapshot: () => snapshot,
    subscribe,
    load,
    refresh,
    pair,
    signOut,
    markUnpaired,
    requireSession,
    redirectIfPaired,
    /** Why the last sign-out happened, for the pair screen. Cleared by the next pairing. */
    notice: () => notice,
  };
}

export type SessionStore = ReturnType<typeof createSessionStore>;

const store = createSessionStore({
  transport:
    engineMode() === 'gateway' ? createHttpGatewayTransport() : createMockGatewayTransport(),
});

export const sessionStore = store;
export const getSession = store.get;
export const pair = store.pair;
export const signOut = store.signOut;
export const refreshSession = store.refresh;
export const markUnpaired = store.markUnpaired;
export const requireSession = store.requireSession;
export const redirectIfPaired = store.redirectIfPaired;
export const getSignOutNotice = store.notice;
export const gatewayTransport = store.transport;
export const subscribeSession = store.subscribe;

export function useSession(): Session | null {
  return useSyncExternalStore(store.subscribe, store.get);
}

export function useSessionSnapshot(): SessionSnapshot {
  return useSyncExternalStore(store.subscribe, store.snapshot);
}
