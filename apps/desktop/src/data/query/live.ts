// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The window's live queries (`@tern/client` LiveQueries) over its engine connection, as the
window's EngineEvents exposes it: requests while connected, 'notification' for
`query.changed`, 'connected' after each hello (every live query subscribes again, one
`query.subscribe` each) and 'gone' when the connection drops.

QuerySubscription runs on one when reads go to the engine and the engine has live queries
(liveReadsMode); otherwise it keeps reading SQLite or `query` and patching from deltas.
*/
import { LiveQueries, type HelloResult, type LiveSource } from '@tern/client';
import EngineEvents, { EngineEvents as EngineEventsType } from '../../engine/client/events';
import { Feature } from '../../engine/protocol';
import EngineReads from '../database/engine-reads';
import type ModelQuery from './query';
import type { ModelQueryJSON } from './query';

export function eventsLiveSource(events: EngineEventsType): LiveSource {
  return {
    get hello() {
      return events.features
        ? ({ features: [...events.features] } as unknown as HelloResult)
        : null;
    },
    request: (method, params) => events.request(method, params),
    onNotification(listener) {
      events.on('notification', listener);
      return () => events.removeListener('notification', listener);
    },
    onHello(listener) {
      const onConnected = () => listener({} as HelloResult);
      events.on('connected', onConnected);
      return () => events.removeListener('connected', onConnected);
    },
    onDisconnect(listener) {
      events.on('gone', listener);
      return () => events.removeListener('gone', listener);
    },
  };
}

/*
How a subscription to `query` reads: 'live' over `query.subscribe`, 'reads' with one-shot
reads patched from deltas (SQLite, or `query` when the engine has no live queries), or 'wait'
until the first hello says which.
*/
export function liveReadsMode(
  query: ModelQuery<any>,
  events: EngineEventsType = EngineEvents
): 'live' | 'reads' | 'wait' {
  if (!EngineReads.usesEngine() || liveJSON(query) === null) return 'reads';
  if (!events.features) return 'wait';
  return events.has(Feature.LiveQueries) ? 'live' : 'reads';
}

// The query's JSON form, or null for one that has none.
export function liveJSON(query: ModelQuery<any>): ModelQueryJSON | null {
  try {
    return query.toJSON();
  } catch (err) {
    return null;
  }
}

export default new LiveQueries(eventsLiveSource(EngineEvents), 'desktop-');
