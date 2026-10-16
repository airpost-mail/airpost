import type { QueryClient } from '@tanstack/react-query';
import {
  accountsQuery,
  ensureConnected,
  foldersQuery,
  labelsQuery,
  type CategoryJSON,
  type EngineClient,
} from '@/engine';

/** The first account's inbox, for redirecting `/`. Warms the same caches the views use. */
export async function resolveDefaultMailbox(
  queryClient: QueryClient,
  engine: EngineClient,
): Promise<{ accountId: string; folderId: string } | null> {
  await ensureConnected(engine);
  const accounts = await queryClient.ensureQueryData(accountsQuery(engine));
  const account = accounts[0];
  if (!account) return null;
  const [folders, labels] = await Promise.all([
    queryClient.ensureQueryData(foldersQuery(engine, account.id)),
    queryClient.ensureQueryData(labelsQuery(engine, account.id)),
  ]);
  const all: CategoryJSON[] = [...labels.labels, ...folders.folders];
  const inbox = all.find((c) => c.role === 'inbox') ?? all[0];
  return inbox ? { accountId: account.id, folderId: inbox.id } : null;
}
