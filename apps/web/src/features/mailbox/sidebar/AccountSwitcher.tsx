import { Link, useNavigate } from '@tanstack/react-router';
import { ChevronsUpDown, Layers, Plus } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { AccountJSON } from '@/engine';
import { UNIFIED_ACCOUNT_ID } from '../mailbox';
import type { AccountData } from '../use-account-data';
import { sidebarCount } from './nav-model';

function AccountDot({ account }: { account: AccountJSON }) {
  return (
    <span
      aria-hidden
      className="size-1.5 shrink-0 rounded-full"
      style={{ background: account.color || 'var(--primary)' }}
    />
  );
}

function inboxUnread(a: AccountData) {
  const inbox = a.categories?.find((c) => c.role === 'inbox');
  return inbox ? sidebarCount('inbox', a.counts?.[inbox.id]) : 0;
}

export function AccountSwitcher({
  accounts,
  current,
}: {
  accounts: AccountData[];
  /** An account id, or `all`. */
  current: string;
}) {
  const navigate = useNavigate();
  const active = accounts.find((a) => a.account.id === current)?.account;
  const unifiedUnread = accounts.reduce((s, a) => s + inboxUnread(a), 0);

  const go = (accountId: string) => {
    if (accountId === UNIFIED_ACCOUNT_ID) {
      void navigate({ to: '/$accountId/$folderId', params: { accountId, folderId: 'inbox' } });
      return;
    }
    const inbox = accounts
      .find((a) => a.account.id === accountId)
      ?.categories?.find((c) => c.role === 'inbox');
    if (inbox)
      void navigate({ to: '/$accountId/$folderId', params: { accountId, folderId: inbox.id } });
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="mx-2 mt-2 flex h-11 items-center gap-2.5 rounded-md px-2 text-left outline-offset-2 transition-colors duration-(--duration-fast) hover:bg-sidebar-accent data-[state=open]:bg-sidebar-accent">
        {active ? (
          <>
            <AccountDot account={active} />
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-sm font-medium">{active.label || active.name}</span>
              <span className="truncate text-2xs text-muted-foreground">{active.emailAddress}</span>
            </span>
          </>
        ) : (
          <>
            <Layers className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-sm font-medium">All accounts</span>
              <span className="truncate text-2xs text-muted-foreground">
                {accounts.length} {accounts.length === 1 ? 'account' : 'accounts'}
              </span>
            </span>
          </>
        )}
        <ChevronsUpDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-64">
        <DropdownMenuLabel className="text-2xs tracking-widest uppercase">
          Accounts
        </DropdownMenuLabel>
        {accounts.length > 1 && (
          <DropdownMenuItem onSelect={() => go(UNIFIED_ACCOUNT_ID)}>
            <Layers className="text-muted-foreground" />
            <span className="flex-1">All accounts</span>
            {unifiedUnread > 0 && (
              <span className="font-mono text-2xs text-muted-foreground tabular">
                {unifiedUnread}
              </span>
            )}
          </DropdownMenuItem>
        )}
        {accounts.map((a) => (
          <DropdownMenuItem key={a.account.id} onSelect={() => go(a.account.id)}>
            <span className="grid size-4 place-items-center">
              <AccountDot account={a.account} />
            </span>
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate">{a.account.label || a.account.name}</span>
              <span className="truncate text-2xs text-muted-foreground">
                {a.account.emailAddress}
              </span>
            </span>
            {inboxUnread(a) > 0 && (
              <span className="font-mono text-2xs text-muted-foreground tabular">
                {inboxUnread(a)}
              </span>
            )}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link to="/settings">
            <Plus className="text-muted-foreground" />
            Add account
          </Link>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
