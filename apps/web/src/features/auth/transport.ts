// The gateway's pairing and session routes. The device
// token lives in an httpOnly cookie the gateway sets; this code never sees it.
import { GATEWAY_BASE } from '@/engine';

export interface PairedDevice {
  id: string;
  name: string;
  /** `admin` lets this browser call the engine's local-only methods. */
  scopes: string[];
  /** ms since epoch. */
  pairedAt: number;
}

export interface EngineHealth {
  state: 'up' | 'down';
  protocol?: number;
  engine?: string;
}

/** `GET /gateway/v1/session`. */
export interface GatewaySession {
  device: PairedDevice;
  engine: EngineHealth;
  gateway: { version: string };
}

export type AuthErrorCode = 'unreachable' | 'invalid-code' | 'unpaired' | 'rate-limited' | 'server';

export class AuthError extends Error {
  readonly code: AuthErrorCode;
  /** From `Retry-After` on a 429. */
  readonly retryAfterSecs?: number;
  constructor(code: AuthErrorCode, message: string, retryAfterSecs?: number) {
    super(message);
    this.name = 'AuthError';
    this.code = code;
    this.retryAfterSecs = retryAfterSecs;
  }
}

export interface GatewayTransport {
  readonly kind: 'mock' | 'http';
  /** Throws `AuthError('unpaired')` when the gateway doesn't know this browser. */
  session(): Promise<GatewaySession>;
  /** `code` as `normalizePairingCode` returns it. */
  pair(code: string, deviceName: string): Promise<PairedDevice>;
  /** Best effort: the caller forgets the session whatever happens. */
  unpair(): Promise<void>;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

const MOCK_KEY = 'airpost.mock-pairing';

interface MockPairing {
  paired: boolean;
  name: string;
}

/** Pairs with any well-formed code; used with the in-browser mock engine. */
export function createMockGatewayTransport(
  storage: Storage = localStorage,
  latencyMs = 250,
): GatewayTransport {
  const read = (): MockPairing => {
    try {
      const v = JSON.parse(storage.getItem(MOCK_KEY) ?? 'null') as MockPairing | null;
      if (v && typeof v.paired === 'boolean') return v;
    } catch {
      // fall through to the default
    }
    return { paired: true, name: 'Demo browser' };
  };
  const write = (v: MockPairing) => storage.setItem(MOCK_KEY, JSON.stringify(v));
  const device = (name: string): PairedDevice => ({
    id: 'mock-device',
    name,
    scopes: ['admin'],
    pairedAt: 0,
  });
  return {
    kind: 'mock',
    async session() {
      await sleep(latencyMs);
      const p = read();
      if (!p.paired) throw new AuthError('unpaired', 'This browser is not paired.');
      return {
        device: device(p.name),
        engine: { state: 'up', protocol: 1, engine: '0.0.0-mock' },
        gateway: { version: 'mock' },
      };
    },
    async pair(_code, deviceName) {
      await sleep(latencyMs);
      write({ paired: true, name: deviceName });
      return device(deviceName);
    },
    async unpair() {
      write({ paired: false, name: read().name });
    },
  };
}

async function request(path: string, init?: RequestInit): Promise<Response> {
  try {
    return await fetch(`${GATEWAY_BASE}${path}`, {
      credentials: 'same-origin',
      cache: 'no-store',
      ...init,
      headers: { Accept: 'application/json', ...init?.headers },
    });
  } catch {
    throw new AuthError('unreachable', "Couldn't reach the server.");
  }
}

function retryAfter(res: Response): number | undefined {
  const v = Number(res.headers.get('Retry-After'));
  return Number.isFinite(v) && v > 0 ? v : undefined;
}

export function createHttpGatewayTransport(): GatewayTransport {
  return {
    kind: 'http',
    async session() {
      const res = await request('/session');
      if (res.status === 401) throw new AuthError('unpaired', 'This browser is not paired.');
      if (!res.ok) throw new AuthError('server', `The server answered ${res.status}.`);
      const body = (await res.json().catch(() => null)) as GatewaySession | null;
      if (!body?.device?.id) throw new AuthError('server', 'The server sent an unexpected reply.');
      return body;
    },
    async pair(code, deviceName) {
      const res = await request('/pair', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code, deviceName }),
      });
      if (res.status === 400) throw new AuthError('invalid-code', "That isn't a pairing code.");
      if (res.status === 401) {
        throw new AuthError(
          'invalid-code',
          'That code is wrong, used or expired. Run airpost-gateway pair for a new one.',
        );
      }
      if (res.status === 429) {
        const secs = retryAfter(res);
        throw new AuthError(
          'rate-limited',
          secs ? `Too many attempts. Try again in ${secs} s.` : 'Too many attempts. Wait a minute.',
          secs,
        );
      }
      if (!res.ok) throw new AuthError('server', `The server answered ${res.status}.`);
      const body = (await res.json().catch(() => null)) as { device?: PairedDevice } | null;
      if (!body?.device?.id) throw new AuthError('server', 'The server sent an unexpected reply.');
      return body.device;
    },
    async unpair() {
      await request('/unpair', { method: 'POST' }).catch(() => undefined);
    },
  };
}
