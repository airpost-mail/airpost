import { Link } from '@tanstack/react-router';
import { Paperclip, Star } from 'lucide-react';
import { memo, type MouseEvent } from 'react';
import { Checkbox } from '@/components/ui/checkbox';
import type { CategoryJSON, ThreadJSON } from '@/engine';
import { TriageChip } from '@/features/ai';
import { formatListDate } from '@/lib/format';
import { cn } from '@/lib/utils';
import { LabelChip } from '../LabelChip';
import { visibleLabels } from '../labels';
import { messageCount, senderLabel } from './senders';

export interface ThreadRowProps {
  thread: ThreadJSON;
  /** Route params of the list this row is in. */
  accountId: string;
  folderId: string;
  own: ReadonlySet<string>;
  /** Shown as a 6px dot in unified views. */
  accountColor?: string;
  viewing?: CategoryJSON;
  checked: boolean;
  open: boolean;
  cursor: boolean;
  /** Plain clicks open (the link); modified clicks select. Return true to cancel navigation. */
  onRowClick: (e: MouseEvent, thread: ThreadJSON) => boolean;
  onCheck: (thread: ThreadJSON, shift: boolean) => void;
  onStar: (thread: ThreadJSON) => void;
}

export const ThreadRow = memo(function ThreadRow({
  thread: t,
  accountId,
  folderId,
  own,
  accountColor,
  viewing,
  checked,
  open,
  cursor,
  onRowClick,
  onCheck,
  onStar,
}: ThreadRowProps) {
  const unread = t.unread > 0;
  const starred = t.starred > 0;
  const count = messageCount(t);
  const labels = visibleLabels(t.labels, viewing?.id).slice(0, 3);
  const sender = senderLabel(t, own);

  return (
    <li
      data-thread-id={t.id}
      data-cursor={cursor || undefined}
      data-checked={checked || undefined}
      className={cn(
        'group/row relative flex row-height items-center gap-3 border-b border-hairline px-(--pane-gutter) text-sm',
        'transition-colors duration-(--duration-fast) [contain-intrinsic-size:auto_var(--row-height)] [content-visibility:auto]',
        'hover:bg-row-hover data-checked:bg-row-selected',
        open && 'bg-row-selected',
        // The keyboard cursor: a 2px cobalt rule on the leading edge.
        'before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-transparent data-cursor:before:bg-row-focus',
      )}
    >
      <span className="relative z-10 grid size-4 shrink-0 place-items-center">
        <span
          aria-hidden
          className={cn(
            'size-1.5 rounded-full transition-opacity group-hover/row:opacity-0 group-data-checked/row:opacity-0',
            unread ? 'bg-unread' : 'bg-transparent',
          )}
        />
        <Checkbox
          checked={checked}
          aria-label={`Select ${t.subject || 'thread'}`}
          onClick={(e) => {
            e.preventDefault();
            onCheck(t, e.shiftKey);
          }}
          className="absolute opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100 data-checked:opacity-100"
        />
      </span>

      <Link
        to="/$accountId/$folderId/$threadId"
        params={{ accountId, folderId, threadId: t.id }}
        search={accountId === t.aid ? {} : { account: t.aid }}
        onClick={(e) => {
          if (onRowClick(e, t)) e.preventDefault();
        }}
        className={cn(
          'flex min-w-0 flex-1 items-baseline gap-3 outline-none after:absolute after:inset-0',
          'in-data-[density=comfortable]:flex-col in-data-[density=comfortable]:items-stretch in-data-[density=comfortable]:gap-0.5',
          'focus-visible:after:outline-2 focus-visible:after:-outline-offset-2 focus-visible:after:outline-ring',
        )}
      >
        <span
          className={cn(
            'flex w-32 shrink-0 items-center gap-1.5 truncate in-data-[density=comfortable]:w-auto @xl:w-44',
            unread && 'font-semibold',
          )}
        >
          {accountColor && (
            <span
              aria-hidden
              className="size-1.5 shrink-0 rounded-full"
              style={{ background: accountColor }}
            />
          )}
          <span className="truncate">{sender}</span>
          {count > 1 && (
            <span className="font-mono text-2xs font-normal text-muted-foreground tabular">
              {count}
            </span>
          )}
        </span>
        <span className="flex min-w-0 flex-1 items-baseline gap-1.5">
          {labels.map((l) => (
            <LabelChip key={l.id} label={l} className="self-center" />
          ))}
          <span className="truncate">
            <span className={cn(unread && 'font-semibold')}>{t.subject || '(no subject)'}</span>
            {t.snippet && <span className="text-muted-foreground"> — {t.snippet}</span>}
          </span>
        </span>
      </Link>

      <span className="relative z-10 flex shrink-0 items-center gap-2">
        <TriageChip thread={t} />
        {t.attachmentCount > 0 && (
          <Paperclip className="size-3.5 text-muted-foreground" aria-label="Has attachments" />
        )}
        <button
          type="button"
          aria-label={starred ? 'Unstar' : 'Star'}
          aria-pressed={starred}
          onClick={() => onStar(t)}
          className={cn(
            'grid size-5 place-items-center rounded-sm text-muted-foreground/60 hover:text-star',
            !starred && 'opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100',
          )}
        >
          <Star className={cn('size-3.5', starred && 'fill-star text-star')} />
        </button>
        <time
          dateTime={new Date(t.lmt * 1000).toISOString()}
          className={cn(
            'w-14 text-right font-mono text-2xs text-muted-foreground tabular',
            unread && 'text-foreground',
          )}
        >
          {formatListDate(t.lmt)}
        </time>
      </span>
    </li>
  );
});
