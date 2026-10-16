import {
  Archive,
  FolderInput,
  MailOpen,
  Mail,
  OctagonAlert,
  RefreshCw,
  Star,
  Tag,
  Trash2,
  X,
} from 'lucide-react';
import { Checkbox } from '@/components/ui/checkbox';
import { Separator } from '@/components/ui/separator';
import { cn } from '@/lib/utils';
import { ActionButton } from '../ActionButton';

export interface BulkHandlers {
  archive: () => void;
  trash: () => void;
  spam: () => void;
  markRead: () => void;
  markUnread: () => void;
  star: () => void;
  move: () => void;
  label?: () => void;
}

export function ListHeader({
  title,
  subtitle,
  unread,
  loaded,
  selectedCount,
  anyUnread,
  allStarred,
  onToggleAll,
  onClear,
  onRefresh,
  refreshing,
  bulk,
}: {
  title: string;
  subtitle?: string;
  unread?: number;
  loaded: number;
  selectedCount: number;
  anyUnread: boolean;
  allStarred: boolean;
  onToggleAll: () => void;
  onClear: () => void;
  onRefresh: () => void;
  refreshing: boolean;
  bulk: BulkHandlers;
}) {
  const selecting = selectedCount > 0;
  const allChecked = selecting && selectedCount === loaded;
  return (
    <header className="flex h-11 shrink-0 items-center gap-3 border-b border-hairline px-(--pane-gutter)">
      <span className="grid size-4 shrink-0 place-items-center">
        <Checkbox
          aria-label={allChecked ? 'Select none' : 'Select all'}
          checked={allChecked ? true : selecting ? 'indeterminate' : false}
          disabled={loaded === 0}
          onCheckedChange={onToggleAll}
        />
      </span>
      {selecting ? (
        <div className="flex min-w-0 flex-1 animate-in items-center gap-0.5 duration-(--duration-base) fade-in">
          <span className="mr-2 font-mono text-xs tabular">{selectedCount} selected</span>
          <ActionButton icon={Archive} label="Archive" keys="e" onClick={bulk.archive} />
          <ActionButton icon={Trash2} label="Trash" keys="#" onClick={bulk.trash} />
          <ActionButton icon={OctagonAlert} label="Spam" keys="!" onClick={bulk.spam} />
          <Separator orientation="vertical" className="mx-1 h-4" />
          {anyUnread ? (
            <ActionButton
              icon={MailOpen}
              label="Mark read"
              keys="shift+i"
              onClick={bulk.markRead}
            />
          ) : (
            <ActionButton
              icon={Mail}
              label="Mark unread"
              keys="shift+u"
              onClick={bulk.markUnread}
            />
          )}
          <ActionButton
            icon={Star}
            label={allStarred ? 'Unstar' : 'Star'}
            keys="s"
            active={allStarred}
            onClick={bulk.star}
          />
          <ActionButton icon={FolderInput} label="Move to" keys="v" onClick={bulk.move} />
          {bulk.label && <ActionButton icon={Tag} label="Label" keys="l" onClick={bulk.label} />}
          <ActionButton
            icon={X}
            label="Clear selection"
            keys="escape"
            onClick={onClear}
            className="ml-auto"
          />
        </div>
      ) : (
        <div className="flex min-w-0 flex-1 items-baseline gap-2">
          <h1 className="truncate text-sm font-semibold">{title}</h1>
          {subtitle && <span className="truncate text-xs text-muted-foreground">{subtitle}</span>}
          {unread !== undefined && unread > 0 && (
            <span className="font-mono text-2xs text-unread tabular">{unread} unread</span>
          )}
          <ActionButton
            icon={RefreshCw}
            label="Check for mail"
            onClick={onRefresh}
            className={cn('ml-auto self-center', refreshing && '[&_svg]:animate-spin')}
          />
        </div>
      )}
    </header>
  );
}
