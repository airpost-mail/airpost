import {
  hashKey,
  queryOptions,
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { ConnectionProblem, ConnectionState, EngineClient } from './client';
import { ensureConnected } from './connect';
import { useEngine } from './context';
import {
  engineKeys,
  EVERY_ACCOUNT,
  listsOutsideAllMail,
  type EventRange,
  type SearchQuery,
  type ThreadQuery,
} from './keys';
import type { ServerIdentityState } from './feed';
import { liveQueryOptions, setLiveWindow, type LiveSnapshot } from './live';
import { sortCategories, THREAD_SORT_ATTR, type ThreadSort } from './model';
import { applyModels, type ThreadIntent } from './intents';
import {
  RpcError,
  RpcErrorCode,
  type AccountListEntry,
  type MatcherJSON,
  type QueryParams,
  type CategoryCount,
  type RpcParams,
  type RpcParamsArg,
  type SearchResult,
} from './rpc';
import type {
  AccountJSON,
  CalendarJSON,
  CategoryJSON,
  ContactJSON,
  Delta,
  EventOccurrenceJSON,
  FolderJSON,
  LabelJSON,
  MessageJSON,
  ModelClass,
  ProcessStateJSON,
  TaskJSON,
  ThreadJSON,
} from './types';

export { useEngine } from './context';
export { EVERY_ACCOUNT, type EventRange, type SearchQuery, type ThreadQuery } from './keys';

const PAGE_SIZE = 50;
const NO_COUNTS: Record<string, CategoryCount> = {};
const CONTACTS_LIMIT = 10;
const NO_THREADS: ThreadJSON[] = [];

async function call<T>(client: EngineClient, fn: () => Promise<T>): Promise<T> {
  await ensureConnected(client);
  return fn();
}

/** The engine's `hello.features`; null until the first handshake. */
export function useEngineFeatures(): ReadonlySet<string> | null {
  const client = useEngine();
  return useSyncExternalStore(
    (cb) => client.onStateChange(cb),
    () => client.features,
  );
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
  // `syncState` and `syncError` as the desktop writes them.
  const a = (e.account ?? {}) as Partial<AccountJSON>;
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

/** An account's folders, live. */
export interface FolderList {
  folders: FolderJSON[];
}

/** An account's labels, live. */
export interface LabelList {
  labels: LabelJSON[];
}

export function foldersQuery(client: EngineClient, accountId: string) {
  return liveQueryOptions(client, {
    key: engineKeys.folders(accountId),
    query: () => ({
      class: 'Folder',
      accounts: [accountId],
      order: [{ attr: 'path', dir: 'asc' }],
    }),
    select: (s: LiveSnapshot<FolderJSON>): FolderList => ({ folders: s.rows }),
  });
}

export function labelsQuery(client: EngineClient, accountId: string) {
  return liveQueryOptions(client, {
    key: engineKeys.labels(accountId),
    query: () => ({ class: 'Label', accounts: [accountId], order: [{ attr: 'path', dir: 'asc' }] }),
    select: (s: LiveSnapshot<LabelJSON>): LabelList => ({ labels: s.rows }),
  });
}

/** Unread and total threads of every account's folders and labels, by category id. */
export function countsQuery(client: EngineClient) {
  return queryOptions({
    queryKey: engineKeys.counts(),
    queryFn: async () => (await call(client, () => client.call('counts.get', {}))).counts,
    staleTime: Infinity,
  });
}

export function useCounts() {
  return useQuery(countsQuery(useEngine()));
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
  const { data: counts = NO_COUNTS } = useCounts();
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
export function threadQueryFor(
  category: Pick<CategoryJSON, 'aid' | 'id' | '__cls' | 'role'>,
): ThreadQuery {
  const categoryRole = category.role;
  return category.__cls === 'Label'
    ? { accountId: category.aid, labelId: category.id, categoryRole }
    : { accountId: category.aid, folderId: category.id, categoryRole };
}

/** The sort of a thread list read with `query`: Sent by last sent, the rest by last received. */
export function threadSortFor(q: ThreadQuery): Exclude<ThreadSort, 'lmt'> {
  return q.categoryRole === 'sent' ? 'lmst' : 'lmrt';
}

/**
 * The live query of a thread list, as the desktop's mailbox perspectives build it: the
 * categories, `inAllMail` outside spam and trash, the flags, newest first, and the `summary`
 * a row shows of the thread's messages.
 */
export function threadListQuery(q: ThreadQuery): QueryParams<'Thread'> {
  const one = q.folderId ?? q.labelId;
  const categories = q.categoryIds ?? (one ? [one] : null);
  const where: MatcherJSON[] = [];
  if (categories) where.push({ attr: 'categories', op: 'containsAny', value: categories });
  if (!listsOutsideAllMail(q)) where.push({ attr: 'inAllMail', op: '=', value: true });
  if (q.starred !== undefined) where.push({ attr: 'starred', op: '=', value: q.starred });
  if (q.unread !== undefined) where.push({ attr: 'unread', op: '=', value: q.unread });
  return {
    class: 'Thread',
    accounts: q.accountId === EVERY_ACCOUNT ? null : [q.accountId],
    where,
    order: [{ attr: THREAD_SORT_ATTR[threadSortFor(q)], dir: 'desc' }],
    include: ['summary'],
  };
}

/** A thread list's rows as the cache holds them. */
export type ThreadList = LiveSnapshot<ThreadJSON>;

const asThreadList = (s: ThreadList): ThreadList => s;

/**
 * A thread list over a live query: the window starts at one page and grows by a page per
 * `fetchNextPage`; the engine keeps the rows in it current.
 */
export function useThreads(query: ThreadQuery | null, opts: { pageSize?: number } = {}) {
  const client = useEngine();
  const qc = useQueryClient();
  const pageSize = opts.pageSize ?? PAGE_SIZE;
  const key = useMemo(
    () => (query ? engineKeys.threadList(query) : (['engine', 'threads', '', {}] as const)),
    [query],
  );
  const hash = hashKey(key);
  const [shown, setShown] = useState({ hash, pages: 1 });
  const pages = shown.hash === hash ? shown.pages : 1;
  const limit = pages * pageSize;
  const result = useQuery({
    ...liveQueryOptions(client, {
      key,
      query: () => threadListQuery(query!),
      window: { offset: 0, limit },
      select: asThreadList,
    }),
    enabled: !!query,
  });
  useEffect(() => {
    if (query) setLiveWindow(qc, key, { offset: 0, limit });
  }, [qc, key, query, limit]);
  const threads = result.data?.rows ?? NO_THREADS;
  const total = result.data?.total ?? 0;
  const hasNextPage = threads.length < total;
  const isFetchingNextPage = hasNextPage && threads.length < limit;
  const fetchNextPage = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) setShown({ hash, pages: pages + 1 });
  }, [hash, pages, hasNextPage, isFetchingNextPage]);
  return {
    threads,
    isLoading: result.isLoading,
    error: result.error,
    hasNextPage,
    isFetchingNextPage,
    fetchNextPage,
  };
}

function findCachedThread(qc: QueryClient, accountId: string, id: string): ThreadJSON | undefined {
  const single = qc.getQueryData<ThreadJSON | null>(engineKeys.thread(accountId, id));
  if (single) return single;
  for (const key of [engineKeys.threadLists(accountId), engineKeys.threadLists(EVERY_ACCOUNT)]) {
    for (const [, data] of qc.getQueriesData<ThreadList>({ queryKey: key })) {
      const hit = data?.rows.find((t) => t.id === id && t.aid === accountId);
      if (hit) return hit;
    }
  }
  for (const [, result] of qc.getQueriesData<SearchResult>({
    queryKey: engineKeys.searchAll(),
  })) {
    const hit = result?.threads?.find((t) => t.id === id && t.aid === accountId);
    if (hit) return hit;
  }
  return undefined;
}

/** A thread, live; null when it is gone. A list's copy shows until the engine answers. */
export function useThread(accountId: string | undefined, threadId: string | undefined) {
  const client = useEngine();
  const qc = useQueryClient();
  return useQuery({
    ...liveQueryOptions(client, {
      key: engineKeys.thread(accountId ?? '', threadId ?? ''),
      query: () => ({
        class: 'Thread',
        accounts: [accountId!],
        where: [{ attr: 'id', op: '=', value: threadId! }],
      }),
      window: { offset: 0, limit: 1 },
      select: (s: LiveSnapshot<ThreadJSON>): ThreadJSON | null => s.rows[0] ?? null,
    }),
    placeholderData: () =>
      accountId && threadId ? findCachedThread(qc, accountId, threadId) : undefined,
    enabled: !!accountId && !!threadId,
  });
}

/**
 * Asks the engine to download the bodies it doesn't have (`bodies.fetch`); they arrive in
 * the messages' live query. Drafts live in the local store, so there is nothing to download
 * for them. Each message is asked for once while the thread is open.
 */
function useBodies(accountId: string | undefined, messages?: MessageJSON[]) {
  const client = useEngine();
  const asked = useRef(new Set<string>());
  useEffect(() => {
    if (!accountId || !messages) return;
    const missing = messages
      .filter((m) => m.body == null && !m.draft && !asked.current.has(m.id))
      .map((m) => m.id);
    if (missing.length === 0) return;
    for (const id of missing) asked.current.add(id);
    void client.call('bodies.fetch', { accountId, ids: missing }).catch(() => {
      for (const id of missing) asked.current.delete(id);
    });
  }, [client, accountId, messages]);
}

/** Messages of a thread with their bodies, oldest first, live. */
export function useMessages(accountId: string | undefined, threadId: string | undefined) {
  const client = useEngine();
  const result = useQuery({
    ...liveQueryOptions(client, {
      key: engineKeys.messages(accountId ?? '', threadId ?? ''),
      query: () => ({
        class: 'Message',
        accounts: [accountId!],
        where: [{ attr: 'threadId', op: '=', value: threadId! }],
        order: [
          { attr: 'date', dir: 'asc' },
          { attr: 'id', dir: 'asc' },
        ],
        include: ['body'],
      }),
      select: (s: LiveSnapshot<MessageJSON>): MessageJSON[] => s.rows,
    }),
    enabled: !!accountId && !!threadId,
  });
  useBodies(accountId, result.data);
  return result;
}

/** `search` results; `truncated` when some account had more than `limit` matches. */
export interface SearchHits extends SearchResult {
  truncated: boolean;
}

/** The zone and clock the engine reads a grammar query's dates in. */
export function searchClock(): { tz: string; now: number } {
  return {
    tz: Intl.DateTimeFormat().resolvedOptions().timeZone,
    now: Math.floor(Date.now() / 1000),
  };
}

export function useSearch(q: SearchQuery | null, opts: { limit?: number } = {}) {
  const client = useEngine();
  const query = q?.query.trim() ?? '';
  const limit = opts.limit ?? PAGE_SIZE;
  const accountId = q?.accountId;
  // One live query across accounts; its dates are read at the clock it subscribed with.
  return useQuery({
    ...liveQueryOptions(client, {
      key: engineKeys.search({ query, accountId, limit }),
      query: () => ({
        class: 'Thread',
        accounts: accountId ? [accountId] : null,
        search: { text: query, grammar: true, ...searchClock() },
        order: [{ attr: THREAD_SORT_ATTR.lmrt, dir: 'desc' }],
      }),
      window: { offset: 0, limit },
      select: (s: LiveSnapshot<ThreadJSON>): SearchHits => ({
        threads: s.rows,
        truncated: s.total > s.rows.length,
      }),
    }),
    enabled: query.length > 0,
    placeholderData: (prev) => prev,
  });
}

/** The engine refused the params: for a grammar search, text that doesn't parse. */
export function isInvalidParams(err: unknown): err is RpcError {
  return err instanceof RpcError && err.code === RpcErrorCode.InvalidParams;
}

export function useContacts(query: string, opts: { accountId?: string; limit?: number } = {}) {
  const client = useEngine();
  const q = query.trim();
  const limit = opts.limit ?? CONTACTS_LIMIT;
  return useQuery({
    queryKey: engineKeys.contacts({ query: q, accountId: opts.accountId, limit }),
    queryFn: async (): Promise<ContactJSON[]> => {
      await ensureConnected(client);
      // One read across accounts; twice the limit leaves room for the same address in
      // several accounts.
      const r = await client.call('query', {
        class: 'Contact',
        accounts: opts.accountId ? [opts.accountId] : null,
        search: { text: q, grammar: false },
        where: [{ attr: 'hidden', op: '=', value: false }],
        order: [{ attr: 'refs', dir: 'desc' }],
        limit: limit * 2,
      });
      const contacts = r.rows as ContactJSON[];
      const seen = new Set<string>();
      return contacts
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

/** Every account's calendars. */
export function useCalendars() {
  const client = useEngine();
  return useQuery({
    queryKey: engineKeys.calendars(),
    queryFn: async (): Promise<CalendarJSON[]> => {
      await ensureConnected(client);
      const r = await client.call('query', { class: 'Calendar' });
      return (r.rows as CalendarJSON[]).sort(
        (a, b) => (a.order ?? 0) - (b.order ?? 0) || a.name.localeCompare(b.name),
      );
    },
    staleTime: Infinity,
  });
}

/** The browser's zone, which `events.expand` counts calendar days in. */
const localZone = () => Intl.DateTimeFormat().resolvedOptions().timeZone;

/**
 * The occurrences overlapping the range in the given calendars, recurring events expanded by
 * the engine (`events.expand`), cancelled ones left out, by start.
 */
export function useEvents(range: EventRange | null) {
  const client = useEngine();
  return useQuery({
    queryKey: range ? engineKeys.events(range) : engineKeys.eventsAll(),
    queryFn: async (): Promise<EventOccurrenceJSON[]> => {
      const { start, end, calendarIds } = range!;
      if (calendarIds.length === 0) return [];
      await ensureConnected(client);
      const r = await client.call('events.expand', { calendarIds, start, end, tz: localZone() });
      return (r.occurrences as EventOccurrenceJSON[]).filter((o) => !o.isCancelled);
    },
    enabled: !!range,
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

/** Queues a task (build it with `@/engine/tasks`); thread changes go through `useIntents`. */
export function useQueueTask() {
  const client = useEngine();
  return useMutation({
    mutationFn: (task: TaskJSON) =>
      call(client, () => client.call('task.queue', { accountId: task.aid, task })),
  });
}

/**
 * The engine's intents and undo: `run` calls a `threads.*` intent and `undo` an entry's
 * `undo.apply`; both write the Thread rows the engine answers with into the cache.
 */
export function useIntents() {
  const client = useEngine();
  const qc = useQueryClient();
  return useMemo(
    () => ({
      run: async <M extends ThreadIntent>(method: M, params: RpcParams<M>) => {
        const result = await call(client, () =>
          client.call(method, ...([params] as RpcParamsArg<M>)),
        );
        applyModels(qc, (result as { models?: unknown[] }).models);
        return result;
      },
      undo: async (token: string) => {
        const result = await call(client, () => client.call('undo.apply', { token }));
        applyModels(qc, result.models);
        return result;
      },
    }),
    [client, qc],
  );
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
