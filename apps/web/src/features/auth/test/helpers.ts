import { vi } from 'vitest';
import type { AuthTransport, MeResult } from '../transport';

export class MemoryStorage implements Storage {
  private map = new Map<string, string>();
  get length() {
    return this.map.size;
  }
  clear() {
    this.map.clear();
  }
  getItem(key: string) {
    return this.map.get(key) ?? null;
  }
  key(i: number) {
    return [...this.map.keys()][i] ?? null;
  }
  removeItem(key: string) {
    this.map.delete(key);
  }
  setItem(key: string, value: string) {
    this.map.set(key, value);
  }
  /** Everything stored, for asserting what never gets written. */
  dump() {
    return [...this.map.values()].join('\n');
  }
}

export const ME: MeResult = {
  userId: 'u1',
  email: 'ada@example.com',
  firstName: 'Ada',
  lastName: 'Lovelace',
  trackingNS: 'ns',
  serverId: 'srv1',
  publicURL: 'https://tern.example.com',
  features: { tracking: true, sharing: false },
};

/** An http-kind transport with spies; override with mockImplementation per test. */
export function fakeTransport() {
  return {
    kind: 'http' as const,
    server: vi.fn(async () => ({
      serverId: 'srv1',
      version: '0.1.0',
      publicURL: 'https://tern.example.com',
      signup: 'closed' as const,
      features: { tracking: true, sharing: false },
    })),
    login: vi.fn(async (_url: string, email: string) => ({
      type: 'tern-login' as const,
      version: 1 as const,
      token: 'tern_abc',
      userId: 'u1',
      serverId: 'srv1',
      email,
    })),
    me: vi.fn(async () => ME),
    revoke: vi.fn(async () => {}),
  } satisfies AuthTransport;
}
