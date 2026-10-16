// Owned by the mailbox feature: reading pane with no thread selected.
import { createFileRoute } from '@tanstack/react-router';
import { ReaderEmpty } from '@/features/mailbox';

export const Route = createFileRoute('/_app/$accountId/$folderId/')({
  component: ReaderEmpty,
});
