// Owned by the settings feature (with everything under settings/).
import { createFileRoute } from '@tanstack/react-router';
import { SettingsLayout } from '@/features/settings';

export const Route = createFileRoute('/_app/settings')({
  component: SettingsLayout,
});
