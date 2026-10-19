// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The serve engine as a window's stores and packages see it: the features its hello announced,
socket requests to it, and the notifications MailsyncBridge doesn't turn into database changes.
MailsyncBridge feeds it from the window's EngineConnection; until the first hello (and in spec
mode) `features` is null and every `has()` is false.

Events: 'connected' (features) after each hello, 'gone' when the connection drops,
'engine-state' when engineState() changes,
'capabilities' (EngineCapabilities | null) when hello or capabilities.changed brings new ones,
'thread-counts' ({type, rows}) for each ThreadCounts delta, 'mail-new' (MailNewParams), and
'notification' ({method, params}) for every notification but deltas, mail.new included.
*/
import { EventEmitter } from 'events';
import { EngineCapabilities, MailNewParams, Method, ThreadCountsRow } from './engine-protocol';
import type { EngineStatus } from './supervisor-channels';

type Requester = (method: string, params: unknown) => Promise<unknown>;

export interface ThreadCountsDelta {
  type: 'persist' | 'unpersist' | string;
  rows: ThreadCountsRow[];
}

export interface EngineNotification {
  method: string;
  params: any;
}

// 'connected' while a hello answered; otherwise 'waiting' for the engine to start, respawn or
// take the connection again, or 'gave-up' once the supervisor stopped respawning it.
export type EngineState = 'connected' | 'waiting' | 'gave-up';

export class EngineEvents extends EventEmitter {
  features: Set<string> | null = null;
  // From the last hello or capabilities.changed; null while disconnected or from an engine
  // without Feature.Capabilities.
  capabilities: EngineCapabilities | null = null;
  _request: Requester | null = null;
  // From the supervisor's last Status.
  _gaveUp = false;

  has(feature: string) {
    return !!this.features && this.features.has(feature);
  }

  request<R = unknown>(method: string, params: unknown): Promise<R> {
    if (!this._request) return Promise.reject(new Error('The sync engine is not connected'));
    return this._request(method, params) as Promise<R>;
  }

  engineState(): EngineState {
    if (this.features) return 'connected';
    return this._gaveUp ? 'gave-up' : 'waiting';
  }

  // Bridge side.

  connected(features: string[], request: Requester, capabilities: EngineCapabilities = null) {
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

  mailNew(params: MailNewParams) {
    this.emit('mail-new', params);
    this.emit('notification', { method: Method.MailNew, params } as EngineNotification);
  }
}

export default new EngineEvents();
