import type { CategoryRole } from './types';

/** `ThreadQuery.accountId` of a list across every account. */
export const EVERY_ACCOUNT = '*';

/**
 * A thread list: the engine filters, sorts and pages it (`threadListQuery` in hooks.ts).
 */
export interface ThreadQuery {
  /** One account, or `EVERY_ACCOUNT`. */
  accountId: string;
  folderId?: string;
  labelId?: string;
  /**
   * The role of `folderId`/`labelId` (or of `categoryIds`): spam and trash list threads
   * outside all mail, and Sent sorts by last sent.
   */
  categoryRole?: CategoryRole;
  /** Threads in any of these categories (a unified list's per-account categories). */
  categoryIds?: string[];
  /** Starred threads only. */
  starred?: boolean;
  /** Unread threads only. */
  unread?: boolean;
}
/** Spam and trash list threads outside all mail (airpost-store `threads()`, the desktop's perspectives). */
export function listsOutsideAllMail(q: ThreadQuery): boolean {
  return q.categoryRole === 'spam' || q.categoryRole === 'trash';
}

/** Events overlapping `[start, end)` (unix seconds) in the given calendars. */
export interface EventRange {
  start: number;
  end: number;
  calendarIds: string[];
}

export interface SearchQuery {
  query: string;
  /** Omit to search every account. */
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
  /** `counts.get` for every account, by category id, kept by ThreadCounts deltas. */
  counts: () => ['engine', 'counts'] as const,
  threadLists: (accountId: string) => ['engine', 'threads', accountId] as const,
  threadList: (q: ThreadQuery) => ['engine', 'threads', q.accountId, compact(q)] as const,
  thread: (accountId: string, threadId: string) =>
    ['engine', 'thread', accountId, threadId] as const,
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
