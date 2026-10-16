// Owned by the settings feature.
import { createFileRoute } from '@tanstack/react-router';
import { SignaturesSettings } from '@/features/settings';

export const Route = createFileRoute('/_app/settings/signatures')({
  component: SignaturesSettings,
});
