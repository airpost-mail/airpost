import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

export function NavItem({
  accountId,
  folderId,
  icon,
  name,
  count,
  depth = 0,
  emphasis,
}: {
  accountId: string;
  folderId: string;
  icon: ReactNode;
  name: string;
  count?: number;
  depth?: number;
  /** Unread counts are cobalt; totals (drafts) stay muted. */
  emphasis?: boolean;
}) {
  return (
    <Link
      to="/$accountId/$folderId"
      params={{ accountId, folderId }}
      activeOptions={{ includeSearch: false }}
      style={depth ? { paddingLeft: `${0.5 + depth * 0.875}rem` } : undefined}
      className={cn(
        'group/nav flex h-7 items-center gap-2 rounded-md px-2 text-sm text-sidebar-foreground',
        'transition-colors duration-(--duration-fast) hover:bg-sidebar-accent',
        'data-[status=active]:bg-sidebar-accent data-[status=active]:font-medium data-[status=active]:text-sidebar-accent-foreground',
      )}
    >
      <span className="grid size-4 shrink-0 place-items-center text-muted-foreground group-data-[status=active]/nav:text-primary [&_svg]:size-4">
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate">{name}</span>
      {!!count && (
        <span
          className={cn(
            'font-mono text-2xs tabular',
            emphasis ? 'font-medium text-unread' : 'text-muted-foreground',
          )}
        >
          {count > 999 ? '999+' : count}
        </span>
      )}
    </Link>
  );
}
