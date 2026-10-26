// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
Waiting for a queued task to finish. An engine with feature `task-done` tells the caller
with `task.done {accountId, taskId, status, error, task}` on the connection that queued it
with `task.queue {notify: true}`; otherwise a task is done when a `Task` delta shows it
`complete` or `cancelled`, or removes it, as crates/tern-client `tasks::finished` decides.
`waitForTask` takes either.
*/
import type { DeltaParams } from './protocol';
import type { IncomingNotification } from './rpc';

/** The `task.done` notification's method. */
export const TASK_DONE = 'task.done';
/** The class of the deltas a task wait follows until `task.done`. */
export const TASK_CLASS = 'Task';

/** How a task ended. */
export interface TaskDone {
  taskId: string;
  /** `complete` or `cancelled`; `removed` when a delta took it away first. */
  status: 'complete' | 'cancelled' | 'removed';
  /** The task's error, when it failed. */
  error: unknown;
  /** The stored task, when a delta carried it. */
  task?: Record<string, unknown>;
}

/** What the wait listens on; a Session, or a client with the same method. */
export interface NotificationSource {
  onNotification(listener: (n: IncomingNotification) => void): () => unknown;
}

/** The task's end in `n`, if `n` reports it. */
export function taskDoneIn(n: IncomingNotification, taskId: string): TaskDone | null {
  if (n.method === TASK_DONE) {
    const p = (n.params ?? {}) as { taskId?: string; status?: string; error?: unknown };
    if (p.taskId !== taskId) return null;
    return {
      taskId,
      status: p.status === 'cancelled' ? 'cancelled' : 'complete',
      error: p.error ?? null,
    };
  }
  if (n.method !== 'delta') return null;
  const d = n.params as DeltaParams;
  if (d.modelClass !== TASK_CLASS) return null;
  const task = (d.modelJSONs as Array<Record<string, unknown>>).find((t) => t.id === taskId);
  if (!task) return null;
  if (d.type === 'unpersist') return { taskId, status: 'removed', error: null, task };
  if (task.status !== 'complete' && task.status !== 'cancelled') return null;
  return { taskId, status: task.status, error: task.error ?? null, task };
}

/** Resolves when the task ends; rejects after `timeoutMs`. */
export function waitForTask(
  source: NotificationSource,
  taskId: string,
  timeoutMs = 60_000,
): Promise<TaskDone> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      off();
      reject(new Error(`task ${taskId} did not finish in ${timeoutMs} ms`));
    }, timeoutMs);
    const off = source.onNotification((n) => {
      const done = taskDoneIn(n, taskId);
      if (!done) return;
      clearTimeout(timer);
      off();
      resolve(done);
    });
  });
}
