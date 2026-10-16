// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The serve engine as a window's stores and packages see it: the features its hello announced,
socket requests to it, and the notifications MailsyncBridge doesn't turn into database changes.
MailsyncBridge feeds it from the window's EngineConnection; with per-account engines or the C++
engine nothing does, so `features` stays null and every `has()` is false.

Events: 'connected' (features) after each hello, 'gone' when the connection drops,
'thread-counts' ({type, rows}) for each ThreadCounts delta, 'mail-new' (MailNewParams).
*/
import { EventEmitter } from 'events';
import type { MailNewParams, ThreadCountsRow } from './engine-protocol';

type Requester = (method: string, params: unknown) => Promise<unknown>;

export interface ThreadCountsDelta {
  type: 'persist' | 'unpersist' | string;
  rows: ThreadCountsRow[];
}

export class EngineEvents extends EventEmitter {
  features: Set<string> | null = null;
  _request: Requester | null = null;

  has(feature: string) {
    return !!this.features && this.features.has(feature);
  }

  request<R = unknown>(method: string, params: unknown): Promise<R> {
    if (!this._request) return Promise.reject(new Error('The sync engine is not connected'));
    return this._request(method, params) as Promise<R>;
  }

  // Bridge side.

  connected(features: string[], request: Requester) {
    this.features = new Set(features);
    this._request = request;
    this.emit('connected', this.features);
  }

  gone() {
    if (!this.features && !this._request) return;
    this.features = null;
    this._request = null;
    this.emit('gone');
  }

  threadCounts(delta: ThreadCountsDelta) {
    this.emit('thread-counts', delta);
  }

  mailNew(params: MailNewParams) {
    this.emit('mail-new', params);
  }
}

export default new EngineEvents();
