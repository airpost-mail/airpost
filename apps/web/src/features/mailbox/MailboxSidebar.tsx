import { Link, useNavigate, useParams } from '@tanstack/react-router';
import { CalendarDays } from 'lucide-react';
import { useEffect } from 'react';
import { ScrollArea } from '@/components/ui/scroll-area';
import type { CategoryRole } from '@/engine';
import { NeedsReplyLink } from '@/features/ai';
import { useShortcut } from '@/lib/shortcuts';
import { setLastAccount, useLastAccount } from './last-account';
import { isUnifiedRole, STARRED_VIEW_ID, UNIFIED_ACCOUNT_ID } from './mailbox';
import { AccountNav } from './sidebar/AccountNav';
import { AccountSwitcher } from './sidebar/AccountSwitcher';
import { UnifiedNav } from './sidebar/UnifiedNav';
import { useAllAccountData } from './use-account-data';

export function MailboxSidebar() {
  const { accounts } = useAllAccountData();
  const params = useParams({ strict: false });
  const navigate = useNavigate();

  const known = (id: string | null | undefined): id is string =>
    !!id && (id === UNIFIED_ACCOUNT_ID || accounts.some((a) => a.account.id === id));
  const fromUrl = known(params.accountId) ? params.accountId : null;
  const last = useLastAccount();
  const scope = known(last)
    ? last
    : accounts.length > 1
      ? UNIFIED_ACCOUNT_ID
      : (accounts[0]?.account.id ?? UNIFIED_ACCOUNT_ID);
  // The unified sidebar holds every account's tree, so it stays put whatever the URL
  // opens. A single-account sidebar follows the URL to another account or to `all`.
  const current = scope === UNIFIED_ACCOUNT_ID ? scope : (fromUrl ?? scope);

  useEffect(() => {
    if (fromUrl && current !== last && accounts.length > 0) setLastAccount(current);
  }, [fromUrl, current, last, accounts.length]);

  const go = (role: CategoryRole) => {
    if (current === UNIFIED_ACCOUNT_ID) {
      if (isUnifiedRole(role)) {
        void navigate({
          to: '/$accountId/$folderId',
          params: { accountId: current, folderId: role },
        });
      }
      return;
    }
    const data = accounts.find((a) => a.account.id === current);
    const cats = data?.categories ?? [];
    const target =
      role === 'all'
        ? (cats.find((c) => c.role === 'all') ?? cats.find((c) => c.role === 'archive'))
        : cats.find((c) => c.role === role);
    const folderId = target?.id ?? (role === 'starred' ? STARRED_VIEW_ID : null);
    if (folderId) {
      void navigate({ to: '/$accountId/$folderId', params: { accountId: current, folderId } });
    }
  };
  const nav = { group: 'Navigation' } as const;
  useShortcut('g i', () => go('inbox'), { ...nav, description: 'Go to inbox' });
  useShortcut('g s', () => go('starred'), { ...nav, description: 'Go to starred' });
  useShortcut('g d', () => go('drafts'), { ...nav, description: 'Go to drafts' });
  useShortcut('g a', () => go('all'), { ...nav, description: 'Go to all mail' });
  useShortcut('g c', () => void navigate({ to: '/calendar' }), {
    ...nav,
    description: 'Go to calendar',
  });

  const active = accounts.find((a) => a.account.id === current);
  return (
    <div className="flex h-full flex-col">
      {accounts.length > 0 && <AccountSwitcher accounts={accounts} current={current} />}
      <ScrollArea className="min-h-0 flex-1">
        {current === UNIFIED_ACCOUNT_ID ? (
          <UnifiedNav accounts={accounts} activeAccountId={fromUrl} />
        ) : (
          active && <AccountNav key={active.account.id} data={active} />
        )}
      </ScrollArea>
      <nav aria-label="Apps" className="border-t p-2">
        <Link
          to="/calendar"
          className="flex h-7 items-center gap-2 rounded-md px-2 text-sm text-sidebar-foreground transition-colors duration-(--duration-fast) hover:bg-sidebar-accent data-[status=active]:bg-sidebar-accent data-[status=active]:font-medium"
        >
          <CalendarDays className="size-4 text-muted-foreground" />
          Calendar
        </Link>
        <NeedsReplyLink />
      </nav>
    </div>
  );
}
