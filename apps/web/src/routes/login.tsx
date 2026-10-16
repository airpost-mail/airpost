// Owned by the auth feature.
import { createFileRoute } from '@tanstack/react-router';
import { LoginView, redirectIfSignedIn, safeRedirect } from '@/features/auth';

export const Route = createFileRoute('/login')({
  validateSearch: (s: Record<string, unknown>): { redirect?: string } => ({
    redirect: safeRedirect(s.redirect),
  }),
  beforeLoad: ({ search }) => redirectIfSignedIn({ search }),
  component: LoginRoute,
});

function LoginRoute() {
  const { redirect } = Route.useSearch();
  return <LoginView redirectTo={redirect} />;
}
