import { useNavigate } from '@tanstack/react-router';
import { useMemo, useRef, useState } from 'react';
import { EmptyState } from '@/components/EmptyState';
import { MailPanes } from '@/components/MailPanes';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Skeleton } from '@/components/ui/skeleton';
import {
  isInvalidParams,
  useAccounts,
  useSearch,
  useSearchGrammar,
  type AccountJSON,
  type ThreadJSON,
} from '@/engine';
import { ReaderEmpty, ThreadReaderView } from '@/features/mailbox';
import { useShortcut } from '@/lib/shortcuts';
import {
  dateRange,
  editSearchQuery,
  inDateRange,
  isEmptySearch,
  parseSearchQuery,
  searchLayout,
  serializeSearchQuery,
  toEngineQuery,
  type ParsedSearch,
} from './query';
import { SearchFilters } from './SearchFilters';
import { SearchResultRow } from './SearchResultRow';

const RESULT_LIMIT = 100;
const HIGHLIGHTED = new Set(['from', 'to', 'subject']);

export interface OpenThread {
  accountId: string;
  threadId: string;
}

export function SearchView({
  query,
  accountId,
  openThread,
}: {
  query: string;
  accountId?: string;
  openThread?: OpenThread | null;
}) {
  const navigate = useNavigate();
  const parsed = useMemo(() => parseSearchQuery(query), [query]);
  const grammar = useSearchGrammar();
  const engineQuery = toEngineQuery(query, grammar);
  const { data: accounts = [] } = useAccounts();
  const search = useSearch(engineQuery ? { query: engineQuery, accountId, grammar } : null, {
    limit: RESULT_LIMIT,
  });

  // An engine without the grammar can't read dates: they are applied here.
  const range = useMemo(() => (grammar ? null : dateRange(parsed)), [grammar, parsed]);
  const threads = useMemo(() => {
    if (!engineQuery) return [];
    const found = search.data?.threads ?? [];
    return range ? found.filter((t) => inDateRange(t.lmt, range)) : found;
  }, [engineQuery, search.data, range]);

  // With the grammar the text's order and operators carry meaning: edit it in place.
  const layout = useMemo(() => (grammar ? searchLayout(query) : undefined), [grammar, query]);
  const setParsed = (next: ParsedSearch) => {
    const q = grammar ? editSearchQuery(query, parsed, next) : serializeSearchQuery(next);
    if (q === null) return;
    void navigate({ to: '/search', search: (prev) => ({ ...prev, q: q || undefined }) });
  };
  const setAccount = (next: string | undefined) =>
    void navigate({ to: '/search', search: (prev) => ({ ...prev, account: next }) });

  const list = (
    <div className="flex h-full flex-col">
      <header className="flex shrink-0 flex-col gap-2.5 border-b px-(--pane-gutter) pt-3 pb-2.5">
        <div className="flex items-baseline justify-between gap-3">
          <h1 className="text-2xl leading-none font-semibold tracking-tight">Search</h1>
          <ResultCount
            show={!!engineQuery}
            fetching={search.isFetching}
            count={threads.length}
            more={!!search.data?.truncated}
          />
        </div>
        <SearchFilters
          parsed={parsed}
          layout={layout}
          onChange={setParsed}
          accounts={accounts}
          accountId={accountId}
          onAccountChange={setAccount}
        />
      </header>
      <div className="min-h-0 flex-1">
        <Results
          parsed={parsed}
          engineQuery={engineQuery}
          threads={threads}
          loading={search.isLoading}
          error={search.error}
          more={!!search.data?.truncated}
          openThread={openThread ?? null}
          accounts={accounts}
          scopedToAccount={!!accountId}
        />
      </div>
    </div>
  );

  return (
    <MailPanes
      list={list}
      open={!!openThread}
      reader={
        openThread ? (
          <ThreadReaderView
            key={`${openThread.accountId}:${openThread.threadId}`}
            accountId={openThread.accountId}
            threadId={openThread.threadId}
          />
        ) : (
          <ReaderEmpty />
        )
      }
    />
  );
}

function ResultCount({
  show,
  fetching,
  count,
  more,
}: {
  show: boolean;
  fetching: boolean;
  count: number;
  more: boolean;
}) {
  if (!show) return null;
  return (
    <span className="font-mono text-2xs text-muted-foreground tabular" aria-live="polite">
      {fetching ? 'searching…' : `${count}${more ? '+' : ''} ${count === 1 ? 'thread' : 'threads'}`}
    </span>
  );
}

function Results({
  parsed,
  engineQuery,
  threads,
  loading,
  error,
  more,
  openThread,
  accounts,
  scopedToAccount,
}: {
  parsed: ParsedSearch;
  engineQuery: string;
  threads: ThreadJSON[];
  loading: boolean;
  error: Error | null;
  more: boolean;
  openThread: OpenThread | null;
  accounts: AccountJSON[];
  scopedToAccount: boolean;
}) {
  const navigate = useNavigate();
  const openIndex = openThread
    ? threads.findIndex((t) => t.id === openThread.threadId && t.aid === openThread.accountId)
    : -1;
  const [cursorState, setCursor] = useState(-1);
  const cursor = openIndex !== -1 ? openIndex : Math.min(cursorState, threads.length - 1);
  const rows = useRef<(HTMLAnchorElement | null)[]>([]);

  const open = (i: number, replace = false) => {
    const t = threads[i];
    if (!t) return;
    void navigate({
      to: '/search',
      search: (prev) => ({ ...prev, thread: t.id, threadAccount: t.aid }),
      replace,
    });
  };
  const move = (delta: number) => {
    if (threads.length === 0) return;
    const next = Math.max(0, Math.min(threads.length - 1, cursor + delta));
    setCursor(next);
    rows.current[next]?.scrollIntoView({ block: 'nearest' });
    // With a thread open, moving opens the neighbour, as in the mailbox's split view.
    if (openThread) open(next, true);
  };
  const close = () =>
    void navigate({
      to: '/search',
      search: (prev) => ({ ...prev, thread: undefined, threadAccount: undefined }),
    });

  const enabled = () => threads.length > 0;
  useShortcut(['j', 'down'], () => move(1), {
    description: 'Next result',
    group: 'Search',
    enabled,
  });
  useShortcut(['k', 'up'], () => move(-1), {
    description: 'Previous result',
    group: 'Search',
    enabled,
  });
  useShortcut(['enter', 'o'], () => open(Math.max(cursor, 0)), {
    description: 'Open result',
    group: 'Search',
    enabled,
  });
  useShortcut(['escape', 'u'], close, {
    description: 'Close result',
    group: 'Search',
    enabled: () => !!openThread,
  });

  if (isEmptySearch(parsed)) {
    return (
      <EmptyState
        title="Find anything"
        description={
          <>
            Press <ShortcutKeys keys="/" className="align-middle" /> and type. Try{' '}
            <code className="font-mono text-xs text-foreground">from:ana has:attachment</code> or{' '}
            <code className="font-mono text-xs text-foreground">is:unread after:7d</code>.
          </>
        }
      />
    );
  }
  if (!engineQuery) {
    // Only an engine without the grammar, which gets no dates: nothing is left to send.
    return (
      <EmptyState
        title="Almost there"
        description="Dates narrow a search. Add a word or another filter to go with them."
      />
    );
  }
  if (isInvalidParams(error)) {
    return (
      <EmptyState
        title="Can’t read that search"
        description={error.message.replace(/^invalid params: (search: )?/, '')}
      />
    );
  }
  if (error) {
    return <EmptyState title="Lost signal" description="Couldn't reach the engine. Try again." />;
  }
  if (loading) {
    return (
      <div className="flex flex-col gap-2 p-3" aria-busy>
        {Array.from({ length: 8 }, (_, i) => (
          <Skeleton key={i} className="h-7" />
        ))}
      </div>
    );
  }
  if (threads.length === 0) {
    return (
      <EmptyState
        title="No sightings"
        description="Nothing matches. Remove a filter or widen the dates."
      />
    );
  }

  const byId = new Map(accounts.map((a) => [a.id, a]));
  const highlight = [
    ...parsed.terms,
    ...parsed.tokens.filter((t) => HIGHLIGHTED.has(t.field)).map((t) => t.value),
  ];
  return (
    <div className="h-full overflow-y-auto" role="list" aria-label="Search results">
      {threads.map((t, i) => (
        <div role="listitem" key={`${t.aid}:${t.id}`}>
          <SearchResultRow
            ref={(el) => {
              rows.current[i] = el;
            }}
            thread={t}
            account={byId.get(t.aid)}
            showAccount={!scopedToAccount && accounts.length > 1}
            terms={highlight}
            selected={i === openIndex}
            cursor={i === cursor && i !== openIndex}
          />
        </div>
      ))}
      {more && (
        <p className="px-(--pane-gutter) py-3 text-xs text-muted-foreground">
          Showing the first {RESULT_LIMIT}. Narrow the search to see the rest.
        </p>
      )}
    </div>
  );
}
