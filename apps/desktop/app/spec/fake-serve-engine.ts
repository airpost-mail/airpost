// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { EventEmitter } from 'events';
import fs from 'fs';
import net from 'net';
import path from 'path';
import { serveSocketPath } from '../src/engine-protocol';

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
  // The next account.add result by account id, instead of `running`.
  addResults: { [id: string]: any } = {};
  exitOnShutdown = true;
  // False stands in for an engine that could not bind and never reads stdin.
  answerList = true;
  // Socket results by method, instead of `{}`.
  socketResults: { [method: string]: any } = {};
  // Deltas published before the first subscribe to every account and class, which takes them.
  backlog: any[] | null = null;

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
            let result = req.method === 'calendar.sync' ? { started: true } : {};
            if (this.socketResults[req.method] !== undefined) {
              result = this.socketResults[req.method];
            }
            socket.write(`${JSON.stringify({ jsonrpc: '2.0', id: req.id, result })}\n`);
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
        const result = this.addResults[id] || { accountId: id, state: 'running' };
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
      case 'engine.shutdown':
        if (!this.exitOnShutdown) return null;
        return respond({}).then(() => child.emit('exit', 0, null));
      default:
        return respond(null, { code: -32601, message: `unknown method ${req.method}` });
    }
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
