import { Inbox } from 'lucide-react';
import { UNIFIED_ACCOUNT_ID, UNIFIED_ROLES, mailboxTitle, roleQuery } from '../mailbox';
import type { AccountData } from '../use-account-data';
import { NavItem } from './NavItem';
import { NavSection } from './NavSection';
import { roleIcon, sidebarCount } from './nav-model';

export function UnifiedNav({ accounts }: { accounts: AccountData[] }) {
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
      <NavSection id="all:accounts" title="Inboxes">
        {loaded.map(({ account, categories, counts }) => {
          const inbox = categories!.find((c) => c.role === 'inbox');
          if (!inbox) return null;
          return (
            <NavItem
              key={account.id}
              accountId={account.id}
              folderId={inbox.id}
              icon={
                <span className="relative">
                  <Inbox />
                  <span
                    aria-hidden
                    className="absolute -right-0.5 -bottom-0.5 size-1.5 rounded-full ring-2 ring-sidebar"
                    style={{ background: account.color || 'var(--primary)' }}
                  />
                </span>
              }
              name={account.label || account.emailAddress}
              count={sidebarCount('inbox', counts?.[inbox.id])}
              emphasis
            />
          );
        })}
      </NavSection>
    </>
  );
}
