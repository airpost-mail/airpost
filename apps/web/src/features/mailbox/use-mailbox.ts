import { useCallback, useMemo } from 'react';
import {
  EngineFeature,
  EVERY_ACCOUNT,
  threadQueryFor,
  useCategories,
  useEngineFeatures,
  useThreads,
  type ThreadQuery,
} from '@/engine';
import { resolveMailbox, roleQuery, UNIFIED_ACCOUNT_ID, type Mailbox } from './mailbox';
import { useAllAccountData } from './use-account-data';
import { useUnifiedThreads } from './use-unified-threads';

/** Resolves the URL to a mailbox and loads its threads (single account or unified). */
export function useMailbox(accountId: string, folderId: string) {
  const { all, isLoading: categoriesLoading } = useCategories(
    accountId === UNIFIED_ACCOUNT_ID ? undefined : accountId,
  );
  const { accounts } = useAllAccountData();
  const features = useEngineFeatures();
  // With `query` the engine pages unified lists itself; without it they merge per account.
  const engineUnified = features?.has(EngineFeature.Query) ?? false;
  const mailbox = useMemo(
    () => resolveMailbox(accountId, folderId, categoriesLoading ? undefined : all),
    [accountId, folderId, categoriesLoading, all],
  );

  const unifiedRole = mailbox?.kind === 'unified' ? mailbox.role : null;
  const single = useMemo<ThreadQuery | null>(() => {
    if (!mailbox || !features) return null;
    if (mailbox.kind === 'starred') return { accountId: mailbox.accountId, starred: true };
    if (mailbox.kind === 'category') return threadQueryFor(mailbox.category);
    if (!engineUnified) return null;
    if (mailbox.role === 'starred') return { accountId: EVERY_ACCOUNT, starred: true };
    if (accounts.some((a) => !a.categories)) return null;
    const role = mailbox.role;
    const categoryIds = accounts.flatMap((a) =>
      (a.categories ?? []).filter((c) => c.role === role).map((c) => c.id),
    );
    return { accountId: EVERY_ACCOUNT, categoryIds, categoryRole: role };
  }, [mailbox, features, engineUnified, accounts]);

  const unifiedQueries = useMemo<ThreadQuery[] | null>(() => {
    if (!unifiedRole || engineUnified) return null;
    if (accounts.some((a) => !a.categories)) return null;
    return accounts
      .map((a) => roleQuery(a.account.id, unifiedRole, a.categories ?? []))
      .filter((q): q is ThreadQuery => q !== null);
  }, [unifiedRole, engineUnified, accounts]);

  const one = useThreads(single);
  const many = useUnifiedThreads(unifiedQueries);
  const source = unifiedRole && !engineUnified ? many : one;
  const fetchNext = source.fetchNextPage;
  const fetchNextPage = useCallback(() => void fetchNext(), [fetchNext]);
  const waiting =
    mailbox === undefined || (mailbox !== null && (!features || (!single && !unifiedQueries)));

  return {
    mailbox: mailbox as Mailbox | null | undefined,
    accounts,
    threads: source.threads,
    isLoading: waiting || (mailbox !== null && source.isLoading),
    error: source.error,
    hasNextPage: !!source.hasNextPage,
    isFetchingNextPage: source.isFetchingNextPage,
    fetchNextPage,
  };
}
