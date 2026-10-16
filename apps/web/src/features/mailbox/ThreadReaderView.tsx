import { useNavigate, useParams, useRouter } from '@tanstack/react-router';
import { useMemo } from 'react';
import { EmptyState } from '@/components/EmptyState';
import { Skeleton } from '@/components/ui/skeleton';
import {
  isGmailAccount,
  useAccount,
  useAccounts,
  useCategories,
  useMessages,
  useThread,
  type LabelJSON,
} from '@/engine';
import { useShortcut } from '@/lib/shortcuts';
import { useThreadActions } from './actions';
import { LabelChip } from './LabelChip';
import { visibleLabels } from './labels';
import { removeThreads } from './list-bus';
import { ownAddresses, viewCategoryFor, resolveMailbox } from './mailbox';
import { openCategoryPicker } from './picker-store';
import { compose, replyTarget } from './reader/compose';
import { useExpansion } from './reader/expansion';
import { MessageList } from './reader/MessageList';
import { ReaderToolbar } from './reader/ReaderToolbar';
import { useMarkReadOnOpen } from './reader/use-mark-read';

function ReaderSkeleton() {
  return (
    <div aria-busy aria-label="Loading thread" className="flex flex-col gap-4 px-6 pt-6">
      <Skeleton className="h-6 w-2/3" />
      <Skeleton className="h-3 w-1/4" />
      <div className="mt-4 flex flex-col gap-2">
        <Skeleton className="h-3 w-full" />
        <Skeleton className="h-3 w-11/12" />
        <Skeleton className="h-3 w-3/4" />
      </div>
    </div>
  );
}

export function ThreadReaderView({ accountId, threadId }: { accountId: string; threadId: string }) {
  const { data: thread, isLoading } = useThread(accountId, threadId);
  const { data: messages } = useMessages(accountId, threadId);
  const { data: accounts } = useAccounts();
  const account = useAccount(accountId);
  const { all, labels } = useCategories(accountId);
  const actions = useThreadActions();
  const navigate = useNavigate();
  const router = useRouter();
  const route = useParams({ strict: false });
  const expansion = useExpansion(messages);
  useMarkReadOnOpen(thread);

  const own = useMemo(() => ownAddresses(accounts ?? []), [accounts]);
  const gmail = !!account && isGmailAccount(account);
  const viewing = useMemo(() => {
    if (!route.accountId || !route.folderId) return undefined;
    const box = resolveMailbox(route.accountId, route.folderId, all);
    return box ? viewCategoryFor(box, accountId, all) : undefined;
  }, [route.accountId, route.folderId, all, accountId]);

  const reply = (mode: 'reply' | 'reply-all' | 'forward') => {
    const m = messages && replyTarget(messages);
    if (m) compose(mode, m);
  };
  const group = { group: 'Compose' } as const;
  useShortcut('r', () => reply('reply'), { ...group, description: 'Reply' });
  useShortcut('a', () => reply('reply-all'), { ...group, description: 'Reply all' });
  useShortcut('f', () => reply('forward'), { ...group, description: 'Forward' });

  if (thread === null) {
    return <EmptyState title="Gone" description="This thread no longer exists." />;
  }
  if (!thread) {
    return isLoading ? <ReaderSkeleton /> : null;
  }

  const back = () => {
    if (route.accountId && route.folderId) {
      void navigate({
        to: '/$accountId/$folderId',
        params: { accountId: route.accountId, folderId: route.folderId },
      });
    } else {
      router.history.back();
    }
  };
  const byId = new Map(labels.map((l) => [l.id, l]));
  const chips = visibleLabels(thread.labels)
    .map((ref) => byId.get(ref.id))
    .filter((l): l is LabelJSON => !!l);
  const starred = thread.starred > 0;

  return (
    <div className="flex h-full flex-col">
      <ReaderToolbar
        onBack={back}
        onArchive={() => removeThreads('archive', [thread], actions)}
        onTrash={() => removeThreads('trash', [thread], actions)}
        onSpam={() => removeThreads('spam', [thread], actions)}
        onMarkUnread={() => {
          actions.setUnread([thread], true);
          back();
        }}
        onStar={() => actions.setStarred([thread], !starred)}
        starred={starred}
        onMove={() => openCategoryPicker({ kind: 'move', threads: [thread], viewing })}
        onLabel={
          gmail
            ? () => openCategoryPicker({ kind: 'label', threads: [thread], viewing })
            : undefined
        }
        allExpanded={expansion.allExpanded}
        onToggleExpanded={() => expansion.setAll(!expansion.allExpanded)}
        messageCount={messages?.length ?? 0}
      />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-4xl pb-16">
          <header className="flex flex-col gap-2 px-6 pt-6 pb-4">
            <h1 className="text-xl font-semibold tracking-tight text-balance">
              {thread.subject || '(no subject)'}
            </h1>
            {chips.length > 0 && (
              <div className="flex flex-wrap gap-1.5" aria-label="Labels">
                {chips.map((l) => (
                  <LabelChip
                    key={l.id}
                    label={l}
                    onRemove={() => actions.removeLabel([thread], l)}
                  />
                ))}
              </div>
            )}
          </header>
          {messages ? (
            <MessageList
              messages={messages}
              own={own}
              isExpanded={expansion.isExpanded}
              onToggle={expansion.toggle}
              onExpand={expansion.expand}
            />
          ) : (
            <ReaderSkeleton />
          )}
        </div>
      </div>
    </div>
  );
}
