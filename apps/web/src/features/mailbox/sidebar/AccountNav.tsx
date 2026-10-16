import { Star } from 'lucide-react';
import type { ReactNode } from 'react';
import { Skeleton } from '@/components/ui/skeleton';
import { categoryDisplayName, isStandardCategory, type CategoryJSON } from '@/engine';
import { LabelDot } from '../LabelChip';
import { STARRED_VIEW_ID } from '../mailbox';
import type { AccountData } from '../use-account-data';
import { NavItem } from './NavItem';
import { NavSection } from './NavSection';
import { roleIcon, sidebarCount, treeName } from './nav-model';

const TOP_ROLES = new Set(['inbox', 'important', 'snoozed']);

function NavSkeleton() {
  return (
    <div className="flex flex-col gap-1.5 px-4 py-3">
      {Array.from({ length: 7 }, (_, i) => (
        <Skeleton key={i} className="h-4" style={{ width: `${55 + ((i * 23) % 35)}%` }} />
      ))}
    </div>
  );
}

export function AccountNav({ data }: { data: AccountData }) {
  const { account, categories, counts } = data;
  if (!categories) return <NavSkeleton />;

  const standard = categories.filter((c) => isStandardCategory(c));
  const folders = categories.filter((c) => !isStandardCategory(c) && c.__cls === 'Folder');
  const labels = categories.filter((c) => !isStandardCategory(c) && c.__cls === 'Label');

  const item = (c: CategoryJSON) => {
    const Icon = roleIcon(c.role, c.__cls);
    return (
      <NavItem
        key={c.id}
        accountId={account.id}
        folderId={c.id}
        icon={<Icon />}
        name={categoryDisplayName(c)}
        count={sidebarCount(c.role, counts?.[c.id])}
        emphasis={c.role !== 'drafts'}
      />
    );
  };
  const starred = (
    <NavItem
      key={STARRED_VIEW_ID}
      accountId={account.id}
      folderId={STARRED_VIEW_ID}
      icon={<Star />}
      name="Starred"
    />
  );

  // Starred has no folder on most servers; it is a view over the starred flag, placed
  // after Inbox / Important / Snoozed.
  const rows: ReactNode[] = [];
  let placed = standard.some((c) => c.role === 'starred');
  for (const c of standard) {
    if (!placed && !TOP_ROLES.has(c.role)) {
      rows.push(starred);
      placed = true;
    }
    rows.push(item(c));
  }
  if (!placed) rows.push(starred);

  return (
    <>
      <NavSection id={`${account.id}:mailboxes`}>{rows}</NavSection>
      {folders.length > 0 && (
        <NavSection id={`${account.id}:folders`} title="Folders">
          {folders.map((c) => {
            const { name, depth } = treeName(c);
            const Icon = roleIcon(c.role, c.__cls);
            return (
              <NavItem
                key={c.id}
                accountId={account.id}
                folderId={c.id}
                icon={<Icon />}
                name={name}
                depth={depth}
                count={sidebarCount(c.role, counts?.[c.id])}
                emphasis
              />
            );
          })}
        </NavSection>
      )}
      {labels.length > 0 && (
        <NavSection id={`${account.id}:labels`} title="Labels">
          {labels.map((c) => {
            const { name, depth } = treeName(c);
            return (
              <NavItem
                key={c.id}
                accountId={account.id}
                folderId={c.id}
                icon={<LabelDot label={c} className="size-2" />}
                name={name}
                depth={depth}
                count={sidebarCount(c.role, counts?.[c.id])}
                emphasis
              />
            );
          })}
        </NavSection>
      )}
    </>
  );
}
