// Talks to the Tern server's login and identity routes (docs/backend-plan.md 3.4). Field
// names are the server's wire names (ms_wire), so they stay camelCase as sent.

export interface ServerFeatures {
  tracking: boolean;
  sharing: boolean;
}

/** `GET /tern/v1/server` (no auth). */
export interface ServerInfo {
  serverId: string;
  version: string;
  publicURL: string;
  signup: 'open' | 'closed';
  features: ServerFeatures;
}

/** The JSON inside `#tern-login-result` after `POST /tern/v1/login`. */
export interface LoginResult {
  type: 'tern-login';
  version: 1;
  token: string;
  userId: string;
  serverId: string;
  email: string;
}

/** `GET /tern/v1/me` (auth). */
export interface MeResult {
  userId: string;
  email: string;
  firstName: string;
  lastName: string;
  trackingNS: string;
  serverId: string;
  publicURL: string;
  features: ServerFeatures;
}

export type AuthErrorCode =
  'unreachable' | 'not-tern' | 'invalid-credentials' | 'unauthorized' | 'rate-limited' | 'server';

export class AuthError extends Error {
  readonly code: AuthErrorCode;
  constructor(code: AuthErrorCode, message: string) {
    super(message);
    this.name = 'AuthError';
    this.code = code;
  }
}

export interface AuthTransport {
  readonly kind: 'mock' | 'http';
  server(baseUrl: string): Promise<ServerInfo>;
  login(baseUrl: string, email: string, password: string, device: string): Promise<LoginResult>;
  me(baseUrl: string, token: string): Promise<MeResult>;
  /** Revokes the calling token. Best effort: callers sign out locally whatever happens. */
  revoke(baseUrl: string, token: string): Promise<void>;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Accepts any credentials; used with the in-browser mock engine. */
export function createMockAuthTransport(latencyMs = 250): AuthTransport {
  const info: ServerInfo = {
    serverId: 'mock',
    version: 'mock',
    publicURL: '',
    signup: 'closed',
    features: { tracking: false, sharing: false },
  };
  return {
    kind: 'mock',
    async server(baseUrl) {
      await sleep(latencyMs);
      return { ...info, publicURL: baseUrl };
    },
    async login(_baseUrl, email) {
      await sleep(latencyMs);
      return {
        type: 'tern-login',
        version: 1,
        token: `tern_mock_${crypto.randomUUID()}`,
        userId: 'mock-user',
        serverId: info.serverId,
        email,
      };
    },
    async me(baseUrl) {
      await sleep(latencyMs);
      return {
        userId: 'mock-user',
        email: 'maya.okafor@gmail.com',
        firstName: 'Maya',
        lastName: 'Okafor',
        trackingNS: 'mock',
        serverId: info.serverId,
        publicURL: baseUrl,
        features: info.features,
      };
    },
    async revoke() {},
  };
}

function authHeader(token: string) {
  return { Authorization: `Bearer ${token}` };
}

async function request(url: string, init?: RequestInit): Promise<Response> {
  try {
    // credentials: 'omit' until the server sets a session cookie; the token goes in a header.
    return await fetch(url, { credentials: 'omit', cache: 'no-store', ...init });
  } catch {
    // TODO(engine): a cross-origin web client needs the server's CORS layer to allow the web
    // origin (backend-plan 3.4 only echoes file:// and null). Same-origin hosting avoids it.
    throw new AuthError('unreachable', "Couldn't reach the server.");
  }
}

function statusError(res: Response): AuthError {
  if (res.status === 401 || res.status === 403) {
    return new AuthError('unauthorized', 'The server did not accept this token.');
  }
  if (res.status === 429) return new AuthError('rate-limited', 'Too many attempts. Wait a minute.');
  return new AuthError('server', `The server answered ${res.status}.`);
}

function decodeBase64Url(s: string): string {
  const b64 = s.replace(/-/g, '+').replace(/_/g, '/');
  return atob(b64 + '='.repeat((4 - (b64.length % 4)) % 4));
}

/** Reads the login result the server embeds in its result page. */
export function parseLoginResultPage(html: string): LoginResult | null {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const encoded = doc.getElementById('tern-login-result')?.textContent?.trim();
  if (!encoded) return null;
  try {
    const result = JSON.parse(decodeBase64Url(encoded)) as Partial<LoginResult>;
    if (result.type !== 'tern-login' || result.version !== 1 || !result.token) return null;
    return result as LoginResult;
  } catch {
    return null;
  }
}

export function createHttpAuthTransport(): AuthTransport {
  return {
    kind: 'http',
    async server(baseUrl) {
      const res = await request(`${baseUrl}/tern/v1/server`);
      if (!res.ok) throw new AuthError('not-tern', 'No Tern server answers at this address.');
      const body = (await res.json().catch(() => null)) as ServerInfo | null;
      if (!body?.serverId)
        throw new AuthError('not-tern', 'No Tern server answers at this address.');
      return body;
    },
    async login(baseUrl, email, password, device) {
      // TODO(engine): the server's login is an HTML form whose result page embeds the token
      // (built for the desktop's login window). A web client wants a JSON variant, e.g.
      // `POST /tern/v1/login` with `Accept: application/json` answering the LoginResult or 401.
      // Until then we post the form and read `#tern-login-result` from the page.
      const res = await request(`${baseUrl}/tern/v1/login`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({ email, password, action: 'login', device }),
      });
      if (res.status === 429) throw statusError(res);
      if (res.status >= 500) throw statusError(res);
      const result = parseLoginResultPage(await res.text());
      if (!result) throw new AuthError('invalid-credentials', 'Wrong email or password.');
      return result;
    },
    async me(baseUrl, token) {
      const res = await request(`${baseUrl}/tern/v1/me`, { headers: authHeader(token) });
      if (!res.ok) throw statusError(res);
      return (await res.json()) as MeResult;
    },
    async revoke(baseUrl, token) {
      await request(`${baseUrl}/tern/v1/tokens/current`, {
        method: 'DELETE',
        headers: authHeader(token),
      }).catch(() => undefined);
    },
  };
}
