import type { QueryClient } from '@tanstack/react-query';
import { createRouter } from '@tanstack/react-router';
import type { EngineClient } from '@/engine';
import { routeTree } from './routeTree.gen';

export interface RouterContext {
  queryClient: QueryClient;
  engine: EngineClient;
}

export function createAppRouter(context: RouterContext) {
  return createRouter({
    routeTree,
    context,
    defaultPreload: 'intent',
    // Query owns caching; the router should always ask it.
    defaultPreloadStaleTime: 0,
    scrollRestoration: true,
  });
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}
