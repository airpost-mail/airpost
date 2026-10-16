// The route guard end to end, against a small route tree shaped like the app's.
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router';
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { MockEngine } from '@/engine';
import { EngineContext } from '@/engine/context';
import { PairView } from '../PairView';
import { safeRedirect } from '../redirect';
import {
  getSession,
  markUnpaired,
  pair,
  redirectIfPaired,
  requireSession,
  signOut,
} from '../session';
import { SessionGuard } from '../SessionGuard';
import { SessionMenu } from '../SessionMenu';

function renderApp(path: string) {
  const root = createRootRoute({ component: Outlet });
  const pairRoute = createRoute({
    getParentRoute: () => root,
    path: '/pair',
    validateSearch: (s: Record<string, unknown>): { redirect?: string } => ({
      redirect: safeRedirect(s.redirect),
    }),
    beforeLoad: ({ search }) => redirectIfPaired({ search }),
    component: function Pair() {
      return <PairView redirectTo={pairRoute.useSearch().redirect} />;
    },
  });
  const app = createRoute({
    getParentRoute: () => root,
    id: '_app',
    beforeLoad: ({ location }) => requireSession({ location }),
    component: () => (
      <>
        <SessionGuard />
        <SessionMenu />
        <Outlet />
      </>
    ),
  });
  const home = createRoute({ getParentRoute: () => app, path: '/', component: () => <p>Home</p> });
  const inbox = createRoute({
    getParentRoute: () => app,
    path: '/inbox',
    component: () => <p>Inbox</p>,
  });
  const router = createRouter({
    routeTree: root.addChildren([pairRoute, app.addChildren([home, inbox])]),
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <EngineContext.Provider value={new MockEngine({ latency: 0 })}>
        <RouterProvider router={router} />
      </EngineContext.Provider>
    </QueryClientProvider>,
  );
  return router;
}

describe('route guard (mock gateway)', () => {
  beforeEach(async () => {
    await pair('ABCD-EFGH-JKLM', 'Laptop');
  });

  it('lets a paired browser in and keeps it off /pair', async () => {
    const router = renderApp('/pair?redirect=%2Finbox');
    expect(await screen.findByText('Inbox')).toBeInTheDocument();
    expect(router.state.location.pathname).toBe('/inbox');
  });

  it('sends an unpaired browser to /pair and back to where it was going', async () => {
    await signOut();
    const router = renderApp('/inbox?x=1');

    expect(await screen.findByRole('heading', { name: 'Pair this browser' })).toBeInTheDocument();
    expect(router.state.location.search).toEqual({ redirect: '/inbox?x=1' });

    const user = userEvent.setup();
    await user.type(screen.getByLabelText('Pairing code'), 'wxyz-2345-abcd');
    await user.clear(screen.getByLabelText('Name for this browser'));
    await user.type(screen.getByLabelText('Name for this browser'), 'Work laptop');
    await user.click(screen.getByRole('button', { name: 'Pair' }));

    expect(await screen.findByText('Inbox')).toBeInTheDocument();
    expect(router.state.location.href).toBe('/inbox?x=1');
    expect(getSession()?.device.name).toBe('Work laptop');
  });

  it('shows why a malformed code is refused', async () => {
    await signOut();
    renderApp('/pair');
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText('Pairing code'), 'abc');
    await user.click(screen.getByRole('button', { name: 'Pair' }));
    expect(await screen.findByText(/12 letters and digits/)).toBeInTheDocument();
  });

  it('signs out from the session menu', async () => {
    const user = userEvent.setup();
    const router = renderApp('/inbox');
    await user.click(await screen.findByRole('button', { name: /paired as laptop/i }));
    await user.click(await screen.findByRole('menuitem', { name: 'Sign out' }));

    expect(await screen.findByText('Signed out.')).toBeInTheDocument();
    expect(router.state.location.pathname).toBe('/pair');
    expect(getSession()).toBeNull();
  });

  it('follows a pairing the gateway ends while the page is open', async () => {
    const router = renderApp('/inbox');
    await screen.findByText('Inbox');

    act(() => markUnpaired('revoked'));

    expect(await screen.findByText(/This browser was unpaired/)).toBeInTheDocument();
    expect(router.state.location.search).toEqual({ redirect: '/inbox' });
  });
});

describe('PairView', () => {
  it('takes the code from the link and clears it from the address bar', async () => {
    await signOut();
    window.history.replaceState(null, '', '/pair#ABCD-EFGH-JKLM');
    renderApp('/pair');
    expect(await screen.findByLabelText('Pairing code')).toHaveValue('ABCD-EFGH-JKLM');
    expect(window.location.hash).toBe('');
    expect(screen.getByText('Check the name, then pair.')).toBeInTheDocument();
  });
});
