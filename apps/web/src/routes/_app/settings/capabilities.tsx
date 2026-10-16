// Owned by the settings feature.
import { createFileRoute } from '@tanstack/react-router';
import { CapabilitiesSettings } from '@/features/settings';

export const Route = createFileRoute('/_app/settings/capabilities')({
  component: CapabilitiesSettings,
});
