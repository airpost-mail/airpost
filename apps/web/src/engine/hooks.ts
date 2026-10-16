import {
  queryOptions,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query';
import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { applyDelta, type ServerIdentityState, type ThreadPages } from './cache';
import type { ConnectionProblem, ConnectionState, EngineClient } from './client';
import { ensureConnected } from './connect';
import { useEngine } from './context';
import { engineKeys, type SearchQuery, type ThreadQuery } from './keys';
import { compareThreads, sortCategories } from './model';
import { optimisticThreads } from './optimistic';
import {
  RpcError,
  RpcErrorCode,
  type AccountListEntry,
  type CategoryCount,
  type SearchResult,
  type ThreadsGetResult,
  type ThreadsListParams,
  type ThreadsListResult,
} from './rpc';
import type {
  AccountJSON,
  CategoryJSON,
  ContactJSON,
  Delta,
  MessageJSON,
  ModelClass,
  ProcessStateJSON,
  TaskJSON,
  ThreadJSON,
} from './types';

export { useEngine } from './context';
export type { SearchQuery, ThreadQuery } from './keys';

const PAGE_SIZE = 50;
const CONTACTS_LIMIT = 10;
/** Pages of all mail a client-filtered list (Starred) reads per page it shows, at most. */
const FILTER_ROUNDS = 10;

async function call<T>(client: EngineClient, fn: () => Promise<T>): Promise<T> {
  await ensureConnected(client);
  return fn();
}

function isNotFound(err: unknown): boolean {
  return err instanceof RpcError && err.code === RpcErrorCode.NotFound;
}

export function useConnectionState(): ConnectionState {
  const client = useEngine();
  return useSyncExternalStore(
    (cb) => client.onStateChange(cb),
    () => client.state,
  );
}

/**
 * Account JSON from an `account.list` row. Until the engine's registry (DCP stage 11) the
 * `account` copy may be missing, so the row's id and email fill a minimal account.
 */
export function accountFromListEntry(e: AccountListEntry): AccountJSON {
  const a = e.account ?? {};
  const stopped = e.state === 'stopped';
  return {
    name: '',
    provider: 'imap',
    label: e.email,
    settings: {},
    autoaddress: { value: '', type: 'bcc' },
    aliases: [],
    defaultAlias: null,
    color: '',
    ...a,
    __cls: 'Account',
    id: e.accountId,
    aid: e.accountId,
    v: a.v ?? 0,
    emailAddress: a.emailAddress ?? e.email,
    syncState: stopped ? 'sync_error' : (a.syncState ?? 'ok'),
    syncError: stopped ? (e.error ?? a.syncError ?? {}) : (a.syncError ?? null),
  };
}

export function accountsQuery(client: EngineClient) {
  return queryOptions({
    queryKey: engineKeys.accounts(),
    queryFn: async () => {
      const { accounts } = await call(client, () => client.call('account.list'));
      return accounts.filter((e) => e.state !== 'removed').map(accountFromListEntry);
    },
    staleTime: Infinity,
  });
}

export function useAccounts() {
  return useQuery(accountsQuery(useEngine()));
}

export function useAccount(accountId: string | undefined): AccountJSON | undefined {
  const { data } = useAccounts();
  return data?.find((a) => a.id === accountId);
}

export function engineStatusQuery(client: EngineClient) {
  return queryOptions({
    queryKey: engineKeys.status(),
    queryFn: () => call(client, () => client.call('engine.status')),
  });
}

export function useEngineStatus() {
  return useQuery({ ...engineStatusQuery(useEngine()), refetchInterval: 30_000 });
}

/**
 * Seeds online/offline per account from `engine.status`. ProcessState deltas only report
 * changes, so an account already offline when the tab connects would otherwise look online.
 */
export async function seedProcessState(qc: QueryClient, client: EngineClient) {
  const status = await qc.fetchQuery(engineStatusQuery(client));
  qc.setQueryData<Record<string, ProcessStateJSON>>(engineKeys.processState(), (old = {}) => {
    const next = { ...old };
    for (const a of status.accounts) {
      next[a.accountId] = { accountId: a.accountId, connectionError: a.connectionError };
    }
    return next;
  });
}

/** Online/offline per account: seeded from `engine.status`, then kept by ProcessState deltas. */
export function useProcessState(): Record<string, ProcessStateJSON> {
  const qc = useQueryClient();
  const { data } = useQuery({
    queryKey: engineKeys.processState(),
    queryFn: () =>
      qc.getQueryData<Record<string, ProcessStateJSON>>(engineKeys.processState()) ?? {},
    staleTime: Infinity,
  });
  return data ?? {};
}

/** Whether the engine's Tern server rejected its token since this tab connected. */
export function useServerIdentityState(): ServerIdentityState {
  const qc = useQueryClient();
  const { data } = useQuery({
    queryKey: engineKeys.serverIdentity(),
    queryFn: () =>
      qc.getQueryData<ServerIdentityState>(engineKeys.serverIdentity()) ?? { refreshNeeded: false },
    staleTime: Infinity,
  });
  return data ?? { refreshNeeded: false };
}

export function useConnectionProblem(): ConnectionProblem | null {
  const client = useEngine();
  return useSyncExternalStore(
    (cb) => client.onStateChange(cb),
    () => client.problem,
  );
}

export function foldersQuery(client: EngineClient, accountId: string) {
  return queryOptions({
    queryKey: engineKeys.folders(accountId),
    queryFn: () => call(client, () => client.call('folders.list', { accountId })),
    staleTime: Infinity,
  });
}

export function labelsQuery(client: EngineClient, accountId: string) {
  return queryOptions({
    queryKey: engineKeys.labels(accountId),
    queryFn: () => call(client, () => client.call('labels.list', { accountId })),
    staleTime: Infinity,
  });
}

export function useFolders(accountId: string | undefined) {
  const client = useEngine();
  return useQuery({ ...foldersQuery(client, accountId ?? ''), enabled: !!accountId });
}

export function useLabels(accountId: string | undefined) {
  const client = useEngine();
  return useQuery({ ...labelsQuery(client, accountId ?? ''), enabled: !!accountId });
}

/**
 * Folders and labels of one account, in sidebar order (standard roles first), and their
 * unread/total counts by category id.
 */
export function useCategories(accountId: string | undefined) {
  const folders = useFolders(accountId);
  const labels = useLabels(accountId);
  const all = useMemo<CategoryJSON[]>(
    () => sortCategories([...(folders.data?.folders ?? []), ...(labels.data?.labels ?? [])]),
    [folders.data, labels.data],
  );
  const counts = useMemo<Record<string, CategoryCount>>(
    () => ({ ...folders.data?.counts, ...labels.data?.counts }),
    [folders.data, labels.data],
  );
  return {
    folders: folders.data?.folders ?? [],
    labels: labels.data?.labels ?? [],
    all,
    counts,
    isLoading: folders.isLoading || labels.isLoading,
    error: folders.error ?? labels.error,
  };
}

/** The thread list query for a folder or label. */
export function threadQueryFor(category: Pick<CategoryJSON, 'aid' | 'id' | '__cls'>): ThreadQuery {
  return category.__cls === 'Label'
    ? { accountId: category.aid, labelId: category.id }
    : { accountId: category.aid, folderId: category.id };
}

/**
 * One page of a thread list. A client-side filter (`starred`) reads pages of the account's
 * whole list until it has `limit` matches or the list ends, and pages on from where it
 * stopped reading.
 */
export async function fetchThreadPage(
  client: EngineClient,
  query: ThreadQuery,
  limit: number,
  before: string | null,
): Promise<ThreadsListResult> {
  const { starred, ...rest } = query;
  const params = (cursor: string | null): ThreadsListParams =>
    cursor ? { ...rest, limit, before: cursor } : { ...rest, limit };
  await ensureConnected(client);
  if (starred === undefined) return client.call('threads.list', params(before));
  // TODO(engine): threads.list has no flag filters (starred, unread), so Starred reads the
  // whole account and filters here. DCP stage 7's `query` with a `where` on `starred` lets
  // the engine page it.
  const threads: ThreadJSON[] = [];
  let next = before;
  for (let i = 0; i < FILTER_ROUNDS; i++) {
    const page = await client.call('threads.list', params(next));
    threads.push(...page.threads.filter((t) => t.starred > 0 === starred));
    next = page.next;
    if (!next || threads.length >= limit) break;
  }
  return { threads, next };
}

export function useThreads(query: ThreadQuery | null, opts: { pageSize?: number } = {}) {
  const client = useEngine();
  const limit = opts.pageSize ?? PAGE_SIZE;
  const result = useInfiniteQuery({
    queryKey: query ? engineKeys.threadList(query) : ['engine', 'threads', '', {}],
    queryFn: ({ pageParam }) => fetchThreadPage(client, query!, limit, pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (last: ThreadsListResult) => last.next,
    enabled: !!query,
  });
  const threads = useMemo(() => result.data?.pages.flatMap((p) => p.threads) ?? [], [result.data]);
  return { ...result, threads };
}

function findCachedThread(qc: QueryClient, accountId: string, id: string): ThreadJSON | undefined {
  const single = qc.getQueryData<ThreadJSON | null>(engineKeys.thread(accountId, id));
  if (single) return single;
  for (const [, data] of qc.getQueriesData<ThreadPages>({
    queryKey: engineKeys.threadLists(accountId),
  })) {
    const hit = data?.pages.flatMap((p) => p.threads).find((t) => t.id === id);
    if (hit) return hit;
  }
  for (const [, result] of qc.getQueriesData<SearchResult>({
    queryKey: engineKeys.searchAll(),
  })) {
    const hit = result?.threads?.find((t) => t.id === id && t.aid === accountId);
    if (hit) return hit;
  }
  return undefined;
}

/**
 * `threads.get`, shared by `useThread` and `useMessages` so one call fills both. Null when the
 * thread is gone.
 */
function fetchThreadDetail(
  qc: QueryClient,
  client: EngineClient,
  accountId: string,
  id: string,
): Promise<ThreadsGetResult | null> {
  return qc.fetchQuery({
    queryKey: engineKeys.threadGet(accountId, id),
    queryFn: () =>
      call(client, () => client.call('threads.get', { accountId, id })).catch((err: unknown) => {
        if (isNotFound(err)) return null;
        throw err;
      }),
    staleTime: 0,
  });
}

export function useThread(accountId: string | undefined, threadId: string | undefined) {
  const client = useEngine();
  const qc = useQueryClient();
  return useQuery({
    queryKey: engineKeys.thread(accountId ?? '', threadId ?? ''),
    queryFn: async () =>
      (await fetchThreadDetail(qc, client, accountId!, threadId!))?.thread ?? null,
    initialData: () =>
      accountId && threadId ? findCachedThread(qc, accountId, threadId) : undefined,
    enabled: !!accountId && !!threadId,
  });
}

/** Keeps the cached body of an unchanged message so a refetch doesn't reload every body. */
function withCachedBodies(messages: MessageJSON[], cached: MessageJSON[] | undefined) {
  if (!cached) return messages;
  const byId = new Map(cached.map((m) => [m.id, m]));
  return messages.map((m) => {
    const old = byId.get(m.id);
    return old?.body != null && old.v === m.v ? { ...m, body: old.body } : m;
  });
}

/**
 * Loads bodies for messages that have none: `messages.get body:true` for what the engine has
 * stored, then `bodies.fetch` for what it hasn't downloaded (those arrive as Message deltas).
 * Each message version is looked up once.
 */
function useBodies(
  accountId: string | undefined,
  threadId: string | undefined,
  messages?: MessageJSON[],
) {
  const client = useEngine();
  const qc = useQueryClient();
  const checked = useRef(new Set<string>());
  const downloading = useRef(new Set<string>());
  // A body the engine downloads may arrive without `body` on the delta; look it up again.
  useDeltas(
    (d) => {
      if (d.type !== 'persist') return;
      for (const m of d.modelJSONs as MessageJSON[]) {
        if (downloading.current.delete(m.id)) checked.current.delete(`${m.id}:${m.v}`);
      }
    },
    { accountId, modelClass: 'Message' },
  );
  useEffect(() => {
    if (!accountId || !threadId || !messages) return;
    const pending = messages.filter(
      (m) => m.body == null && !checked.current.has(`${m.id}:${m.v}`),
    );
    if (pending.length === 0) return;
    const key = engineKeys.messages(accountId, threadId);
    for (const m of pending) checked.current.add(`${m.id}:${m.v}`);
    void Promise.all(
      pending.map((m) =>
        client.call('messages.get', { accountId, id: m.id, body: true }).catch(() => {
          checked.current.delete(`${m.id}:${m.v}`);
          return null;
        }),
      ),
    ).then((results) => {
      const found = new Map<string, MessageJSON>();
      const missing: string[] = [];
      results.forEach((r, i) => {
        if (!r) return;
        if (r.body != null) found.set(r.message.id, { ...r.message, body: r.body });
        // Drafts live in the local store; there is nothing to download for them.
        else if (!pending[i].draft) missing.push(r.message.id);
      });
      if (found.size > 0) {
        qc.setQueryData<MessageJSON[]>(key, (list) =>
          list?.map((m) => {
            // A later version may have replaced it meanwhile; that one is looked up itself.
            const hit = found.get(m.id);
            return m.body == null && hit?.v === m.v ? { ...m, body: hit.body } : m;
          }),
        );
      }
      if (missing.length > 0) {
        for (const id of missing) downloading.current.add(id);
        void client.call('bodies.fetch', { accountId, ids: missing }).catch(() => {
          for (const id of missing) {
            const m = pending.find((p) => p.id === id)!;
            checked.current.delete(`${m.id}:${m.v}`);
          }
        });
      }
    });
  }, [client, qc, accountId, threadId, messages]);
}

/** Messages of a thread, oldest first, with bodies filled in as they load. */
export function useMessages(accountId: string | undefined, threadId: string | undefined) {
  const client = useEngine();
  const qc = useQueryClient();
  const result = useQuery({
    queryKey: engineKeys.messages(accountId ?? '', threadId ?? ''),
    queryFn: async () => {
      const detail = await fetchThreadDetail(qc, client, accountId!, threadId!);
      const cached = qc.getQueryData<MessageJSON[]>(engineKeys.messages(accountId!, threadId!));
      return withCachedBodies(detail?.messages ?? [], cached);
    },
    enabled: !!accountId && !!threadId,
  });
  useBodies(accountId, threadId, result.data);
  return result;
}

/** Ids to fan a cross-account read out to: `accountId` alone, or every account. */
async function targetAccounts(qc: QueryClient, client: EngineClient, accountId?: string) {
  if (accountId) return [accountId];
  const accounts = await qc.ensureQueryData(accountsQuery(client));
  return accounts.map((a) => a.id);
}

/** `search` results; `truncated` when some account had more than `limit` matches. */
export interface SearchHits extends SearchResult {
  truncated: boolean;
}

export function useSearch(q: SearchQuery | null, opts: { limit?: number } = {}) {
  const client = useEngine();
  const qc = useQueryClient();
  const query = q?.query.trim() ?? '';
  const limit = opts.limit ?? PAGE_SIZE;
  return useQuery({
    queryKey: engineKeys.search({ query, accountId: q?.accountId, limit }),
    queryFn: async (): Promise<SearchHits> => {
      // TODO(engine): `search` takes one accountId; searching every account means one call
      // per account, merged and cut to `limit` here, with no paging. `search` gains
      // `accounts` with DCP stage 9.
      const ids = await targetAccounts(qc, client, q?.accountId);
      const results = await Promise.all(
        ids.map((accountId) =>
          call(client, () => client.call('search', { accountId, query, limit })),
        ),
      );
      const threads = results.flatMap((r) => r.threads).sort(compareThreads);
      return {
        threads: threads.slice(0, limit),
        truncated: threads.length > limit || results.some((r) => r.threads.length >= limit),
      };
    },
    enabled: query.length > 0,
    placeholderData: (prev) => prev,
  });
}

export function useContacts(query: string, opts: { accountId?: string; limit?: number } = {}) {
  const client = useEngine();
  const qc = useQueryClient();
  const q = query.trim();
  const limit = opts.limit ?? CONTACTS_LIMIT;
  return useQuery({
    queryKey: engineKeys.contacts({ query: q, accountId: opts.accountId, limit }),
    queryFn: async (): Promise<ContactJSON[]> => {
      // TODO(engine): `contacts.search` takes one accountId; autocomplete across accounts
      // calls each one and merges by reference count here. DCP stage 7's `query` on Contact
      // with `accounts: null` replaces it.
      const ids = await targetAccounts(qc, client, opts.accountId);
      const results = await Promise.all(
        ids.map((accountId) =>
          call(client, () => client.call('contacts.search', { accountId, query: q, limit })),
        ),
      );
      const seen = new Set<string>();
      return results
        .flatMap((r) => r.contacts)
        .sort((a, b) => b.refs - a.refs)
        .filter((c) => {
          const email = c.email.toLowerCase();
          return seen.has(email) ? false : (seen.add(email), true);
        })
        .slice(0, limit);
    },
    enabled: q.length > 0,
    placeholderData: (prev) => prev,
  });
}

/** Recent tasks of an account, as reported by Task deltas (newest last). */
export function useTasks(accountId: string | undefined): TaskJSON[] {
  const qc = useQueryClient();
  const { data } = useQuery({
    queryKey: engineKeys.tasks(accountId ?? ''),
    queryFn: () => qc.getQueryData<TaskJSON[]>(engineKeys.tasks(accountId ?? '')) ?? [],
    enabled: !!accountId,
    staleTime: Infinity,
  });
  return data ?? [];
}

/**
 * Queues a task (build it with `@/engine/tasks`). Thread changes are applied to the cache
 * at once; on failure the account's lists are refetched.
 */
export function useQueueTask() {
  const client = useEngine();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (task: TaskJSON) =>
      call(client, () => client.call('task.queue', { accountId: task.aid, task })),
    onMutate: (task) => {
      const accounts = qc.getQueryData<AccountJSON[]>(engineKeys.accounts()) ?? [];
      const threads = optimisticThreads(task, {
        thread: (id) => findCachedThread(qc, task.aid, id),
        isGmail: (aid) => accounts.find((a) => a.id === aid)?.provider === 'gmail',
      });
      if (threads.length > 0) {
        applyDelta(qc, {
          accountId: task.aid,
          modelClass: 'Thread',
          type: 'persist',
          modelJSONs: threads,
        });
      }
    },
    onError: (_err, task) => {
      void qc.invalidateQueries({ queryKey: engineKeys.threadLists(task.aid) });
      void qc.invalidateQueries({ queryKey: ['engine', 'thread', task.aid] });
    },
  });
}

export function useCancelTask() {
  const client = useEngine();
  return useMutation({
    mutationFn: ({ accountId, taskId }: { accountId: string; taskId: string }) =>
      call(client, () => client.call('task.cancel', { accountId, taskId })),
  });
}

export function useWakeSync() {
  const client = useEngine();
  return useMutation({
    mutationFn: (accountId?: string) =>
      call(client, () => client.call('sync.wake', accountId ? { accountId } : {})),
  });
}

/** Calls `listener` for every delta, optionally filtered. The listener may change freely. */
export function useDeltas(
  listener: (delta: Delta) => void,
  filter: { accountId?: string; modelClass?: ModelClass } = {},
) {
  const client = useEngine();
  const ref = useRef(listener);
  useEffect(() => {
    ref.current = listener;
  });
  const { accountId, modelClass } = filter;
  useEffect(
    () =>
      client.onNotification((n) => {
        if (n.method !== 'delta') return;
        if (accountId && n.params.accountId !== accountId) return;
        if (modelClass && n.params.modelClass !== modelClass) return;
        ref.current(n.params);
      }),
    [client, accountId, modelClass],
  );
}

export type { MessageJSON, ThreadJSON };
