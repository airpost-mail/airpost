// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
An engine for specs that answers `query` from fixture models, standing in for the window's
connection to `serve` (docs/desktop-client-plan.md 4.1 step 5). It reads Query.toJSON() the way
crates/ms-store/src/model_query.rs does for what specs use: matchers run as the desktop's own
Matcher on the fixtures, then the order, offset and limit, `count`, `idsOnly` and `include`.
`search` keeps the models whose JSON contains the text. Answers go through JSON, as on the wire.

install() connects it to the window's EngineEvents and turns `core.engine.reads` to "engine";
kill() drops the connection (requests out fail as a closed socket fails them) and start()
connects again, which is how the reconnect specs drive the engine.
*/
import EngineEvents, { EngineEvents as EngineEventsType } from '../src/engine-events';
import { EngineConnectionError } from '../src/engine-connection';
import { ErrorCode, Feature, Method, QueryResult } from '../src/engine-protocol';
import EngineReads from '../src/flux/stores/engine-reads';
import { Matcher } from '../src/flux/attributes/matcher';
import { AttributeJoinedData } from '../src/flux/attributes/attribute-joined-data';
import DatabaseObjectRegistry from '../src/registries/database-object-registry';
import type { ModelQueryJSON } from '../src/flux/models/query';
import type { MatcherJSON } from '../src/flux/attributes/matcher';
import { Model } from '../src/flux/models/model';

type Pending = { reject: (err: Error) => void };

export class MockEngine {
  events: EngineEventsType;
  features: string[];
  // Fixtures by class name; joined data (Message bodies) by model id.
  models = new Map<string, Model[]>();
  joined = new Map<string, { [modelKey: string]: string | null }>();
  requests: Array<{ method: string; params: any }> = [];
  running = false;
  _pending = new Set<Pending>();

  constructor({
    events = EngineEvents,
    features = [Feature.Query, Feature.Counts],
  }: { events?: EngineEventsType; features?: string[] } = {}) {
    this.events = events;
    this.features = features;
  }

  install() {
    spyOn(EngineReads, '_flag').andReturn('engine');
    this.start();
    return this;
  }

  uninstall() {
    this.kill();
  }

  start() {
    this.running = true;
    this.events.connected(this.features, (method, params) => this._request(method, params));
  }

  kill() {
    if (!this.running) return;
    this.running = false;
    const gone = new EngineConnectionError('The sync engine connection closed');
    for (const pending of [...this._pending]) pending.reject(gone);
    this._pending.clear();
    this.events.gone();
  }

  add(...models: Model[]) {
    for (const model of models) {
      const name = this._tableOf(model.constructor as typeof Model);
      const list = (this.models.get(name) || []).filter((m) => m.id !== model.id);
      list.push(model);
      this.models.set(name, list);
      const klass = model.constructor as typeof Model;
      for (const key of Object.keys(klass.attributes)) {
        if (!(klass.attributes[key] instanceof AttributeJoinedData)) continue;
        const value = model[key];
        if (value === undefined) continue;
        this.joined.set(model.id, { ...(this.joined.get(model.id) || {}), [key]: value });
      }
    }
  }

  remove(klass: typeof Model, id: string) {
    const name = this._tableOf(klass);
    this.models.set(
      name,
      (this.models.get(name) || []).filter((m) => m.id !== id)
    );
  }

  queries(className?: string): ModelQueryJSON[] {
    return this.requests
      .filter((r) => r.method === Method.Query && (!className || r.params.class === className))
      .map((r) => r.params);
  }

  answer(json: ModelQueryJSON): QueryResult {
    const klass = DatabaseObjectRegistry.get(json.class) as unknown as typeof Model;
    if (!klass) throw rpcError(`invalid params: unknown class ${json.class}`);
    let rows = [...(this.models.get(json.class) || [])];
    if (json.accounts) rows = rows.filter((m) => json.accounts.includes(m.accountId));
    if (json.where) {
      const matcher = matcherFor(klass, json.where);
      rows = rows.filter((m) => matcher.evaluate(m));
    }
    if (json.search) {
      const text = json.search.text.toLowerCase();
      rows = rows.filter((m) => JSON.stringify(m.toJSON()).toLowerCase().includes(text));
    }
    if (json.count) return { rows: [], count: rows.length };
    for (const { attr, dir } of [...(json.order || [])].reverse()) {
      if (!klass.attributes[attr]) throw rpcError(`invalid params: unknown attribute ${attr}`);
      const sign = dir === 'desc' ? -1 : 1;
      rows.sort((a, b) => sign * compare(a[attr], b[attr]));
    }
    const offset = json.offset || 0;
    rows = rows.slice(offset, json.limit === null ? undefined : offset + json.limit);
    if (json.idsOnly) return { rows: [], ids: rows.map((m) => m.id) };
    return {
      rows: rows.map((m) => {
        const row = m.toJSON();
        for (const key of Object.keys(klass.attributes)) {
          const attr = klass.attributes[key];
          if (!(attr instanceof AttributeJoinedData)) continue;
          delete row[attr.jsonKey || key];
          if (json.include.includes(key)) {
            const value = (this.joined.get(m.id) || {})[key];
            row[key] = value === undefined ? null : value;
          }
        }
        return row;
      }),
    };
  }

  _tableOf(klass: typeof Model): string {
    return ((klass as any).SubclassesUseModelTable || klass).name;
  }

  _request(method: string, params: any): Promise<unknown> {
    this.requests.push({ method, params });
    return new Promise((resolve, reject) => {
      const pending = { reject };
      this._pending.add(pending);
      // Answered after the caller's current turn, as a socket reply is.
      Promise.resolve().then(() => {
        if (!this._pending.has(pending)) return;
        this._pending.delete(pending);
        try {
          resolve(JSON.parse(JSON.stringify(this._handle(method, params))));
        } catch (err) {
          reject(err);
        }
      });
    });
  }

  _handle(method: string, params: any): unknown {
    switch (method) {
      case Method.Query:
        return this.answer(params);
      case Method.CountsGet:
        return { counts: {} };
      default:
        throw new EngineConnectionError(`unknown method ${method}`, {
          code: ErrorCode.MethodNotFound,
        });
    }
  }
}

function rpcError(message: string) {
  return new EngineConnectionError(message, { code: ErrorCode.InvalidParams });
}

function compare(a: any, b: any) {
  const value = (v: any) => (v instanceof Date ? v.getTime() : v);
  const x = value(a);
  const y = value(b);
  if (x === y) return 0;
  if (x === undefined || x === null) return -1;
  if (y === undefined || y === null) return 1;
  return x < y ? -1 : 1;
}

function matcherFor(klass: typeof Model, json: MatcherJSON): Matcher {
  if ('and' in json) return new Matcher.And(json.and.map((m) => matcherFor(klass, m)));
  if ('or' in json) return new Matcher.Or(json.or.map((m) => matcherFor(klass, m)));
  if ('not' in json) return new Matcher.Not(json.not.map((m) => matcherFor(klass, m)));
  const attr = klass.attributes[json.attr];
  if (!attr || !attr.queryable) {
    throw rpcError(`invalid params: ${json.attr} is not queryable on ${klass.name}`);
  }
  return new Matcher(attr, json.op, json.value);
}

// Waits out the mock's replies and what the callers do with them, React's passive effects too.
export async function settle(turns = 20) {
  for (let i = 0; i < turns; i++) await Promise.resolve();
  await new Promise((resolve) => setImmediate(resolve));
}
