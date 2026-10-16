// Owned by the mailbox feature: thread list plus the reading pane outlet.
import { createFileRoute, Outlet, useParams } from '@tanstack/react-router';
import { MailPanes } from '@/components/MailPanes';
import { ThreadListView } from '@/features/mailbox';

export const Route = createFileRoute('/_app/$accountId/$folderId')({
  component: FolderRoute,
});

function FolderRoute() {
  const { accountId, folderId } = Route.useParams();
  const threadId = useParams({ strict: false, select: (p) => p.threadId });
  return (
    <MailPanes
      open={!!threadId}
      list={<ThreadListView accountId={accountId} folderId={folderId} />}
      reader={<Outlet />}
    />
  );
}
