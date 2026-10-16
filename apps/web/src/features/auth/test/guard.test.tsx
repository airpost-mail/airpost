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
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { MockEngine } from '@/engine';
import { EngineContext } from '@/engine/context';
import { LoginView } from '../LoginView';
import { safeRedirect } from '../server-url';
import { SessionGuard } from '../SessionGuard';
import { SessionMenu } from '../SessionMenu';
import { getSession, redirectIfSignedIn, requireSession, signIn, signOut } from '../session';

function renderApp(path: string) {
  const root = createRootRoute({ component: Outlet });
  const login = createRoute({
    getParentRoute: () => root,
    path: '/login',
    validateSearch: (s: Record<string, unknown>): { redirect?: string } => ({
      redirect: safeRedirect(s.redirect),
    }),
    beforeLoad: ({ search }) => redirectIfSignedIn({ search }),
    component: function Login() {
      return <LoginView redirectTo={login.useSearch().redirect} />;
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
    routeTree: root.addChildren([login, app.addChildren([home, inbox])]),
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

async function fillAndSubmit(email: string) {
  const user = userEvent.setup();
  await user.type(await screen.findByLabelText('Email'), email);
  await user.type(screen.getByLabelText('Password', { selector: 'input' }), 'anything');
  const button = screen.getByRole('button', { name: 'Sign in' });
  await waitFor(() => expect(button).toBeEnabled());
  await user.click(button);
}

describe('route guard (mock engine)', () => {
  beforeEach(async () => {
    await signIn({
      method: 'password',
      serverUrl: 'http://localhost:3000',
      email: 'maya.okafor@gmail.com',
      password: '',
    });
  });

  it('lets a signed-in user in and keeps them off /login', async () => {
    const router = renderApp('/login?redirect=%2Finbox');
    expect(await screen.findByText('Inbox')).toBeInTheDocument();
    expect(router.state.location.pathname).toBe('/inbox');
  });

  it('sends a signed-out user to /login and back to where they were going', async () => {
    await signOut();
    const router = renderApp('/inbox?x=1');

    expect(await screen.findByRole('heading', { name: 'Sign in' })).toBeInTheDocument();
    expect(router.state.location.search).toEqual({ redirect: '/inbox?x=1' });

    await fillAndSubmit('sam@example.org');

    expect(await screen.findByText('Inbox')).toBeInTheDocument();
    expect(router.state.location.href).toBe('/inbox?x=1');
    expect(getSession()?.user.email).toBe('sam@example.org');
  });

  it('signs out from the session menu', async () => {
    const user = userEvent.setup();
    const router = renderApp('/inbox');
    await user.click(await screen.findByRole('button', { name: /signed in as/i }));
    await user.click(await screen.findByRole('menuitem', { name: 'Sign out' }));

    expect(await screen.findByText('Signed out.')).toBeInTheDocument();
    expect(router.state.location.pathname).toBe('/login');
    expect(getSession()).toBeNull();
    expect(localStorage.getItem('tern.session')).not.toContain('tern_mock_');
  });

  it('follows a session that ends while the page is open', async () => {
    const router = renderApp('/inbox');
    await screen.findByText('Inbox');

    await act(() => signOut('revoked'));

    expect(await screen.findByText(/The server signed you out/)).toBeInTheDocument();
    expect(router.state.location.search).toEqual({ redirect: '/inbox' });
  });
});
