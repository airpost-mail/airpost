// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
One window's socket connection to the `serve` engine the main process supervises
(engine/supervisor/supervisor.ts). It follows the supervisor's EngineStatus: it connects when the
status names a ready engine, says hello, subscribes to the deltas and emits them as 'deltas'
(delta params as JSON text, the shape EngineBridge reads), and connects again to the next
engine after a respawn.

Only the main window subscribes to every account, which makes it the one connection that takes
the deltas the engine held from before anyone subscribed (Task completions of the last session,
the first ProcessState of an account that starts offline). The other windows, the hidden hot
window too, subscribe to the accounts the engine runs and follow the status. The main window
also asks engine.status on each connect and applies every account's connection state as a
ProcessState: the engine sends one only on a change, and a reload no longer restarts it.

The main window also sends its accounts' commands through it (task.queue, bodies.fetch, ...),
routed by the account states in the status the way EngineClient.send routes them: held until
account.add answered, dropped while a reset holds the account, refused when the engine doesn't
run it, and kept while the socket is down so the reconnect sends them. Credentials never pass
here.

EngineBridge feeds the window's EngineEvents from it: the hello's features and request() while
connected, and each mail.new.
*/
import { EventEmitter } from 'events';
import net from 'net';
import {
  AccountStatus,
  DisconnectParams,
  EngineStatusResult,
  HelloParams,
  HelloResult,
  MailNewParams,
  MAX_LINE_BYTES,
  Method,
  PROTOCOL_VERSION,
  RequestId,
  RpcError,
  RpcResponse,
  SubscribeParams,
  deltaParamsText,
  requestForCommand,
} from '../protocol';
import { EngineStatus } from '../channels';

// Reconnects to the same engine after its socket dropped us back off up to this.
export const RECONNECT_MAX_DELAY_MS = 10000;
const RECONNECT_FIRST_DELAY_MS = 500;
// Commands kept while the socket is down; the oldest go first past this.
export const MAX_OFFLINE_COMMANDS = 500;

export class EngineConnectionError extends Error {
  code: number | null;
  data: unknown;

  constructor(
    message: string,
    { code = null, data }: { code?: number | null; data?: unknown } = {}
  ) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

type Command = { type?: string; [key: string]: any };

type PendingRequest = {
  method: string;
  resolve: (r: any) => void;
  reject: (e: Error) => void;
  // Runs as the response is read, so what it adds to `deltas` keeps its place in the stream.
  inline?: (result: any, deltas: string[]) => void;
};

// engine.status as the ProcessState deltas the main window's OnlineStatusStore reads.
export function processStateDeltas(accounts: AccountStatus[]) {
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
deltas after it), 'mail-new' (MailNewParams) after the deltas read with it, 'notification'
({method, params}) for the other notifications but disconnect and account.state, after those,
and 'disconnected'.
*/
export class EngineConnection extends EventEmitter {
  clientName: string;
  mainWindow: boolean;
  status: EngineStatus | null = null;
  hello: HelloResult | null = null;

  _socket: net.Socket | null = null;
  // The engine generation `_socket` belongs to.
  _generation: number | null = null;
  _requests = new Map<RequestId, PendingRequest>();
  _nextId = 1;
  // Commands for accounts whose account.add hasn't answered, in order.
  _pending = new Map<string, Command[]>();
  // Commands for running accounts sent while the socket was down, in order.
  _offline: Array<{ accountId: string; json: Command }> = [];
  // A non-main window's last subscribed account list, joined; null when it needs one.
  _subscribedAccounts: string | null = null;
  _reconnectTimer: ReturnType<typeof setTimeout> | undefined = undefined;
  _reconnectDelay = RECONNECT_FIRST_DELAY_MS;
  _closed = false;

  constructor({ clientName, mainWindow = false }: { clientName: string; mainWindow?: boolean }) {
    super();
    this.clientName = clientName;
    this.mainWindow = mainWindow;
  }

  setStatus(status: EngineStatus | null) {
    if (this._closed) return;
    this.status = status;
    if (!status || !status.socketPath) {
      this._disconnect();
    } else if (status.generation !== this._generation) {
      this._disconnect();
      this._reconnectDelay = RECONNECT_FIRST_DELAY_MS;
      this._connect(status.socketPath, status.generation);
    } else {
      this._followAccounts();
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
    if (!this._socket) {
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

  request<R = unknown>(
    method: string,
    params: unknown,
    inline?: PendingRequest['inline']
  ): Promise<R> {
    const socket = this._socket;
    if (!socket) {
      return Promise.reject(
        new EngineConnectionError(`Not connected to the sync engine (${method})`)
      );
    }
    const id = this._nextId++;
    return new Promise<R>((resolve, reject) => {
      this._requests.set(id, { method, resolve, reject, inline });
      socket.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`, 'utf-8');
    });
  }

  close() {
    this._closed = true;
    this._disconnect();
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
    this.request(request.method, request.params).catch((err) =>
      console.warn(`Sync engine refused ${request.method}: ${err.message}`)
    );
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

  _subscribe() {
    if (this.mainWindow) {
      this.request(Method.Subscribe, { accounts: null, classes: null } as SubscribeParams).catch(
        () => {}
      );
      return;
    }
    const socket = this._socket;
    const accounts = this._runningAccounts();
    const key = accounts.join(',');
    this._subscribedAccounts = key;
    this.request(Method.Subscribe, { accounts, classes: null } as SubscribeParams).catch((err) => {
      // An account went away meanwhile. The engine keeps the last subscription; the next
      // status tries again.
      if (this._socket === socket && this._subscribedAccounts === key) {
        this._subscribedAccounts = null;
      }
      if (err.code !== null) console.warn(`The sync engine refused subscribe: ${err.message}`);
    });
  }

  _followAccounts() {
    if (this.mainWindow || !this._socket) return;
    if (this._runningAccounts().join(',') !== this._subscribedAccounts) this._subscribe();
  }

  _connect(socketPath: string, generation: number | null) {
    clearTimeout(this._reconnectTimer);
    this._reconnectTimer = undefined;
    const socket = net.createConnection(socketPath);
    socket.setEncoding('utf8');
    this._socket = socket;
    this._generation = generation;

    // Writes before 'connect' are buffered by the socket, and the engine reads in order, so
    // hello is handled first. 'connected' goes out as its answer is read, so the features are
    // known before the deltas that follow it are applied.
    this.request<HelloResult>(
      Method.Hello,
      { protocol: PROTOCOL_VERSION, client: this.clientName } as HelloParams,
      (hello) => {
        if (this._socket !== socket) return;
        this.hello = hello;
        this._reconnectDelay = RECONNECT_FIRST_DELAY_MS;
        try {
          this.emit('connected', hello);
        } catch (err) {
          console.error(`A sync engine 'connected' listener failed: ${err}`);
        }
      }
    ).catch((err) => {
      if (this._socket !== socket) return;
      console.warn(`The sync engine refused hello: ${err.message}`);
      this._disconnect();
    });
    this._subscribe();
    if (this.mainWindow) {
      this.request<EngineStatusResult>(Method.EngineStatus, {}, (result, deltas) => {
        deltas.push(...processStateDeltas(result && result.accounts));
      }).catch((err) => console.warn(`engine.status failed: ${err.message}`));
    }
    const offline = this._offline;
    this._offline = [];
    for (const { accountId, json } of offline) {
      if (!this.send(accountId, json)) {
        console.warn(`Dropped ${json.type} for account ${accountId}: the engine no longer runs it`);
      }
    }

    let inBuffer = '';
    socket.on('data', (chunk: string) => {
      inBuffer += chunk;
      if (chunk.indexOf('\n') === -1) {
        if (inBuffer.length > MAX_LINE_BYTES) {
          console.warn('The sync engine sent a line over the size limit');
          this._dropped(socket);
        }
        return;
      }
      const lines = inBuffer.split('\n');
      inBuffer = lines.pop() ?? '';
      const deltas: string[] = [];
      const mailNew: MailNewParams[] = [];
      const notes: Array<{ method: string; params: unknown }> = [];
      let drop = false;
      for (const line of lines) {
        drop = this._onLine(line, deltas, mailNew, notes) || drop;
      }
      if (deltas.length) this.emit('deltas', deltas);
      for (const params of mailNew) this.emit('mail-new', params);
      for (const note of notes) {
        try {
          this.emit('notification', note);
        } catch (err) {
          console.error(`A sync engine 'notification' listener failed: ${err}`);
        }
      }
      if (drop) this._dropped(socket);
    });
    socket.on('error', (err: Error) => {
      console.warn(`Sync engine socket failed: ${err.message}`);
      this._dropped(socket);
    });
    socket.on('close', () => this._dropped(socket));
  }

  // Adds a delta to `deltas`, a mail.new to `mailNew` or another notification to `notes`,
  // settles a request, or returns true when the engine dropped us.
  _onLine(
    line: string,
    deltas: string[],
    mailNew: MailNewParams[] = [],
    notes: Array<{ method: string; params: unknown }> = []
  ): boolean {
    if (!line) return false;
    const params = deltaParamsText(line);
    if (params !== null) {
      deltas.push(params);
      return false;
    }
    let msg = null;
    try {
      msg = JSON.parse(line);
    } catch (err) {
      console.warn('The sync engine sent a non-JSON line');
      return false;
    }
    if (msg.method === Method.Delta) {
      deltas.push(JSON.stringify(msg.params));
      return false;
    }
    if (msg.method === Method.MailNew) {
      if (msg.params) mailNew.push(msg.params as MailNewParams);
      return false;
    }
    if (msg.method === Method.Disconnect) {
      const { reason } = (msg.params || {}) as DisconnectParams;
      console.warn(`Sync engine dropped the connection: ${reason}`);
      return true;
    }
    // account.state is the supervisor's; it reaches windows as Status.
    if (typeof msg.method === 'string' && msg.method !== Method.AccountState) {
      if (msg.id === undefined) notes.push({ method: msg.method, params: msg.params });
      return false;
    }
    if (msg.method === undefined && msg.id !== undefined && msg.id !== null) {
      const id: RequestId = msg.id;
      const response = msg as RpcResponse;
      const pending = this._requests.get(id);
      if (!pending) return false;
      this._requests.delete(id);
      if (response.error) {
        const { message, code, data } = response.error;
        pending.reject(new EngineConnectionError(message, { code, data }));
      } else {
        if (pending.inline) pending.inline(response.result, deltas);
        pending.resolve(response.result);
      }
    }
    return false;
  }

  // The engine went away or dropped us. A ready engine of the same generation is connected
  // again after a delay; a new one when Status names it.
  _dropped(socket: net.Socket) {
    if (this._socket !== socket) return;
    const generation = this._generation;
    this._disconnect();
    const status = this.status;
    if (this._closed || !status || !status.socketPath || status.generation !== generation) return;
    const delay = this._reconnectDelay;
    this._reconnectDelay = Math.min(this._reconnectDelay * 2, RECONNECT_MAX_DELAY_MS);
    this._reconnectTimer = setTimeout(() => {
      this._reconnectTimer = undefined;
      const current = this.status;
      if (!this._closed && !this._socket && current && current.socketPath) {
        this._connect(current.socketPath, current.generation);
      }
    }, delay);
  }

  _disconnect() {
    clearTimeout(this._reconnectTimer);
    this._reconnectTimer = undefined;
    const socket = this._socket;
    if (!socket) return;
    this._socket = null;
    this._generation = null;
    this._subscribedAccounts = null;
    this.hello = null;
    socket.removeAllListeners();
    socket.on('error', () => {});
    socket.destroy();
    const gone = new EngineConnectionError('The sync engine connection closed');
    for (const { reject } of this._requests.values()) reject(gone);
    this._requests.clear();
    this.emit('disconnected');
  }
}
