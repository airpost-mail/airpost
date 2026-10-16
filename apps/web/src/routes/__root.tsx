import { createRootRouteWithContext, Link, Outlet } from '@tanstack/react-router';
import { EmptyState } from '@/components/EmptyState';
import { Toaster } from '@/components/ui/sonner';
import type { RouterContext } from '@/router';

export const Route = createRootRouteWithContext<RouterContext>()({
  component: () => (
    <>
      <Outlet />
      <Toaster position="bottom-left" />
    </>
  ),
  notFoundComponent: () => (
    <EmptyState
      className="h-dvh"
      title="Off the chart"
      description="There is nothing at this address."
      action={
        <Link to="/" className="text-sm text-primary underline-offset-4 hover:underline">
          Back to the inbox
        </Link>
      }
    />
  ),
});
