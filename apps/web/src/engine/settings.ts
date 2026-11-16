// The engine's settings (DCP stage 3) and capabilities (DCP stage 5) as this tab last heard
// them. EngineProvider fills both on every connect (`settings.get`, `hello.capabilities`)
// and keeps them current from `settings.changed` and `capabilities.changed`; the values are
// `@airpost/client`'s SettingsCache, and nothing is cached across tabs or reloads.
import { useSyncExternalStore } from 'react';
import { SettingsCache, type SettingsCaller } from '@airpost/client';
import type { EngineClient } from './client';
import { type EngineCapabilities, type HelloResult, type SettingsValues } from './rpc';

/** Settings keys the web reads or writes. */
export const SettingKey = {
  /** Milliseconds; 0 sends at once. */
  UndoSend: 'core.sending.undoSend',
} as const;

const cache = new SettingsCache(Object.values(SettingKey));
let capabilities: EngineCapabilities | null = null;
const capabilityListeners = new Set<() => void>();

function setCapabilities(next: EngineCapabilities | null) {
  capabilities = next;
  for (const l of capabilityListeners) l();
}

function subscribe(cb: () => void) {
  const off = cache.subscribe(cb);
  capabilityListeners.add(cb);
  return () => {
    off();
    capabilityListeners.delete(cb);
  };
}

function caller(client: EngineClient): SettingsCaller {
  return client;
}

/** For tests: forget what the last engine said. */
export function resetEngineState() {
  cache.reset();
  setCapabilities(null);
}

/** Reads what the engine has after a handshake. */
export async function loadEngineState(client: EngineClient, hello: HelloResult) {
  setCapabilities(hello.capabilities ?? null);
  // An unreadable store leaves the cache empty: this browser's values apply until the next
  // connect.
  await cache.load(caller(client));
}

/** `settings.changed`: another connection's change (the desktop's, or another tab's). */
export function applySettingsChanged(changes: SettingsValues) {
  cache.apply(changes);
}

export function applyCapabilitiesChanged(next: EngineCapabilities) {
  setCapabilities(next);
}

/** The engine's value for `key`, or undefined when it has none or no settings at all. */
export function getEngineSetting(key: string): unknown {
  return cache.get(key);
}

export function engineHasSettings(): boolean {
  return cache.loaded;
}

export function useEngineSetting(key: string): { supported: boolean; value: unknown } {
  const supported = useSyncExternalStore(subscribe, () => cache.loaded);
  const value = useSyncExternalStore(subscribe, () => cache.get(key));
  return { supported, value };
}

/**
 * Writes `values` to the engine, showing them at once; the old values come back if the
 * engine refuses (a device without the admin scope may set only some keys).
 */
export function setEngineSettings(client: EngineClient, values: SettingsValues): Promise<void> {
  return cache.set(caller(client), values);
}

export function getEngineCapabilities(): EngineCapabilities | null {
  return capabilities;
}

export function useEngineCapabilities(): EngineCapabilities | null {
  return useSyncExternalStore(subscribe, () => capabilities);
}
