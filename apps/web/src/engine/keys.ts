import type { CategoryRole } from './types';

/** `ThreadQuery.accountId` of a list across every account (read with `query` only). */
export const EVERY_ACCOUNT = '*';

/**
 * A thread list. Without the engine's `query` it is `threads.list` params without paging
 * plus client-side filters; with `query` the engine filters, sorts and pages it all.
 */
export interface ThreadQuery {
  /** One account, or `EVERY_ACCOUNT`. */
  accountId: string;
  folderId?: string;
  labelId?: string;
  role?: CategoryRole;
  /**
   * The role of `folderId`/`labelId` (or of `categoryIds`), read by the `query` form: spam
   * and trash list threads outside all mail, and Sent sorts by last sent.
   */
  categoryRole?: CategoryRole;
  /** Threads in any of these categories (a unified list's per-account categories). */
  categoryIds?: string[];
  /** Starred threads only. Without `query` the client filters the account's whole list. */
  starred?: boolean;
  /** Unread threads only (`query` only). */
  unread?: boolean;
}
/** Spam and trash list threads outside all mail (ms-store `threads()`, the desktop's perspectives). */
export function listsOutsideAllMail(q: ThreadQuery): boolean {
  const role = q.categoryRole ?? q.role;
  return role === 'spam' || role === 'trash';
}

/** Events overlapping `[start, end)` (unix seconds) in the given calendars. */
export interface EventRange {
  start: number;
  end: number;
  calendarIds: string[];
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
  /** Set when the engine reports `ProcessIdentityRefreshNeeded`. */
  serverIdentity: () => ['engine', 'serverIdentity'] as const,
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
  calendars: () => ['engine', 'calendars'] as const,
  eventsAll: () => ['engine', 'events'] as const,
  events: (q: EventRange) => ['engine', 'events', compact(q)] as const,
};
