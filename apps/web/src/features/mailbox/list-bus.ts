// Lets the reader's toolbar archive or trash the open thread through the thread list, so
// it advances to the next thread exactly as the `e` / `#` keys do. Snoozing goes the same way.
import type { ThreadJSON } from '@/engine';
import type { ThreadActions } from './actions';

export type RemoveAction = 'archive' | 'trash' | 'spam';
type Remove = (threads: ThreadJSON[]) => void;
type Remover = (remove: Remove, threads: ThreadJSON[]) => void;

let remover: Remover | null = null;

export function registerListRemover(fn: Remover): () => void {
  remover = fn;
  return () => {
    if (remover === fn) remover = null;
  };
}

/** `action` is one of the thread actions, or any call that takes the threads out of the list. */
export function removeThreads(
  action: RemoveAction | Remove,
  threads: ThreadJSON[],
  fallback: ThreadActions,
) {
  const remove: Remove = typeof action === 'function' ? action : (ts) => fallback[action](ts);
  if (remover) remover(remove, threads);
  else remove(threads);
}
