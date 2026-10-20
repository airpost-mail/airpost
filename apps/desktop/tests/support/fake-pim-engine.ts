// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The window's engine connection for the calendar, events and contacts specs: `events.*` and
`contacts.*` answered from `answers` by method, every request recorded in `calls`. What the engine
does with them (the ICS and vCards, recurrence, ownership) is crates/ms-pim's tests and the
desktop corpus they replay.
*/
import EngineEvents from '../../src/engine/client/events';
import { EngineConnectionError } from '../../src/engine/client/connection';
import { ErrorCode, Feature } from '../../src/engine/protocol';

export interface PimCall {
  method: string;
  params: any;
}

export function fakePimEngine() {
  const calls: PimCall[] = [];
  const answers: { [method: string]: (params: any) => unknown } = {};
  EngineEvents.connected([Feature.CalendarEditing, Feature.ContactEditing], (method, params) => {
    calls.push({ method, params });
    const answer = answers[method];
    if (!answer) return Promise.reject(new Error(`no answer for ${method}`));
    try {
      return Promise.resolve(answer(params));
    } catch (err) {
      return Promise.reject(err);
    }
  });
  jasmine.getEnv().currentSpec.after(() => EngineEvents.gone());
  return {
    calls,
    answers,
    callsTo: (method: string) => calls.filter((c) => c.method === method).map((c) => c.params),
  };
}

/** An engine refusal, as `events.*` sends one: INVALID_PARAMS with `data.key`. */
export function refusal(key: string) {
  return new EngineConnectionError(`refused: ${key}`, {
    code: ErrorCode.InvalidParams,
    data: { key },
  });
}

// The spec runner fakes timers; the engine's answers settle in microtasks.
export async function flush() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}
