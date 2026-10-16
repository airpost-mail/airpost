// The "Move to" / "Label" picker is one dialog for the whole mailbox; the toolbar, the
// reader and the `v` / `l` shortcuts all open it here.
import { useSyncExternalStore } from 'react';
import type { CategoryJSON, ThreadJSON } from '@/engine';

export interface PickerRequest {
  kind: 'move' | 'label';
  threads: ThreadJSON[];
  /** The category being viewed, so a Gmail "move" can drop it. */
  viewing?: CategoryJSON;
}

let current: PickerRequest | null = null;
const listeners = new Set<() => void>();

function set(next: PickerRequest | null) {
  current = next;
  for (const l of listeners) l();
}

export function openCategoryPicker(req: PickerRequest) {
  if (req.threads.length > 0) set(req);
}

export function closeCategoryPicker() {
  set(null);
}

export function useCategoryPicker(): PickerRequest | null {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => current,
  );
}
