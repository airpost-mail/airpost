// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
The engine's settings (`settings.get|set|changed`) as this client last heard them, held so
they can be read synchronously. `load` fills it (on every connect: the values may have changed
while it was down), `apply` takes `settings.changed`, and `set` shows a write at once and puts
the old values back if the engine refuses it. Before the first load, and after a failed one,
`loaded` is false and every key reads as absent. Nothing is persisted yet (the plan's stage 9
keeps the last values for before hello).
*/
import type { SettingsGetParams, SettingsValues } from './protocol';

export type SettingsMap = SettingsValues['values'];

/** What the cache calls; a Session, or anything with the same two methods. */
export interface SettingsCaller {
  call(
    method: 'settings.get' | 'settings.set',
    params: SettingsGetParams | SettingsValues,
  ): Promise<unknown>;
}

function merge(values: SettingsMap, changes: SettingsMap): SettingsMap {
  const out = { ...values };
  for (const [k, v] of Object.entries(changes)) {
    if (v === null) delete out[k];
    else out[k] = v;
  }
  return out;
}

export class SettingsCache {
  private _loaded = false;
  private _values: SettingsMap = {};
  private readonly listeners = new Set<() => void>();

  /** `keys`: the keys to read, or null for every key the engine has. */
  constructor(private readonly keys: string[] | null = null) {}

  /** The values came from the engine. */
  get loaded(): boolean {
    return this._loaded;
  }

  /** A new object after every change, so it can be compared by identity. */
  get values(): Readonly<SettingsMap> {
    return this._values;
  }

  /** The engine's value, or undefined when it has none or nothing is loaded. */
  get(key: string): unknown {
    return this._values[key];
  }

  /** Called after every change. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Reads the keys from the engine. On failure it is empty and unloaded, and rethrows. */
  async load(caller: SettingsCaller): Promise<void> {
    try {
      const { values } = (await caller.call('settings.get', { keys: this.keys })) as SettingsValues;
      this.update(true, values);
    } catch (err) {
      this.update(false, {});
      throw err;
    }
  }

  /** `settings.changed`: a change from another connection. Ignored until loaded. */
  apply(changes: SettingsMap) {
    if (!this._loaded) return;
    this.update(true, merge(this._values, changes));
  }

  /** Writes `values` (null clears a key), showing them at once; reverted if refused. */
  async set(caller: SettingsCaller, values: SettingsMap): Promise<void> {
    const before = this._values;
    this.update(this._loaded, merge(before, values));
    try {
      await caller.call('settings.set', { values });
    } catch (err) {
      const reverted = { ...this._values };
      for (const k of Object.keys(values)) {
        if (k in before) reverted[k] = before[k];
        else delete reverted[k];
      }
      this.update(this._loaded, reverted);
      throw err;
    }
  }

  /** Forgets everything (tests, sign-out). */
  reset() {
    this.update(false, {});
  }

  private update(loaded: boolean, values: SettingsMap) {
    this._loaded = loaded;
    this._values = values;
    for (const l of [...this.listeners]) l();
  }
}
