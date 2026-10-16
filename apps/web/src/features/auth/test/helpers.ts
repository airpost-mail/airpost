import { vi } from 'vitest';
import type { GatewaySession, GatewayTransport, PairedDevice } from '../transport';
import { AuthError } from '../transport';

export const DEVICE: PairedDevice = {
  id: 'dev1',
  name: 'Firefox on macOS',
  scopes: [],
  pairedAt: 1_792_108_800_000,
};

export const SESSION: GatewaySession = {
  device: DEVICE,
  engine: { state: 'up', protocol: 1, engine: '0.1.0' },
  gateway: { version: '0.1.0' },
};

/** An http-kind transport with spies; override with mockImplementation per test. */
export function fakeTransport() {
  return {
    kind: 'http' as const,
    session: vi.fn(async (): Promise<GatewaySession> => SESSION),
    pair: vi.fn(async (_code: string, name: string) => ({ ...DEVICE, name })),
    unpair: vi.fn(async () => {}),
  } satisfies GatewayTransport;
}

export const unpaired = () => new AuthError('unpaired', 'This browser is not paired.');
export const unreachable = () => new AuthError('unreachable', "Couldn't reach the server.");
