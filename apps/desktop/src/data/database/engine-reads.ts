// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Where DatabaseStore reads in this window. With
`core.engine.reads: "engine"` a ModelQuery goes to the engine's `query` over the window's own
connection as Query.toJSON(); otherwise it runs as Query.sql() on the read-only tern.db.

Reconnect contract. A read issued while the connection is down, or dropped while it was out,
waits for the next hello up to READ_WAIT_MS and then rejects with EngineUnavailableError. Deltas
sent while a window had no connection are lost to it, so after a reconnect (and after a read that
gave up) 'resync' tells QuerySubscriptionPool to run its subscriptions again; ThreadCountsStore
reads counts.get on every hello itself. This holds on the SQLite transport too: it misses the
same deltas.

Emits 'resync' and 'state' (when unavailable() may have changed).
*/
import { EventEmitter } from 'events';
import EngineEvents, { EngineEvents as EngineEventsType } from '../../engine/client/events';
import { Method, QueryResult } from '../../engine/protocol';
import type { ModelQueryJSON } from '../query/query';

export const ENGINE_READS_KEY = 'core.engine.reads';
export const READ_WAIT_MS = 10000;
// A read whose connection dropped under it this many times fails.
const MAX_ATTEMPTS = 3;

export class EngineUnavailableError extends Error {
  constructor(message = 'The sync engine is not available') {
    super(message);
    this.name = 'EngineUnavailable';
  }
}

// An answer from the engine, as opposed to the connection failing under the request.
function isEngineError(err: any) {
  return !!err && typeof err.code === 'number';
}

export class EngineReads extends EventEmitter {
  _events: EngineEventsType;
  _flag: () => unknown;
  // The window lost its connection, or a read gave up, since the last hello.
  _missedDeltas = false;
  _waiters = new Set<() => void>();

  constructor(
    events: EngineEventsType = EngineEvents,
    flag: () => unknown = () => AppEnv.config.get(ENGINE_READS_KEY)
  ) {
    super();
    // Each EngineUnavailableState on screen listens for 'state'.
    this.setMaxListeners(100);
    this._events = events;
    this._flag = flag;
    events.on('connected', this._onConnected);
    events.on('gone', this._onGone);
    events.on('engine-state', () => this.emit('state'));
  }

  // The flag asks for the engine. Reads before the first hello wait for it.
  usesEngine() {
    return this._flag() === 'engine';
  }

  // What the window should show instead of its lists: null while reads work.
  unavailable(): 'waiting' | 'gave-up' | null {
    if (!this.usesEngine()) return null;
    const state = this._events.engineState();
    return state === 'connected' ? null : state;
  }

  // Resolves with the engine's answer.
  async query(json: ModelQueryJSON): Promise<QueryResult> {
    let deadline = Date.now() + READ_WAIT_MS;
    for (let attempt = 1; ; attempt++) {
      if (!this._events.features) await this._waitForHello(deadline);
      try {
        return await this._events.request<QueryResult>(Method.Query, json);
      } catch (err) {
        if (isEngineError(err)) throw err;
        if (attempt >= MAX_ATTEMPTS) {
          this._missedDeltas = true;
          throw new EngineUnavailableError(`The sync engine connection kept dropping (${err})`);
        }
        deadline = Date.now() + READ_WAIT_MS;
      }
    }
  }

  _waitForHello(deadline: number) {
    return new Promise<void>((resolve, reject) => {
      const done = () => {
        clearTimeout(timer);
        this._waiters.delete(done);
        resolve();
      };
      const timer = setTimeout(
        () => {
          this._waiters.delete(done);
          this._missedDeltas = true;
          reject(new EngineUnavailableError());
        },
        Math.max(0, deadline - Date.now())
      );
      this._waiters.add(done);
    });
  }

  _onConnected = () => {
    for (const resume of [...this._waiters]) resume();
    if (this._missedDeltas) {
      this._missedDeltas = false;
      this.emit('resync');
    }
  };

  _onGone = () => {
    this._missedDeltas = true;
  };
}

export default new EngineReads();
