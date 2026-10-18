// The engine's settings (DCP stage 3) and capabilities (DCP stage 5) as this tab last heard
// them. EngineProvider fills both on every connect (`settings.get`, `hello.capabilities`)
// and keeps them current from `settings.changed` and `capabilities.changed`; nothing is
// cached across tabs or reloads.
import { useSyncExternalStore } from 'react';
import type { EngineClient } from './client';
import {
  EngineFeature,
  type EngineCapabilities,
  type HelloResult,
  type SettingsValues,
} from './rpc';

/** Settings keys the web reads or writes. */
export const SettingKey = {
  /** Milliseconds; 0 sends at once. */
  UndoSend: 'core.sending.undoSend',
} as const;

const WEB_SETTINGS_KEYS: string[] = Object.values(SettingKey);

interface EngineState {
  /** The engine announced `settings`, and `values` came from it. */
  settings: boolean;
  values: SettingsValues;
  capabilities: EngineCapabilities | null;
}

const EMPTY: EngineState = { settings: false, values: {}, capabilities: null };
let state: EngineState = EMPTY;
const listeners = new Set<() => void>();

function update(next: EngineState) {
  state = next;
  for (const l of listeners) l();
}

function subscribe(cb: () => void) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/** For tests: forget what the last engine said. */
export function resetEngineState() {
  update(EMPTY);
}

/** Reads what the engine has after a handshake. */
export async function loadEngineState(client: EngineClient, hello: HelloResult) {
  const features = new Set(hello.features ?? []);
  const capabilities = features.has(EngineFeature.Capabilities)
    ? (hello.capabilities ?? null)
    : null;
  if (!features.has(EngineFeature.Settings)) {
    update({ settings: false, values: {}, capabilities });
    return;
  }
  update({ ...state, capabilities });
  try {
    const { values } = await client.call('settings.get', { keys: WEB_SETTINGS_KEYS });
    update({ settings: true, values, capabilities: state.capabilities });
  } catch (err) {
    // Unreadable store: fall back to this browser's values until the next connect.
    update({ settings: false, values: {}, capabilities: state.capabilities });
    throw err;
  }
}

function merge(values: SettingsValues, changes: SettingsValues): SettingsValues {
  const out = { ...values };
  for (const [k, v] of Object.entries(changes)) {
    if (v === null) delete out[k];
    else out[k] = v;
  }
  return out;
}

/** `settings.changed`: another connection's change (the desktop's, or another tab's). */
export function applySettingsChanged(changes: SettingsValues) {
  if (!state.settings) return;
  update({ ...state, values: merge(state.values, changes) });
}

export function applyCapabilitiesChanged(capabilities: EngineCapabilities) {
  update({ ...state, capabilities });
}

/** The engine's value for `key`, or undefined when it has none or no settings at all. */
export function getEngineSetting(key: string): unknown {
  return state.values[key];
}

export function engineHasSettings(): boolean {
  return state.settings;
}

export function useEngineSetting(key: string): { supported: boolean; value: unknown } {
  const supported = useSyncExternalStore(subscribe, () => state.settings);
  const value = useSyncExternalStore(subscribe, () => state.values[key]);
  return { supported, value };
}

/**
 * Writes `values` to the engine, showing them at once; the old values come back if the
 * engine refuses (a device without the admin scope may set only some keys).
 */
export async function setEngineSettings(client: EngineClient, values: SettingsValues) {
  const before = state.values;
  update({ ...state, values: merge(before, values) });
  try {
    await client.call('settings.set', { values });
  } catch (err) {
    const reverted = { ...state.values };
    for (const k of Object.keys(values)) {
      if (k in before) reverted[k] = before[k];
      else delete reverted[k];
    }
    update({ ...state, values: reverted });
    throw err;
  }
}

export function getEngineCapabilities(): EngineCapabilities | null {
  return state.capabilities;
}

export function useEngineCapabilities(): EngineCapabilities | null {
  return useSyncExternalStore(subscribe, () => state.capabilities);
}
