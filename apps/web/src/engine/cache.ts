// Applies delta notifications to the TanStack Query cache, so views update without
// refetching. This plays the role of the desktop's DatabaseStore change records.
import type { InfiniteData, QueryClient } from '@tanstack/react-query';
import { engineKeys, EVERY_ACCOUNT, listsOutsideAllMail, type ThreadQuery } from './keys';
import {
  compareMessages,
  compareThreads,
  threadHasRole,
  threadInCategory,
  type ThreadSort,
} from './model';
import type { FoldersListResult, LabelsListResult, SearchResult, ThreadsListResult } from './rpc';
import type {
  AccountJSON,
  CategoryJSON,
  Delta,
  MessageJSON,
  ProcessStateJSON,
  TaskJSON,
  ThreadJSON,
} from './types';

/** `sort` is the key the page is ordered by (`threads.list`: `lmt`). */
export type ThreadPages = InfiniteData<ThreadsListResult & { sort?: ThreadSort }, string | null>;

const MAX_TASKS = 100;

export function matchesThreadQuery(t: ThreadJSON, q: ThreadQuery): boolean {
  if (q.accountId !== EVERY_ACCOUNT && t.aid !== q.accountId) return false;
  if (q.folderId && !threadInCategory(t, q.folderId)) return false;
  if (q.labelId && !threadInCategory(t, q.labelId)) return false;
  if (q.categoryIds && !q.categoryIds.some((id) => threadInCategory(t, id))) return false;
  if (q.role && !threadHasRole(t, q.role)) return false;
  if (q.starred !== undefined && t.starred > 0 !== q.starred) return false;
  if (q.unread !== undefined && t.unread > 0 !== q.unread) return false;
  // Lists outside spam and trash hold threads in all mail, role or not, as `query` and
  // `threads.list` read them (only an older engine's flag lists without a category don't).
  if (!listsOutsideAllMail(q) && !t.inAllMail) return false;
  return true;
}

function removeThread(data: ThreadPages, id: string): ThreadPages {
  let changed = false;
  const pages = data.pages.map((p) => {
    if (!p.threads.some((t) => t.id === id)) return p;
    changed = true;
    return { ...p, threads: p.threads.filter((t) => t.id !== id) };
  });
  return changed ? { ...data, pages } : data;
}

/**
 * Inserts in sort order. A thread that sorts after the last loaded item of a list that
 * has more pages is left out: the next page fetch will include it.
 */
function insertThread(data: ThreadPages, thread: ThreadJSON): ThreadPages {
  const sort = data.pages[0]?.sort ?? 'lmt';
  const compare = (a: ThreadJSON, b: ThreadJSON) => compareThreads(a, b, sort);
  const pages = [...data.pages];
  for (let i = 0; i < pages.length; i++) {
    const page = pages[i];
    const last = page.threads[page.threads.length - 1];
    const isLast = i === pages.length - 1;
    if (last && compare(thread, last) > 0 && !isLast) continue;
    if (isLast && page.next && last && compare(thread, last) > 0) return data;
    const threads = [...page.threads, thread].sort(compare);
    pages[i] = { ...page, threads };
    return { ...data, pages };
  }
  return data;
}

/** Replaces, inserts or removes one thread in a cached list according to the query. */
export function upsertThreadInPages(
  data: ThreadPages | undefined,
  thread: ThreadJSON,
  q: ThreadQuery,
): ThreadPages | undefined {
  if (!data) return data;
  const present = data.pages.some((p) => p.threads.some((t) => t.id === thread.id));
  if (!matchesThreadQuery(thread, q)) return present ? removeThread(data, thread.id) : data;
  if (present) {
    // Re-sort only when the position could change.
    const without = removeThread(data, thread.id);
    return insertThread(without, thread);
  }
  return insertThread(data, thread);
}

function replaceInResult(result: SearchResult | undefined, thread: ThreadJSON) {
  if (!result || !result.threads.some((t) => t.id === thread.id)) return result;
  return { ...result, threads: result.threads.map((t) => (t.id === thread.id ? thread : t)) };
}

function upsertById<T extends { id: string }>(list: T[] | undefined, items: T[]): T[] | undefined {
  if (!list) return list;
  const byId = new Map(items.map((i) => [i.id, i]));
  const out = list.map((x) => byId.get(x.id) ?? x);
  for (const i of items) if (!list.some((x) => x.id === i.id)) out.push(i);
  return out;
}

function removeById<T extends { id: string }>(list: T[] | undefined, ids: Set<string>) {
  return list?.filter((x) => !ids.has(x.id));
}

/** Counts come with the category lists; refetching those is how they stay current. */
function invalidateCounts(qc: QueryClient, accountId: string) {
  void qc.invalidateQueries({ queryKey: engineKeys.folders(accountId) });
  void qc.invalidateQueries({ queryKey: engineKeys.labels(accountId) });
}

function applyThreads(qc: QueryClient, d: Delta<'Thread'>) {
  const cache = qc.getQueryCache();
  const lists = [
    ...cache.findAll({ queryKey: engineKeys.threadLists(d.accountId) }),
    ...cache.findAll({ queryKey: engineKeys.threadLists(EVERY_ACCOUNT) }),
  ];
  const persist = d.type === 'persist';
  for (const thread of d.modelJSONs) {
    for (const query of lists) {
      const q = query.queryKey[3] as ThreadQuery;
      qc.setQueryData<ThreadPages>(query.queryKey, (data) =>
        persist ? upsertThreadInPages(data, thread, q) : data && removeThread(data, thread.id),
      );
    }
    qc.setQueryData<ThreadJSON | null>(engineKeys.thread(d.accountId, thread.id), (old) =>
      persist ? thread : old === undefined ? undefined : null,
    );
    for (const query of qc.getQueryCache().findAll({ queryKey: engineKeys.searchAll() })) {
      qc.setQueryData<SearchResult>(query.queryKey, (result) =>
        persist
          ? replaceInResult(result, thread)
          : result && { ...result, threads: result.threads.filter((t) => t.id !== thread.id) },
      );
    }
  }
  invalidateCounts(qc, d.accountId);
}

/** Drafts are rewritten in place, so a cached draft body is stale once its version moves. */
function keepBody(existing: MessageJSON | undefined, msg: MessageJSON): boolean {
  if (msg.body !== undefined || existing?.body == null) return false;
  return !msg.draft || existing.v === msg.v;
}

function applyMessages(qc: QueryClient, d: Delta<'Message'>) {
  const lists = qc.getQueryCache().findAll({ queryKey: engineKeys.messagesAll(d.accountId) });
  for (const msg of d.modelJSONs) {
    for (const query of lists) {
      const threadId = query.queryKey[3] as string;
      qc.setQueryData<MessageJSON[]>(query.queryKey, (list) => {
        if (!list) return list;
        const existing = list.find((m) => m.id === msg.id);
        const rest = list.filter((m) => m.id !== msg.id);
        if (d.type === 'unpersist' || msg.threadId !== threadId) {
          return existing ? rest : list;
        }
        // Deltas carry `body` only when it was just downloaded; keep the one we have.
        const merged: MessageJSON = keepBody(existing, msg)
          ? { ...msg, body: existing!.body, plaintext: existing!.plaintext }
          : msg;
        return [...rest, merged].sort(compareMessages);
      });
    }
  }
}

function applyCategories(qc: QueryClient, d: Delta<'Folder'> | Delta<'Label'>) {
  const items = d.modelJSONs as CategoryJSON[];
  const update = <T extends CategoryJSON>(list: T[]) =>
    (d.type === 'persist'
      ? upsertById(list, items as T[])
      : removeById(list, new Set(items.map((c) => c.id)))) ?? list;
  if (d.modelClass === 'Folder') {
    qc.setQueryData<FoldersListResult>(
      engineKeys.folders(d.accountId),
      (r) => r && { ...r, folders: update(r.folders) },
    );
  } else {
    qc.setQueryData<LabelsListResult>(
      engineKeys.labels(d.accountId),
      (r) => r && { ...r, labels: update(r.labels) },
    );
  }
  invalidateCounts(qc, d.accountId);
}

function applyAccounts(qc: QueryClient, d: Delta<'Account'>) {
  qc.setQueryData<AccountJSON[]>(engineKeys.accounts(), (list) =>
    d.type === 'persist'
      ? upsertById(list, d.modelJSONs)
      : removeById(list, new Set(d.modelJSONs.map((a) => a.id))),
  );
}

function applyTasks(qc: QueryClient, d: Delta<'Task'>) {
  qc.setQueryData<TaskJSON[]>(engineKeys.tasks(d.accountId), (list = []) => {
    const next =
      d.type === 'persist'
        ? (upsertById(list, d.modelJSONs) ?? [])
        : (removeById(list, new Set(d.modelJSONs.map((t) => t.id))) ?? []);
    return next.slice(-MAX_TASKS);
  });
}

function applyProcessState(qc: QueryClient, d: Delta<'ProcessState'>) {
  qc.setQueryData<Record<string, ProcessStateJSON>>(engineKeys.processState(), (old = {}) => {
    const next = { ...old };
    for (const s of d.modelJSONs) next[s.accountId || d.accountId] = s;
    return next;
  });
}

/** What the engine said about its Tern server connection (plan 1.8). */
export interface ServerIdentityState {
  refreshNeeded: boolean;
}

/**
 * The delta classes the web uses: those `applyDelta` handles, plus the backend's identity
 * event (plan 1.8). Metadata expirations arrive as Thread and Message deltas. Subscribing to
 * fewer classes keeps a large initial sync off the WebSocket.
 */
export const RENDERED_CLASSES = [
  'Thread',
  'Message',
  'Folder',
  'Label',
  'Account',
  'Task',
  'ProcessState',
  'Contact',
  'Calendar',
  'Event',
  'ProcessIdentityRefreshNeeded',
] as const;

export function applyDelta(qc: QueryClient, delta: Delta) {
  // Expirations are handled by whoever owns the metadata (see send-later); the models
  // themselves did not change.
  if (delta.type === 'metadata-expiration') return;
  switch (delta.modelClass) {
    case 'Thread':
      return applyThreads(qc, delta as Delta<'Thread'>);
    case 'Message':
      return applyMessages(qc, delta as Delta<'Message'>);
    case 'Folder':
    case 'Label':
      return applyCategories(qc, delta as Delta<'Folder'>);
    case 'Account':
      return applyAccounts(qc, delta as Delta<'Account'>);
    case 'Task':
      return applyTasks(qc, delta as Delta<'Task'>);
    case 'ProcessState':
      return applyProcessState(qc, delta as Delta<'ProcessState'>);
    case 'Contact':
      return void qc.invalidateQueries({ queryKey: engineKeys.contactsAll() });
    case 'ProcessIdentityRefreshNeeded':
      return void qc.setQueryData<ServerIdentityState>(engineKeys.serverIdentity(), {
        refreshNeeded: true,
      });
    case 'Calendar':
      return void qc.invalidateQueries({ queryKey: engineKeys.calendars() });
    case 'Event':
      return void qc.invalidateQueries({ queryKey: engineKeys.eventsAll() });
    default:
      // Contact groups and books have no views.
      return;
  }
}
