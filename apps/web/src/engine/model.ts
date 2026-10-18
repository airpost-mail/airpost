// Small helpers over the wire types that every feature needs. Ported from the desktop's
// Category, Thread and Message models; keep the semantics in step with them.
import type {
  AccountJSON,
  CategoryJSON,
  CategoryRole,
  ContactRef,
  MessageJSON,
  ThreadCategoryRef,
  ThreadJSON,
} from './types';

const ROLE_NAMES: Record<string, string> = {
  inbox: 'Inbox',
  important: 'Important',
  snoozed: 'Snoozed',
  sent: 'Sent',
  drafts: 'Drafts',
  all: 'All Mail',
  spam: 'Spam',
  archive: 'Archive',
  trash: 'Trash',
  starred: 'Starred',
};

/** Sidebar order for standard roles (desktop `Category.StandardRoles`). */
export const ROLE_ORDER: CategoryRole[] = [
  'inbox',
  'important',
  'snoozed',
  'starred',
  'drafts',
  'sent',
  'archive',
  'all',
  'spam',
  'trash',
];

/** Decodes IMAP modified UTF-7 (RFC 3501 5.1.3). */
export function decodeImapUtf7(input: string): string {
  return input.replace(/&([^-]*)-/g, (_, chunk: string) => {
    if (chunk === '') return '&';
    const b64 = chunk.replace(/,/g, '/');
    const bin = atob(b64 + '==='.slice((b64.length + 3) % 4));
    let out = '';
    for (let i = 0; i + 1 < bin.length; i += 2) {
      out += String.fromCharCode((bin.charCodeAt(i) << 8) | bin.charCodeAt(i + 1));
    }
    return out;
  });
}

export function categoryDisplayName(cat: Pick<CategoryJSON, 'path' | 'role'>): string {
  if (cat.role && ROLE_NAMES[cat.role]) return ROLE_NAMES[cat.role];
  const decoded = decodeImapUtf7(cat.path);
  // '[Mailspring]' is the container folder older desktop installs created on the server.
  for (const prefix of ['INBOX', '[Gmail]', '[Mailspring]']) {
    if (decoded.startsWith(prefix) && decoded.length > prefix.length + 1) {
      return decoded.slice(prefix.length + 1);
    }
  }
  if (decoded === 'INBOX') return 'Inbox';
  return decoded;
}

export function isStandardCategory(cat: Pick<CategoryJSON, 'role'>): boolean {
  return cat.role !== '' && ROLE_NAMES[cat.role] !== undefined;
}

export function sortCategories<T extends CategoryJSON>(cats: T[]): T[] {
  return [...cats].sort((a, b) => {
    const ra = a.role ? ROLE_ORDER.indexOf(a.role) : -1;
    const rb = b.role ? ROLE_ORDER.indexOf(b.role) : -1;
    if (ra !== -1 || rb !== -1) {
      if (ra === -1) return 1;
      if (rb === -1) return -1;
      return ra - rb;
    }
    return categoryDisplayName(a).localeCompare(categoryDisplayName(b));
  });
}

export function isGmailAccount(account: Pick<AccountJSON, 'provider'>): boolean {
  return account.provider === 'gmail';
}

export function threadCategories(thread: ThreadJSON): ThreadCategoryRef[] {
  return [...thread.folders, ...thread.labels];
}

export function threadInCategory(thread: ThreadJSON, categoryId: string): boolean {
  return threadCategories(thread).some((c) => c.id === categoryId && c._refs > 0);
}

export function threadHasRole(thread: ThreadJSON, role: CategoryRole): boolean {
  return threadCategories(thread).some((c) => c.role === role && c._refs > 0);
}

/**
 * The timestamp a thread list sorts by, newest first with the id descending as the tiebreak:
 * `lmt` (last message) for `threads.list`, and with `query` the desktop's perspectives,
 * `lmrt` (last received) for most lists and `lmst` (last sent) for Sent.
 */
export type ThreadSort = 'lmt' | 'lmrt' | 'lmst';

/** The ModelQuery attribute of a `query` sort key (`lmt` isn't queryable). */
export const THREAD_SORT_ATTR: Record<Exclude<ThreadSort, 'lmt'>, string> = {
  lmrt: 'lastMessageReceivedTimestamp',
  lmst: 'lastMessageSentTimestamp',
};

export function compareThreads(a: ThreadJSON, b: ThreadJSON, sort: ThreadSort = 'lmt'): number {
  if (a[sort] !== b[sort]) return b[sort] - a[sort];
  return a.id < b.id ? 1 : a.id > b.id ? -1 : 0;
}

/** True when `thread` sorts strictly after `last` in thread list order. */
export function sortsAfter(
  thread: ThreadJSON,
  last: Pick<ThreadJSON, 'id'> & Partial<Pick<ThreadJSON, ThreadSort>>,
  sort: ThreadSort = 'lmt',
): boolean {
  const a = thread[sort];
  const b = last[sort] ?? 0;
  if (a !== b) return a < b;
  return thread.id < last.id;
}

export function compareMessages(a: MessageJSON, b: MessageJSON): number {
  return a.date - b.date || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

export function contactDisplayName(c: ContactRef): string {
  return c.name?.trim() || c.email;
}

export function contactInitials(c: ContactRef): string {
  const name = contactDisplayName(c)
    .replace(/[^\p{L}\p{N} ]/gu, ' ')
    .trim();
  const parts = name.split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '?';
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

export function isFromAccount(
  message: MessageJSON,
  account: Pick<AccountJSON, 'emailAddress' | 'aliases'>,
): boolean {
  const own = new Set([account.emailAddress, ...account.aliases].map((e) => e.toLowerCase()));
  return message.from.some((c) => own.has(c.email.toLowerCase()));
}

export function generateId(): string {
  return crypto.randomUUID();
}

export function generateHeaderMessageId(domain = 'tern.local'): string {
  return `${crypto.randomUUID().toUpperCase()}@${domain}`;
}

export function nowSeconds(): number {
  return Math.floor(Date.now() / 1000);
}
