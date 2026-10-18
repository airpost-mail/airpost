// The "Needs reply" list: `ai.triage.list` per opted-in account (it pages thread ids, newest
// first, and drops threads the user answered), then the threads themselves through `query`
// or, on engines without it, `threads.get`.
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef } from 'react';
import {
  ensureConnected,
  hasQuery,
  RpcError,
  RpcErrorCode,
  useDeltas,
  useEngine,
  type EngineClient,
  type ThreadJSON,
} from '@/engine';
import { aiKeys, useAiSettings, useTriageShown } from './hooks';

/** Threads per account; the engine allows 500 (ms-ipc READ_MAX_LIMIT). */
export const NEEDS_REPLY_LIMIT = 100;

export interface NeedsReplyList {
  threads: ThreadJSON[];
  /** Every match, beyond the ones shown. */
  total: number;
}

async function threadsOf(client: EngineClient, accountId: string, ids: string[]) {
  if (ids.length === 0) return [];
  if (hasQuery(client)) {
    const r = await client.call('query', {
      class: 'Thread',
      accounts: [accountId],
      where: { attr: 'id', op: 'in', value: ids },
      limit: ids.length,
    });
    return r.rows as ThreadJSON[];
  }
  const got = await Promise.all(
    ids.map((id) =>
      client.call('threads.get', { accountId, id }).then(
        (r) => r.thread,
        (err: unknown) => {
          if (err instanceof RpcError && err.code === RpcErrorCode.NotFound) return null;
          throw err;
        },
      ),
    ),
  );
  return got.filter((t): t is ThreadJSON => !!t);
}

export async function fetchNeedsReply(
  client: EngineClient,
  accounts: readonly string[],
): Promise<NeedsReplyList> {
  await ensureConnected(client);
  const per = await Promise.all(
    accounts.map(async (accountId) => {
      const { threadIds, total } = await client.call('ai.triage.list', {
        accountIds: [accountId],
        category: 'needs-reply',
        limit: NEEDS_REPLY_LIMIT,
      });
      const threads = await threadsOf(client, accountId, threadIds);
      const order = new Map(threadIds.map((id, i) => [id, i]));
      threads.sort((a, b) => order.get(a.id)! - order.get(b.id)!);
      return { threads, total };
    }),
  );
  const threads = per.flatMap((p) => p.threads);
  // Each account's page is newest first; merged, the newest received mail leads.
  if (per.length > 1) threads.sort((a, b) => b.lmrt - a.lmrt || (a.id < b.id ? 1 : -1));
  return { threads, total: per.reduce((n, p) => n + p.total, 0) };
}

/** The accounts the list covers: opted in with triage on, or just `accountId`. */
export function useNeedsReplyAccounts(accountId?: string): string[] {
  const settings = useAiSettings();
  const shown = useTriageShown(accountId ?? '');
  if (accountId) return shown ? [accountId] : [];
  return settings.features.triage.enabled ? settings.accounts : [];
}

export function useNeedsReply(accounts: string[]) {
  const client = useEngine();
  const qc = useQueryClient();
  const key = aiKeys.needsReply(accounts);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // A triage value, a reply or a move changes the list: read it again, at most every 500 ms.
  useDeltas(
    (d) => {
      const relevant = accounts.includes(d.accountId);
      if (!relevant || timer.current) return;
      timer.current = setTimeout(() => {
        timer.current = null;
        void qc.invalidateQueries({ queryKey: aiKeys.needsReplyAll() });
      }, 500);
    },
    { modelClass: 'Thread' },
  );
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  return useQuery({
    queryKey: key,
    queryFn: () => fetchNeedsReply(client, accounts),
    enabled: accounts.length > 0,
    placeholderData: (prev) => prev,
  });
}
