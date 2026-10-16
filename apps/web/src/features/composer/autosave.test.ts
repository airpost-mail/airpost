import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createAutosave, type AutosaveStatus } from './autosave';

describe('createAutosave', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function setup(delayMs = 1000, maxWaitMs = 5000) {
    const saved: string[] = [];
    const statuses: AutosaveStatus[] = [];
    const save = vi.fn(async (v: string) => {
      saved.push(v);
    });
    const autosave = createAutosave({
      save,
      delayMs,
      maxWaitMs,
      onStatus: (s) => statuses.push(s),
    });
    return { autosave, save, saved, statuses };
  }

  it('collapses a burst of edits into one save of the last value', async () => {
    const { autosave, saved } = setup();
    autosave.schedule('a');
    await vi.advanceTimersByTimeAsync(400);
    autosave.schedule('ab');
    await vi.advanceTimersByTimeAsync(400);
    autosave.schedule('abc');
    await vi.advanceTimersByTimeAsync(999);
    expect(saved).toEqual([]);
    await vi.advanceTimersByTimeAsync(1);
    expect(saved).toEqual(['abc']);
    expect(autosave.status).toBe('saved');
  });

  it('saves at maxWait while edits keep coming', async () => {
    const { autosave, saved } = setup(1000, 3000);
    for (let i = 0; i < 8; i++) {
      autosave.schedule(`v${i}`);
      await vi.advanceTimersByTimeAsync(500);
    }
    // First save lands at 3000ms (v5 was the newest then); the rest are still debouncing.
    expect(saved).toEqual(['v5']);
    await vi.advanceTimersByTimeAsync(1000);
    expect(saved).toEqual(['v5', 'v7']);
  });

  it('flush saves immediately and cancel drops the pending value', async () => {
    const { autosave, saved } = setup();
    autosave.schedule('x');
    await autosave.flush();
    expect(saved).toEqual(['x']);

    autosave.schedule('y');
    await autosave.cancel();
    await vi.advanceTimersByTimeAsync(5000);
    expect(saved).toEqual(['x']);
  });

  it('runs saves one at a time and reports errors', async () => {
    let release!: () => void;
    const calls: string[] = [];
    const statuses: AutosaveStatus[] = [];
    const autosave = createAutosave<string>({
      delayMs: 100,
      save: (v) => {
        calls.push(`start ${v}`);
        if (v === 'bad') return Promise.reject(new Error('nope'));
        return new Promise<void>((r) => {
          release = () => {
            calls.push(`end ${v}`);
            r();
          };
        });
      },
      onStatus: (s) => statuses.push(s),
    });
    autosave.schedule('one');
    await vi.advanceTimersByTimeAsync(100);
    autosave.schedule('two');
    await vi.advanceTimersByTimeAsync(100);
    expect(calls).toEqual(['start one']);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(calls).toEqual(['start one', 'end one', 'start two']);
    release();
    await vi.advanceTimersByTimeAsync(0);

    autosave.schedule('bad');
    await autosave.flush();
    expect(autosave.status).toBe('error');
    expect(statuses).toContain('saving');
  });
});
