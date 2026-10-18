import {
  Archive,
  ArrowLeft,
  ChevronsDownUp,
  ChevronsUpDown,
  FolderInput,
  Mail,
  OctagonAlert,
  Star,
  Tag,
  Trash2,
} from 'lucide-react';
import { Separator } from '@/components/ui/separator';
import { ActionButton } from '../ActionButton';
import { SnoozeMenu } from './SnoozeMenu';

export interface ReaderToolbarProps {
  onBack: () => void;
  onArchive: () => void;
  onTrash: () => void;
  onSpam: () => void;
  /** Set when the engine has `threads.snooze`. */
  onSnooze?: (at: Date) => void;
  onUnsnooze?: () => void;
  onMarkUnread: () => void;
  onStar: () => void;
  starred: boolean;
  onMove: () => void;
  onLabel?: () => void;
  allExpanded: boolean;
  onToggleExpanded: () => void;
  messageCount: number;
}

export function ReaderToolbar(p: ReaderToolbarProps) {
  return (
    <div
      role="toolbar"
      aria-label="Thread actions"
      className="flex h-11 shrink-0 items-center gap-0.5 border-b border-hairline px-3"
    >
      <ActionButton icon={ArrowLeft} label="Back to list" keys="escape" onClick={p.onBack} />
      <Separator orientation="vertical" className="mx-1 h-4" />
      <ActionButton icon={Archive} label="Archive" keys="e" onClick={p.onArchive} />
      <ActionButton icon={Trash2} label="Trash" keys="#" onClick={p.onTrash} />
      <ActionButton icon={OctagonAlert} label="Spam" keys="!" onClick={p.onSpam} />
      {p.onSnooze && <SnoozeMenu onSnooze={p.onSnooze} onUnsnooze={p.onUnsnooze} />}
      <Separator orientation="vertical" className="mx-1 h-4" />
      <ActionButton icon={Mail} label="Mark unread" keys="shift+u" onClick={p.onMarkUnread} />
      <ActionButton
        icon={Star}
        label={p.starred ? 'Unstar' : 'Star'}
        keys="s"
        active={p.starred}
        onClick={p.onStar}
        className={p.starred ? '[&_svg]:fill-star [&_svg]:text-star' : undefined}
      />
      <ActionButton icon={FolderInput} label="Move to" keys="v" onClick={p.onMove} />
      {p.onLabel && <ActionButton icon={Tag} label="Label" keys="l" onClick={p.onLabel} />}
      {p.messageCount > 1 && (
        <ActionButton
          icon={p.allExpanded ? ChevronsDownUp : ChevronsUpDown}
          label={p.allExpanded ? 'Collapse all' : 'Expand all'}
          onClick={p.onToggleExpanded}
          className="ml-auto"
        />
      )}
    </div>
  );
}
