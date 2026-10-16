// Thread-level guesses at a task's effect, applied to the cache before the engine answers.
// The engine's own Thread deltas follow within milliseconds and replace these.
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
