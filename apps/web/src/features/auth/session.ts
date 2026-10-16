// Session: who is signed in to which Tern server, and the token the engine connection uses.
//
// Expected flow against a hosted Tern server (docs/backend-plan.md 3.4 and 4.2):
//  1. Server URL. Defaults to the origin of VITE_ENGINE_URL, else the page's own origin. The
//     login form probes `GET /tern/v1/server` (no auth) and shows the version it answers with.
//  2. Token issue, one revocable device token per sign-in:
//     a. Email and password: `POST /tern/v1/login` (form fields email, password, action,
//        device). The 200 result page carries `#tern-login-result`, base64url of
//        {type: 'tern-login', version: 1, token: 'tern_…', userId, serverId, email}.
//     b. Device token: the user pastes a `tern_…` token from `tern-backend token create`.
//  3. Identity: `GET /tern/v1/me` with `Authorization: Bearer <token>` returns
//     {userId, email, firstName, lastName, serverId, publicURL, features}. 401 means the token
//     is gone.
//  4. Engine socket: the engine WebSocket authenticates with the same token, sent in `hello`
//     (src/engine/ws.ts).
//  5. Refresh: device tokens don't expire, so there is no refresh token. We re-read
//     `/tern/v1/me` once per page load. On 401 we sign out locally and tell the user the server
//     signed them out. Network errors change nothing.
//  6. Sign out: `DELETE /tern/v1/tokens/current` (best effort), then drop the local session.
//
// Passwords are never stored. Only the token and the profile it maps to are persisted.
//
// TODO(engine): the plan has the Tern server (/tern/v1/*) and the hosted engine socket as
// separate programs. This assumes one origin serves both and accepts the same device token.
// TODO(engine): call `revalidateSession()` when the backend's `identity-refresh-needed`
// event reaches the web client (it only goes to the engine today).
import { redirect, type ParsedLocation } from '@tanstack/react-router';
import { useSyncExternalStore } from 'react';
import { defaultServerUrl, parseServerUrl } from './server-url';
import {
  AuthError,
  createHttpAuthTransport,
  createMockAuthTransport,
  type AuthTransport,
  type MeResult,
  type ServerFeatures,
} from './transport';

export interface Session {
  server: { url: string; serverId: string | null; features: ServerFeatures | null };
  user: { id: string | null; name: string; email: string };
  /** Device token for the server and the engine; null for the mock demo session. */
  token: string | null;
  method: 'demo' | 'password' | 'device-token';
}

export type Credentials =
  | { method: 'password'; serverUrl: string; email: string; password: string }
  | { method: 'device-token'; serverUrl: string; token: string };

export type SignOutReason = 'user' | 'revoked';

export interface SessionStoreOptions {
  storage: Storage;
  transport: AuthTransport;
  /** Session used when nothing is stored; null when the server requires a sign-in. */
  demo: Session | null;
  /** Device name sent with a password login; the server names the token after it. */
  device?: string;
}

const KEY = 'tern.session';
const TOKEN_PREFIX = 'tern_';

export const DEMO_SESSION: Session = {
  server: { url: '', serverId: 'mock', features: null },
  user: { id: 'mock-user', name: 'Maya Okafor', email: 'maya.okafor@gmail.com' },
  token: null,
  method: 'demo',
};

// TODO(engine): move the token to an httpOnly, Secure, SameSite=Strict cookie set by the
// server's login, so page scripts can't read it. Until the server can set one, the token sits
// in localStorage, where any script on this origin can read it.
interface Persisted {
  v: 1;
  /** null records an explicit sign-out, so the mock's demo session doesn't come back. */
  session: Session | null;
}

export function displayName(firstName: string, lastName: string, email: string): string {
  const full = `${firstName} ${lastName}`.trim();
  if (full) return full;
  return email
    .split('@')[0]
    .split(/[._-]+/)
    .filter(Boolean)
    .map((w) => w[0].toUpperCase() + w.slice(1))
    .join(' ');
}

function isSession(v: unknown): v is Session {
  const s = v as Session | null;
  return (
    !!s &&
    typeof s.server?.url === 'string' &&
    typeof s.user?.email === 'string' &&
    (s.token === null || typeof s.token === 'string')
  );
}

function sessionFrom(url: string, me: MeResult, token: string, method: Session['method']): Session {
  return {
    server: { url, serverId: me.serverId, features: me.features },
    user: {
      id: me.userId,
      name: displayName(me.firstName, me.lastName, me.email),
      email: me.email,
    },
    token,
    method,
  };
}

export function createSessionStore(opts: SessionStoreOptions) {
  const { storage, transport, demo } = opts;
  const device = (opts.device ?? 'Tern web').slice(0, 64);
  const listeners = new Set<() => void>();
  let cached: Session | null = read();
  let notice: SignOutReason | null = null;
  let revalidated: Promise<void> | null = null;

  function read(): Session | null {
    try {
      const raw = storage.getItem(KEY);
      if (raw) {
        const p = JSON.parse(raw) as Persisted;
        if (p?.v === 1) return isSession(p.session) ? p.session : null;
      }
    } catch {
      // Corrupt or unreadable storage: treat as nothing stored.
    }
    return demo;
  }

  function emit() {
    for (const l of listeners) l();
  }

  function write(session: Session | null) {
    const p: Persisted = { v: 1, session };
    storage.setItem(KEY, JSON.stringify(p));
    cached = session;
    emit();
  }

  function subscribe(cb: () => void) {
    listeners.add(cb);
    return () => {
      listeners.delete(cb);
    };
  }

  /** Picks up a sign-in or sign-out from another tab. */
  function syncFromStorage(e: Pick<StorageEvent, 'key'>) {
    if (e.key !== KEY && e.key !== null) return;
    cached = read();
    emit();
  }

  async function signIn(creds: Credentials): Promise<Session> {
    const parsed = parseServerUrl(creds.serverUrl);
    if (!parsed.ok) throw new AuthError('not-tern', parsed.reason);
    const url = parsed.url;
    const strict = transport.kind === 'http';

    let token: string;
    if (creds.method === 'password') {
      const email = creds.email.trim();
      if (!email) throw new AuthError('invalid-credentials', 'Enter your email.');
      if (strict && !creds.password) {
        throw new AuthError('invalid-credentials', 'Enter your password.');
      }
      token = (await transport.login(url, email, creds.password, device)).token;
    } else {
      token = creds.token.trim();
      if (!token) throw new AuthError('invalid-credentials', 'Paste a device token.');
      if (strict && !token.startsWith(TOKEN_PREFIX)) {
        throw new AuthError('invalid-credentials', 'Device tokens start with "tern_".');
      }
    }

    let me: MeResult;
    try {
      me = await transport.me(url, token);
    } catch (err) {
      if (err instanceof AuthError && err.code === 'unauthorized') {
        throw new AuthError('invalid-credentials', 'The server did not accept this token.');
      }
      throw err;
    }
    if (!strict && creds.method === 'password') {
      // The mock's /me doesn't know who signed in; show the address that was typed.
      me = { ...me, email: creds.email.trim(), firstName: '', lastName: '' };
    }
    const session = sessionFrom(url, me, token, creds.method);
    notice = null;
    // /tern/v1/me was just read; the per-load revalidation has nothing to add.
    revalidated = Promise.resolve();
    write(session);
    return session;
  }

  async function signOut(reason: SignOutReason = 'user') {
    const s = cached;
    notice = reason;
    revalidated = null;
    write(null);
    if (reason === 'user' && s?.token) await transport.revoke(s.server.url, s.token);
  }

  /** Re-reads the profile once per load; a 401 signs out with the 'revoked' notice. */
  function revalidate(): Promise<void> {
    const s = cached;
    if (!s?.token || transport.kind === 'mock') return Promise.resolve();
    revalidated ??= transport.me(s.server.url, s.token).then(
      (me) => {
        if (cached === s) write(sessionFrom(s.server.url, me, s.token!, s.method));
      },
      (err: unknown) => {
        if (cached === s && err instanceof AuthError && err.code === 'unauthorized') {
          void signOut('revoked');
        }
      },
    );
    return revalidated;
  }

  /** For `beforeLoad`: sends signed-out users to /login and back afterwards. */
  function requireSession({ location }: { location: Pick<ParsedLocation, 'href'> }) {
    if (!cached) {
      throw redirect({ to: '/login', search: { redirect: location.href } });
    }
    void revalidate();
  }

  /** For the login route's `beforeLoad`: signed-in users go straight to where they were headed. */
  function redirectIfSignedIn({ search }: { search: { redirect?: string } }) {
    if (cached) throw redirect({ href: search.redirect ?? '/', replace: true });
  }

  return {
    transport,
    get: () => cached,
    subscribe,
    syncFromStorage,
    signIn,
    signOut,
    revalidate,
    requireSession,
    redirectIfSignedIn,
    /** Why the last sign-out happened, for the login screen. Cleared by the next sign-in. */
    notice: () => notice,
  };
}

export type SessionStore = ReturnType<typeof createSessionStore>;

/** A login is only required when a hosted engine is configured. */
export const authRequired = !!import.meta.env.VITE_ENGINE_URL;

const store = createSessionStore({
  storage: localStorage,
  transport: authRequired ? createHttpAuthTransport() : createMockAuthTransport(),
  demo: authRequired
    ? null
    : { ...DEMO_SESSION, server: { ...DEMO_SESSION.server, url: defaultServerUrl() } },
});
window.addEventListener('storage', store.syncFromStorage);

export const getSession = store.get;
export const signIn = store.signIn;
export const signOut = store.signOut;
export const revalidateSession = store.revalidate;
export const requireSession = store.requireSession;
export const redirectIfSignedIn = store.redirectIfSignedIn;
export const getSignOutNotice = store.notice;
export const authTransport = store.transport;

/** The device token for the engine socket and server calls, or null with the mock. */
export function getSessionToken(): string | null {
  return store.get()?.token ?? null;
}

/** Calls `fn` whenever the session changes (sign-in, sign-out, another tab, revalidation). */
export const subscribeSession = store.subscribe;

export function useSession(): Session | null {
  return useSyncExternalStore(store.subscribe, store.get);
}
