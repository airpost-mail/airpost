// Owned by the mailbox feature: the open thread. In unified views (`/all/...`) the
// thread's own account travels in `?account=`.
import { createFileRoute } from '@tanstack/react-router';
import { ThreadReaderView } from '@/features/mailbox';

interface ThreadSearch {
  account?: string;
}

export const Route = createFileRoute('/_app/$accountId/$folderId/$threadId')({
  validateSearch: (search: Record<string, unknown>): ThreadSearch =>
    typeof search.account === 'string' && search.account ? { account: search.account } : {},
  component: ThreadRoute,
});

function ThreadRoute() {
  const { accountId, threadId } = Route.useParams();
  const { account } = Route.useSearch();
  const owner = account ?? accountId;
  return <ThreadReaderView key={`${owner}:${threadId}`} accountId={owner} threadId={threadId} />;
}
