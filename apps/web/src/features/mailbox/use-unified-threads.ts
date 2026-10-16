// One thread list merged from several accounts. The engine pages per account, so each
// account's list is cached on its own (under its `threadLists` prefix, so deltas reach it)
// and the merge only shows threads down to the oldest point every account has loaded.
import { useQueries, useQueryClient, type InfiniteData } from '@tanstack/react-query';
import { useCallback, useMemo, useState } from 'react';
import {
  compareThreads,
  engineKeys,
  fetchThreadPage,
  sortsAfter,
  useEngine,
  type ThreadJSON,
  type ThreadQuery,
  type ThreadsListResult,
} from '@/engine';

type Pages = InfiniteData<ThreadsListResult, string | null>;

const PAGE_SIZE = 50;
// TODO(engine): threads.list takes one accountId; a cross-account query (accountId omitted,
// as `search` allows) would let the engine page the unified views itself.

/** Distinct from `useThreads`' key (plain query vs infinite query), same delta prefix. */
const unifiedKey = (q: ThreadQuery) => [...engineKeys.threadList(q), 'unified'] as const;

function combine(results: { data: Pages | undefined; isLoading: boolean; error: Error | null }[]) {
  return {
    data: results.map((r) => r.data),
    isLoading: results.some((r) => r.isLoading),
    error: results.find((r) => r.error)?.error ?? null,
  };
}

/**
 * Merges per-account pages down to the oldest position every account with more pages has
 * loaded: past that, an account that hasn't loaded further could still have threads that
 * sort earlier. Cursors are opaque, so the position is each list's last loaded thread.
 */
export function mergeAccountPages(lists: (Pages | undefined)[]): {
  threads: ThreadJSON[];
  hasNextPage: boolean;
} {
  let horizon: ThreadJSON | null = null;
  let more = false;
  for (const list of lists) {
    const last = list?.pages[list.pages.length - 1];
    if (!last?.next) continue;
    more = true;
    const edge = list!.pages.flatMap((p) => p.threads).at(-1);
    // Only a client-filtered list (Starred) can have more pages and nothing loaded yet.
    if (!edge) continue;
    if (!horizon || sortsAfter(horizon, edge)) horizon = edge;
  }
  const all = lists.flatMap((l) => l?.pages.flatMap((p) => p.threads) ?? []);
  const h = horizon;
  const visible = h ? all.filter((t) => !sortsAfter(t, h)) : all;
  return { threads: visible.sort(compareThreads), hasNextPage: more };
}

export function useUnifiedThreads(queries: ThreadQuery[] | null) {
  const client = useEngine();
  const qc = useQueryClient();
  const [isFetchingNextPage, setFetching] = useState(false);
  const list = useMemo(() => queries ?? [], [queries]);

  const result = useQueries({
    queries: list.map((q) => ({
      queryKey: unifiedKey(q),
      queryFn: async (): Promise<Pages> => {
        const page = await fetchThreadPage(client, q, PAGE_SIZE, null);
        return { pages: [page], pageParams: [null] };
      },
    })),
    combine,
  });

  const merged = useMemo(() => mergeAccountPages(result.data), [result.data]);

  const fetchNextPage = useCallback(async () => {
    setFetching(true);
    try {
      await Promise.all(
        list.map(async (q) => {
          const data = qc.getQueryData<Pages>(unifiedKey(q));
          const cursor = data?.pages[data.pages.length - 1]?.next;
          if (!cursor) return;
          const page = await fetchThreadPage(client, q, PAGE_SIZE, cursor);
          qc.setQueryData<Pages>(
            unifiedKey(q),
            (old) =>
              old && { pages: [...old.pages, page], pageParams: [...old.pageParams, cursor] },
          );
        }),
      );
    } finally {
      setFetching(false);
    }
  }, [client, qc, list]);

  return {
    threads: merged.threads,
    hasNextPage: merged.hasNextPage,
    isLoading: queries === null || result.isLoading,
    error: result.error,
    isFetchingNextPage,
    fetchNextPage,
  };
}
