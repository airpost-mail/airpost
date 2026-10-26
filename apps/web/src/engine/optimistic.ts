// Thread-level guesses at a task's effect, applied to the cache before the engine answers.
// The live queries' changes follow within milliseconds and replace these.
import type { QueryClient } from '@tanstack/react-query';
import { engineKeys, EVERY_ACCOUNT, listsOutsideAllMail, type ThreadQuery } from './keys';
import type { LiveSnapshot } from './live';
import { threadInCategory } from './model';
import type { SearchResult } from './rpc';
import type { CategoryJSON, TaskJSON, ThreadCategoryRef, ThreadJSON } from './types';

export interface OptimisticContext {
  thread: (id: string) => ThreadJSON | undefined;
  isGmail: (accountId: string) => boolean;
}

const LOCKED_ROLES = new Set(['sent', 'drafts']);

function messageCount(t: ThreadJSON): number {
  return Math.max(1, ...[...t.folders, ...t.labels].map((r) => r._refs));
}

function ref(cat: CategoryJSON, t: ThreadJSON): ThreadCategoryRef {
  const { localStatus: _ignored, ...rest } = cat;
  return { ...rest, _refs: messageCount(t), _u: t.unread };
}

function apply(task: TaskJSON, t: ThreadJSON, ctx: OptimisticContext): ThreadJSON | null {
  switch (task.__cls) {
    case 'ChangeUnreadTask': {
      const unread = task.unread ? messageCount(t) : 0;
      const refs = (rs: ThreadCategoryRef[]) =>
        rs.map((r) => ({ ...r, _u: task.unread ? r._refs : 0 }));
      return { ...t, unread, folders: refs(t.folders), labels: refs(t.labels) };
    }
    case 'ChangeStarredTask':
      return { ...t, starred: task.starred ? Math.max(1, t.starred) : 0 };
    case 'ChangeFolderTask': {
      const target = ref(task.folder, t);
      if (ctx.isGmail(t.aid)) {
        const hide = task.folder.role === 'trash' || task.folder.role === 'spam';
        return { ...t, folders: [target], labels: hide ? [] : t.labels };
      }
      const moved = (r: ThreadCategoryRef) =>
        task.sourceFolderIds?.length
          ? task.sourceFolderIds.includes(r.id)
          : !LOCKED_ROLES.has(r.role);
      const kept = t.folders.filter((r) => !moved(r) && r.id !== target.id);
      return { ...t, folders: [...kept, target] };
    }
    case 'ChangeLabelsTask': {
      const remove = new Set(task.labelsToRemove.map((l) => l.id));
      const labels = t.labels.filter((l) => !remove.has(l.id));
      for (const l of task.labelsToAdd) {
        if (!labels.some((x) => x.id === l.id)) labels.push(ref(l, t));
      }
      return { ...t, labels };
    }
    default:
      return null;
  }
}

/** The cached threads as they should look after `task`, or [] if it has no thread effect. */
export function optimisticThreads(task: TaskJSON, ctx: OptimisticContext): ThreadJSON[] {
  if (!('threadIds' in task)) return [];
  const out: ThreadJSON[] = [];
  for (const id of task.threadIds) {
    const t = ctx.thread(id);
    if (!t) continue;
    const next = apply(task, t, ctx);
    if (next) out.push(next);
  }
  return out;
}

/**
 * Whether `t` belongs in the list `q` (categories, flags, all mail), only so a guess can take
 * a thread out of a list at once; the engine's live query decides the rows. Goes with this
 * file once the intents answer with their models (plan stage 7).
 */
export function matchesThreadQuery(t: ThreadJSON, q: ThreadQuery): boolean {
  if (q.accountId !== EVERY_ACCOUNT && t.aid !== q.accountId) return false;
  if (q.folderId && !threadInCategory(t, q.folderId)) return false;
  if (q.labelId && !threadInCategory(t, q.labelId)) return false;
  if (q.categoryIds && !q.categoryIds.some((id) => threadInCategory(t, id))) return false;
  if (q.starred !== undefined && t.starred > 0 !== q.starred) return false;
  if (q.unread !== undefined && t.unread > 0 !== q.unread) return false;
  if (!listsOutsideAllMail(q) && !t.inAllMail) return false;
  return true;
}

/**
 * Writes the guessed threads into the cached lists, threads and search results that hold
 * them, dropping a thread from a list it leaves. Nothing is inserted or re-sorted: the next
 * `query.changed` of each live query replaces these rows with the engine's.
 */
export function applyOptimistic(qc: QueryClient, accountId: string, threads: ThreadJSON[]) {
  const byId = new Map(threads.map((t) => [t.id, t]));
  const patch = (rows: ThreadJSON[], q?: ThreadQuery) =>
    rows.flatMap((r) => {
      const next = r.aid === accountId ? byId.get(r.id) : undefined;
      if (!next) return [r];
      if (q && !matchesThreadQuery(next, q)) return [];
      return [{ ...next, summary: r.summary }];
    });
  for (const key of [engineKeys.threadLists(accountId), engineKeys.threadLists(EVERY_ACCOUNT)]) {
    for (const query of qc.getQueryCache().findAll({ queryKey: key })) {
      const q = query.queryKey[3] as ThreadQuery;
      qc.setQueryData<LiveSnapshot<ThreadJSON>>(query.queryKey, (list) => {
        if (!list) return list;
        const rows = patch(list.rows, q);
        return { ...list, rows, total: list.total - (list.rows.length - rows.length) };
      });
    }
  }
  for (const t of threads) {
    qc.setQueryData<ThreadJSON | null>(engineKeys.thread(accountId, t.id), (old) =>
      old ? { ...t, summary: old.summary } : old,
    );
  }
  for (const query of qc.getQueryCache().findAll({ queryKey: engineKeys.searchAll() })) {
    qc.setQueryData<SearchResult & { truncated?: boolean }>(
      query.queryKey,
      (r) => r && { ...r, threads: patch(r.threads) },
    );
  }
}
