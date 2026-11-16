// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
The engine's mail intents and undo log as the windows use them, each window on its own
engine connection.

- `runIntent` calls a `threads.*` or `categories.*` intent: the engine builds every account's
  tasks (Gmail labels or folders, the archive fallbacks, the copies the view shows), queues
  them and keeps an undo entry. TaskFactory and the perspectives hand these out as IntentTask,
  which EngineBridge runs here when it is queued.
- `undoApply`, `undoRedo`, `undoList` and `undoRecord` are the log (`undo.*`); UndoRedoStore
  follows it through `undo.changed`. Entries belong to the `hello.client` name up to its `/`,
  `airpost-desktop` for every window.
- `createCategory` is `categories.create`, resolving with the new folder or label.
- `snoozeThreads` is `threads.snooze`: the move and the metadata that brings the threads back.
*/
import EngineEvents, { EngineEvents as EngineEventsType } from './events';
import {
  Feature,
  IntentResult,
  Method,
  ThreadsSnoozeResult,
  UndoApplyResult,
  UndoEntry,
  UndoListResult,
  UndoRecordResult,
} from '../protocol';
import { localTimeZone } from './drafts';
import TaskQueue from '../../data/stores/task-queue';
import { Task } from '../../data/tasks/task';

// The client name of this app's entries in the log.
export const OWN_CLIENT = 'airpost-desktop';

async function call<R>(engine: EngineEventsType, feature: string, method: string, params: unknown) {
  await engine.whenConnected();
  if (!engine.has(feature)) {
    throw new Error(`The sync engine has no ${method}; update Airpost.`);
  }
  return engine.request<R>(method, params);
}

// The message of an engine error, without the JSON-RPC prefix.
export function intentErrorMessage(err: unknown): string {
  const e = err as { data?: { message?: string }; message?: string };
  const message = (e && ((e.data && e.data.message) || e.message)) || String(err);
  return message.replace(/^invalid params: /, '');
}

export function runIntent(
  method: string,
  params: Record<string, unknown>,
  engine: EngineEventsType = EngineEvents
): Promise<IntentResult> {
  return call<IntentResult>(engine, Feature.Intents, method, params);
}

// `categories.create`: the new folder or label once the server made it (the finished task's
// `created`), or an Error with the engine's or the server's reason.
export async function createCategory(
  accountId: string,
  name: string,
  engine: EngineEventsType = EngineEvents
): Promise<{ id: string; [key: string]: unknown }> {
  const { taskIds } = await runIntent(Method.CategoriesCreate, { accountId, name }, engine);
  const finished = await TaskQueue.waitForPerformRemote({ id: taskIds[0] } as Task);
  const created = (finished as Task & { created?: { id: string } }).created;
  if (!created) {
    throw new Error(finished.error ? String((finished.error as any).message || '') : '');
  }
  return created;
}

export function snoozeThreads(
  { accountId, threadIds, until }: { accountId: string; threadIds: string[]; until: number },
  engine: EngineEventsType = EngineEvents
): Promise<ThreadsSnoozeResult> {
  return call<ThreadsSnoozeResult>(engine, Feature.Snooze, Method.ThreadsSnooze, {
    accountId,
    threadIds,
    until,
  });
}

// With a token, that entry; without, this app's latest one.
export function undoApply(
  token?: string,
  engine: EngineEventsType = EngineEvents
): Promise<UndoApplyResult> {
  return call<UndoApplyResult>(engine, Feature.Undo, Method.UndoApply, {
    token,
    tz: localTimeZone(),
  });
}

export function undoRedo(
  token?: string,
  engine: EngineEventsType = EngineEvents
): Promise<UndoApplyResult> {
  return call<UndoApplyResult>(engine, Feature.Undo, Method.UndoRedo, {
    token,
    tz: localTimeZone(),
  });
}

export async function undoList(engine: EngineEventsType = EngineEvents): Promise<UndoEntry[]> {
  const { entries } = await call<UndoListResult>(engine, Feature.Undo, Method.UndoList, {
    mine: true,
  });
  return entries;
}

// Tasks this window queued itself, after their local phases, as one entry.
export async function undoRecord(
  tasks: Array<{ accountId: string; id: string }>,
  label: string | undefined,
  engine: EngineEventsType = EngineEvents
): Promise<string | null> {
  const { token } = await call<UndoRecordResult>(engine, Feature.Undo, Method.UndoRecord, {
    tasks: tasks.map((t) => ({ accountId: t.accountId, taskId: t.id })),
    label: label || undefined,
  });
  return token || null;
}
