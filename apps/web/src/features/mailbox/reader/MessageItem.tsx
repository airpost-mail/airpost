import { Forward, PenLine, Reply, ReplyAll } from 'lucide-react';
import { useCallback } from 'react';
import { MessageBody } from '@/components/email';
import { Badge } from '@/components/ui/badge';
import { Skeleton } from '@/components/ui/skeleton';
import {
  contactDisplayName,
  contactInitials,
  useEngine,
  type ContactRef,
  type MessageJSON,
} from '@/engine';
import { formatFullDate, formatListDate } from '@/lib/format';
import { cn } from '@/lib/utils';
import { ActionButton } from '../ActionButton';
import { AttachmentList } from './AttachmentList';
import { listedAttachments } from './attachments';
import { compose } from './compose';

function Initials({ contact }: { contact?: ContactRef }) {
  return (
    <span
      aria-hidden
      className="grid size-7 shrink-0 place-items-center rounded-full bg-muted font-mono text-2xs text-muted-foreground"
    >
      {contact ? contactInitials(contact) : '?'}
    </span>
  );
}

function names(list: ContactRef[], own: ReadonlySet<string>) {
  return list
    .map((c) => (own.has(c.email.toLowerCase()) ? 'me' : contactDisplayName(c)))
    .join(', ');
}

export function MessageItem({
  message: m,
  expanded,
  onToggle,
  own,
}: {
  message: MessageJSON;
  expanded: boolean;
  onToggle: () => void;
  own: ReadonlySet<string>;
}) {
  const engine = useEngine();
  const from = m.from[0];
  const resolveCid = useCallback(
    (cid: string) => {
      const file = m.files.find((f) => f.contentId === cid);
      return file ? engine.fileUrl(file) : null;
    },
    [m.files, engine],
  );

  if (!expanded) {
    return (
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={false}
        className="flex w-full items-center gap-3 border-b border-hairline px-6 py-2.5 text-left text-sm transition-colors duration-(--duration-fast) hover:bg-row-hover"
      >
        <Initials contact={from} />
        <span className={cn('w-40 shrink-0 truncate', m.unread && 'font-semibold')}>
          {from ? contactDisplayName(from) : '(unknown sender)'}
        </span>
        {m.draft && <Badge variant="destructive">Draft</Badge>}
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{m.snippet}</span>
        <time className="shrink-0 font-mono text-2xs text-muted-foreground tabular">
          {formatListDate(m.date)}
        </time>
      </button>
    );
  }

  const recipients = names([...m.to, ...m.cc], own);
  return (
    <article
      aria-label={`Message from ${from ? contactDisplayName(from) : 'unknown sender'}`}
      className="flex flex-col gap-4 border-b border-hairline px-6 pt-4 pb-6"
    >
      <header className="flex items-start gap-3">
        <Initials contact={from} />
        <button
          type="button"
          onClick={onToggle}
          aria-expanded
          className="flex min-w-0 flex-1 flex-col items-start text-left"
        >
          <span className="flex max-w-full items-baseline gap-2 text-sm">
            <span className="truncate font-semibold">
              {from ? contactDisplayName(from) : '(unknown sender)'}
            </span>
            {from?.name && (
              <span className="truncate text-xs text-muted-foreground">{from.email}</span>
            )}
            {m.draft && <Badge variant="destructive">Draft</Badge>}
          </span>
          {recipients && (
            <span className="max-w-full truncate text-xs text-muted-foreground">
              to {recipients}
            </span>
          )}
        </button>
        <time
          dateTime={new Date(m.date * 1000).toISOString()}
          className="mt-0.5 shrink-0 font-mono text-2xs text-muted-foreground tabular"
        >
          {formatFullDate(m.date)}
        </time>
        <div className="-mt-1 flex shrink-0 items-center">
          {m.draft ? (
            <ActionButton icon={PenLine} label="Edit draft" onClick={() => compose('reply', m)} />
          ) : (
            <>
              <ActionButton icon={Reply} label="Reply" onClick={() => compose('reply', m)} />
              <ActionButton
                icon={ReplyAll}
                label="Reply all"
                onClick={() => compose('reply-all', m)}
              />
              <ActionButton icon={Forward} label="Forward" onClick={() => compose('forward', m)} />
            </>
          )}
        </div>
      </header>
      <div className="pl-10">
        {m.body == null ? (
          <div aria-busy aria-label="Downloading message" className="flex flex-col gap-2 py-1">
            <Skeleton className="h-3 w-11/12" />
            <Skeleton className="h-3 w-4/5" />
            <Skeleton className="h-3 w-2/3" />
          </div>
        ) : m.body.trim() === '' ? (
          <p className="text-sm text-muted-foreground">This message has no text.</p>
        ) : (
          <MessageBody body={m.body} plaintext={m.plaintext} resolveCid={resolveCid} />
        )}
      </div>
      {m.files.length > 0 && (
        <div className="pl-10">
          <AttachmentList files={listedAttachments(m)} />
        </div>
      )}
    </article>
  );
}
