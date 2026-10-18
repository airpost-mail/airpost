import { Link, useNavigate } from '@tanstack/react-router';
import { MessageSquareReply } from 'lucide-react';
import type { ReactNode } from 'react';
import { EmptyState } from '@/components/EmptyState';
import { MailPanes } from '@/components/MailPanes';
import { Skeleton } from '@/components/ui/skeleton';
import {
  useAccounts,
  useEngineFeatures,
  EngineFeature,
  type AccountJSON,
  type ThreadJSON,
} from '@/engine';
import { formatListDate } from '@/lib/format';
import { useShortcut } from '@/lib/shortcuts';
import { cn } from '@/lib/utils';
import { unavailableMessage, useAiAvailability, useAiSettings } from './hooks';
import { useNeedsReply, useNeedsReplyAccounts } from './needs-reply';
import { triageOf } from './triage';

export interface OpenThread {
  accountId: string;
  threadId: string;
}

/** The sidebar link, shown when the engine triages at least one account. */
export function NeedsReplyLink() {
  const features = useEngineFeatures();
  const accounts = useNeedsReplyAccounts();
  if (!features?.has(EngineFeature.AiTriage) || accounts.length === 0) return null;
  return (
    <Link
      to="/needs-reply"
      className="flex h-7 items-center gap-2 rounded-md px-2 text-sm text-sidebar-foreground transition-colors duration-(--duration-fast) hover:bg-sidebar-accent data-[status=active]:bg-sidebar-accent data-[status=active]:font-medium"
    >
      <MessageSquareReply className="size-4 text-muted-foreground" />
      Needs reply
    </Link>
  );
}

function sender(t: ThreadJSON, own: ReadonlySet<string>): string {
  const p = t.participants.find((c) => !own.has(c.email.toLowerCase())) ?? t.participants[0];
  return p ? p.name || p.email : '(no sender)';
}

/**
 * Threads triage says need a reply, across the opted-in accounts or one of them. The
 * route hands in the reader, so this feature doesn't depend on the mailbox.
 */
export function NeedsReplyView({
  accountId,
  openThread,
  reader,
}: {
  accountId?: string;
  openThread: OpenThread | null;
  reader: ReactNode;
}) {
  const navigate = useNavigate();
  const settings = useAiSettings();
  const triage = useAiAvailability('triage', accountId);
  const accounts = useNeedsReplyAccounts(accountId);
  const { data: allAccounts = [] } = useAccounts();
  const list = useNeedsReply(accounts);
  const threads = list.data?.threads ?? [];
  const byId = new Map(allAccounts.map((a) => [a.id, a]));
  const own = new Set(
    allAccounts.flatMap((a) => [a.emailAddress, ...(a.aliases ?? [])]).map((e) => e.toLowerCase()),
  );
  const optedIn = allAccounts.filter((a) => settings.accounts.includes(a.id));

  const openIndex = openThread
    ? threads.findIndex((t) => t.id === openThread.threadId && t.aid === openThread.accountId)
    : -1;
  const open = (i: number) => {
    const t = threads[i];
    if (t)
      void navigate({
        to: '/needs-reply',
        search: (prev) => ({ ...prev, thread: t.id, threadAccount: t.aid }),
        replace: openThread !== null,
      });
  };
  const enabled = () => threads.length > 0;
  useShortcut(['j', 'down'], () => open(Math.min(threads.length - 1, openIndex + 1)), {
    description: 'Next thread',
    group: 'Navigation',
    enabled,
  });
  useShortcut(['k', 'up'], () => open(Math.max(0, openIndex - 1)), {
    description: 'Previous thread',
    group: 'Navigation',
    enabled,
  });

  let body: ReactNode;
  if (accounts.length === 0) {
    body = (
      <EmptyState
        title="No triage here"
        description={
          triage.available ? 'Triage is off for this account.' : unavailableMessage(triage.reason)
        }
      />
    );
  } else if (list.isLoading) {
    body = (
      <div className="flex flex-col gap-2 p-3" aria-busy>
        {Array.from({ length: 6 }, (_, i) => (
          <Skeleton key={i} className="h-7" />
        ))}
      </div>
    );
  } else if (list.error) {
    body = <EmptyState title="Lost signal" description="Couldn't reach the engine. Try again." />;
  } else if (threads.length === 0) {
    body = <EmptyState title="All caught up" description="Nothing is waiting for your reply." />;
  } else {
    body = (
      <ul className="h-full overflow-y-auto" aria-label="Needs reply">
        {threads.map((t, i) => (
          <Row
            key={`${t.aid}:${t.id}`}
            thread={t}
            account={byId.get(t.aid)}
            showAccount={!accountId && optedIn.length > 1}
            sender={sender(t, own)}
            selected={i === openIndex}
          />
        ))}
      </ul>
    );
  }

  const shownCount = threads.length;
  const listPane = (
    <div className="flex h-full flex-col">
      <header className="flex shrink-0 flex-col gap-2 border-b px-(--pane-gutter) pt-3 pb-2.5">
        <div className="flex items-baseline justify-between gap-3">
          <h1 className="text-2xl leading-none font-semibold tracking-tight">Needs reply</h1>
          {list.data && (
            <span className="font-mono text-2xs text-muted-foreground tabular" aria-live="polite">
              {shownCount}
              {list.data.total > shownCount ? ` of ${list.data.total}` : ''}{' '}
              {list.data.total === 1 ? 'thread' : 'threads'}
            </span>
          )}
        </div>
        {optedIn.length > 1 && (
          <div className="flex flex-wrap gap-1" role="group" aria-label="Account">
            <AccountFilter active={!accountId} label="All accounts" />
            {optedIn.map((a) => (
              <AccountFilter
                key={a.id}
                id={a.id}
                active={accountId === a.id}
                label={a.label || a.emailAddress}
              />
            ))}
          </div>
        )}
        <p className="text-2xs text-muted-foreground">
          Sorted by the triage model on the engine. Threads leave this list once you reply.
        </p>
      </header>
      <div className="min-h-0 flex-1">{body}</div>
    </div>
  );

  return <MailPanes list={listPane} open={!!openThread} reader={reader} />;
}

function AccountFilter({ id, active, label }: { id?: string; active: boolean; label: string }) {
  return (
    <Link
      to="/needs-reply"
      search={{ account: id }}
      aria-current={active ? 'true' : undefined}
      className={cn(
        'h-6 rounded-md border px-2 text-xs leading-6 text-muted-foreground hover:text-foreground',
        active && 'border-primary text-foreground',
      )}
    >
      {label}
    </Link>
  );
}

function Row({
  thread: t,
  account,
  showAccount,
  sender,
  selected,
}: {
  thread: ThreadJSON;
  account?: AccountJSON;
  showAccount: boolean;
  sender: string;
  selected: boolean;
}) {
  const unread = t.unread > 0;
  const reason = triageOf(t)?.reason;
  return (
    <li>
      <Link
        to="/needs-reply"
        search={(prev) => ({ ...prev, thread: t.id, threadAccount: t.aid })}
        aria-current={selected ? 'true' : undefined}
        className={cn(
          'flex row-height items-center gap-3 border-b border-hairline px-(--pane-gutter) text-sm outline-offset-[-2px] transition-colors duration-(--duration-fast) hover:bg-row-hover',
          selected && 'bg-row-selected hover:bg-row-selected',
        )}
      >
        <span
          aria-label={unread ? 'Unread' : undefined}
          className={cn('size-1.5 shrink-0 rounded-full', unread && 'bg-unread')}
        />
        <span className="flex w-40 shrink-0 items-center gap-1.5">
          {showAccount && account && (
            <span
              aria-hidden
              className="size-1.5 shrink-0 rounded-full"
              style={{ backgroundColor: account.color }}
            />
          )}
          <span className={cn('truncate', unread && 'font-semibold')}>{sender}</span>
        </span>
        <span className="min-w-0 flex-1 truncate">
          <span className={cn(unread && 'font-semibold')}>{t.subject || '(no subject)'}</span>
          {reason && <span className="text-muted-foreground"> — {reason}</span>}
        </span>
        <span className="shrink-0 font-mono text-2xs text-muted-foreground tabular">
          {formatListDate(t.lmrt || t.lmt)}
        </span>
      </Link>
    </li>
  );
}
