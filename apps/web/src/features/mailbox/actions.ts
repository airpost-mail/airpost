// Thread actions shared by the list, the bulk toolbar and the reader. Each is one of the
// engine's intents for threads of any accounts (unified views mix them): the engine decides
// what it means on each account and keeps an undo entry, which the toast's Undo takes back.
// The answer's Thread rows go into the cache until the live queries send theirs.
import { useQueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';
import { toast } from 'sonner';
import {
  categoryDisplayName,
  engineKeys,
  ensureConnected,
  RpcError,
  useEngine,
  useIntents,
  threadInCategory,
  type CategoryJSON,
  type FolderList,
  type LabelJSON,
  type LabelList,
  type ThreadJSON,
} from '@/engine';
import { formatSnooze } from './snooze';

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
  /** Moves to a folder (the copies in the one viewed), or on Gmail to a label (replacing the threads' labels). */
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
  const intents = useIntents();
  const engine = useEngine();

  return useMemo(() => {
    const role = (aid: string, r: CategoryJSON['role']) =>
      [
        ...(qc.getQueryData<FolderList>(engineKeys.folders(aid))?.folders ?? []),
        ...(qc.getQueryData<LabelList>(engineKeys.labels(aid))?.labels ?? []),
      ].find((c) => c.role === r);

    const failed = (what: string) => (err: unknown) =>
      toast.error(
        err instanceof RpcError
          ? `Couldn't ${what}: ${err.message}`
          : "Couldn't reach the engine. Nothing was changed.",
      );

    // The toast after a change, with Undo while the engine keeps its entry.
    const done = (text: string | undefined, token: string | null | undefined) => {
      if (!text) return;
      if (!token) return void toast(text);
      toast(text, {
        action: {
          label: 'Undo',
          onClick: () => void intents.undo(token).catch(failed('undo')),
        },
      });
    };

    type Intent = Parameters<typeof intents.run>[0];
    const run = (
      threads: ThreadJSON[],
      method: Intent,
      params: Record<string, unknown>,
      what: string,
      text?: string,
    ) => {
      if (threads.length === 0) return;
      const threadIds = ids(threads);
      void intents
        .run(method, { threadIds, ...params } as never)
        .then((r) => done(text, r.undo), failed(what));
    };

    const count = (threads: ThreadJSON[], one: string, many: (n: number) => string) =>
      threads.length === 1 ? one : many(threads.length);

    /** One engine call per account (`threads.snooze`, `threads.unsnooze`). */
    const perAccountCall = (
      threads: ThreadJSON[],
      call: (aid: string, ids: string[]) => Promise<{ undo?: string | null }>,
      what: string,
      text: string,
    ) => {
      if (threads.length === 0) return;
      void ensureConnected(engine)
        .then(() => Promise.all([...byAccount(threads)].map(([aid, ts]) => call(aid, ids(ts)))))
        .then((results) => {
          // A snooze of one account can be undone; several are several entries.
          const tokens = results.map((r) => r.undo).filter(Boolean);
          done(text, tokens.length === 1 ? tokens[0] : null);
        }, failed(what));
    };

    const actions: ThreadActions = {
      archive: (threads) =>
        run(
          threads,
          'threads.archive',
          {},
          'archive',
          count(threads, 'Archived', (n) => `Archived ${n} threads`),
        ),

      trash: (threads) =>
        run(
          threads,
          'threads.trash',
          {},
          'move to trash',
          count(threads, 'Moved to Trash', (n) => `Moved ${n} threads to Trash`),
        ),

      spam: (threads) => {
        const allSpam = threads.every((t) => {
          const spam = role(t.aid, 'spam');
          return spam && threadInCategory(t, spam.id);
        });
        if (allSpam) {
          run(threads, 'threads.notSpam', {}, 'move out of spam', 'Moved to Inbox');
        } else {
          run(threads, 'threads.spam', {}, 'report spam', 'Reported as spam');
        }
      },

      setUnread: (threads, unread) =>
        run(threads, 'threads.markRead', unread ? { unread } : {}, 'mark them'),

      setStarred: (threads, starred) => run(threads, 'threads.star', { starred }, 'star them'),

      toggleStarred: (threads) => actions.setStarred(threads, !threads.every((t) => t.starred > 0)),

      moveTo: (threads, target, viewing) => {
        if (threads.some((t) => t.aid !== target.aid)) {
          toast.error('Pick a folder in the same account.');
          return;
        }
        // A folder takes the copies in the one being viewed; a Gmail label replaces the
        // threads' labels.
        const perspective = viewing ? { categoryIds: [viewing.id] } : undefined;
        run(
          threads,
          'threads.move',
          { to: target.id, perspective },
          'move them',
          `Moved to ${categoryDisplayName(target)}`,
        );
      },

      toggleLabel: (threads, label) => {
        if (threads.some((t) => t.aid !== label.aid)) {
          toast.error('Pick a label in the same account.');
          return;
        }
        const remove = threads.every((t) => threadInCategory(t, label.id));
        run(
          threads,
          'threads.label',
          remove ? { remove: [label.id] } : { add: [label.id] },
          'change labels',
        );
      },

      removeLabel: (threads, label) =>
        run(threads, 'threads.label', { remove: [label.id] }, 'remove the label'),

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
  }, [qc, intents, engine]);
}
