// Lets the reader's toolbar archive or trash the open thread through the thread list, so
// it advances to the next thread exactly as the `e` / `#` keys do.
import type { ThreadJSON } from '@/engine';
import type { ThreadActions } from './actions';

export type RemoveAction = 'archive' | 'trash' | 'spam';
type Remover = (action: RemoveAction, threads: ThreadJSON[]) => void;

let remover: Remover | null = null;

export function registerListRemover(fn: Remover): () => void {
  remover = fn;
  return () => {
    if (remover === fn) remover = null;
  };
}

export function removeThreads(
  action: RemoveAction,
  threads: ThreadJSON[],
  fallback: ThreadActions,
) {
  if (remover) remover(action, threads);
  else fallback[action](threads);
}
