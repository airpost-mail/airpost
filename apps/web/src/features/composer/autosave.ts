// Debounced draft saving. Saves run one at a time and always carry the newest value; a
// burst of edits collapses into one save, and `maxWaitMs` bounds how stale the saved copy
// can get while someone types without pausing.

export type AutosaveStatus = 'idle' | 'pending' | 'saving' | 'saved' | 'error';

export interface AutosaveOptions<T> {
  save: (value: T) => Promise<unknown>;
  delayMs?: number;
  maxWaitMs?: number;
  onStatus?: (status: AutosaveStatus, error?: unknown) => void;
}

export interface Autosave<T> {
  /** Records a new value and (re)starts the debounce. */
  schedule(value: T): void;
  /** Saves the pending value now. Resolves when every save so far has finished. */
  flush(): Promise<void>;
  /** Drops the pending value. Resolves when an in-flight save has finished. */
  cancel(): Promise<void>;
  readonly status: AutosaveStatus;
}

export const AUTOSAVE_DELAY_MS = 1500;
export const AUTOSAVE_MAX_WAIT_MS = 10_000;

export function createAutosave<T>({
  save,
  delayMs = AUTOSAVE_DELAY_MS,
  maxWaitMs = AUTOSAVE_MAX_WAIT_MS,
  onStatus,
}: AutosaveOptions<T>): Autosave<T> {
  let pending: { value: T } | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let firstPendingAt = 0;
  let chain: Promise<void> = Promise.resolve();
  let status: AutosaveStatus = 'idle';

  const setStatus = (s: AutosaveStatus, error?: unknown) => {
    status = s;
    onStatus?.(s, error);
  };

  const clearTimer = () => {
    if (timer) clearTimeout(timer);
    timer = null;
  };

  const run = (): Promise<void> => {
    clearTimer();
    if (!pending) return chain;
    const { value } = pending;
    pending = null;
    chain = chain.then(async () => {
      setStatus('saving');
      try {
        await save(value);
        setStatus(pending ? 'pending' : 'saved');
      } catch (err) {
        setStatus('error', err);
      }
    });
    return chain;
  };

  return {
    schedule(value) {
      const now = Date.now();
      if (!pending) firstPendingAt = now;
      pending = { value };
      setStatus('pending');
      clearTimer();
      const wait = Math.max(0, Math.min(delayMs, firstPendingAt + maxWaitMs - now));
      timer = setTimeout(() => void run(), wait);
    },
    flush: run,
    cancel() {
      clearTimer();
      pending = null;
      return chain;
    },
    get status() {
      return status;
    },
  };
}
