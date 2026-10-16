import {
  Archive,
  Clock,
  Flag,
  Folder,
  Inbox,
  Mails,
  OctagonAlert,
  PenLine,
  Send,
  Star,
  Tag,
  Trash2,
  type LucideIcon,
} from 'lucide-react';
import {
  categoryDisplayName,
  type CategoryCount,
  type CategoryJSON,
  type CategoryRole,
} from '@/engine';

const ROLE_ICONS: Partial<Record<CategoryRole, LucideIcon>> = {
  inbox: Inbox,
  important: Flag,
  snoozed: Clock,
  starred: Star,
  drafts: PenLine,
  sent: Send,
  archive: Archive,
  all: Mails,
  spam: OctagonAlert,
  trash: Trash2,
};

export function roleIcon(role: CategoryRole, cls?: CategoryJSON['__cls']): LucideIcon {
  return ROLE_ICONS[role] ?? (cls === 'Label' ? Tag : Folder);
}

/** The number a sidebar row shows: unread for most, the total for drafts, none for bins. */
export function sidebarCount(role: CategoryRole, count: CategoryCount | undefined): number {
  if (!count) return 0;
  if (role === 'drafts') return count.total;
  if (role === 'spam' || role === 'trash' || role === 'sent' || role === 'all') return 0;
  return count.unread;
}

/** Nested folders (`Clients/Northwind`) show their last segment, indented by depth. */
export function treeName(cat: CategoryJSON): { name: string; depth: number } {
  const parts = categoryDisplayName(cat).split('/').filter(Boolean);
  return { name: parts[parts.length - 1] ?? categoryDisplayName(cat), depth: parts.length - 1 };
}
