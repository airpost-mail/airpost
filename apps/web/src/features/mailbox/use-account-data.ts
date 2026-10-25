// Folders, labels and counts for every account at once, for the unified views. Same query
// keys and functions as the per-account hooks in `@/engine`, so the caches are shared.
import { useQueries } from '@tanstack/react-query';
import { useMemo } from 'react';
import {
  foldersQuery,
  labelsQuery,
  sortCategories,
  useAccounts,
  useCounts,
  useEngine,
  type AccountJSON,
  type CategoryCount,
  type CategoryJSON,
} from '@/engine';

export interface AccountData {
  account: AccountJSON;
  categories: CategoryJSON[] | undefined;
  /** Every account's counts (category ids are unique across accounts). */
  counts: Record<string, CategoryCount> | undefined;
}

/** Stable while no query result changes (TanStack memoises `combine`). */
function combineData<T>(results: { data: T | undefined; isLoading: boolean }[]) {
  return { data: results.map((r) => r.data), isLoading: results.some((r) => r.isLoading) };
}

export function useAllAccountData(): { accounts: AccountData[]; isLoading: boolean } {
  const client = useEngine();
  const { data: accounts, isLoading } = useAccounts();
  const list = useMemo(() => accounts ?? [], [accounts]);

  const folders = useQueries({
    queries: list.map((a) => foldersQuery(client, a.id)),
    combine: combineData,
  });
  const labels = useQueries({
    queries: list.map((a) => labelsQuery(client, a.id)),
    combine: combineData,
  });
  const counts = useCounts();

  const data = useMemo(
    () =>
      list.map((account, i) => {
        const f = folders.data[i];
        const l = labels.data[i];
        return {
          account,
          categories:
            f && l ? sortCategories<CategoryJSON>([...f.folders, ...l.labels]) : undefined,
          counts: counts.data,
        };
      }),
    [list, folders.data, labels.data, counts.data],
  );
  return {
    accounts: data,
    isLoading: isLoading || folders.isLoading || labels.isLoading,
  };
}
