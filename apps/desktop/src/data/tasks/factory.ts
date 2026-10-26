import { ChangeUnreadTask } from './change-unread';
import { Thread } from '../models/thread';
import { Task } from './task';
import { IntentTask, perspectiveParams, PerspectiveLike } from './intent';
import { Method } from '../../engine/protocol';

/*
The engine's intents for the common thread actions, as IntentTasks to queue with
`Actions.queueTask(s)`. The engine decides what each means on each account (Gmail removes the
Inbox label to archive, Not Spam moves only the Spam copies, a move takes the copies the view
shows), builds every account's tasks before it queues any and keeps one undo entry per call.
The methods return arrays as they did when they built a task per account; a call is one task
for threads of any accounts.
*/
export const TaskFactory = {
  tasksForThreadsByAccountId(
    threads: Thread[],
    callback: (accountThreads: Thread[], accountId: string) => Task | Task[] | null
  ) {
    const byAccount: { [accountId: string]: { accountThreads: Thread[]; accountId: string } } = {};
    threads.forEach((thread) => {
      if (!(thread instanceof Thread)) {
        throw new Error('tasksForThreadsByAccountId: `threads` must be instances of Thread');
      }
      const { accountId } = thread;
      if (!byAccount[accountId]) {
        byAccount[accountId] = { accountThreads: [], accountId: accountId };
      }
      byAccount[accountId].accountThreads.push(thread);
    });

    const tasks: Task[] = [];
    Object.values(byAccount).forEach(({ accountThreads, accountId }) => {
      const taskOrTasks = callback(accountThreads, accountId);
      if (Array.isArray(taskOrTasks)) {
        tasks.push(...taskOrTasks);
      } else if (taskOrTasks) {
        tasks.push(taskOrTasks);
      }
    });
    return tasks;
  },

  _intent(
    intent: string,
    threads: Thread[],
    source: string,
    params: Record<string, unknown> = {}
  ): Task[] {
    return threads.length ? [IntentTask.forThreads(intent, threads, params, source)] : [];
  },

  tasksForMarkingAsSpam({ threads, source }: { threads: Thread[]; source: string }): Task[] {
    return this._intent(Method.ThreadsSpam, threads, source);
  },

  tasksForMarkingNotSpam({ threads, source }: { threads: Thread[]; source: string }): Task[] {
    return this._intent(Method.ThreadsNotSpam, threads, source);
  },

  // Pass the perspective the user archived from so only the copies it shows move; a
  // self-sent message archived from Sent otherwise loses its Inbox copy instead. Callers
  // with no view (notifications, send-and-archive) omit it and get every copy outside Sent
  // and Drafts.
  tasksForArchiving({
    threads,
    source,
    perspective,
  }: {
    threads: Thread[];
    source: string;
    perspective?: PerspectiveLike;
  }): Task[] {
    const p = perspectiveParams(perspective);
    return this._intent(Method.ThreadsArchive, threads, source, p ? { perspective: p } : {});
  },

  tasksForMovingToTrash({ threads, source }: { threads: Thread[]; source: string }): Task[] {
    return this._intent(Method.ThreadsTrash, threads, source);
  },

  taskForInvertingUnread({
    threads,
    source,
    canBeUndone,
  }: {
    threads: Thread[];
    source: string;
    canBeUndone?: boolean;
  }): Task {
    const unread = threads.every((t) => t.unread === false);
    return this.taskForSettingUnread({ threads, unread, source, canBeUndone });
  },

  // `canBeUndone: false` (marking a thread read as it opens) stays a ChangeUnreadTask: the
  // intent would add an undo entry and its toast.
  taskForSettingUnread({
    threads,
    unread,
    source,
    canBeUndone,
  }: {
    threads: Thread[];
    source: string;
    unread: boolean;
    canBeUndone?: boolean;
  }): Task {
    if (canBeUndone === false) {
      return new ChangeUnreadTask({ threads, unread, source, canBeUndone });
    }
    return IntentTask.forThreads(Method.ThreadsMarkRead, threads, unread ? { unread } : {}, source);
  },

  taskForInvertingStarred({ threads, source }: { threads: Thread[]; source: string }): Task {
    const starred = threads.every((t) => t.starred === false);
    return IntentTask.forThreads(Method.ThreadsStar, threads, { starred }, source);
  },
};
