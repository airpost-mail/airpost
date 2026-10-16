// Owned by the auth feature.
import { createFileRoute } from '@tanstack/react-router';
import { PairView, redirectIfPaired, safeRedirect } from '@/features/auth';

export const Route = createFileRoute('/pair')({
  validateSearch: (s: Record<string, unknown>): { redirect?: string } => ({
    redirect: safeRedirect(s.redirect),
  }),
  beforeLoad: ({ search }) => redirectIfPaired({ search }),
  component: PairRoute,
});

function PairRoute() {
  const { redirect } = Route.useSearch();
  return <PairView redirectTo={redirect} />;
}
