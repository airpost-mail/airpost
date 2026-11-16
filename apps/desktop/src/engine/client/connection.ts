// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
One window's socket connection to the `serve` engine the main process supervises
(engine/supervisor/supervisor.ts): `@airpost/client`'s Session over its Unix socket transport,
pointed at the engine the supervisor's EngineStatus names. A new engine generation is a new
endpoint, connected to at once; the same engine dropping us is connected to again with backoff;
a status without a ready engine disconnects until one has. The session says hello and
subscribes on each connection; this file adds what is the desktop's.

Deltas come out as 'deltas' (delta params as JSON text, the shape EngineBridge reads), one
batch per read, without parsing the delta lines (`deltaParamsText`).

Only the main window subscribes to every account, which makes it the one connection that takes
the deltas the engine held from before anyone subscribed (Task completions of the last session,
the first ProcessState of an account that starts offline). The other windows, the hidden hot
window too, subscribe to the accounts the engine runs and follow the status. The main window
also asks engine.status on each connect and applies every account's connection state as a
ProcessState: the engine sends one only on a change, and a reload no longer restarts it.

The main window also sends its accounts' commands through it (task.queue, bodies.fetch, ...),
routed by the account states in the status the way EngineClient.send routes them: held until
account.add answered, dropped while a reset holds the account, refused when the engine doesn't
run it, and kept while there is no connection so the next one sends them. Credentials never
pass here.

A task it queues asks for `task.done` (`task.queue {notify: true}`), which the engine sends
on this connection when the task finishes; the `task.queue` answer is the task after its
local phase. Both reach TaskQueue through EngineEvents ('task-queued', 'notification').

The connection keeps the last `delta.seq` it read (FeedCursor), so the `subscribe` of the next
connection asks for the deltas it missed (`since`); 'subscribed' says whether the engine
could replay them.

A request that gets no answer rejects with EngineUnavailableError (`code` null), one the engine
refuses with RpcError (`code` a number). EngineBridge feeds the window's EngineEvents from it:
the hello's features and request() while connected, and each mail.new.
*/
import { EventEmitter } from 'events';
import { FeedCursor, Session, deltaSeqOf, unixTransport } from '@airpost/client/node';
import {
  AccountStatus,
  EngineStatusResult,
  Feature,
  HelloResult,
  MailNewParams,
  Method,
  SubscribeAnswer,
  TaskJson,
  deltaParamsText,
  requestForCommand,
} from '../protocol';
import { EngineStatus } from '../channels';

// Reconnects to the same engine after its socket dropped us back off up to this.
export const RECONNECT_MAX_DELAY_MS = 10000;
const RECONNECT_FIRST_DELAY_MS = 500;
// Commands kept while there is no connection; the oldest go first past this.
export const MAX_OFFLINE_COMMANDS = 500;

type Command = { type?: string; [key: string]: any };

// engine.status as the ProcessState deltas the main window's OnlineStatusStore reads.
export function processStateDeltas(accounts: AccountStatus[] | undefined) {
  return (accounts || []).map(({ accountId, connectionError }) =>
    JSON.stringify({
      accountId,
      modelClass: 'ProcessState',
      type: 'persist',
      modelJSONs: [{ accountId, id: accountId, connectionError: !!connectionError }],
    })
  );
}

/*
Emits 'deltas' (string[]), 'connected' (HelloResult) as hello's answer is read (before the
deltas after it), 'subscribed' ({resumed}) for the subscribe of each connection,
'mail-new' (MailNewParams) after the deltas read with it, 'notification' ({method, params})
for the other notifications but disconnect and account.state, after those, 'task-queued'
(TaskJson) with each task.queue answer, and 'disconnected'.
*/
export class EngineConnection extends EventEmitter {
  clientName: string;
  mainWindow: boolean;
  status: EngineStatus | null = null;
  readonly session: Session;
  readonly feed = new FeedCursor();

  // Commands for accounts whose account.add hasn't answered, in order.
  _pending = new Map<string, Command[]>();
  // Commands for running accounts sent while there was no connection, in order.
  _offline: Array<{ accountId: string; json: Command }> = [];
  _closed = false;
  // What the batch being read brought, emitted when it ends.
  _deltas: string[] = [];
  _mailNew: MailNewParams[] = [];
  _notes: Array<{ method: string; params: unknown }> = [];

  constructor({ clientName, mainWindow = false }: { clientName: string; mainWindow?: boolean }) {
    super();
    this.clientName = clientName;
    this.mainWindow = mainWindow;
    this.session = new Session({
      client: clientName,
      // The engine refuses a subscribe naming an account it doesn't hold; the session keeps
      // the connection and the next status tries again.
      subscribe: () => ({ accounts: mainWindow ? null : this._runningAccounts(), classes: null }),
      since: () => this.feed.since(),
      backoff: (attempt) =>
        Math.min(RECONNECT_FIRST_DELAY_MS * 2 ** attempt, RECONNECT_MAX_DELAY_MS),
      stableMs: 0,
      // A refused hello won't change for this engine: wait for the next one.
      onEnd: (end) => (end.handshakeFailed ? { next: 'wait' } : { next: 'retry' }),
      afterHandshake: () => this._afterHandshake(),
      takeRaw: (text) => {
        const params = deltaParamsText(text);
        if (params === null) return false;
        this.feed.delta(deltaSeqOf(params));
        this._deltas.push(params);
        return true;
      },
      unavailableMessage: () => 'The sync engine is not connected',
      droppedMessage: 'The sync engine connection closed',
      closedMessage: 'The sync engine connection closed',
      notConnectedMessage: (method) => `Not connected to the sync engine (${method})`,
    });
    this.session.onHello((hello) => this.emit('connected', hello));
    this.session.onSubscribed((answer) => this._onSubscribed(answer));
    this.session.onNotification(({ method, params }) => this._onNotification(method, params));
    this.session.onBatch(() => this._emitBatch());
    this.session.onDisconnect(() => this.emit('disconnected'));
  }

  // The current connection's hello; null while there is none or it hasn't answered.
  get hello(): HelloResult | null {
    return this.session.hello;
  }

  setStatus(status: EngineStatus | null) {
    if (this._closed) return;
    this.status = status;
    if (!status || !status.socketPath) {
      this.session.setEndpoint(null);
    } else {
      this.session.setEndpoint({
        key: status.generation,
        transport: unixTransport(status.socketPath),
      });
      if (!this.mainWindow) this.session.resubscribe();
    }
    this._flushPending();
  }

  // Sends one of EngineBridge's stdin-shaped commands for the account. False when the engine
  // doesn't run it, so the bridge can say the account can't sync.
  send(accountId: string, json: Command) {
    const route = this.status && this.status.accounts[accountId];
    if (!route || route.state === null || route.state === 'removing') {
      if (route && route.held) {
        console.warn(`Dropped ${json.type} for account ${accountId} while it is reset`);
        return true;
      }
      return false;
    }
    if (route.state === 'adding') {
      const pending = this._pending.get(accountId) || [];
      pending.push(json);
      this._pending.set(accountId, pending);
      return true;
    }
    if (!this.session.hasConnection) {
      this._offline.push({ accountId, json });
      if (this._offline.length > MAX_OFFLINE_COMMANDS) {
        const dropped = this._offline.shift();
        if (dropped) {
          console.warn(`Dropped ${dropped.json.type}: too many commands while disconnected`);
        }
      }
      return true;
    }
    this._sendCommand(accountId, json);
    return true;
  }

  // The accounts the engine was asked to run.
  accountIds() {
    const status = this.status;
    if (!status) return [];
    return Object.keys(status.accounts).filter((id) => status.accounts[id].state);
  }

  request<R = unknown>(method: string, params: unknown): Promise<R> {
    return this.session.request<R>(method, params);
  }

  close() {
    this._closed = true;
    this.session.close();
    this._pending.clear();
    this._offline = [];
  }

  // Private

  _sendCommand(accountId: string, json: Command) {
    console.log(`Sending to the sync engine for ${accountId}`, json);
    const request = requestForCommand(accountId, json);
    if (!request) {
      console.warn(`The sync engine has no socket method for ${json.type}`);
      return;
    }
    const queuesTask = request.method === Method.TaskQueue;
    if (queuesTask && this.session.features?.has(Feature.TaskDone)) {
      request.params = { ...(request.params as object), notify: true };
    }
    this.request(request.method, request.params).then(
      (result) => {
        if (queuesTask && result) this.emit('task-queued', result as TaskJson);
      },
      (err) => console.warn(`Sync engine refused ${request.method}: ${err.message}`)
    );
  }

  // Only a new connection's subscribe carries `since`; a later one (resubscribe) says nothing.
  _onSubscribed({ result, since, handshake }: SubscribeAnswer) {
    const resumed = this.feed.subscribed(result, since);
    if (handshake) this.emit('subscribed', { resumed });
  }

  _flushPending() {
    for (const [accountId, commands] of [...this._pending]) {
      const route = this.status && this.status.accounts[accountId];
      if (route && route.state === 'adding') continue;
      this._pending.delete(accountId);
      if (route && route.state === 'added') {
        for (const json of commands) this._sendCommand(accountId, json);
      }
    }
  }

  // The accounts the engine holds: one it doesn't hold makes it refuse the whole subscribe.
  _runningAccounts() {
    const status = this.status;
    if (!status) return [];
    return Object.keys(status.accounts)
      .filter((id) => status.accounts[id].state === 'added')
      .sort();
  }

  // Right after hello and subscribe are written: the main window's engine.status, whose
  // answer joins the deltas of its batch, then the commands kept while disconnected.
  _afterHandshake() {
    if (this.mainWindow) {
      this.session
        .request<EngineStatusResult>(
          Method.EngineStatus,
          {},
          {
            onAnswer: (result, error) => {
              if (error) return;
              const accounts = (result as EngineStatusResult | null)?.accounts;
              this._deltas.push(...processStateDeltas(accounts));
            },
          }
        )
        .catch((err) => console.warn(`engine.status failed: ${err.message}`));
    }
    const offline = this._offline;
    this._offline = [];
    for (const { accountId, json } of offline) {
      if (!this.send(accountId, json)) {
        console.warn(`Dropped ${json.type} for account ${accountId}: the engine no longer runs it`);
      }
    }
  }

  _onNotification(method: string, params: unknown) {
    if (method === Method.Delta) {
      this._deltas.push(JSON.stringify(params));
    } else if (method === Method.MailNew) {
      if (params) this._mailNew.push(params as MailNewParams);
    } else if (method !== Method.AccountState) {
      // account.state is the supervisor's; it reaches windows as Status.
      this._notes.push({ method, params });
    }
  }

  _emitBatch() {
    const deltas = this._deltas;
    const mailNew = this._mailNew;
    const notes = this._notes;
    this._deltas = [];
    this._mailNew = [];
    this._notes = [];
    if (deltas.length) this.emit('deltas', deltas);
    for (const params of mailNew) this.emit('mail-new', params);
    for (const note of notes) {
      try {
        this.emit('notification', note);
      } catch (err) {
        console.error(`A sync engine 'notification' listener failed: ${err}`);
      }
    }
  }
}
