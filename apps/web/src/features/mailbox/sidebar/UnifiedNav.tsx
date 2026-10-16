import { ChevronRight } from 'lucide-react';
import { cn } from '@/lib/utils';
import { UNIFIED_ACCOUNT_ID, UNIFIED_ROLES, mailboxTitle, roleQuery } from '../mailbox';
import type { AccountData } from '../use-account-data';
import { AccountNav } from './AccountNav';
import { useCollapsed } from './collapsed';
import { NavItem } from './NavItem';
import { NavSection } from './NavSection';
import { roleIcon, sidebarCount } from './nav-model';

export function UnifiedNav({
  accounts,
  activeAccountId,
}: {
  accounts: AccountData[];
  /** The account whose folder the URL shows, if any; its tree starts expanded. */
  activeAccountId: string | null;
}) {
  const loaded = accounts.filter((a) => a.categories);
  const roles = UNIFIED_ROLES.filter((r) =>
    loaded.some((a) => roleQuery(a.account.id, r, a.categories!)),
  );
  const total = (role: (typeof UNIFIED_ROLES)[number]) =>
    loaded.reduce((sum, a) => {
      const c = a.categories!.find((x) => x.role === role);
      return sum + (c ? sidebarCount(role, a.counts?.[c.id]) : 0);
    }, 0);

  return (
    <>
      <NavSection id="all:mailboxes">
        {roles.map((role) => {
          const Icon = roleIcon(role);
          return (
            <NavItem
              key={role}
              accountId={UNIFIED_ACCOUNT_ID}
              folderId={role}
              icon={<Icon />}
              name={mailboxTitle({ kind: 'unified', role })}
              count={total(role)}
              emphasis={role !== 'drafts'}
            />
          );
        })}
      </NavSection>
      <NavSection id="all:accounts" title="Accounts">
        {loaded.map((data) => (
          <AccountTree
            key={data.account.id}
            data={data}
            initiallyOpen={data.account.id === activeAccountId}
          />
        ))}
      </NavSection>
    </>
  );
}

/** One account in the unified sidebar: a toggle row over that account's folders. */
function AccountTree({ data, initiallyOpen }: { data: AccountData; initiallyOpen: boolean }) {
  const { account, categories, counts } = data;
  const [collapsed, toggle] = useCollapsed(`all:account:${account.id}`, !initiallyOpen);
  const inbox = categories?.find((c) => c.role === 'inbox');
  const unread = inbox ? sidebarCount('inbox', counts?.[inbox.id]) : 0;
  return (
    <div className="flex flex-col">
      <button
        type="button"
        onClick={toggle}
        aria-expanded={!collapsed}
        className="group/acct flex h-7 items-center gap-2 rounded-md px-2 text-left text-sm text-sidebar-foreground transition-colors duration-(--duration-fast) hover:bg-sidebar-accent"
      >
        <span className="grid size-4 shrink-0 place-items-center text-muted-foreground">
          <ChevronRight
            aria-hidden
            className={cn(
              'size-3.5 transition-transform duration-(--duration-fast)',
              !collapsed && 'rotate-90',
            )}
          />
        </span>
        <span
          aria-hidden
          className="size-1.5 shrink-0 rounded-full"
          style={{ background: account.color || 'var(--primary)' }}
        />
        <span className="min-w-0 flex-1 truncate">{account.label || account.emailAddress}</span>
        {collapsed && unread > 0 && (
          <span className="font-mono text-2xs font-medium text-unread tabular">
            {unread > 999 ? '999+' : unread}
          </span>
        )}
      </button>
      {!collapsed && (
        <div className="ml-4 border-l border-sidebar-border">
          <AccountNav data={data} />
        </div>
      )}
    </div>
  );
}
