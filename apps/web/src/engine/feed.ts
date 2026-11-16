// The raw delta feed, for what the web shows that has no live query: counts, accounts,
// tasks, online state, and the reads that are read again when their class changes. Thread
// lists, threads, messages, folders, labels and search results are live queries (live.ts).
import type { QueryClient } from '@tanstack/react-query';
import { engineKeys } from './keys';
import type { CategoryCount } from './rpc';
import type { AccountJSON, Delta, ProcessStateJSON, TaskJSON } from './types';

const MAX_TASKS = 100;

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

/** The engine sends a category's counts when they change, and unpersists a removed one's. */
function applyThreadCounts(qc: QueryClient, d: Delta<'ThreadCounts'>) {
  qc.setQueryData<Record<string, CategoryCount>>(engineKeys.counts(), (old) => {
    if (!old) return old;
    const next = { ...old };
    for (const c of d.modelJSONs) {
      if (d.type === 'persist') next[c.categoryId] = { unread: c.unread ?? 0, total: c.total ?? 0 };
      else delete next[c.categoryId];
    }
    return next;
  });
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

/** What the engine said about its Airpost server connection. */
export interface ServerIdentityState {
  refreshNeeded: boolean;
}

/**
 * The delta classes the web subscribes to: those `applyFeedDelta` handles, the Thread and
 * Message deltas a few views still follow (`useDeltas`: the composer's draft, needs-reply),
 * and the backend's identity event. Metadata expirations arrive as Thread and Message deltas.
 * Subscribing to fewer classes keeps a large initial sync off the WebSocket.
 */
export const FEED_CLASSES = [
  'Thread',
  'Message',
  'ThreadCounts',
  'Account',
  'Task',
  'ProcessState',
  'Contact',
  'Calendar',
  'Event',
  'ProcessIdentityRefreshNeeded',
] as const;

export function applyFeedDelta(qc: QueryClient, delta: Delta) {
  // Expirations are handled by whoever owns the metadata (see send-later); the models
  // themselves did not change.
  if (delta.type === 'metadata-expiration') return;
  switch (delta.modelClass) {
    case 'ThreadCounts':
      return applyThreadCounts(qc, delta as Delta<'ThreadCounts'>);
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
      void qc.invalidateQueries({ queryKey: engineKeys.eventsAll() });
      return void qc.invalidateQueries({ queryKey: engineKeys.calendars() });
    case 'Event':
      return void qc.invalidateQueries({ queryKey: engineKeys.eventsAll() });
    default:
      // Thread, Message, Folder and Label are live queries; contact groups and books have no
      // views.
      return;
  }
}
