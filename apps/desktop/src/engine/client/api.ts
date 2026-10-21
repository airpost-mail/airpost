// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
`Engine`, the plugin API's way to the sync engine (plugin API 1.27):
`request` any socket method on the window's connection (reads, `query`,
`metadata.list`, `integrations.*`), `subscribe` to its notifications, and the `features` and
`capabilities` of the engine it is connected to. Each window has its own connection; requests made
while it is down reject.

Plugins are fully trusted code: they run in the window with Node and can reach everything the app
can, so nothing here is a security boundary (docs/tech-debt.md TD29). What it does is keep a plugin
from calling a method that writes accounts or secrets or touches the engine itself (the local-only
class of T13) by accident or without the user knowing: such a call is refused unless the plugin's
package.json lists the method in `tern.engineMethods` and the user allowed it once. The calling
package is found from the call's stack, so a request made from the app's own code (src/) or a
bundled package is never asked about; one from code outside the app that belongs to no package
is refused. `hello` and `subscribe` belong to the window's connection and are refused for
everyone: a second `subscribe` would replace the app's own subscription.
*/
import fs from 'fs';
import path from 'path';
import { localized } from '../../intl/localize';
import EngineEvents, { EngineEvents as EngineEventsType, EngineNotification } from './events';
import { EngineCapabilities, ErrorCode, Feature, rejectionFields } from '../protocol';
import {
  Integrations,
  IntegrationError,
  IntegrationMethod,
  IntegrationsTransport,
} from './integrations';

// T13's local-only class. Most are refused on the socket anyway (stdin only, or not there yet).
const LOCAL_ONLY_METHODS = [
  'account.add',
  'account.test',
  'account.setSecrets',
  'account.oauth.begin',
  'account.oauth.complete',
  'account.update',
  'account.remove',
  'account.reset',
  'engine.logs.tail',
  'engine.setLogLevel',
  'engine.resetDatabase',
  'engine.shutdown',
];
const LOCAL_ONLY_PREFIXES = ['secrets.', 'credentials.', 'server.'];

// The window connection's own; Engine.subscribe filters its notifications instead.
const CONNECTION_METHODS = ['hello', 'subscribe'];

export function isLocalOnlyMethod(method: string) {
  return (
    LOCAL_ONLY_METHODS.includes(method) || LOCAL_ONLY_PREFIXES.some((p) => method.startsWith(p))
  );
}

// Package name -> the local-only methods the user allowed it.
export const ENGINE_METHOD_APPROVALS_KEY = 'core.plugins.engineMethodApprovals';

export interface CallerPackage {
  name: string;
  // Shipped with the app (src/features/, plugins/, themes/), never asked about.
  bundled: boolean;
  // `tern.engineMethods` from its package.json.
  engineMethods: string[];
}

export class EngineRequestRefused extends Error {
  method: string;
  // Null when no package made the call.
  packageName: string | null;

  constructor(message: string, method: string, packageName: string | null) {
    super(message);
    this.name = 'EngineRequestRefused';
    this.method = method;
    this.packageName = packageName;
  }
}

export interface EngineSubscribeFilter {
  // Notification methods, e.g. ['mail.new']; every one but deltas when absent.
  methods?: string[];
  // Only notifications whose params name one of these accounts (`accountId`).
  accounts?: string[];
}

export interface Disposable {
  dispose(): void;
}

// Source paths in a V8 stack, `at fn (/path/file.js:1:2)` or `at /path/file.js:1:2`, with or
// without file://. The path may hold parentheses and colons; the last `:line:column` ends it.
const FRAME_PATH = /^at (?:.*? \()?(?:file:\/\/)?((?:\/|[A-Za-z]:[\\/]).*?):\d+:\d+\)?$/;

export function stackPaths(stack: string): string[] {
  const paths: string[] = [];
  for (const line of (stack || '').split('\n')) {
    const m = FRAME_PATH.exec(line.trim());
    if (m) paths.push(decodeURI(m[1]));
  }
  return paths;
}

const within = (file: string, dir: string) => {
  const rel = path.relative(dir, file);
  return !!rel && !rel.startsWith('..') && !path.isAbsolute(rel);
};

// `dir` as given and resolved: stack paths are realpaths, package directories may be symlinks.
function dirForms(dir: string) {
  try {
    const real = fs.realpathSync(dir);
    return real === dir ? [dir] : [dir, real];
  } catch (err) {
    return [dir];
  }
}

const withinAny = (file: string, dirs: string[]) => dirs.some((dir) => within(file, dir));

/**
 * The package the first frame outside the app's src/ and node_modules belongs to, or null for
 * the app itself. A frame outside the app that is in no package is an unknown caller, which
 * gets no local-only method.
 */
export function callerPackageFromStack(stack: string): CallerPackage | null {
  const resourcePath = AppEnv.getLoadSettings().resourcePath;
  const app = dirForms(resourcePath);
  const passThrough = app.flatMap((dir) => [path.join(dir, 'src'), path.join(dir, 'node_modules')]);
  const packages = (AppEnv.packages ? AppEnv.packages.getAvailablePackages() : []).map((pkg) => ({
    pkg,
    dirs: dirForms(pkg.directory),
  }));
  for (const file of stackPaths(stack)) {
    if (withinAny(file, passThrough)) continue;
    const found = packages.find(({ dirs }) => withinAny(file, dirs));
    if (found) {
      return {
        name: found.pkg.name,
        bundled: found.dirs.some((dir) => withinAny(dir, app)),
        engineMethods: found.pkg.engineMethods(),
      };
    }
    if (withinAny(file, app)) return null;
    return { name: file, bundled: false, engineMethods: [] };
  }
  return null;
}

function askUser(pkg: CallerPackage, method: string) {
  // eslint-disable-next-line no-restricted-syntax
  const { dialog } = require('@electron/remote');
  const response = dialog.showMessageBoxSync(AppEnv.getCurrentWindow(), {
    type: 'warning',
    buttons: [localized('Allow'), localized('Deny')],
    defaultId: 1,
    cancelId: 1,
    message: localized('Allow the plugin %1$@ to call %2$@?', pkg.name, method),
    detail: localized(
      'This lets the plugin change your accounts, their passwords or the sync engine itself. Plugins run with full access to Tern; only allow this for plugins you trust.'
    ),
  });
  return response === 0;
}

export class EngineAPI {
  events: EngineEventsType;
  // Swappable for specs.
  findCaller: (stack: string) => CallerPackage | null = callerPackageFromStack;
  askUser: (pkg: CallerPackage, method: string) => boolean = askUser;
  // Refused this session, so a plugin that retries doesn't ask again until a restart.
  private _denied = new Set<string>();

  constructor(events: EngineEventsType = EngineEvents) {
    this.events = events;
  }

  /** The features the connected engine's hello listed; empty while disconnected. */
  get features(): string[] {
    return this.events.features ? [...this.events.features] : [];
  }

  has(feature: string) {
    return this.events.has(feature);
  }

  get connected() {
    return !!this.events.features;
  }

  /** What the engine can do now (`capabilities`); null while disconnected or from an older engine. */
  get capabilities(): EngineCapabilities | null {
    return this.events.capabilities;
  }

  /**
   * Calls `method` on the window's engine connection. Rejects with the engine's error (`code`,
   * `message`, `data`), or EngineRequestRefused for `hello`, `subscribe` or a local-only method
   * the plugin may not call.
   */
  request<R = unknown>(method: string, params: unknown = {}): Promise<R> {
    if (CONNECTION_METHODS.includes(method)) {
      return Promise.reject(
        new EngineRequestRefused(
          `${method} belongs to the window's connection; use Engine.subscribe for notifications`,
          method,
          null
        )
      );
    }
    try {
      this._checkCaller(method, new Error().stack ?? '');
    } catch (err) {
      return Promise.reject(err);
    }
    return this.events.request<R>(method, params);
  }

  /** Calls `callback` for each notification that passes `filter`. Deltas come as DatabaseStore changes instead. */
  subscribe(
    filter: EngineSubscribeFilter | null,
    callback: (notification: EngineNotification) => void
  ): Disposable {
    const { methods = null, accounts = null } = filter || {};
    const listener = (n: EngineNotification) => {
      if (methods && !methods.includes(n.method)) return;
      if (accounts) {
        const accountId = n.params && n.params.accountId;
        if (!accounts.includes(accountId)) return;
      }
      callback(n);
    };
    this.events.on('notification', listener);
    return { dispose: () => this.events.removeListener('notification', listener) };
  }

  /** Calls `callback` when the engine connects, goes away or its capabilities change. */
  onDidChange(callback: () => void): Disposable {
    const events = ['connected', 'gone', 'capabilities'];
    for (const e of events) this.events.on(e, callback);
    return { dispose: () => events.forEach((e) => this.events.removeListener(e, callback)) };
  }

  _checkCaller(method: string, stack: string) {
    if (!isLocalOnlyMethod(method)) return;
    const caller = this.findCaller(stack);
    if (!caller || caller.bundled) return;
    const refuse = (why: string) => {
      throw new EngineRequestRefused(
        `${caller.name} may not call ${method}: ${why}`,
        method,
        caller.name
      );
    };
    if (!caller.engineMethods.includes(method)) {
      refuse(`it is a local-only method and package.json's tern.engineMethods doesn't list it`);
    }
    const approvals = AppEnv.config.get(ENGINE_METHOD_APPROVALS_KEY) || {};
    const approved: string[] = Array.isArray(approvals[caller.name]) ? approvals[caller.name] : [];
    if (approved.includes(method)) return;
    const key = `${caller.name}\u0000${method}`;
    if (this._denied.has(key) || !this.askUser(caller, method)) {
      this._denied.add(key);
      refuse('the user did not allow it');
    }
    AppEnv.config.set(ENGINE_METHOD_APPROVALS_KEY, {
      ...approvals,
      [caller.name]: [...approved, method],
    });
  }
}

export const Engine = new EngineAPI();
export default Engine;

/**
 * The integrations transport on the window's engine connection: available while the connected
 * engine lists Feature.Integrations. Socket errors become IntegrationErrors.
 */
export function engineIntegrationsTransport(
  events: EngineEventsType = EngineEvents
): IntegrationsTransport {
  return {
    get available() {
      return events.has(Feature.Integrations);
    },
    async send(method: IntegrationMethod, params: object) {
      try {
        return await events.request(method, params);
      } catch (err) {
        const { code, data, message } = rejectionFields(err);
        const text = typeof message === 'string' ? message : undefined;
        if (err && code === ErrorCode.IntegrationFailed && data) throw data;
        if (err && code === ErrorCode.EngineBusy) {
          throw new IntegrationError({ key: 'rate-limited', message: text, retryable: true });
        }
        if (err && (code === null || code === undefined)) {
          throw new IntegrationError({ key: 'engine-unavailable', message: text });
        }
        throw err;
      }
    },
  };
}

/** Points Integrations at the window's engine connection, and tells its listeners on each change. */
export function wireIntegrationsTransport(events: EngineEventsType = EngineEvents) {
  const transport = engineIntegrationsTransport(events);
  const changed = () => Integrations.setTransport(transport);
  changed();
  events.on('connected', changed);
  events.on('gone', changed);
  return () => {
    events.removeListener('connected', changed);
    events.removeListener('gone', changed);
  };
}
