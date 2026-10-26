// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import * as Attributes from '../attributes';
import { AttributeValues } from '../models/model';
import { Thread } from '../models/thread';
import { Task } from './task';
import { localized } from '../../intl/localize';
import type { Perspective } from '../../engine/protocol';

// What an intent's `perspective` reads of the list the user acted from: the categories it
// shows and whether it is the starred or the unread view. A plain object without
// `categories()` (a v1 plugin's) gives none.
export interface PerspectiveLike {
  categories?: () => Array<{ id: string }>;
  starred?: boolean;
  unread?: boolean;
}

export function perspectiveParams(perspective?: PerspectiveLike | null): Perspective | undefined {
  if (!perspective || typeof perspective.categories !== 'function') return undefined;
  const p: Perspective = {};
  const categoryIds = perspective.categories().map((c) => c.id);
  if (categoryIds.length) p.categoryIds = categoryIds;
  if (perspective.starred) p.starred = true;
  if (perspective.unread) p.unread = true;
  return p;
}

// The undo toast's wording for an intent the engine's entry can't name (it has the ids it
// was given): `text` with `%@` for "N threads", dropped for one.
export function withThreadCount(text: string, count: number) {
  return count > 1
    ? text.replace('%@', `${count} ${localized('threads')}`)
    : text.replace('%@ ', '').replace(' %@', '');
}

export function describeMove(count: number, name: string) {
  return count > 1
    ? localized(`Moved %@ threads to %@`, count, name)
    : localized(`Moved to %@`, name);
}

export function describeLabel(count: number, name: string, added: boolean) {
  if (added) {
    return count > 1
      ? localized(`Added %@ to %@ threads`, name, count)
      : localized(`Added %@`, name);
  }
  return count > 1
    ? localized(`Removed %@ from %@ threads`, name, count)
    : localized(`Removed %@`, name);
}

/*
Public: One of the engine's mail intents (`threads.*`, `categories.*`) as a task, so
`Actions.queueTask(s)` keeps working for what TaskFactory and the perspectives return. The
engine never stores it: EngineBridge calls `intent` with `params` when it is queued in the
main window (from any window, through the action bridge), and the engine queues the tasks
that do the change and keeps its undo entry (`undo.changed`, UndoRedoStore).

`threadIds` are the threads it acts on, for the stores that follow queued tasks (the unread
view keeps a thread the user just read). The tasks it queues arrive as Task deltas like any
other; waiting on the IntentTask itself (`TaskQueue.waitForPerformRemote`) never resolves.
*/
export class IntentTask extends Task {
  static attributes = {
    ...Task.attributes,

    intent: Attributes.String({
      modelKey: 'intent',
    }),
    params: Attributes.Obj({
      modelKey: 'params',
    }),
    threadIds: Attributes.Collection({
      modelKey: 'threadIds',
    }),
    taskDescription: Attributes.String({
      modelKey: 'taskDescription',
    }),
  };

  declare intent: string;
  declare params: Record<string, unknown>;
  declare threadIds: string[];
  declare taskDescription: string;

  constructor(data: AttributeValues<typeof IntentTask.attributes> = {}) {
    super(data);
    this.params = this.params || {};
    this.threadIds = this.threadIds || [];
  }

  // A `threads.*` intent on `threads` (of any accounts).
  static forThreads(
    intent: string,
    threads: Thread[],
    params: Record<string, unknown> = {},
    source = '',
    description?: string
  ) {
    const threadIds = threads.map((t) => t.id);
    return new IntentTask({
      intent,
      params: { ...params, threadIds },
      threadIds,
      accountId: threads.length ? threads[0].accountId : undefined,
      source,
      taskDescription: description,
    });
  }

  description() {
    return this.taskDescription || '';
  }

  willBeQueued() {
    if (!this.intent) {
      throw new Error('IntentTask: no intent');
    }
    if (this.intent.startsWith('threads.') && this.threadIds.length === 0) {
      throw new Error('IntentTask: You must provide `threads`.');
    }
    super.willBeQueued();
  }
}
