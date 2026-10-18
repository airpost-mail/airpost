import { useNavigate, useParams } from '@tanstack/react-router';
import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent } from 'react';
import { EmptyState } from '@/components/EmptyState';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import {
  isGmailAccount,
  useWakeSync,
  type CategoryJSON,
  type CategoryRole,
  type ThreadJSON,
} from '@/engine';
import { isMac } from '@/lib/shortcuts';
import { useThreadActions } from './actions';
import { CategoryPicker } from './CategoryPicker';
import { ListHeader } from './list/ListHeader';
import { moveCursor, nextAfterRemoval, useListSelection } from './list/selection';
import { ThreadRow } from './list/ThreadRow';
import { useListShortcuts } from './list/use-list-shortcuts';
import {
  mailboxRole,
  mailboxTitle,
  ownAddresses,
  UNIFIED_ACCOUNT_ID,
  viewCategoryFor,
} from './mailbox';
import { registerListRemover } from './list-bus';
import { openCategoryPicker } from './picker-store';
import { useMailbox } from './use-mailbox';

const EMPTY: Partial<Record<CategoryRole, [string, string]>> = {
  inbox: ['All clear', 'Nothing new has come in.'],
  starred: ['No stars yet', 'Press s on a thread to keep it here.'],
  drafts: ['No drafts', 'Unsent messages wait here.'],
  sent: ['Nothing sent', 'Messages you send appear here.'],
  spam: ['No spam', 'Mail you report as spam lands here.'],
  trash: ['Trash is empty', 'Deleted threads stay here for a while.'],
};

function ListSkeleton() {
  return (
    <div aria-busy aria-label="Loading threads" className="flex flex-col">
      {Array.from({ length: 12 }, (_, i) => (
        <div
          key={i}
          className="flex row-height items-center gap-3 border-b border-hairline px-(--pane-gutter)"
        >
          <span className="size-4 shrink-0" />
          <Skeleton className="h-3 w-32 shrink-0" />
          <Skeleton className="h-3 flex-1" style={{ maxWidth: `${50 + ((i * 37) % 45)}%` }} />
          <Skeleton className="ml-auto h-3 w-10" />
        </div>
      ))}
    </div>
  );
}

export function ThreadListView({ accountId, folderId }: { accountId: string; folderId: string }) {
  const box = useMailbox(accountId, folderId);
  const { mailbox, threads, accounts, hasNextPage, isFetchingNextPage, fetchNextPage } = box;
  const navigate = useNavigate();
  const { threadId: openId } = useParams({ strict: false });
  const actions = useThreadActions();
  const wake = useWakeSync();
  const unified = accountId === UNIFIED_ACCOUNT_ID;

  const ids = useMemo(() => threads.map((t) => t.id), [threads]);
  const byId = useMemo(() => new Map(threads.map((t) => [t.id, t])), [threads]);
  const sel = useListSelection(ids);
  const [cursorRaw, setCursor] = useState<string | null>(null);
  const cursor =
    openId && byId.has(openId) ? openId : cursorRaw && byId.has(cursorRaw) ? cursorRaw : null;

  const own = useMemo(() => ownAddresses(accounts.map((a) => a.account)), [accounts]);
  const categoriesOf = useCallback(
    (aid: string): CategoryJSON[] => accounts.find((a) => a.account.id === aid)?.categories ?? [],
    [accounts],
  );
  const viewingFor = useCallback(
    (aid: string) => (mailbox ? viewCategoryFor(mailbox, aid, categoriesOf(aid)) : undefined),
    [mailbox, categoriesOf],
  );
  const colorOf = (aid: string) =>
    accounts.find((a) => a.account.id === aid)?.account.color || 'var(--primary)';

  const openThread = useCallback(
    (t: ThreadJSON) => {
      setCursor(t.id);
      void navigate({
        to: '/$accountId/$folderId/$threadId',
        params: { accountId, folderId, threadId: t.id },
        search: t.aid === accountId ? {} : { account: t.aid },
      });
    },
    [navigate, accountId, folderId],
  );
  const closeThread = () => {
    void navigate({ to: '/$accountId/$folderId', params: { accountId, folderId } });
  };

  /** What an action applies to: the checked rows, else the open thread or the cursor. */
  const targets = (): ThreadJSON[] => {
    if (sel.selected.size > 0) return threads.filter((t) => sel.selected.has(t.id));
    const t = cursor ? byId.get(cursor) : undefined;
    return t ? [t] : [];
  };

  /** Runs an action that takes threads out of this list, then moves on as Gmail does. */
  const removeThreads = (fn: (ts: ThreadJSON[]) => void, ts: ThreadJSON[]) => {
    if (ts.length === 0) return;
    const gone = new Set(ts.map((t) => t.id));
    const next = nextAfterRemoval(ids, gone, cursor);
    fn(ts);
    sel.clear();
    if (openId && gone.has(openId)) {
      const t = next ? byId.get(next) : undefined;
      if (t) openThread(t);
      else closeThread();
    } else {
      setCursor(next);
    }
  };
  const removing = (fn: (ts: ThreadJSON[]) => void) => () => removeThreads(fn, targets());
  useEffect(() =>
    registerListRemover((remove, ts) =>
      removeThreads(
        remove,
        ts.map((t) => byId.get(t.id) ?? t),
      ),
    ),
  );
  const on = (fn: (ts: ThreadJSON[]) => void) => () => {
    const ts = targets();
    if (ts.length > 0) fn(ts);
  };

  const step = (delta: number) => {
    const next = moveCursor(ids, cursor, delta);
    if (!next) return;
    if (openId) openThread(byId.get(next)!);
    else setCursor(next);
  };

  const canLabel = (ts: ThreadJSON[]) =>
    ts.length > 0 &&
    ts.every((t) => {
      const a = accounts.find((x) => x.account.id === t.aid)?.account;
      return !!a && isGmailAccount(a);
    });
  const picker = (kind: 'move' | 'label') =>
    on((ts) => {
      if (kind === 'label' && !canLabel(ts)) return;
      openCategoryPicker({ kind, threads: ts, viewing: viewingFor(ts[0].aid) });
    });

  const commands = {
    archive: removing(actions.archive),
    trash: removing(actions.trash),
    spam: removing(actions.spam),
    star: on(actions.toggleStarred),
    markRead: on((ts) => actions.setUnread(ts, false)),
    markUnread: on((ts) => actions.setUnread(ts, true)),
    move: picker('move'),
    label: picker('label'),
  };

  useListShortcuts({
    ...commands,
    next: () => step(1),
    prev: () => step(-1),
    open: () => {
      const t = cursor ? byId.get(cursor) : threads[0];
      if (t) openThread(t);
    },
    back: () => {
      if (sel.selected.size > 0) sel.clear();
      else if (openId) closeThread();
    },
    toggleSelect: () => {
      const id = cursor ?? ids[0];
      if (!id) return;
      sel.toggle(id);
      setCursor(id);
    },
    selectAll: sel.selectAll,
    selectNone: sel.clear,
  });

  const { toggle, extendTo, clear, setAnchor } = sel;
  const onRowClick = useCallback(
    (e: MouseEvent, t: ThreadJSON) => {
      setCursor(t.id);
      if (e.shiftKey) {
        extendTo(t.id);
        return true;
      }
      if (isMac ? e.metaKey : e.ctrlKey) {
        toggle(t.id);
        return true;
      }
      clear();
      setAnchor(t.id);
      return false;
    },
    [toggle, extendTo, clear, setAnchor],
  );
  const onCheck = useCallback(
    (t: ThreadJSON, shift: boolean) => {
      if (shift) extendTo(t.id);
      else toggle(t.id);
      setCursor(t.id);
    },
    [toggle, extendTo],
  );
  const onStar = useCallback((t: ThreadJSON) => actions.toggleStarred([t]), [actions]);

  const listRef = useRef<HTMLUListElement>(null);
  useEffect(() => {
    if (!cursor || !listRef.current) return;
    const row = [...listRef.current.children].find(
      (el) => (el as HTMLElement).dataset.threadId === cursor,
    );
    row?.scrollIntoView?.({ block: 'nearest' });
  }, [cursor]);

  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !hasNextPage || typeof IntersectionObserver === 'undefined') return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting) && !isFetchingNextPage) fetchNextPage();
      },
      { rootMargin: '400px' },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  if (mailbox === null) {
    return <EmptyState title="Off the chart" description="This mailbox doesn't exist." />;
  }

  const selected = threads.filter((t) => sel.selected.has(t.id));
  const role = mailbox ? mailboxRole(mailbox) : '';
  const unreadCount =
    !mailbox || mailbox.kind === 'starred' || role === 'drafts' || role === 'sent'
      ? undefined
      : accounts.reduce((sum, a) => {
          const c = viewingFor(a.account.id);
          return sum + (c ? (a.counts?.[c.id]?.unread ?? 0) : 0);
        }, 0);
  const subtitle = unified
    ? undefined
    : accounts.find((a) => a.account.id === accountId)?.account.label;
  const [emptyTitle, emptyHelp] = EMPTY[role] ?? ['Nothing here', 'This folder is empty.'];

  return (
    <div className="flex h-full flex-col">
      <ListHeader
        title={mailbox ? mailboxTitle(mailbox) : ''}
        subtitle={subtitle}
        unread={unreadCount}
        loaded={threads.length}
        selectedCount={selected.length}
        anyUnread={selected.some((t) => t.unread > 0)}
        allStarred={selected.length > 0 && selected.every((t) => t.starred > 0)}
        onToggleAll={() => (selected.length === threads.length ? clear() : sel.selectAll())}
        onClear={clear}
        onRefresh={() => wake.mutate(unified ? undefined : accountId)}
        refreshing={wake.isPending}
        bulk={{ ...commands, label: canLabel(selected) ? commands.label : undefined }}
      />
      {box.isLoading ? (
        <ListSkeleton />
      ) : box.error ? (
        <EmptyState
          title="Lost signal"
          description="Couldn't reach the engine to load this mailbox."
          action={
            <Button size="sm" variant="outline" onClick={() => wake.mutate(undefined)}>
              Try again
            </Button>
          }
        />
      ) : threads.length === 0 ? (
        <EmptyState
          title={emptyTitle}
          description={
            <span className="inline-flex flex-wrap items-center justify-center gap-1.5">
              {emptyHelp}
              {role === 'inbox' && (
                <>
                  <span>Compose with</span>
                  <ShortcutKeys keys="c" />
                </>
              )}
            </span>
          }
        />
      ) : (
        <div className="@container min-h-0 flex-1 overflow-y-auto overscroll-contain">
          <ul
            ref={listRef}
            aria-label={mailbox ? mailboxTitle(mailbox) : 'Threads'}
            className="flex flex-col"
          >
            {threads.map((t) => (
              <ThreadRow
                key={`${t.aid}:${t.id}`}
                thread={t}
                accountId={accountId}
                folderId={folderId}
                own={own}
                accountColor={unified ? colorOf(t.aid) : undefined}
                viewing={viewingFor(t.aid)}
                checked={sel.selected.has(t.id)}
                open={t.id === openId}
                cursor={t.id === cursor}
                onRowClick={onRowClick}
                onCheck={onCheck}
                onStar={onStar}
              />
            ))}
          </ul>
          {hasNextPage && (
            <div ref={sentinel} className="flex justify-center py-3">
              <Button
                size="xs"
                variant="ghost"
                disabled={isFetchingNextPage}
                onClick={fetchNextPage}
                className="font-mono text-2xs text-muted-foreground"
              >
                {isFetchingNextPage ? 'Loading…' : 'Load more'}
              </Button>
            </div>
          )}
        </div>
      )}
      <CategoryPicker />
    </div>
  );
}
