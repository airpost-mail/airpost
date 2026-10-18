// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Mirrors the engine-owned settings (docs/desktop-client-plan.md 3.4 and 4.2, T11) between
config.json, which the main process's ConfigPersistenceManager holds, and the `serve` engine's
settings store, over the supervisor's socket connection. Windows keep reading config
synchronously and never talk to the store.

- On each engine whose hello lists `settings`: the first time ever, push the values config.json
  has (the one-time move); then push the keys written while no engine took them; then
  `settings.get`, and the engine wins: its values replace config.json's, and a key it has no
  value for is cleared.
- A local write of an engine-owned key goes to `settings.set`, one call per key, so a value the
  engine refuses doesn't hold back the others.
- `settings.changed` (another client's change) is applied to config.json and the windows.

config.json keeps every key either way, so an engine without `settings` (or a downgrade) still
finds them. The engine answers a connection in the order its store applied the changes, so a
`settings.changed` or `settings.get` answer for a key with a `settings.set` still unanswered was
applied before that set, and is skipped: the local write wins.
*/
import _ from 'underscore';
import { Feature, Method, SettingsValues } from '../engine-protocol';

// The keys the desktop mirrors. The engine also keeps `templates` (the desktop's are files in
// <config>/templates), `mailRules` and `mailRulesAutoSince` (in the main window's localStorage
// until the rules move, stage 14) and `resurfacing.*` (written from stage 4).
export const ENGINE_SETTINGS_KEYS = [
  'core.sending.undoSend',
  'core.sending.defaultAccountIdForSend',
  'core.notifications.unsnoozeToTop',
  'core.composing.html',
  'signatures',
  'defaultSignatures',
];

// Set in config.json once its values have been pushed to an engine store.
export const PUSHED_KEY = 'engineSettingsPushed';

// How long launch waits for the first sync before it opens windows with config.json's values.
export const ENGINE_SETTINGS_WAIT_MS = 3000;

// The supervisor's EngineServeProcess, as the mirror uses it.
export interface SettingsEngine {
  features: string[] | null;
  request<R = unknown>(method: string, params: unknown): Promise<R>;
}

// ConfigPersistenceManager, as the mirror uses it.
export interface SettingsHost {
  get(keyPath: string): unknown;
  // Sets each key path (undefined clears it), saves config.json once and tells every window.
  // Not sent back to the engine.
  applyEngineValues(changes: { [keyPath: string]: unknown }): void;
}

// Whether a config key path write at `keyPath` (null: every setting) can change `key`.
export function touchesKey(keyPath: string | null, key: string) {
  return (
    !keyPath || keyPath === key || key.startsWith(`${keyPath}.`) || keyPath.startsWith(`${key}.`)
  );
}

const wire = (v: unknown) => (v === undefined ? null : v);
const local = (v: unknown) => (v === null ? undefined : v);

export class EngineSettingsMirror {
  host: SettingsHost;
  engine: SettingsEngine = null;
  // Written locally and not yet stored by an engine.
  _unsent = new Set<string>();
  // Keys with a settings.set awaiting its answer, and how many.
  _inFlight = new Map<string, number>();
  _ready: Promise<void>;
  _markReady: () => void;

  constructor(host: SettingsHost) {
    this.host = host;
    this._ready = new Promise((resolve) => (this._markReady = resolve));
  }

  // Resolves after the first engine's sync, once an engine without `settings` greeted, or after
  // `timeoutMs`, whichever comes first.
  whenReady(timeoutMs: number) {
    return Promise.race([
      this._ready,
      new Promise<void>((resolve) => setTimeout(resolve, timeoutMs)),
    ]);
  }

  // The engine's socket hello answered.
  attach(engine: SettingsEngine) {
    if (!(engine.features || []).includes(Feature.Settings)) {
      this.engine = null;
      this._markReady();
      return;
    }
    this.engine = engine;
    this._sync(engine).finally(() => this._markReady());
  }

  // The engine exited. Writes still unanswered stay unsent, for the next one.
  detach(engine: SettingsEngine) {
    if (this.engine === engine) this.engine = null;
  }

  // ConfigPersistenceManager stored a local write that changed these engine-owned keys.
  localChanged(keys: string[]) {
    for (const key of keys) this._unsent.add(key);
    if (this.engine) this._sendKeys(this.engine, keys);
  }

  // settings.changed: another client's change.
  engineChanged(engine: SettingsEngine, { values }: SettingsValues) {
    if (engine !== this.engine || !values) return;
    this._apply(values, Object.keys(values));
  }

  async _sync(engine: SettingsEngine) {
    const firstPush = !this.host.get(PUSHED_KEY);
    const keys = ENGINE_SETTINGS_KEYS.filter((key) =>
      firstPush ? this.host.get(key) !== undefined : this._unsent.has(key)
    );
    try {
      const pushed = this._sendKeys(engine, keys);
      if (firstPush) {
        await pushed;
        if (this.engine !== engine) return;
        this.host.applyEngineValues({ [PUSHED_KEY]: true });
      }
      const { values } = await engine.request<SettingsValues>(Method.SettingsGet, {
        keys: ENGINE_SETTINGS_KEYS,
      });
      if (this.engine !== engine) return;
      this._apply(values || {}, ENGINE_SETTINGS_KEYS);
    } catch (err) {
      console.warn(`Could not read the engine's settings: ${err.message}`);
    }
  }

  // Applies the engine's values of `keys` (a key absent from `values` has none).
  _apply(values: { [key: string]: unknown }, keys: string[]) {
    const changes: { [key: string]: unknown } = {};
    for (const key of keys) {
      if (!ENGINE_SETTINGS_KEYS.includes(key) || this._inFlight.has(key)) continue;
      const value = local(values[key]);
      if (!_.isEqual(value, this.host.get(key))) changes[key] = value;
    }
    if (Object.keys(changes).length) this.host.applyEngineValues(changes);
  }

  // Resolves once every set has been answered; a refused one is logged and dropped.
  _sendKeys(engine: SettingsEngine, keys: string[]) {
    return Promise.all(
      keys.map((key) => {
        this._inFlight.set(key, (this._inFlight.get(key) || 0) + 1);
        const values = { [key]: wire(this.host.get(key)) };
        return engine.request(Method.SettingsSet, { values }).then(
          () => this._answered(key, true),
          (err) => {
            if (!err.engineExited) {
              console.warn(`The engine refused the setting ${key}: ${err.message}`);
            }
            this._answered(key, !err.engineExited);
          }
        );
      })
    );
  }

  _answered(key: string, done: boolean) {
    const left = (this._inFlight.get(key) || 1) - 1;
    if (left > 0) {
      this._inFlight.set(key, left);
      return;
    }
    this._inFlight.delete(key);
    if (done) this._unsent.delete(key);
  }
}
