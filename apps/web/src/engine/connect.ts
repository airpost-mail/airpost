import { RENDERED_CLASSES } from './cache';
import type { EngineClient } from './client';
import { MockEngine } from './mock/engine';
import type { HelloResult } from './rpc';
import type { Delta, TaskJSON } from './types';
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
    return new WsEngineClient({ url: gatewayWsUrl(), classes: [...RENDERED_CLASSES], ...opts });
  }
  const test = import.meta.env.MODE === 'test';
  return new MockEngine({ incomingEveryMs: test ? null : 90_000, aiDemo: !test });
}

/** Resolves when a Task delta reports the task complete or cancelled. */
export function waitForTask(
  client: EngineClient,
  taskId: string,
  timeoutMs = 60_000,
): Promise<TaskJSON> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      off();
      reject(new Error(`task ${taskId} did not finish in ${timeoutMs} ms`));
    }, timeoutMs);
    const off = client.onNotification((n) => {
      if (n.method !== 'delta' || n.params.modelClass !== 'Task') return;
      const task = (n.params as Delta<'Task'>).modelJSONs.find((t) => t.id === taskId);
      if (task && (task.status === 'complete' || task.status === 'cancelled')) {
        clearTimeout(timer);
        off();
        resolve(task);
      }
    });
  });
}
