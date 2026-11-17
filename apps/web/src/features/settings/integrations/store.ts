// Integration settings. Non-secret fields (enabled, URLs, usernames) persist in localStorage;
// secrets are held in memory for this tab and handed to the engine.
import { useSyncExternalStore } from 'react';
import { PROVIDERS, providerDef, type FieldKey, type ProviderId } from './providers';

export interface ProviderConfig {
  enabled: boolean;
  /** Non-secret field values. */
  values: Partial<Record<FieldKey, string>>;
}

export interface IntegrationsState {
  providers: Record<ProviderId, ProviderConfig>;
  /** Which secret fields the engine holds a value for. */
  secrets: Record<ProviderId, FieldKey[]>;
}

const STORAGE_KEY = 'airpost.integrations';

function defaults(): IntegrationsState {
  const providers = {} as Record<ProviderId, ProviderConfig>;
  const secrets = {} as Record<ProviderId, FieldKey[]>;
  for (const p of PROVIDERS) {
    const values: ProviderConfig['values'] = {};
    for (const f of p.fields) if (!f.secret && f.defaultValue) values[f.key] = f.defaultValue;
    providers[p.id] = { enabled: false, values };
    secrets[p.id] = [];
  }
  return { providers, secrets };
}

function load(): IntegrationsState {
  const base = defaults();
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return base;
    const saved = JSON.parse(raw) as Partial<Record<ProviderId, Partial<ProviderConfig>>>;
    for (const p of PROVIDERS) {
      const s = saved[p.id];
      if (!s) continue;
      base.providers[p.id] = {
        enabled: !!s.enabled,
        values: { ...base.providers[p.id].values, ...stripSecrets(p.id, s.values ?? {}) },
      };
    }
  } catch {
    // Corrupt or unavailable storage: start from defaults.
  }
  return base;
}

function stripSecrets(id: ProviderId, values: ProviderConfig['values']): ProviderConfig['values'] {
  const out: ProviderConfig['values'] = {};
  for (const f of providerDef(id).fields) {
    const v = values[f.key];
    if (!f.secret && v !== undefined) out[f.key] = v;
  }
  return out;
}

/**
 * Secret field values per provider, held in this tab's memory only: sent per request and
 * lost on reload.
 *
 * TODO(engine): secrets belong in the engine, which has `secrets.set {name:
 * "integration:<provider>"}` for a device paired with the admin scope; send them there and
 * keep only the `secrets` flags.
 */
const secretValues = new Map<ProviderId, Partial<Record<FieldKey, string>>>();

const listeners = new Set<() => void>();
let state: IntegrationsState = load();

function commit(next: IntegrationsState) {
  state = next;
  try {
    const persisted = Object.fromEntries(
      PROVIDERS.map((p) => [
        p.id,
        { enabled: next.providers[p.id].enabled, values: next.providers[p.id].values },
      ]),
    );
    localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
  } catch {
    // Keep the in-memory value.
  }
  for (const l of listeners) l();
}

export function getIntegrations(): IntegrationsState {
  return state;
}

export function subscribeIntegrations(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

export function useIntegrations(): IntegrationsState {
  return useSyncExternalStore(subscribeIntegrations, getIntegrations);
}

/** Saves a provider's fields. Secret fields are split off; an empty secret leaves it unchanged. */
export function saveProvider(id: ProviderId, values: Partial<Record<FieldKey, string>>) {
  const def = providerDef(id);
  const plain: ProviderConfig['values'] = {};
  const secret = { ...secretValues.get(id) };
  for (const f of def.fields) {
    const v = values[f.key]?.trim();
    if (v === undefined) continue;
    if (f.secret) {
      if (v) secret[f.key] = v;
    } else {
      plain[f.key] = v;
    }
  }
  secretValues.set(id, secret);
  commit({
    providers: { ...state.providers, [id]: { ...state.providers[id], values: plain } },
    secrets: { ...state.secrets, [id]: Object.keys(secret) as FieldKey[] },
  });
}

export function clearSecret(id: ProviderId, key: FieldKey) {
  const secret = { ...secretValues.get(id) };
  delete secret[key];
  secretValues.set(id, secret);
  commit({ ...state, secrets: { ...state.secrets, [id]: Object.keys(secret) as FieldKey[] } });
}

/** Turns a provider on or off. Only one provider per kind is on at a time. */
export function setProviderEnabled(id: ProviderId, enabled: boolean) {
  const kind = providerDef(id).kind;
  const providers = { ...state.providers };
  for (const p of PROVIDERS) {
    if (p.id === id) providers[p.id] = { ...providers[p.id], enabled };
    else if (enabled && p.kind === kind && providers[p.id].enabled) {
      providers[p.id] = { ...providers[p.id], enabled: false };
    }
  }
  commit({ ...state, providers });
}

/** Every required field has a value (secrets: the engine holds one). */
export function isProviderConfigured(s: IntegrationsState, id: ProviderId): boolean {
  return providerDef(id).fields.every(
    (f) =>
      !f.required || (f.secret ? s.secrets[id].includes(f.key) : !!s.providers[id].values[f.key]),
  );
}

/** Request params for a provider: its fields, secrets included, with unsaved edits on top. */
export function providerParams(id: ProviderId, draft: Partial<Record<FieldKey, string>> = {}) {
  const merged: Record<string, string> = {};
  for (const [k, v] of Object.entries({ ...state.providers[id].values, ...secretValues.get(id) }))
    if (v) merged[k] = v;
  for (const [k, v] of Object.entries(draft)) if (v?.trim()) merged[k] = v.trim();
  return { provider: id, ...merged };
}

/** For tests. */
export function resetIntegrations() {
  secretValues.clear();
  state = load();
  for (const l of listeners) l();
}
