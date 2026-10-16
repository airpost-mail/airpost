// What a `/$accountId/$folderId` URL shows. Besides real folders and labels there are two
// views the engine has no category for: the unified mailbox of every account (accountId
// `all`, folderId a role) and per-account Starred (folderId `starred`).
import {
  categoryDisplayName,
  threadQueryFor,
  type AccountJSON,
  type CategoryJSON,
  type CategoryRole,
  type ThreadQuery,
} from '@/engine';

export const UNIFIED_ACCOUNT_ID = 'all';
export const STARRED_VIEW_ID = 'starred';

/** Roles the unified sidebar offers, in order. */
export const UNIFIED_ROLES = ['inbox', 'starred', 'drafts', 'sent', 'spam', 'trash'] as const;
export type UnifiedRole = (typeof UNIFIED_ROLES)[number];

export type Mailbox =
  | { kind: 'category'; accountId: string; category: CategoryJSON }
  | { kind: 'starred'; accountId: string }
  | { kind: 'unified'; role: UnifiedRole };

export function isUnifiedRole(role: string): role is UnifiedRole {
  return (UNIFIED_ROLES as readonly string[]).includes(role);
}

/** `undefined` while categories are still loading, `null` when the URL matches nothing. */
export function resolveMailbox(
  accountId: string,
  folderId: string,
  categories: CategoryJSON[] | undefined,
): Mailbox | null | undefined {
  if (accountId === UNIFIED_ACCOUNT_ID) {
    return isUnifiedRole(folderId) ? { kind: 'unified', role: folderId } : null;
  }
  if (!categories) return undefined;
  const category = categories.find((c) => c.id === folderId);
  if (category) return { kind: 'category', accountId, category };
  if (folderId === STARRED_VIEW_ID) return { kind: 'starred', accountId };
  return null;
}

/** The thread query for `role` in one account, or null if the account has no such mailbox. */
export function roleQuery(
  accountId: string,
  role: CategoryRole,
  categories: CategoryJSON[],
): ThreadQuery | null {
  const category = categories.find((c) => c.role === role);
  if (category) return threadQueryFor(category);
  return role === 'starred' ? { accountId, starred: true } : null;
}

const UNIFIED_TITLES: Record<UnifiedRole, string> = {
  inbox: 'All inboxes',
  starred: 'Starred',
  drafts: 'Drafts',
  sent: 'Sent',
  spam: 'Spam',
  trash: 'Trash',
};

export function mailboxTitle(m: Mailbox): string {
  if (m.kind === 'unified') return UNIFIED_TITLES[m.role];
  if (m.kind === 'starred') return 'Starred';
  return categoryDisplayName(m.category);
}

export function mailboxRole(m: Mailbox): CategoryRole {
  if (m.kind === 'unified') return m.role;
  if (m.kind === 'starred') return 'starred';
  return m.category.role;
}

/** The category the user is looking at, for one account (unified views resolve per account). */
export function viewCategoryFor(
  m: Mailbox,
  accountId: string,
  categories: CategoryJSON[],
): CategoryJSON | undefined {
  if (m.kind === 'category') return m.accountId === accountId ? m.category : undefined;
  if (m.kind === 'unified') return categories.find((c) => c.role === m.role);
  return undefined;
}

/** Owner addresses, lower-cased, for "is this me" checks. */
export function ownAddresses(accounts: Pick<AccountJSON, 'emailAddress' | 'aliases'>[]) {
  return new Set(
    accounts.flatMap((a) => [a.emailAddress, ...a.aliases]).map((e) => e.toLowerCase()),
  );
}
