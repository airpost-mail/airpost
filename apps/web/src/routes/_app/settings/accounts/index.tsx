// Owned by the settings feature.
import { createFileRoute } from '@tanstack/react-router';
import { AccountsSettings } from '@/features/settings';

export const Route = createFileRoute('/_app/settings/accounts/')({
  component: AccountsSettings,
});
