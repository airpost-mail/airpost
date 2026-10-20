// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { EventEmitter } from 'events';
import fs from 'fs';
import net from 'net';
import path from 'path';
import { serveSocketPath } from '../../src/engine/protocol';

// Lets real I/O (the socket) and promise callbacks run; the spec helper fakes setTimeout.
export const tick = () => new Promise((resolve) => setImmediate(resolve));

export async function until(what: string, condition: () => boolean) {
  for (let i = 0; i < 5000; i++) {
    if (condition()) return;
    await tick();
  }
  throw new Error(`timed out waiting for ${what}`);
}

/*
Stands in for `mailsync-rs --mode serve`: each spawn gets a stub child whose stdin answers the
parent-channel requests on stdout, and one Unix socket server at the serve socket path takes
the client's connections.
*/
export class FakeEngine {
  dir: string;
  socketPath: string;
  server: net.Server;
  conn: net.Socket = null;
  conns: net.Socket[] = [];
  // The requests of each connection, by its index in `conns`.
  connRequests: any[][] = [];
  spawns: { mode: string; args: string[]; child: any }[] = [];
  stdin: any[] = [];
  socket: any[] = [];
  socketText = '';
  stillRunning: string[] = [];
  // account.add responses held back by account id, when set.
  holdAdds = new Set<string>();
  heldAdds: (() => void)[] = [];
  // The next account.add result by account id, instead of `running` (and `stored` once
  // keepsAccounts() was called).
  addResults: { [id: string]: any } = {};
  exitOnShutdown = true;
  // False stands in for an engine that could not bind and never reads stdin.
  answerList = true;
  // The credentials.import result, instead of every account imported.
  importResult: any = null;
  // The account.test result, instead of a passed test that echoes the account.
  testResult: any = null;
  // Socket results by method, instead of `{}`, and errors, instead of a result.
  socketResults: { [method: string]: any } = {};
  socketErrors: { [method: string]: { code: number; message: string } } = {};
  // Deltas published before the first subscribe to every account and class, which takes them.
  backlog: any[] | null = null;
  // The settings store, when the engine announces `settings`: settings.get and settings.set
  // answer from it.
  settings: { [key: string]: any } | null = null;
  // settings.get's storeId.
  settingsStoreId = 'store-1';
  // The hello's features when the engine has the settings store.
  settingsFeatures = ['settings'];

  constructor(dir: string) {
    this.dir = dir;
    this.socketPath = serveSocketPath(dir).socketPath;
    fs.mkdirSync(path.dirname(this.socketPath), { recursive: true });
  }

  start() {
    return new Promise<void>((resolve) => {
      this.server = net.createServer((socket) => {
        this.conn = socket;
        this.conns.push(socket);
        const requests: any[] = [];
        this.connRequests.push(requests);
        // A client that went away mid-write.
        socket.on('error', () => {});
        let buffer = '';
        socket.setEncoding('utf8');
        socket.on('data', (chunk: string) => {
          this.socketText += chunk;
          buffer += chunk;
          const lines = buffer.split('\n');
          buffer = lines.pop();
          for (const line of lines) {
            const req = JSON.parse(line);
            this.socket.push(req);
            requests.push(req);
            let result: any = req.method === 'calendar.sync' ? { started: true } : {};
            if (this.settings) result = this._settingsResult(req) || result;
            if (this.socketResults[req.method] !== undefined) {
              result = this.socketResults[req.method];
            }
            const error = this.socketErrors[req.method];
            const reply = error
              ? { jsonrpc: '2.0', id: req.id, error }
              : { jsonrpc: '2.0', id: req.id, result };
            socket.write(`${JSON.stringify(reply)}\n`);
            const p = req.params || {};
            if (req.method === 'subscribe' && this.backlog && !p.accounts && !p.classes) {
              for (const params of this.backlog) this.notify('delta', params, socket);
              this.backlog = null;
            }
          }
        });
      });
      this.server.listen(this.socketPath, () => resolve());
    });
  }

  _settingsResult(req: any) {
    const p = req.params || {};
    switch (req.method) {
      case 'hello':
        return {
          protocol: 1,
          engine: 'fake',
          pid: 1,
          accounts: [],
          features: this.settingsFeatures,
        };
      case 'settings.get': {
        const values = {};
        for (const k of Object.keys(this.settings)) {
          if (!p.keys || p.keys.includes(k)) values[k] = this.settings[k];
        }
        return { values, storeId: this.settingsStoreId };
      }
      case 'settings.set':
        for (const [k, v] of Object.entries(p.values || {})) {
          if (v === null) delete this.settings[k];
          else this.settings[k] = v;
        }
        return {};
      default:
        return null;
    }
  }

  stop() {
    for (const c of this.conns) c.destroy();
    this.server.close();
  }

  get child() {
    return this.spawns[this.spawns.length - 1].child;
  }

  spawn(mode: string, args: string[]) {
    const child: any = new EventEmitter();
    child.pid = 4000 + this.spawns.length;
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.stdin = new EventEmitter();
    child.kill = jasmine.createSpy('kill');
    child.stdin.write = (line: string) => {
      for (const text of line.split('\n').filter(Boolean)) {
        const req = JSON.parse(text);
        this.stdin.push({ ...req, child });
        this._answer(child, req);
      }
    };
    this.spawns.push({ mode, args, child });
    return child;
  }

  _answer(child: any, req: any) {
    const respond = (result: any, error?: any) =>
      Promise.resolve().then(() => {
        const msg = error
          ? { jsonrpc: '2.0', id: req.id, error }
          : { jsonrpc: '2.0', id: req.id, result };
        child.stdout.emit('data', Buffer.from(`${JSON.stringify(msg)}\n`));
      });
    switch (req.method) {
      case 'account.list':
        return this.answerList ? respond({ accounts: [] }) : null;
      case 'account.add': {
        const id = req.params.account.id;
        // An engine that keeps the accounts stores the secrets the add carries.
        const hello = this.socketResults.hello;
        const keeps = !!hello && (hello.features || []).includes('accounts');
        const result = this.addResults[id] || {
          accountId: id,
          state: 'running',
          ...(keeps ? { stored: true } : {}),
        };
        delete this.addResults[id];
        const send = () => respond(result);
        if (this.holdAdds.has(id)) {
          this.heldAdds.push(send);
          return null;
        }
        return send();
      }
      case 'account.remove':
        return respond({ stillRunning: this.stillRunning });
      case 'account.test':
        return respond(
          this.testResult || {
            account: req.params.account,
            error: null,
            log: '',
            error_service: 'smtp',
          }
        );
      case 'credentials.import':
        return respond(
          this.importResult || {
            imported: req.params.accounts.map((a: any) => a.id),
            failed: [],
            secrets: Object.keys(req.params.secrets),
          }
        );
      case 'account.reset':
        return respond({});
      case 'engine.resetDatabase':
        return respond({ removed: [] });
      case 'engine.setLogLevel':
        return respond({ restarted: [] });
      case 'engine.shutdown':
        if (!this.exitOnShutdown) return null;
        return respond({}).then(() => child.emit('exit', 0, null));
      default:
        return respond(null, { code: -32601, message: `unknown method ${req.method}` });
    }
  }

  // Answers hello as an engine that keeps the accounts (Feature.Accounts) and has `registered`.
  keepsAccounts(registered: string[] = [], extra: any = {}) {
    this.socketResults.hello = {
      protocol: 1,
      engine: 'fake',
      pid: 1,
      accounts: registered.map((id) => ({
        accountId: id,
        email: `${id}@example.com`,
        state: 'stopped',
      })),
      features: ['accounts'],
      credentials: { store: 'file', accounts: registered.length },
      ...extra,
    };
  }

  // The requests of one method on stdin, in order.
  stdinOf(method: string) {
    return this.stdin.filter((r) => r.method === method);
  }

  socketOf(method: string) {
    return this.socket.filter((r) => r.method === method);
  }

  notify(method: string, params: unknown, conn = this.conn) {
    conn.write(`${JSON.stringify({ jsonrpc: '2.0', method, params })}\n`);
  }

  // A delta as the hub sends it, to every connection whose subscription takes its account
  // and class, and to the backlog while there is one.
  publishDelta(params: { modelClass: string; accountId?: string; [key: string]: unknown }) {
    if (this.backlog) this.backlog.push(params);
    this.conns.forEach((conn, i) => {
      if (conn.destroyed) return;
      const subscribe = this.connRequests[i].filter((r) => r.method === 'subscribe').pop();
      const { accounts, classes } = (subscribe && subscribe.params) || ({} as any);
      if (!subscribe || (classes && !classes.includes(params.modelClass))) return;
      if (accounts && !accounts.includes(params.accountId)) return;
      this.notify('delta', params, conn);
    });
  }
}
