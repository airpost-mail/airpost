import { FEED_CLASSES } from './feed';
import type { EngineClient } from './client';
import { MockEngine } from './mock/engine';
import type { HelloResult } from './rpc';
import { engineMode, gatewayWsUrl } from './mode';
import { WsEngineClient, type HealthOutcome } from './ws';

const connections = new WeakMap<EngineClient, Promise<HelloResult>>();

/** Connects once per client; every query and loader awaits this before calling. */
export function ensureConnected(client: EngineClient): Promise<HelloResult> {
  let p = connections.get(client);
  if (!p || client.state === 'closed') {
    p = client.connect();
    p.catch(() => connections.delete(client));
    connections.set(client, p);
  }
  return p;
}

export interface CreateEngineOptions {
  /** `GET /gateway/v1/session` before each connect (gateway mode). */
  health?: () => Promise<HealthOutcome>;
  /** The gateway no longer knows this browser. */
  onUnpaired?: () => void;
}

/** The mock, or the gateway that served this page, per `engineMode()`. */
export function createEngineClient(opts: CreateEngineOptions = {}): EngineClient {
  if (engineMode() === 'gateway') {
    return new WsEngineClient({ url: gatewayWsUrl(), classes: [...FEED_CLASSES], ...opts });
  }
  const test = import.meta.env.MODE === 'test';
  return new MockEngine({ incomingEveryMs: test ? null : 90_000, aiDemo: !test });
}
