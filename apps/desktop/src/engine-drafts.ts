// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The draft pipeline as the windows use it (docs/desktop-client-plan.md 3.3 and 4.2, stage 13,
Feature.DraftPipeline), each window on its own engine connection.

- `buildDraft` asks the engine to build a new, reply, forward or mailto draft (`draft.create`
  with `save: false`): the same quoting, signature and tracking-image rules the engine uses for
  drafts the CLI, MCP and web create. DraftStore saves it as before, through
  `_finalizeAndPersistNewMessage`, which skips the built-in `prepareNewDraft` hooks the engine
  already applied (ENGINE_PREPARED_EXTENSIONS) and runs the plugins' own.
- `sendDraft` sends a saved draft (`draft.send`), with the window's `applyTransformsForSending`
  output as `bodies`. With a delay the engine keeps the bodies and the undo window itself, so
  the send goes out even when the app quits inside it; `cancelSend` calls it back.

Without the feature, DraftFactory and DraftStore keep their JavaScript paths.
*/
import EngineEvents, { EngineEvents as EngineEventsType } from './engine-events';
import {
  DraftBodies,
  DraftCreateParams,
  DraftSendParams,
  DraftSendResult,
  Feature,
  Method,
} from './shared/engine-protocol';
import { Message } from './flux/models/message';
import { Task } from './flux/tasks/task';
import { SyncbackMetadataTask } from './flux/tasks/syncback-metadata-task';

// The built-in prepareNewDraft hooks `draft.create` already ran.
export const ENGINE_PREPARED_EXTENSIONS = [
  'SignatureComposerExtension',
  'TrackingPixelsComposerExtension',
];

const engineBuilt = new WeakSet<Message>();

export function draftPipelineAvailable(engine: EngineEventsType = EngineEvents) {
  return engine.has(Feature.DraftPipeline);
}

export function isEngineBuilt(draft: Message) {
  return engineBuilt.has(draft);
}

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
  const json = await engine.request<any>(Method.DraftCreate, {
    tz: localTimeZone(),
    ...params,
    save: false,
  });
  const draft = new Message({}).fromJSON(json);
  // The engine's id is the one a save gives it; SyncbackDraftTask assigns it again.
  draft.id = undefined;
  engineBuilt.add(draft);
  return draft;
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
