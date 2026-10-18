// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Mirrors the engine-owned settings (docs/desktop-client-plan.md 3.4 and 4.2, T11) between
config.json, which the main process's ConfigPersistenceManager holds, and the `serve` engine's
settings store, over the supervisor's socket connection. Windows keep reading config
synchronously and never talk to the store.

- On each engine whose hello lists `settings`: `settings.get`, which also names the store. A
  store config.json's values were never copied into (a first launch, or a store the engine
  started anew after losing or setting aside settings.json) gets them, one `settings.set` per
  key, and so does a key mirrored since the store was copied (COPIED_KEY lists the keys it got);
  then the keys written while no engine took them are pushed; and the engine wins for the
  rest: its values replace config.json's, and a key it has no value for is cleared. A key whose
  copy the engine refused is kept in config.json instead (KEPT_KEY) until the engine has a value.
  When `settings.get` fails, config.json is left as it is.
- A local write of an engine-owned key goes to `settings.set`, one call per key, so a value the
  engine refuses doesn't hold back the others.
- `settings.changed` (another client's change) is applied to config.json and the windows.
- An engine that runs the expiration executor (`expiration-executor`) gets the texts of the
  resurfacing messages it sends (`resurfacing.*`) in the user's language after each sync and
  when the language changes; config.json has no copy of them.

config.json keeps every key either way, so an engine without `settings` (or a downgrade) still
finds them. A key that needs a feature (KEY_FEATURES) is left out entirely with an engine that
doesn't list it, so an older engine that refuses or doesn't know the key never clears it. The engine answers a connection in the order its store applied the changes, so a
`settings.changed` or `settings.get` answer for a key with a `settings.set` still unanswered was
applied before that set, and is skipped: the local write wins.
*/
import _ from 'underscore';
import { Feature, Method, SettingsValues } from '../engine-protocol';
import { localizerFor } from '../intl';
import { REMINDER_BODY_KEY, SNOOZE_BODY_KEY, resurfacingBodies } from '../resurfacing';

// The keys the desktop mirrors. The engine also keeps `templates` (the desktop's are files in
// <config>/templates), `mailRules` and `mailRulesAutoSince` (in the main window's localStorage
// until the rules move, stage 14) and `resurfacing.*` (written, not mirrored: _writeResurfacing).
export const ENGINE_SETTINGS_KEYS = [
  'core.sending.undoSend',
  'core.sending.defaultAccountIdForSend',
  'core.notifications.unsnoozeToTop',
  'core.composing.html',
  'signatures',
  'defaultSignatures',
  // Provider settings without keys; the engine's capabilities follow them.
  'core.integrations.translation',
  'core.integrations.grammar',
  'core.integrations.enrichment',
  // The AI providers (without keys), switches and opted-in accounts (src/ai.ts); background
  // triage reads them from the engine's store. `core.ai.limits` has no UI yet.
  'core.ai.providers',
  'core.ai.features',
  'core.ai.accounts',
  'core.ai.includeAddresses',
  'core.ai.limits',
];

// Mirrored only with an engine whose hello lists the feature.
const KEY_FEATURES: { [key: string]: string } = {
  'core.integrations.translation': Feature.Integrations,
  'core.integrations.grammar': Feature.Integrations,
  'core.integrations.enrichment': Feature.Integrations,
  'core.ai.providers': Feature.AiTriage,
  'core.ai.features': Feature.AiTriage,
  'core.ai.accounts': Feature.AiTriage,
  'core.ai.includeAddresses': Feature.AiTriage,
  'core.ai.limits': Feature.AiTriage,
};

// The keys a store named in STORE_KEY got when COPIED_KEY didn't exist yet (before stage 5).
const FIRST_COPIED_KEYS = [
  'core.sending.undoSend',
  'core.sending.defaultAccountIdForSend',
  'core.notifications.unsnoozeToTop',
  'core.composing.html',
  'signatures',
  'defaultSignatures',
];

// The keys mirrored with `engine`.
export function mirroredKeys(engine: SettingsEngine) {
  const features = engine.features || [];
  return ENGINE_SETTINGS_KEYS.filter(
    (key) => !KEY_FEATURES[key] || features.includes(KEY_FEATURES[key])
  );
}

// The config key of the UI language; a change rewrites the resurfacing texts.
export const LANGUAGE_KEY = 'core.intl.language';
const RESURFACING_KEYS = [SNOOZE_BODY_KEY, REMINDER_BODY_KEY];

// The id (settings.get's `storeId`) of the engine store config.json's values were copied into.
export const STORE_KEY = 'engineSettingsStore';
// The keys config.json's values were copied for into that store.
export const COPIED_KEY = 'engineSettingsCopiedKeys';
// Engine-owned keys config.json keeps although the store has no value: the copy was refused.
export const KEPT_KEY = 'engineSettingsKept';
// The store of an engine that names none.
const UNNAMED_STORE = 'unnamed';

type SetOutcome = 'stored' | 'refused' | 'exited';

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
  // A key the engine doesn't mirror stays unsent, for an engine that does.
  localChanged(keys: string[]) {
    for (const key of keys) this._unsent.add(key);
    if (!this.engine) return;
    const mirrored = mirroredKeys(this.engine);
    this._sendKeys(
      this.engine,
      keys.filter((key) => mirrored.includes(key))
    );
  }

  // The UI language changed: the engine's resurfacing messages follow it.
  languageChanged() {
    if (this.engine) this._writeResurfacing(this.engine, {});
  }

  // settings.changed: another client's change.
  engineChanged(engine: SettingsEngine, { values }: SettingsValues) {
    if (engine !== this.engine || !values) return;
    const mirrored = mirroredKeys(engine);
    this._apply(
      values,
      Object.keys(values).filter((key) => mirrored.includes(key))
    );
  }

  async _sync(engine: SettingsEngine) {
    try {
      const mirrored = mirroredKeys(engine);
      const { values, storeId } = await engine.request<SettingsValues>(Method.SettingsGet, {
        keys: [...mirrored, ...RESURFACING_KEYS],
      });
      if (this.engine !== engine) return;
      this._writeResurfacing(engine, values || {});
      const store = storeId || UNNAMED_STORE;
      const sameStore = store === this.host.get(STORE_KEY);
      const copied = sameStore ? this._copied() : [];
      const toCopy = mirrored.filter((key) => !copied.includes(key));
      const keys = mirrored.filter(
        (key) => this._unsent.has(key) || (toCopy.includes(key) && this.host.get(key) !== undefined)
      );
      const sent = this._sendKeys(engine, keys);
      // The keys just sent are in flight, so _apply leaves them.
      this._apply(values || {}, mirrored);
      if (!toCopy.length) return;
      const outcomes = await sent;
      if (this.engine !== engine || outcomes.includes('exited')) return;
      const refused = keys.filter((key, i) => outcomes[i] === 'refused' && toCopy.includes(key));
      const kept = [...(sameStore ? this._kept() : []), ...refused];
      this.host.applyEngineValues({
        [STORE_KEY]: store,
        [COPIED_KEY]: [...copied, ...toCopy],
        [KEPT_KEY]: kept.length ? _.uniq(kept) : undefined,
      });
    } catch (err) {
      console.warn(`Could not read the engine's settings: ${err.message}`);
    }
  }

  // Writes the resurfacing texts in the configured language unless `current` has them already.
  _writeResurfacing(engine: SettingsEngine, current: { [key: string]: unknown }) {
    if (!(engine.features || []).includes(Feature.ExpirationExecutor)) return;
    const bodies = resurfacingBodies(localizerFor(this.host.get(LANGUAGE_KEY) as string));
    const values = {
      [SNOOZE_BODY_KEY]: bodies.snoozeBody,
      [REMINDER_BODY_KEY]: bodies.reminderBody,
    };
    if (RESURFACING_KEYS.every((key) => current[key] === values[key])) return;
    engine.request(Method.SettingsSet, { values }).catch((err) => {
      if (!err.engineExited) console.warn(`Could not write the resurfacing texts: ${err.message}`);
    });
  }

  _copied(): string[] {
    const copied = this.host.get(COPIED_KEY);
    return Array.isArray(copied) ? copied : FIRST_COPIED_KEYS;
  }

  _kept(): string[] {
    const kept = this.host.get(KEPT_KEY);
    return Array.isArray(kept) ? kept : [];
  }

  // Applies the engine's values of `keys` (a key absent from `values` has none).
  _apply(values: { [key: string]: unknown }, keys: string[]) {
    const kept = this._kept();
    const changes: { [key: string]: unknown } = {};
    const valued: string[] = [];
    for (const key of keys) {
      if (!ENGINE_SETTINGS_KEYS.includes(key) || this._inFlight.has(key) || this._unsent.has(key)) {
        continue;
      }
      const value = local(values[key]);
      if (kept.includes(key)) {
        if (value === undefined) continue;
        valued.push(key);
      }
      if (!_.isEqual(value, this.host.get(key))) changes[key] = value;
    }
    if (valued.length) changes[KEPT_KEY] = this._without(kept, valued);
    if (Object.keys(changes).length) this.host.applyEngineValues(changes);
  }

  _without(kept: string[], keys: string[]) {
    const left = kept.filter((k) => !keys.includes(k));
    return left.length ? left : undefined;
  }

  // Resolves once every set has been answered, with each one's outcome; a refused one is
  // logged and dropped.
  _sendKeys(engine: SettingsEngine, keys: string[]): Promise<SetOutcome[]> {
    return Promise.all(
      keys.map((key) => {
        this._inFlight.set(key, (this._inFlight.get(key) || 0) + 1);
        const values = { [key]: wire(this.host.get(key)) };
        return engine.request(Method.SettingsSet, { values }).then(
          (): SetOutcome => {
            this._answered(key, true);
            const kept = this._kept();
            if (kept.includes(key)) {
              this.host.applyEngineValues({ [KEPT_KEY]: this._without(kept, [key]) });
            }
            return 'stored';
          },
          (err): SetOutcome => {
            if (!err.engineExited) {
              console.warn(`The engine refused the setting ${key}: ${err.message}`);
            }
            this._answered(key, !err.engineExited);
            return err.engineExited ? 'exited' : 'refused';
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
