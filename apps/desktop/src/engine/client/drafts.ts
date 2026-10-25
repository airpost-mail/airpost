// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The draft pipeline as the windows use it,
each window on its own engine connection.

- `buildDraft` asks the engine to build a new, reply, forward or mailto draft (`draft.create`
  with `save: false`): the same quoting, signature and tracking-image rules the engine uses for
  drafts the CLI, MCP and web create. DraftStore saves it through
  `_finalizeAndPersistNewMessage`, which runs the plugins' `prepareNewDraft` hooks.
- `sendDraft` sends a saved draft (`draft.send`), with the window's `applyTransformsForSending`
  output as `bodies`. With a delay the engine keeps the bodies and the undo window itself, so
  the send goes out even when the app quits inside it; `cancelSend` calls it back.
*/
import EngineEvents, { EngineEvents as EngineEventsType } from './events';
import {
  DraftBodies,
  DraftCreateParams,
  DraftSendParams,
  DraftSendResult,
  Method,
} from '../protocol';
import { Message } from '../../data/models/message';
import { Task } from '../../data/tasks/task';
import { SyncbackMetadataTask } from '../../data/tasks/syncback-metadata';

export function localTimeZone(): string | undefined {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || undefined;
  } catch (err) {
    return undefined;
  }
}

export async function buildDraft(
  params: DraftCreateParams,
  engine: EngineEventsType = EngineEvents
): Promise<Message> {
  await engine.whenConnected();
  const json = await engine.request<any>(Method.DraftCreate, {
    tz: localTimeZone(),
    ...params,
    save: false,
  });
  // The engine's id is the one a save gives it, so the draft starts without one;
  // SyncbackDraftTask assigns it again.
  return new Message({}).fromJSON({ ...json, id: undefined });
}

export function sendDraft(
  params: DraftSendParams,
  engine: EngineEventsType = EngineEvents
): Promise<DraftSendResult> {
  return engine.request<DraftSendResult>(Method.DraftSend, params);
}

export async function cancelSend(
  { accountId, headerMessageId }: { accountId: string; headerMessageId: string },
  engine: EngineEventsType = EngineEvents
): Promise<boolean> {
  const result = await engine.request<{ cancelled: boolean }>(Method.DraftCancelSend, {
    accountId,
    headerMessageId,
  });
  return !!(result && result.cancelled);
}

// The draft an Undo Send entry's task calls back: the send-later metadata write DraftStore
// registers for a send with an undo window. The task reaches the main window's UndoRedoStore
// through the action bridge as JSON, so it is recognised by its fields.
export function undoSendTarget(task: Task): { accountId: string; headerMessageId: string } | null {
  if (!(task instanceof SyncbackMetadataTask)) return null;
  const value = task.value as { expiration?: unknown; isUndoSend?: boolean } | null;
  if (task.pluginId !== 'send-later' || !value || !value.isUndoSend || !value.expiration) {
    return null;
  }
  if (!task.modelHeaderMessageId) return null;
  return { accountId: task.accountId, headerMessageId: task.modelHeaderMessageId };
}

// `bodies` from a SendDraftTask.forSending task: the transformed body, and each recipient's
// when an extension asked for separate bodies.
export function bodiesFromSendTask(task: {
  draft: Message;
  perRecipientBodies?: { [email: string]: string };
}): DraftBodies {
  const per = task.perRecipientBodies;
  if (!per) return { self: task.draft.body };
  const { self, ...perRecipient } = per;
  return { self: self !== undefined ? self : task.draft.body, perRecipient };
}
