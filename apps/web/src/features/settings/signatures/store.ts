// Signatures, in the desktop's shape (signature-store.ts): HTML bodies keyed by id, and a
// default signature per account email address.
// TODO(engine): signatures live in this browser only. The engine keeps `signatures` and
// `defaultSignatures` (DCP stage 3), but a device without the admin scope may set only
// `defaultSignatures`: the signature HTML is rendered by the desktop, so `signatures` joins
// ms-ipc `RELAYED_SETTINGS_KEYS` only once the desktop treats web-written HTML as untrusted.
import { useSyncExternalStore } from 'react';
import { generateId } from '@/engine';

export interface Signature {
  id: string;
  title: string;
  /** HTML. */
  body: string;
}

export interface SignaturesState {
  signatures: Record<string, Signature>;
  /** Account email address to signature id. */
  defaults: Record<string, string>;
}

const STORAGE_KEY = 'tern.signatures';
const EMPTY: SignaturesState = { signatures: {}, defaults: {} };

function load(): SignaturesState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return EMPTY;
    const s = JSON.parse(raw) as Partial<SignaturesState>;
    return { signatures: s.signatures ?? {}, defaults: s.defaults ?? {} };
  } catch {
    return EMPTY;
  }
}

const listeners = new Set<() => void>();
let state: SignaturesState = load();

function commit(next: SignaturesState) {
  state = next;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Keep the in-memory value.
  }
  for (const l of listeners) l();
}

export function getSignatures(): SignaturesState {
  return state;
}

export function useSignatures(): SignaturesState {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => state,
  );
}

export function createSignature(title = 'Untitled'): Signature {
  const sig: Signature = { id: generateId(), title, body: '' };
  commit({ ...state, signatures: { ...state.signatures, [sig.id]: sig } });
  return sig;
}

export function updateSignature(id: string, patch: Partial<Omit<Signature, 'id'>>) {
  const sig = state.signatures[id];
  if (!sig) return;
  commit({ ...state, signatures: { ...state.signatures, [id]: { ...sig, ...patch } } });
}

export function removeSignature(id: string) {
  const signatures = { ...state.signatures };
  delete signatures[id];
  const defaults = Object.fromEntries(Object.entries(state.defaults).filter(([, v]) => v !== id));
  commit({ signatures, defaults });
}

/** Makes `id` the default for `email`, or clears the default with null. */
export function setDefaultSignature(email: string, id: string | null) {
  const defaults = { ...state.defaults };
  if (id) defaults[email] = id;
  else delete defaults[email];
  commit({ ...state, defaults });
}

/** The default signature for an account email, for the composer. */
export function signatureForEmail(email: string, s: SignaturesState = state): Signature | null {
  const id = s.defaults[email];
  return (id && s.signatures[id]) || null;
}

/** For tests. */
export function resetSignatures() {
  state = load();
  for (const l of listeners) l();
}
