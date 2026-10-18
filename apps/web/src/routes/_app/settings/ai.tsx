// Owned by the ai feature.
import { createFileRoute } from '@tanstack/react-router';
import { AiSettingsView } from '@/features/ai';

export const Route = createFileRoute('/_app/settings/ai')({
  component: AiSettingsView,
});
