import type { CategoryRole } from './types';

/** A thread list: `threads.list` params without paging, plus client-side filters. */
export interface ThreadQuery {
  accountId: string;
  folderId?: string;
  labelId?: string;
  role?: CategoryRole;
  /**
   * Starred threads only. Not an engine param: the client lists every thread of the account
   * and filters (see `fetchThreadPage`).
   */
  starred?: boolean;
}

export interface SearchQuery {
  query: string;
  /** Omit to search every account (the client fans out; see `useSearch`). */
  accountId?: string;
}

/** Normalised so `{a, b: undefined}` and `{a}` share one cache entry. */
function compact<T extends object>(o: T): T {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as T;
}

export const engineKeys = {
  all: ['engine'] as const,
  accounts: () => ['engine', 'accounts'] as const,
  status: () => ['engine', 'status'] as const,
  processState: () => ['engine', 'processState'] as const,
  folders: (accountId: string) => ['engine', 'folders', accountId] as const,
  labels: (accountId: string) => ['engine', 'labels', accountId] as const,
  threadLists: (accountId: string) => ['engine', 'threads', accountId] as const,
  threadList: (q: ThreadQuery) => ['engine', 'threads', q.accountId, compact(q)] as const,
  thread: (accountId: string, threadId: string) =>
    ['engine', 'thread', accountId, threadId] as const,
  /** The raw `threads.get` result that `thread` and `messages` are both filled from. */
  threadGet: (accountId: string, threadId: string) =>
    ['engine', 'threadGet', accountId, threadId] as const,
  messagesAll: (accountId: string) => ['engine', 'messages', accountId] as const,
  messages: (accountId: string, threadId: string) =>
    ['engine', 'messages', accountId, threadId] as const,
  searchAll: () => ['engine', 'search'] as const,
  search: (q: SearchQuery & { limit?: number }) => ['engine', 'search', compact(q)] as const,
  contactsAll: () => ['engine', 'contacts'] as const,
  contacts: (q: SearchQuery & { limit?: number }) => ['engine', 'contacts', compact(q)] as const,
  tasks: (accountId: string) => ['engine', 'tasks', accountId] as const,
};
