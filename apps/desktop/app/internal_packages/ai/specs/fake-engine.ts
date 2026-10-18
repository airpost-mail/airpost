// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { Engine, KeyManager } from 'mailspring-exports';

export const ACCOUNT_ID = 'test-account-server-id';
export const SYNTHETIC_KEY = 'synthetic-ai-test-key-0000';

export interface Sent {
  method: string;
  params: any;
}

type Answer = (params: any, emitDelta: (text: string) => void) => unknown | Promise<unknown>;

/**
 * Engine.request mocked: `answers` by method; each streamed answer can send `ai.delta`
 * notifications for the call's streamId before it resolves, as the engine does.
 */
export function fakeEngine(answers: { [method: string]: Answer }, { hasAi = true } = {}) {
  const sent: Sent[] = [];
  spyOn(Engine, 'has').andCallFake((feature: string) => hasAi && feature === 'ai');
  spyOn(Engine, 'request').andCallFake(async (method: string, params: any) => {
    sent.push({ method, params });
    const answer = answers[method];
    if (!answer) return {};
    const emitDelta = (text: string) =>
      Engine.events.notification('ai.delta', { streamId: params.streamId, text });
    return answer(params, emitDelta);
  });
  return sent;
}

/** The keyset as a plain object; no keychain item is read or written. */
export function fakeKeyset(initial: { [name: string]: string } = {}) {
  const keys = { ...initial };
  spyOn(KeyManager, 'getPassword').andCallFake(async (name: string) => keys[name]);
  spyOn(KeyManager, 'replacePassword').andCallFake(async (name: string, value: string) => {
    keys[name] = value;
  });
  spyOn(KeyManager, 'deletePassword').andCallFake(async (name: string) => {
    delete keys[name];
  });
  return keys;
}

export function aiError(key: string, message = key, extra: object = {}) {
  return Object.assign(new Error('AI request failed'), {
    code: -32011,
    data: { key, message, retryable: false, ...extra },
  });
}

/** Anthropic for both features, the test account opted in. */
export function setUpAi(overrides: object = {}) {
  AppEnv.config.set('core.ai', {
    providers: [{ id: 'anthropic', kind: 'anthropic', model: 'claude-test' }],
    features: {
      summary: { enabled: true, provider: 'anthropic' },
      reply: { enabled: true, provider: 'anthropic' },
    },
    accounts: [ACCOUNT_ID],
    includeAddresses: false,
    ...overrides,
  });
}

/** Lets pending promise callbacks run until `done` holds (or a bounded number of turns). */
export async function settle(done: () => boolean = () => false, turns = 50) {
  for (let i = 0; i < turns && !done(); i++) await Promise.resolve();
}
