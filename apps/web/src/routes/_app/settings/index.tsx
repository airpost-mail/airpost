// Owned by the settings feature.
import { createFileRoute } from '@tanstack/react-router';
import { SettingsIndex } from '@/features/settings';

export const Route = createFileRoute('/_app/settings/')({
  component: SettingsIndex,
});
