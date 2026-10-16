// Owned by the settings feature.
import { createFileRoute } from '@tanstack/react-router';
import { ShortcutsSettings } from '@/features/settings';

export const Route = createFileRoute('/_app/settings/shortcuts')({
  component: ShortcutsSettings,
});
