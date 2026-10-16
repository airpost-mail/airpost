// Owned by the mailbox feature: `/` goes to the first account's inbox.
import { createFileRoute, redirect } from '@tanstack/react-router';
import { EmptyState } from '@/components/EmptyState';
import { resolveDefaultMailbox } from '@/features/mailbox';

export const Route = createFileRoute('/_app/')({
  beforeLoad: async ({ context }) => {
    const target = await resolveDefaultMailbox(context.queryClient, context.engine);
    if (target) throw redirect({ to: '/$accountId/$folderId', params: target });
  },
  component: () => <EmptyState title="No accounts" description="Add an account in settings." />,
});
