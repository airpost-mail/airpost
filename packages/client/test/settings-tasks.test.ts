// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsCache, waitForTask, type IncomingNotification } from '../src';

describe('SettingsCache', () => {
  const caller = (values: Record<string, unknown>, fail = false) => ({
    call: vi.fn(async (method: string) => {
      if (fail) throw new Error('refused');
      return method === 'settings.get' ? { values } : {};
    }),
  });

  it('loads, applies changes, and ignores changes before a load', async () => {
    const cache = new SettingsCache(['a', 'b']);
    cache.apply({ a: 1 });
    expect(cache.get('a')).toBeUndefined();
    const c = caller({ a: 1, b: 2 });
    await cache.load(c);
    expect(c.call).toHaveBeenCalledWith('settings.get', { keys: ['a', 'b'] });
    expect(cache.loaded).toBe(true);
    cache.apply({ a: 3, b: null });
    expect(cache.values).toEqual({ a: 3 });
  });

  it('shows a write at once and reverts it when refused', async () => {
    const cache = new SettingsCache();
    await cache.load(caller({ a: 1 }));
    const seen: unknown[] = [];
    cache.subscribe(() => seen.push(cache.get('a')));
    await expect(cache.set(caller({}, true), { a: 2, b: 3 })).rejects.toThrow('refused');
    expect(seen).toEqual([2, 1]);
    expect(cache.values).toEqual({ a: 1 });
  });

  it('is empty and unloaded after a failed load', async () => {
    const cache = new SettingsCache();
    await cache.load(caller({ a: 1 }));
    await expect(cache.load(caller({}, true))).rejects.toThrow();
    expect(cache.loaded).toBe(false);
    expect(cache.values).toEqual({});
  });
});

describe('waitForTask', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const source = () => {
    const listeners = new Set<(n: IncomingNotification) => void>();
    return {
      onNotification: (l: (n: IncomingNotification) => void) => {
        listeners.add(l);
        return () => listeners.delete(l);
      },
      push: (n: IncomingNotification) => [...listeners].forEach((l) => l(n)),
      size: () => listeners.size,
    };
  };
  const delta = (type: string, tasks: unknown[]) => ({
    method: 'delta',
    params: { accountId: 'a', modelClass: 'Task', type, modelJSONs: tasks },
  });

  it('resolves on task.done for its task', async () => {
    const s = source();
    const done = waitForTask(s, 't1');
    s.push({ method: 'task.done', params: { taskId: 't2', status: 'complete' } });
    s.push({ method: 'task.done', params: { taskId: 't1', status: 'cancelled', error: 'x' } });
    await expect(done).resolves.toEqual({ taskId: 't1', status: 'cancelled', error: 'x' });
    expect(s.size()).toBe(0);
  });

  it('resolves on a Task delta that shows it final or removes it', async () => {
    const s = source();
    const a = waitForTask(s, 't1');
    const b = waitForTask(s, 't2');
    s.push(delta('persist', [{ id: 't1', status: 'remote' }]));
    s.push(delta('persist', [{ id: 't1', status: 'complete', error: null }]));
    s.push(delta('unpersist', [{ id: 't2' }]));
    await expect(a).resolves.toMatchObject({ status: 'complete', error: null });
    await expect(b).resolves.toMatchObject({ status: 'removed' });
  });

  it('gives up after the timeout', async () => {
    const s = source();
    const done = waitForTask(s, 't1', 1000);
    const failed = expect(done).rejects.toThrow('did not finish');
    await vi.advanceTimersByTimeAsync(1000);
    await failed;
    expect(s.size()).toBe(0);
  });
});
