import { Link } from '@tanstack/react-router';
import { Paperclip, Star } from 'lucide-react';
import type { Ref } from 'react';
import type { AccountJSON, ThreadJSON } from '@/engine';
import { formatListDate } from '@/lib/format';
import { cn } from '@/lib/utils';
import { Highlight } from './Highlight';
import { threadSender } from './thread-sender';

export interface SearchResultRowProps {
  thread: ThreadJSON;
  account?: AccountJSON;
  /** Show the account colour dot (unified results). */
  showAccount: boolean;
  terms: string[];
  selected: boolean;
  cursor: boolean;
  ref?: Ref<HTMLAnchorElement>;
}

export function SearchResultRow({
  thread: t,
  account,
  showAccount,
  terms,
  selected,
  cursor,
  ref,
}: SearchResultRowProps) {
  const unread = t.unread > 0;
  return (
    <Link
      ref={ref}
      to="/search"
      search={(prev) => ({ ...prev, thread: t.id, threadAccount: t.aid })}
      aria-current={selected ? 'true' : undefined}
      className={cn(
        'flex row-height items-center gap-3 border-b border-hairline px-(--pane-gutter) text-sm outline-offset-[-2px] transition-colors duration-(--duration-fast) hover:bg-row-hover',
        selected && 'bg-row-selected hover:bg-row-selected',
        cursor && 'shadow-[inset_2px_0_0_var(--color-row-focus)]',
      )}
    >
      <span
        aria-label={unread ? 'Unread' : undefined}
        className={cn('size-1.5 shrink-0 rounded-full', unread && 'bg-unread')}
      />
      <span
        className={cn(
          'flex min-w-0 flex-1 items-center gap-3',
          'in-data-[density=comfortable]:flex-col in-data-[density=comfortable]:items-stretch in-data-[density=comfortable]:gap-0.5',
        )}
      >
        <span className="flex w-40 shrink-0 items-center gap-1.5 in-data-[density=comfortable]:w-auto">
          {showAccount && account && (
            <span
              className="size-1.5 shrink-0 rounded-full"
              style={{ backgroundColor: account.color }}
              title={account.label || account.emailAddress}
            />
          )}
          <span className={cn('truncate', unread && 'font-semibold')}>
            <Highlight text={threadSender(t, account)} terms={terms} />
          </span>
        </span>
        <span className="min-w-0 truncate">
          <span className={cn(unread && 'font-semibold')}>
            <Highlight text={t.subject || '(no subject)'} terms={terms} />
          </span>
          {t.snippet && (
            <span className="text-muted-foreground">
              {' '}
              — <Highlight text={t.snippet} terms={terms} />
            </span>
          )}
        </span>
      </span>
      <span className="flex shrink-0 items-center gap-2 self-center text-muted-foreground in-data-[density=comfortable]:self-start in-data-[density=comfortable]:pt-2.5">
        {t.attachmentCount > 0 && <Paperclip className="size-3.5" aria-label="Has attachments" />}
        {t.starred > 0 && <Star className="size-3.5 fill-star text-star" aria-label="Starred" />}
        <span className="font-mono text-2xs tabular">{formatListDate(t.lmt)}</span>
      </span>
    </Link>
  );
}
