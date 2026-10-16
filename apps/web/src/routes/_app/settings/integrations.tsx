// Owned by the settings feature.
import { createFileRoute } from '@tanstack/react-router';
import { IntegrationsSettings } from '@/features/settings';

export const Route = createFileRoute('/_app/settings/integrations')({
  component: IntegrationsSettings,
});
