import { useQueries } from '@tanstack/react-query';
import { useMemo } from 'react';
import {
  foldersQuery,
  labelsQuery,
  sortCategories,
  useAccounts,
  useEngine,
  type AccountJSON,
  type CategoryJSON,
} from '@/engine';

export interface AccountCategories {
  account: AccountJSON;
  categories: CategoryJSON[];
}

function combineData(
  results: { data?: { folders: CategoryJSON[] } | { labels: CategoryJSON[] } }[],
) {
  return results.map((r) =>
    r.data ? ('folders' in r.data ? r.data.folders : r.data.labels) : undefined,
  );
}

/**
 * Folders and labels of every account, in sidebar order. Shares cache entries with
 * `useFolders`/`useLabels`, so this costs nothing once the sidebar has loaded.
 */
export function useAllCategories(): AccountCategories[] {
  const client = useEngine();
  const { data: accounts = [] } = useAccounts();
  const lists = useQueries({
    queries: accounts.flatMap((a) => [foldersQuery(client, a.id), labelsQuery(client, a.id)]),
    combine: combineData,
  });
  return useMemo(
    () =>
      accounts.map((account, i) => ({
        account,
        categories: sortCategories([...(lists[i * 2] ?? []), ...(lists[i * 2 + 1] ?? [])]),
      })),
    [accounts, lists],
  );
}
