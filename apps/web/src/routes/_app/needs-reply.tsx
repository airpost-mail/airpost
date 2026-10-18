// Owned by the ai feature: threads triage says need a reply. The reader comes from the
// mailbox feature, so features/ai doesn't import it.
import { createFileRoute } from '@tanstack/react-router';
import { NeedsReplyView } from '@/features/ai';
import { ReaderEmpty, ThreadReaderView } from '@/features/mailbox';

interface NeedsReplySearch {
  account?: string;
  thread?: string;
  threadAccount?: string;
}

const str = (v: unknown) => (typeof v === 'string' && v ? v : undefined);

export const Route = createFileRoute('/_app/needs-reply')({
  validateSearch: (search: Record<string, unknown>): NeedsReplySearch => ({
    account: str(search.account),
    thread: str(search.thread),
    threadAccount: str(search.threadAccount),
  }),
  component: NeedsReplyRoute,
});

function NeedsReplyRoute() {
  const { account, thread, threadAccount } = Route.useSearch();
  const open = thread && threadAccount ? { accountId: threadAccount, threadId: thread } : null;
  return (
    <NeedsReplyView
      accountId={account}
      openThread={open}
      reader={
        open ? (
          <ThreadReaderView
            key={`${open.accountId}:${open.threadId}`}
            accountId={open.accountId}
            threadId={open.threadId}
          />
        ) : (
          <ReaderEmpty />
        )
      }
    />
  );
}
