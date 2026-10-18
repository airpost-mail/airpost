// Thread actions shared by the list, the bulk toolbar and the reader. Each one splits the
// threads by account (unified views mix them) and queues one task per account; the cache
// updates optimistically in useQueueTask, so rows move at once.
import { useQueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';
import { toast } from 'sonner';
import {
  categoryDisplayName,
  engineKeys,
  ensureConnected,
  RpcError,
  useEngine,
  isGmailAccount,
  tasks,
  threadInCategory,
  useQueueTask,
  type AccountJSON,
  type CategoryJSON,
  type FolderJSON,
  type FoldersListResult,
  type LabelJSON,
  type LabelsListResult,
  type TaskJSON,
  type ThreadJSON,
} from '@/engine';
import { formatSnooze } from './snooze';

const isFolder = (c: CategoryJSON): c is FolderJSON => c.__cls === 'Folder';
const isLabel = (c: CategoryJSON): c is LabelJSON => c.__cls === 'Label';

function byAccount(threads: ThreadJSON[]): Map<string, ThreadJSON[]> {
  const out = new Map<string, ThreadJSON[]>();
  for (const t of threads) out.set(t.aid, [...(out.get(t.aid) ?? []), t]);
  return out;
}

const ids = (threads: ThreadJSON[]) => threads.map((t) => t.id);

export interface ThreadActions {
  archive(threads: ThreadJSON[]): void;
  trash(threads: ThreadJSON[]): void;
  /** Reports spam, or moves back to the inbox when every thread is already in spam. */
  spam(threads: ThreadJSON[]): void;
  setUnread(threads: ThreadJSON[], unread: boolean): void;
  setStarred(threads: ThreadJSON[], starred: boolean): void;
  toggleStarred(threads: ThreadJSON[]): void;
  /** Moves to a folder, or on Gmail to a label (dropping the label being viewed). */
  moveTo(threads: ThreadJSON[], target: CategoryJSON, viewing?: CategoryJSON): void;
  /** Adds the label, or removes it when every thread already has it. */
  toggleLabel(threads: ThreadJSON[], label: LabelJSON): void;
  removeLabel(threads: ThreadJSON[], label: LabelJSON): void;
  /**
   * The engine's `threads.snooze`: the move to Snoozed and the metadata that brings the
   * threads back, in one call. Only on engines with `EngineFeature.Snooze`.
   */
  snooze(threads: ThreadJSON[], until: Date): void;
  /** `threads.unsnooze`: back to the inbox now, the snooze cleared. */
  unsnooze(threads: ThreadJSON[]): void;
}

export function useThreadActions(): ThreadActions {
  const qc = useQueryClient();
  const { mutate } = useQueueTask();
  const engine = useEngine();

  return useMemo(() => {
    const account = (aid: string) =>
      qc.getQueryData<AccountJSON[]>(engineKeys.accounts())?.find((a) => a.id === aid);
    const categories = (aid: string): CategoryJSON[] => [
      ...(qc.getQueryData<FoldersListResult>(engineKeys.folders(aid))?.folders ?? []),
      ...(qc.getQueryData<LabelsListResult>(engineKeys.labels(aid))?.labels ?? []),
    ];
    const role = (aid: string, r: CategoryJSON['role']) =>
      categories(aid).find((c) => c.role === r);

    const queue = (task: TaskJSON) =>
      mutate(task, {
        onError: () => toast.error("Couldn't reach the engine. Nothing was changed."),
      });

    /** Runs `plan` per account; queues nothing if any account can't do it (plan returns why). */
    const perAccount = (
      threads: ThreadJSON[],
      plan: (aid: string, ts: ThreadJSON[]) => TaskJSON[] | string,
      done?: string,
    ) => {
      if (threads.length === 0) return;
      const planned: TaskJSON[] = [];
      for (const [aid, ts] of byAccount(threads)) {
        const result = plan(aid, ts);
        if (typeof result === 'string') {
          toast.error(result);
          return;
        }
        planned.push(...result);
      }
      planned.forEach(queue);
      if (done) toast(done);
    };

    /** One engine call per account; the moves arrive as deltas. */
    const perAccountCall = (
      threads: ThreadJSON[],
      run: (aid: string, ids: string[]) => Promise<unknown>,
      what: string,
      done: string,
    ) => {
      if (threads.length === 0) return;
      void ensureConnected(engine)
        .then(() => Promise.all([...byAccount(threads)].map(([aid, ts]) => run(aid, ids(ts)))))
        .then(
          () => toast(done),
          (err: unknown) =>
            toast.error(
              err instanceof RpcError
                ? `Couldn't ${what}: ${err.message}`
                : "Couldn't reach the engine. Nothing was changed.",
            ),
        );
    };

    const toFolder = (aid: string, ts: ThreadJSON[], folder: FolderJSON) =>
      tasks.changeFolder({ accountId: aid, threadIds: ids(ts), folder });

    const actions: ThreadActions = {
      archive: (threads) =>
        perAccount(
          threads,
          (aid, ts) => {
            const acct = account(aid);
            const inbox = role(aid, 'inbox');
            // Gmail archives by dropping the Inbox label (desktop TaskFactory.tasksForArchiving).
            if (acct && isGmailAccount(acct) && inbox && isLabel(inbox)) {
              return [
                tasks.changeLabels({ accountId: aid, threadIds: ids(ts), labelsToRemove: [inbox] }),
              ];
            }
            const target = role(aid, 'archive') ?? role(aid, 'all');
            if (!target || !isFolder(target))
              return `${acct?.label ?? 'This account'} has no archive folder.`;
            return [toFolder(aid, ts, target)];
          },
          threads.length === 1 ? 'Archived' : `Archived ${threads.length} threads`,
        ),

      trash: (threads) =>
        perAccount(
          threads,
          (aid, ts) => {
            const target = role(aid, 'trash');
            if (!target || !isFolder(target)) return 'This account has no trash folder.';
            return [toFolder(aid, ts, target)];
          },
          threads.length === 1 ? 'Moved to Trash' : `Moved ${threads.length} threads to Trash`,
        ),

      spam: (threads) => {
        const allSpam = threads.every((t) => {
          const spam = role(t.aid, 'spam');
          return spam && threadInCategory(t, spam.id);
        });
        perAccount(
          threads,
          (aid, ts) => {
            if (!allSpam) {
              const target = role(aid, 'spam');
              if (!target || !isFolder(target)) return 'This account has no spam folder.';
              return [toFolder(aid, ts, target)];
            }
            const inbox = role(aid, 'inbox');
            if (!inbox) return 'This account has no inbox.';
            if (isFolder(inbox)) return [toFolder(aid, ts, inbox)];
            if (!isLabel(inbox)) return 'This account has no inbox.';
            const all = role(aid, 'all');
            if (!all || !isFolder(all)) return 'This account has no All Mail folder.';
            return [
              toFolder(aid, ts, all),
              tasks.changeLabels({ accountId: aid, threadIds: ids(ts), labelsToAdd: [inbox] }),
            ];
          },
          allSpam ? 'Moved to Inbox' : 'Reported as spam',
        );
      },

      setUnread: (threads, unread) =>
        perAccount(threads, (aid, ts) => [
          tasks.changeUnread({ accountId: aid, threadIds: ids(ts), unread }),
        ]),

      setStarred: (threads, starred) =>
        perAccount(threads, (aid, ts) => [
          tasks.changeStarred({ accountId: aid, threadIds: ids(ts), starred }),
        ]),

      toggleStarred: (threads) => actions.setStarred(threads, !threads.every((t) => t.starred > 0)),

      moveTo: (threads, target, viewing) =>
        perAccount(
          threads,
          (aid, ts) => {
            if (target.aid !== aid) return 'Pick a folder in the same account.';
            if (isFolder(target)) return [toFolder(aid, ts, target)];
            if (!isLabel(target)) return 'Pick a folder or label.';
            const drop =
              viewing && isLabel(viewing) && viewing.id !== target.id && viewing.aid === aid
                ? [viewing]
                : [];
            return [
              tasks.changeLabels({
                accountId: aid,
                threadIds: ids(ts),
                labelsToAdd: [target],
                labelsToRemove: drop,
              }),
            ];
          },
          `Moved to ${categoryDisplayName(target)}`,
        ),

      toggleLabel: (threads, label) => {
        const remove = threads.every((t) => threadInCategory(t, label.id));
        perAccount(threads, (aid, ts) => {
          if (label.aid !== aid) return 'Pick a label in the same account.';
          return [
            tasks.changeLabels({
              accountId: aid,
              threadIds: ids(ts),
              labelsToAdd: remove ? [] : [label],
              labelsToRemove: remove ? [label] : [],
            }),
          ];
        });
      },

      removeLabel: (threads, label) =>
        perAccount(threads, (aid, ts) => [
          tasks.changeLabels({ accountId: aid, threadIds: ids(ts), labelsToRemove: [label] }),
        ]),

      snooze: (threads, until) =>
        perAccountCall(
          threads,
          (accountId, threadIds) =>
            engine.call('threads.snooze', {
              accountId,
              threadIds,
              until: Math.floor(until.getTime() / 1000),
            }),
          'snooze',
          `Snoozed until ${formatSnooze(until)}`,
        ),

      unsnooze: (threads) =>
        perAccountCall(
          threads,
          (accountId, threadIds) => engine.call('threads.unsnooze', { accountId, threadIds }),
          'unsnooze',
          threads.length === 1
            ? 'Back in the inbox'
            : `${threads.length} threads back in the inbox`,
        ),
    };
    return actions;
  }, [qc, mutate, engine]);
}
