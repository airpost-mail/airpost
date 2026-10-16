// Owned by the settings feature.
import { createFileRoute } from '@tanstack/react-router';
import { AddAccountView } from '@/features/settings';

interface Search {
  /** Preset id (`gmail`, `imap`, ...) to skip the provider step. */
  provider?: string;
}

export const Route = createFileRoute('/_app/settings/accounts/new')({
  validateSearch: (s: Record<string, unknown>): Search =>
    typeof s.provider === 'string' ? { provider: s.provider } : {},
  component: NewAccountRoute,
});

function NewAccountRoute() {
  const { provider } = Route.useSearch();
  return <AddAccountView provider={provider} />;
}
