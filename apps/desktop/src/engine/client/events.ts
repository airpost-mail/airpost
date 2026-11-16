// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
The serve engine as a window's stores and packages see it: the features its hello announced,
socket requests to it, and the notifications EngineBridge doesn't turn into database changes.
EngineBridge feeds it from the window's EngineConnection; until the first hello (and in spec
mode) `features` is null and every `has()` is false.

Events: 'connected' (features) after each hello, 'gone' when the connection drops,
'engine-state' when engineState() changes,
'capabilities' (EngineCapabilities | null) when hello or capabilities.changed brings new ones,
'thread-counts' ({type, rows}) for each ThreadCounts delta, 'mail-new' (MailNewParams),
'notification' ({method, params}) for every notification but deltas, mail.new included,
'subscribed' ({resumed}) after each connection's subscribe (`resumed` null without `since`,
else whether the engine replays the deltas missed), and 'task-queued' (TaskJson, the task
after its local phase) for each task.queue this window sent.
*/
import { EventEmitter } from 'events';
import { localized } from '../../intl/localize';
import {
  EngineCapabilities,
  EngineUnavailableError,
  MailNewParams,
  Method,
  TaskJson,
  ThreadCountsRow,
} from '../protocol';
import type { EngineStatus } from '../channels';

type Requester = (method: string, params: unknown) => Promise<unknown>;

export interface ThreadCountsDelta {
  type: 'persist' | 'unpersist' | string;
  rows: ThreadCountsRow[];
}

export interface EngineNotification {
  method: string;
  params: any;
}

// How long a call waits for the connection's hello before giving up.
export const CONNECT_WAIT_MS = 20000;

// 'connected' while a hello answered; otherwise 'waiting' for the engine to start, respawn or
// take the connection again, or 'gave-up' once the supervisor stopped respawning it.
export type EngineState = 'connected' | 'waiting' | 'gave-up';

export class EngineEvents extends EventEmitter {
  features: Set<string> | null = null;
  // From the last hello or capabilities.changed; null while disconnected.
  capabilities: EngineCapabilities | null = null;
  _request: Requester | null = null;
  // From the supervisor's last Status.
  _gaveUp = false;

  has(feature: string) {
    return !!this.features && this.features.has(feature);
  }

  // Resolves once a hello answered: at once while connected, else on the next one, or
  // rejects after `timeoutMs`.
  whenConnected(timeoutMs = CONNECT_WAIT_MS): Promise<void> {
    if (this.features) return Promise.resolve();
    return new Promise<void>((resolve, reject) => {
      const onConnected = () => {
        clearTimeout(timer);
        resolve();
      };
      const timer = setTimeout(() => {
        this.removeListener('connected', onConnected);
        reject(new Error(localized('The sync engine is not running. Restart Airpost and try again.')));
      }, timeoutMs);
      this.once('connected', onConnected);
    });
  }

  request<R = unknown>(method: string, params: unknown): Promise<R> {
    if (!this._request) {
      return Promise.reject(new EngineUnavailableError('The sync engine is not connected'));
    }
    return this._request(method, params) as Promise<R>;
  }

  engineState(): EngineState {
    if (this.features) return 'connected';
    return this._gaveUp ? 'gave-up' : 'waiting';
  }

  // Bridge side.

  connected(
    features: string[],
    request: Requester,
    capabilities: EngineCapabilities | null = null
  ) {
    this.features = new Set(features);
    this._request = request;
    this.capabilities = capabilities || null;
    this.emit('connected', this.features);
    this.emit('capabilities', this.capabilities);
    this.emit('engine-state', this.engineState());
  }

  gone() {
    if (!this.features && !this._request) return;
    this.features = null;
    this._request = null;
    const hadCapabilities = !!this.capabilities;
    this.capabilities = null;
    this.emit('gone');
    if (hadCapabilities) this.emit('capabilities', null);
    this.emit('engine-state', this.engineState());
  }

  supervisorStatus(status: EngineStatus | null) {
    const gaveUp = !!(status && status.gaveUp);
    if (gaveUp === this._gaveUp) return;
    const before = this.engineState();
    this._gaveUp = gaveUp;
    if (this.engineState() !== before) this.emit('engine-state', this.engineState());
  }

  notification(method: string, params: any) {
    if (method === Method.CapabilitiesChanged && params && typeof params === 'object') {
      this.capabilities = params as EngineCapabilities;
      this.emit('capabilities', this.capabilities);
    }
    this.emit('notification', { method, params } as EngineNotification);
  }

  threadCounts(delta: ThreadCountsDelta) {
    this.emit('thread-counts', delta);
  }

  subscribed(answer: { resumed: boolean | null }) {
    this.emit('subscribed', answer);
  }

  taskQueued(task: TaskJson) {
    this.emit('task-queued', task);
  }

  mailNew(params: MailNewParams) {
    this.emit('mail-new', params);
    this.emit('notification', { method: Method.MailNew, params } as EngineNotification);
  }
}

export default new EngineEvents();
