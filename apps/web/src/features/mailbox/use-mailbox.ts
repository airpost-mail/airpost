import { useCallback, useMemo } from 'react';
import {
  EVERY_ACCOUNT,
  threadQueryFor,
  useCategories,
  useEngineFeatures,
  useThreads,
  type ThreadQuery,
} from '@/engine';
import { resolveMailbox, UNIFIED_ACCOUNT_ID, type Mailbox } from './mailbox';
import { useAllAccountData } from './use-account-data';

/** Resolves the URL to a mailbox and loads its threads (single account or unified). */
export function useMailbox(accountId: string, folderId: string) {
  const { all, isLoading: categoriesLoading } = useCategories(
    accountId === UNIFIED_ACCOUNT_ID ? undefined : accountId,
  );
  const { accounts } = useAllAccountData();
  // Null until the first handshake.
  const features = useEngineFeatures();
  const mailbox = useMemo(
    () => resolveMailbox(accountId, folderId, categoriesLoading ? undefined : all),
    [accountId, folderId, categoriesLoading, all],
  );

  // The engine pages unified lists itself (`query` across accounts).
  const query = useMemo<ThreadQuery | null>(() => {
    if (!mailbox || !features) return null;
    if (mailbox.kind === 'starred') return { accountId: mailbox.accountId, starred: true };
    if (mailbox.kind === 'category') return threadQueryFor(mailbox.category);
    if (mailbox.role === 'starred') return { accountId: EVERY_ACCOUNT, starred: true };
    if (accounts.some((a) => !a.categories)) return null;
    const role = mailbox.role;
    const categoryIds = accounts.flatMap((a) =>
      (a.categories ?? []).filter((c) => c.role === role).map((c) => c.id),
    );
    return { accountId: EVERY_ACCOUNT, categoryIds, categoryRole: role };
  }, [mailbox, features, accounts]);

  const source = useThreads(query);
  const fetchNext = source.fetchNextPage;
  const fetchNextPage = useCallback(() => void fetchNext(), [fetchNext]);
  const waiting = mailbox === undefined || (mailbox !== null && (!features || !query));

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
