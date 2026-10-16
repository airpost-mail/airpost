import { useCallback, useMemo } from 'react';
import { useCategories, useThreads, type ThreadQuery } from '@/engine';
import { resolveMailbox, roleQuery, UNIFIED_ACCOUNT_ID, type Mailbox } from './mailbox';
import { useAllAccountData } from './use-account-data';
import { useUnifiedThreads } from './use-unified-threads';

/** Resolves the URL to a mailbox and loads its threads (single account or unified). */
export function useMailbox(accountId: string, folderId: string) {
  const { all, isLoading: categoriesLoading } = useCategories(
    accountId === UNIFIED_ACCOUNT_ID ? undefined : accountId,
  );
  const { accounts } = useAllAccountData();
  const mailbox = useMemo(
    () => resolveMailbox(accountId, folderId, categoriesLoading ? undefined : all),
    [accountId, folderId, categoriesLoading, all],
  );

  const single = useMemo<ThreadQuery | null>(() => {
    if (!mailbox || mailbox.kind === 'unified') return null;
    if (mailbox.kind === 'starred') return { accountId: mailbox.accountId, starred: true };
    const c = mailbox.category;
    return c.__cls === 'Label'
      ? { accountId: c.aid, labelId: c.id }
      : { accountId: c.aid, folderId: c.id };
  }, [mailbox]);

  const unifiedRole = mailbox?.kind === 'unified' ? mailbox.role : null;
  const unifiedQueries = useMemo<ThreadQuery[] | null>(() => {
    if (!unifiedRole) return null;
    if (accounts.some((a) => !a.categories)) return null;
    return accounts
      .map((a) => roleQuery(a.account.id, unifiedRole, a.categories ?? []))
      .filter((q): q is ThreadQuery => q !== null);
  }, [unifiedRole, accounts]);

  const one = useThreads(single);
  const many = useUnifiedThreads(unifiedQueries);
  const source = unifiedRole ? many : one;
  const fetchNext = source.fetchNextPage;
  const fetchNextPage = useCallback(() => void fetchNext(), [fetchNext]);

  return {
    mailbox: mailbox as Mailbox | null | undefined,
    accounts,
    threads: source.threads,
    isLoading: mailbox === undefined || (mailbox !== null && source.isLoading),
    error: source.error,
    hasNextPage: !!source.hasNextPage,
    isFetchingNextPage: source.isFetchingNextPage,
    fetchNextPage,
  };
}
